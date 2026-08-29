// AI decisions: pitch selection and batter swing planning.

use crate::util::{gauss, frand, chance, clampf};
use crate::data::{PitchDef, AiPitcher, AiBatter, BatterCfg, GameCfg};
use crate::physics::PitchFlight;

// ---------------------------------------------------------------------------
// Count state index: 0=even 1=ahead(pitcher) 2=behind 3=two_strikes
// ---------------------------------------------------------------------------

fn count_state(balls: u8, strikes: u8) -> usize {
    if strikes == 2 { return 3; }
    match (balls, strikes) {
        (0, 1) | (0, 2) | (1, 2) => 1,  // pitcher ahead
        (1, 0) | (2, 0) | (3, 0) |
        (2, 1) | (3, 1)           => 2,  // batter ahead
        _                          => 0,  // even
    }
}

// ---------------------------------------------------------------------------
// AI pitcher
// ---------------------------------------------------------------------------

/// Returns (pitch_index, intended_aim_x, intended_aim_z).
/// Command noise (gaussian with PitcherCfg::command_sigma) is added in main.
pub fn ai_select_pitch(
    pitches: &[PitchDef],
    ai: &AiPitcher,
    balls: u8,
    strikes: u8,
    game: &GameCfg,
) -> (usize, f32, f32) {
    let state = count_state(balls, strikes);
    let weights = &ai.weights[state];
    let zone_rate = ai.zone_rates[state];

    // Weighted random pitch selection
    let total: f32 = weights.iter().map(|(_, w)| w).sum();
    let mut pick = frand(0.0, total.max(0.001));
    let mut pitch_key = String::new();
    for (key, w) in weights {
        pick -= w;
        if pick <= 0.0 { pitch_key = key.clone(); break; }
    }
    if pitch_key.is_empty() {
        pitch_key = weights.last().map(|(k, _)| k.clone()).unwrap_or_default();
    }

    let idx = pitches.iter().position(|p| p.key == pitch_key).unwrap_or(0);

    // Aim location
    let (aim_x, aim_z) = if chance(zone_rate) {
        // In-zone: bias toward lower half
        let x = frand(-game.zone_half_w * 0.7, game.zone_half_w * 0.7);
        let z = frand(game.zone_bottom, game.zone_bottom + (game.zone_top - game.zone_bottom) * 0.55);
        (x, z)
    } else {
        // Out of zone: edge or chase location
        let side = if chance(0.5) { 1.0f32 } else { -1.0 };
        let x = side * frand(game.zone_half_w + 0.1, game.zone_half_w + 0.6);
        let z = frand(game.zone_bottom - 0.4, game.zone_top + 0.25);
        (x, z)
    };

    (idx, aim_x, aim_z)
}

// ---------------------------------------------------------------------------
// AI batter
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct SwingPlan {
    pub will_swing: bool,
    pub swing_t: f32,   // absolute flight time to swing at
    pub pci_x: f32,
    pub pci_z: f32,
}

/// AI batter decides whether and when to swing given full knowledge of the pitch.
/// `recognition_sigma` noise is added to perceived plate location before deciding.
pub fn ai_swing_decision(
    flight: &PitchFlight,
    ai: &AiBatter,
    _batter: &BatterCfg,
    game: &GameCfg,
    _balls: u8,
    strikes: u8,
) -> SwingPlan {
    // Perceived plate crossing location (noisy)
    let perc_x = flight.plate_x + gauss(ai.recognition_sigma);
    let perc_z = flight.plate_z + gauss(ai.recognition_sigma);

    // Zone check with optional 2-strike expansion
    let expand = if strikes == 2 { ai.two_strike_zone_expand } else { 0.0 };
    let hz = game.zone_half_w + expand;
    let zb = game.zone_bottom - expand;
    let zt = game.zone_top + expand;
    let perceived_in_zone = perc_x.abs() <= hz && perc_z >= zb && perc_z <= zt;

    // Swing probability
    let will_swing = if perceived_in_zone {
        let p = if strikes == 2 { ai.zone_swing_two_strikes } else { ai.zone_swing };
        chance(p)
    } else {
        // Chase: probability decays with distance from zone
        let dx = (perc_x.abs() - game.zone_half_w).max(0.0);
        let dz = ((perc_z - game.zone_bottom).min(0.0).min(game.zone_top - perc_z)).abs();
        let dist = (dx * dx + dz * dz).sqrt();
        let p_chase = ai.chase * (-dist / ai.chase_falloff).exp();
        chance(p_chase)
    };

    if !will_swing {
        return SwingPlan { will_swing: false, swing_t: 0.0, pci_x: 0.0, pci_z: 0.0 };
    }

    // Plan swing timing: ideal is plate_t; add Gaussian noise
    let swing_t = clampf(
        flight.plate_t + gauss(ai.timing_sigma),
        flight.plate_t - 0.12,
        flight.plate_t + 0.12,
    );

    // Plan PCI: aim at perceived location with noise
    let pci_x = perc_x + gauss(ai.pci_sigma);
    let pci_z = perc_z + gauss(ai.pci_sigma);

    SwingPlan { will_swing: true, swing_t, pci_x, pci_z }
}
