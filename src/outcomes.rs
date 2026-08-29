// Swing resolution: compute contact quality, launch parameters, classify BIP outcome.

use crate::util::{gauss, chance, clampf, lerpf};
use crate::data::{BatterCfg, GameCfg, PhysicsCfg};
use crate::physics::{PitchFlight, BipFlight, simulate_bip};

// ---------------------------------------------------------------------------
// Outcome types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum PitchOutcome {
    Ball,
    CalledStrike,
    SwingingStrike,
    FoulTip,
    Foul,
    FoulBack,   // straight-back popup / foul popup behind plate
    InPlay,
}

#[derive(Debug, Clone)]
pub enum HitType { Single, Double, Triple, HomeRun }

#[derive(Debug, Clone)]
pub enum BipType { Groundout, Lineout, Flyout, Popup, Hit(HitType) }

/// (position name, honest/default x, honest/default y) — identifies which
/// fielder is responsible for a given batted ball, for both flavor text and
/// the on-field fielder animation.
pub type FielderInfo = (&'static str, f32, f32);

#[derive(Debug, Clone)]
pub struct SwingResult {
    pub outcome: PitchOutcome,
    pub bip: Option<BipFlight>,
    pub bip_type: Option<BipType>,
    pub fielder: Option<FielderInfo>,
    pub ev: f32,
    pub la: f32,
    pub spray: f32,
    pub description: String,
}

impl Default for SwingResult {
    fn default() -> Self {
        SwingResult {
            outcome: PitchOutcome::Ball,
            bip: None, bip_type: None, fielder: None,
            ev: 0.0, la: 0.0, spray: 0.0,
            description: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Zone / ball checker
// ---------------------------------------------------------------------------

/// ABS/MLB "robo-ump" style: a pitch is a strike if any part of the ball
/// touches the zone, not just its center. The configured zone dimensions
/// (game.zone_half_w/bottom/top) are the real rulebook numbers and are never
/// altered — instead the comparison bounds are expanded by one ball radius,
/// which is mathematically equivalent to "ball edge overlaps the true zone".
pub fn in_zone(px: f32, pz: f32, game: &GameCfg, ball_radius: f32) -> bool {
    px.abs() <= game.zone_half_w + ball_radius
        && pz >= game.zone_bottom - ball_radius
        && pz <= game.zone_top + ball_radius
}

// ---------------------------------------------------------------------------
// Reach penalty: power multiplier based on how far the PCI (aim point) sits
// outside the strike zone box. 1.0 = fully inside the zone, shrinks toward
// (1.0 - reach_penalty_cap) the further out the batter reaches.
// ---------------------------------------------------------------------------

fn reach_penalty_mult(pci_x: f32, pci_z: f32, game: &GameCfg) -> f32 {
    let dx = (pci_x.abs() - game.zone_half_w).max(0.0);
    let dz_lo = (game.zone_bottom - pci_z).max(0.0);
    let dz_hi = (pci_z - game.zone_top).max(0.0);
    let dz = dz_lo.max(dz_hi);
    let d = (dx * dx + dz * dz).sqrt();
    let reduction = clampf(d * game.reach_penalty_per_ft, 0.0, game.reach_penalty_cap);
    1.0 - reduction
}

// ---------------------------------------------------------------------------
// Batter contact-style trait multiplier.
//   nz    : normalised vertical PCI offset (+ = swung under a high pitch → up)
//   t_off : timing error, seconds (- = early/pull, + = late/push for a RHB)
// ---------------------------------------------------------------------------

fn trait_mult(nz: f32, t_off: f32, timing_full: f32, batter: &BatterCfg) -> f32 {
    let mut m = 1.0;
    if batter.trait_vert_sign != 0.0 {
        let v = clampf(nz / 1.5, -1.0, 1.0);
        m += batter.trait_bonus * batter.trait_vert_sign * v;
    }
    if batter.trait_horiz_sign != 0.0 {
        let h = clampf(t_off / timing_full, -1.0, 1.0);
        m += batter.trait_bonus * batter.trait_horiz_sign * h;
    }
    clampf(m, 0.5, 1.5)
}

// ---------------------------------------------------------------------------
// Main resolver
// ---------------------------------------------------------------------------

/// Resolve a pitch.
/// `swing_t` = None → batter took (no swing); Some(t) → batter swung at time t seconds.
/// `pci_x/z` = where batter aimed (world space at plate plane). Only used if swing_t is Some.
pub fn resolve(
    flight: &PitchFlight,
    swing_t: Option<f32>,
    pci_x: f32,
    pci_z: f32,
    batter: &BatterCfg,
    game: &GameCfg,
    phys: &PhysicsCfg,
) -> SwingResult {
    let mut out = SwingResult::default();

    // --- No swing ---
    if swing_t.is_none() {
        if in_zone(flight.plate_x, flight.plate_z, game, phys.ball_radius) {
            out.outcome = PitchOutcome::CalledStrike;
            out.description = "Called Strike".into();
        } else {
            out.outcome = PitchOutcome::Ball;
            out.description = "Ball".into();
        }
        return out;
    }

    let t_swing = swing_t.unwrap();

    // --- Swing contact model ---
    // Timing error: positive = late (swung after ball crossed plate)
    let t_off = t_swing - flight.plate_t;

    // Automatic whiff: timing too far off
    if t_off.abs() > game.timing_whiff {
        out.outcome = PitchOutcome::SwingingStrike;
        out.description = format!("Swing and miss ({:.0} ms {})",
            (t_off * 1000.0).abs(),
            if t_off > 0.0 { "late" } else { "early" });
        return out;
    }

    // Normalised PCI error
    let dx = flight.plate_x - pci_x;
    let dz = flight.plate_z - pci_z;
    let nx = dx / batter.pci_rx;
    let nz = dz / batter.pci_rz;
    let nerr = (nx * nx + nz * nz).sqrt();

    if nerr > game.pci_whiff {
        out.outcome = PitchOutcome::SwingingStrike;
        out.description = "Swing and miss (PCI)".into();
        return out;
    }

    // Foul tip zone (barely caught it)
    if nerr > game.foul_tip_band {
        if chance(0.60) {
            out.outcome = PitchOutcome::FoulTip;
            out.description = "Foul tip".into();
            return out;
        }
        // Otherwise continue as a foul ball
        out.outcome = PitchOutcome::Foul;
        out.description = "Foul ball".into();
        return out;
    }

    // --- Contact quality ---
    // This is primarily a TIMING game: timing (qt) drives most of the quality,
    // PCI aim precision (qp) only softly modulates it — even a fairly loose
    // PCI hit still keeps most of its quality if the timing was good.
    let qp = clampf(1.0 - nerr, 0.0, 1.0);
    let qt = clampf(1.0 - t_off.abs() / game.timing_full, 0.0, 1.0);
    let quality = qt * lerpf(0.55, 1.0, qp);

    let mut ev = lerpf(game.ev_min, batter.power_ev, quality);

    // Reach penalty: aiming (PCI) outside the strike zone gradually loses power,
    // even if contact is made — you can't drive a pitch you're reaching for.
    ev *= reach_penalty_mult(pci_x, pci_z, game);

    // Batter contact-style trait: rewards contact that matches this batter's
    // natural swing (up/down, pull/push), penalizes the opposite.
    ev *= trait_mult(nz, t_off, game.timing_full, batter);
    ev = clampf(ev, game.ev_min * 0.6, batter.power_ev * 1.15);

    // Launch angle (clamped to a sane physical range — extreme PCI misses at
    // the whiff boundary could otherwise produce unrealistic angles that
    // sent grounders into the dirt at contact or absurd "popups")
    let la = clampf(
        game.la_center + nz * game.la_per_vert + t_off * game.la_per_late + gauss(game.la_noise),
        -35.0, 80.0,
    );

    // Spray angle (degrees; + = RF, - = LF for RHB)
    let spray = batter.side_sign * (t_off * game.spray_per_late) + gauss(game.spray_noise);

    out.ev    = ev;
    out.la    = la;
    out.spray = spray;

    // Foul ball checks
    if spray.abs() > game.foul_angle {
        out.outcome = PitchOutcome::Foul;
        out.description = format!("Foul ball ({:.0}°)", spray);
        return out;
    }
    if la > 62.0 {
        out.outcome = PitchOutcome::FoulBack;
        out.description = "Foul popup".into();
        return out;
    }

    // --- In play ---
    out.outcome = PitchOutcome::InPlay;

    let backspin  = game.bat_backspin_base + game.bat_backspin_per_la  * la;
    let sidespin  = -spray * game.bat_sidespin_per_spray;
    let contact_z = clampf(flight.plate_z, 0.8, 4.0);

    let bip = simulate_bip(ev, la, spray, backspin, sidespin, contact_z, phys, game);

    let (bip_type, fielder) = classify_bip(&bip, la, ev, game);
    out.description = describe(&bip_type, &bip, ev, la, fielder);
    out.bip_type = Some(bip_type);
    out.fielder = fielder;
    out.bip = Some(bip);

    out
}

// ---------------------------------------------------------------------------
// BIP classification — fielder-proximity model
// ---------------------------------------------------------------------------
//
// Approximate "honest" (no-shift) fielder positions, in the same field
// coordinate system as batted-ball landing spots (feet from home plate;
// +x = RF side, +y = toward CF). A ball's landing ANGLE from home plate is
// compared against each fielder's angle: hit right at someone, it's likely
// an out; hit into a gap between two fielders, it's likely a hit. Harder
// contact (higher EV) and less hang time both cut into a fielder's
// effective range, since there's less time to react. `pub` so the renderer
// can draw the same fielders it's simulating against.

pub const INFIELD_POS: [FielderInfo; 4] = [
    ("1B", 48.0, 78.0),
    ("2B", 22.0, 130.0),
    ("SS", -25.0, 135.0),
    ("3B", -48.0, 78.0),
];
pub const OUTFIELD_POS: [FielderInfo; 3] = [
    ("LF", -110.0, 280.0),
    ("CF", 0.0, 320.0),
    ("RF", 110.0, 280.0),
];

/// Landing angle from home plate, in the same convention as `spray`
/// (0° = straight to CF, + = RF side, − = LF side).
fn landing_angle(bip: &BipFlight) -> f32 {
    bip.land_x.atan2(bip.land_y.max(0.001)).to_degrees()
}

/// Best (nearest-angle) defensive coverage at a given angle, 0..1.
fn nearest_coverage(angle_deg: f32, positions: &[FielderInfo], sigma: f32) -> f32 {
    positions.iter()
        .map(|&(_, fx, fy)| {
            let fielder_angle = fx.atan2(fy).to_degrees();
            let d = angle_deg - fielder_angle;
            (-(d * d) / (2.0 * sigma * sigma)).exp()
        })
        .fold(0.0f32, f32::max)
}

/// The single closest fielder by angle (for flavor text and the fielder
/// animation — same lookup, so the text and the dot always agree).
fn nearest_fielder(angle_deg: f32, positions: &[FielderInfo]) -> FielderInfo {
    positions.iter()
        .map(|&(name, fx, fy)| (name, fx, fy, (angle_deg - fx.atan2(fy).to_degrees()).abs()))
        .min_by(|a, b| a.3.partial_cmp(&b.3).unwrap())
        .map(|(name, fx, fy, _)| (name, fx, fy))
        .unwrap_or(("?", 0.0, 0.0))
}

pub fn classify_bip(bip: &BipFlight, la: f32, ev: f32, game: &GameCfg) -> (BipType, Option<FielderInfo>) {
    if bip.is_hr {
        return (BipType::Hit(HitType::HomeRun), None);
    }

    let dist = bip.distance;
    let angle = landing_angle(bip);

    // A routine infield popup is steep AND weakly hit / short — a well-struck
    // ball with a steep launch angle still carries real distance (potentially
    // out of the park) and should fall through to the fly-ball logic below,
    // not get mislabeled "Popup" while the ball is actually still traveling.
    if la >= 50.0 && (ev < 85.0 || dist < 130.0) {
        return (BipType::Popup, Some(nearest_fielder(angle, &INFIELD_POS)));
    }

    if la < 8.0 {
        // Ground ball — infielder proximity is the dominant factor. Hit
        // right at someone (tight sigma — infielders have quick reactions
        // but limited range) it's usually an out; in a gap, usually a hit.
        // A very hard-hit ball partially overcomes good positioning.
        let coverage = nearest_coverage(angle, &INFIELD_POS, 9.0);
        let ev_break = clampf((ev - 75.0) / 55.0, 0.0, 1.0);
        let effective = coverage * (1.0 - 0.5 * ev_break);
        let p_hit = lerpf(0.66, 0.07, effective);
        let fielder = Some(nearest_fielder(angle, &INFIELD_POS));
        let bt = if chance(p_hit) { BipType::Hit(HitType::Single) } else { BipType::Groundout };
        return (bt, fielder);
    }

    if la < 23.0 {
        // Line drive — can be run down by infielders (fast reaction, tight
        // range) or outfielders (more time, wider range but needs distance
        // to have arrived), whichever covers this angle best.
        let inf_cov = nearest_coverage(angle, &INFIELD_POS, 7.0);
        let of_cov  = nearest_coverage(angle, &OUTFIELD_POS, 12.0)
            * clampf((dist - 120.0) / 150.0, 0.15, 1.0);
        let coverage = inf_cov.max(of_cov);
        let ev_factor = clampf((ev - 85.0) / 50.0, 0.0, 1.0);
        let effective = coverage * (1.0 - 0.55 * ev_factor);
        let p_hit = lerpf(0.82, 0.16, effective);
        let fielder = Some(if of_cov > inf_cov {
            nearest_fielder(angle, &OUTFIELD_POS)
        } else {
            nearest_fielder(angle, &INFIELD_POS)
        });

        let bt = if chance(p_hit) {
            let p_xbh = if ev > 100.0 && dist > 250.0 { 0.45 } else { 0.15 };
            if chance(p_xbh) { BipType::Hit(HitType::Double) } else { BipType::Hit(HitType::Single) }
        } else {
            BipType::Lineout
        };
        (bt, fielder)
    } else {
        // Fly ball — outfielder proximity, modulated by hang time (more
        // time in the air = more chance to get under it) and by depth (a
        // shallow bloop is out of an outfielder's realistic reach even at
        // the "right" angle).
        let of_cov = nearest_coverage(angle, &OUTFIELD_POS, 12.0)
            * clampf((dist - 130.0) / 160.0, 0.15, 1.0);
        let hang_factor = clampf(bip.hang_time / 3.2, 0.25, 1.0);
        let effective = of_cov * hang_factor;
        let p_hit = lerpf(0.58, 0.05, effective);
        let fielder = Some(nearest_fielder(angle, &OUTFIELD_POS));

        let bt = if chance(p_hit) {
            let xbh = if dist > game.fence_line * 0.92 { 0.55 }
                      else if dist > game.fence_line * 0.6 { 0.20 }
                      else { 0.05 };
            if chance(xbh) { BipType::Hit(HitType::Double) } else { BipType::Hit(HitType::Single) }
        } else {
            BipType::Flyout
        };
        (bt, fielder)
    }
}

fn describe(bt: &BipType, bip: &BipFlight, ev: f32, la: f32, fielder: Option<FielderInfo>) -> String {
    let dir = if bip.land_x > 5.0 { "RF" }
              else if bip.land_x < -5.0 { "LF" }
              else { "CF" };
    let pos = fielder.map(|(name, _, _)| name).unwrap_or("?");
    match bt {
        BipType::Hit(HitType::Triple) => format!("Triple to {dir} ({:.0} ft)", bip.distance),
        BipType::Hit(HitType::Double) => format!("Double to {dir} ({:.0} ft)", bip.distance),
        BipType::Hit(HitType::Single) => format!("Single to {dir}"),
        BipType::Hit(HitType::HomeRun) => format!("HOME RUN — {:.0} ft!", bip.distance),
        BipType::Groundout => format!("Ground out to {pos} ({:.0} mph, {:.0}°)", ev, la),
        BipType::Lineout   => format!("Line out to {pos} ({:.0} mph, {:.0}°)", ev, la),
        BipType::Flyout    => format!("Fly out to {pos} ({:.0} ft)", bip.distance),
        BipType::Popup     => format!("Popup ({:.0} ft)", bip.distance),
    }
}
