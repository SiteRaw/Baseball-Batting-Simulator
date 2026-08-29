// Data loading: parse [section] key=value .txt files; embed defaults at compile time.

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct PitchDef {
    pub name: String,
    pub key: String,
    pub velocity_mph: f32,
    pub backspin_rpm: f32,
    pub sidespin_rpm: f32,
    pub gyrospin_rpm: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct PhysicsCfg {
    pub air_density: f32,    // slug/ft³
    pub ball_mass: f32,      // slugs
    pub ball_radius: f32,    // feet
    pub cd: f32,
    pub cl_slope: f32,
    pub cl_break: f32,
    pub cl_a: f32,
    pub cl_b: f32,
    pub gravity: f32,        // ft/s²
    pub sim_dt: f32,
    pub release_x: f32,
    pub release_y: f32,
    pub release_z: f32,
}

impl Default for PhysicsCfg {
    fn default() -> Self {
        Self {
            air_density: 0.002_38,
            ball_mass: 5.125 * 0.001_941,   // oz → slugs (1 lb = 0.031081 slug, 16 oz = 1 lb)
            ball_radius: 1.45 / 12.0,
            cd: 0.35,
            cl_slope: 1.18,
            cl_break: 0.10,
            cl_a: 0.07,
            cl_b: 0.48,
            gravity: 32.174,
            sim_dt: 0.000_5,
            release_x: -1.4,
            release_y: 55.0,
            release_z: 5.9,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct GameCfg {
    pub zone_half_w: f32,
    pub zone_bottom: f32,
    pub zone_top: f32,
    pub timing_full: f32,
    pub timing_whiff: f32,
    pub pci_whiff: f32,
    pub foul_tip_band: f32,
    pub la_center: f32,
    pub la_per_vert: f32,
    pub la_per_late: f32,
    pub spray_per_late: f32,
    pub spray_noise: f32,
    pub la_noise: f32,
    pub ev_min: f32,
    pub foul_angle: f32,
    pub bat_backspin_base: f32,
    pub bat_backspin_per_la: f32,
    pub bat_sidespin_per_spray: f32,
    pub fence_line: f32,
    pub fence_cf: f32,
    pub wall_height: f32,
    pub time_scale: f32,
    pub cam_back: f32,
    pub cam_height: f32,
    pub focal: f32,
    // Reach penalty: power lost when the PCI (aim point) sits outside the
    // strike zone — "reaching" for a pitch away from the zone.
    pub reach_penalty_per_ft: f32,
    pub reach_penalty_cap: f32,
    // Batting-mode pacing: auto-advance to the next pitch without a click.
    pub auto_pitch_default: f32,   // >0.5 = on by default
    pub auto_pitch_delay: f32,     // seconds between pitches when enabled
    // Cosmetic celebration/punishment FX on strikeout (fire) / walk (ice).
    pub enable_outcome_fx: f32,    // >0.5 = on
}

impl Default for GameCfg {
    fn default() -> Self {
        Self {
            zone_half_w: 0.708,
            zone_bottom: 1.55,
            zone_top: 3.45,
            timing_full: 0.100,
            timing_whiff: 0.135,
            pci_whiff: 1.60,
            foul_tip_band: 1.30,
            la_center: 13.0,
            la_per_vert: 55.0,
            la_per_late: 140.0,
            spray_per_late: 800.0,
            spray_noise: 6.0,
            la_noise: 4.0,
            ev_min: 34.0,
            foul_angle: 45.0,
            bat_backspin_base: 500.0,
            bat_backspin_per_la: 60.0,
            bat_sidespin_per_spray: 25.0,
            fence_line: 330.0,
            fence_cf: 404.0,
            wall_height: 9.0,
            time_scale: 1.0,
            cam_back: 13.0,
            cam_height: 4.6,
            focal: 1250.0,
            reach_penalty_per_ft: 0.30,
            reach_penalty_cap: 0.55,
            auto_pitch_default: 1.0,
            auto_pitch_delay: 3.0,
            enable_outcome_fx: 1.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct BatterCfg {
    pub side: char,
    pub side_sign: f32,   // +1 = R, -1 = L
    pub power_ev: f32,
    pub pci_rx: f32,
    pub pci_rz: f32,
    // Contact-style traits (all optional; 0 = no preference):
    //   trait_vert_sign  +1 = "up" hitter (rewards high contact / positive nz)
    //                    -1 = "down" hitter (rewards low contact / negative nz)
    //   trait_horiz_sign +1 = "push"/oppo hitter (rewards late contact)
    //                    -1 = "pull" hitter (rewards early contact)
    //   trait_bonus      magnitude of the resulting power swing (both ways)
    pub trait_vert_sign: f32,
    pub trait_horiz_sign: f32,
    pub trait_bonus: f32,
}

impl Default for BatterCfg {
    fn default() -> Self {
        Self {
            side: 'R', side_sign: 1.0, power_ev: 104.0, pci_rx: 1.0, pci_rz: 0.5,
            trait_vert_sign: 0.0, trait_horiz_sign: 0.0, trait_bonus: 0.0,
        }
    }
}

/// Parse "up" / "down" / "none" (vertical) or "pull" / "push" / "none" (horizontal)
/// into a signed trait value: +1, -1, or 0.
fn parse_trait_sign(s: Option<&String>, pos_word: &str, neg_word: &str) -> f32 {
    match s.map(|v| v.to_lowercase()) {
        Some(w) if w == pos_word => 1.0,
        Some(w) if w == neg_word => -1.0,
        _ => 0.0,
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PitcherCfg {
    pub command_sigma: f32,
}

#[derive(Clone, Debug)]
pub struct AiPitcher {
    /// [0]=even [1]=ahead [2]=behind [3]=two_strikes — weighted pitch lists
    pub weights: [Vec<(String, f32)>; 4],
    pub zone_rates: [f32; 4],
}

impl Default for AiPitcher {
    fn default() -> Self {
        let w = |s: &str| -> Vec<(String, f32)> { parse_weights(s) };
        Self {
            weights: [
                w("FF:35,SI:18,SL:18,CH:9,CB:8,FC:10,KC:2"),
                w("FF:20,SI:12,SL:22,SW:13,CB:9,CH:5,FS:5,FC:10,KC:3,FO:1"),
                w("FF:45,SI:25,CH:15,FC:12,KN:3"),
                w("FF:15,SI:8,SL:22,SW:17,CB:13,FS:8,FC:8,KC:6,FO:3"),
            ],
            zone_rates: [0.58, 0.38, 0.78, 0.34],
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AiBatter {
    pub recognition_sigma: f32,
    pub timing_sigma: f32,
    pub pci_sigma: f32,
    pub zone_swing: f32,
    pub zone_swing_two_strikes: f32,
    pub chase: f32,
    pub chase_falloff: f32,
    pub two_strike_zone_expand: f32,
}

impl Default for AiBatter {
    fn default() -> Self {
        Self {
            recognition_sigma: 0.35,
            timing_sigma: 0.045,
            pci_sigma: 0.35,
            zone_swing: 0.66,
            zone_swing_two_strikes: 0.92,
            chase: 0.32,
            chase_falloff: 0.45,
            two_strike_zone_expand: 0.22,
        }
    }
}

// ---------------------------------------------------------------------------
// Precomputed pitch movement (filled in load())
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct PitchInfo {
    pub hb_in: f32,   // horizontal break (inches, + = moves right/glove-side for RHP)
    pub ivb_in: f32,  // induced vertical break (inches, + = lifts)
}

// ---------------------------------------------------------------------------
// Root data bundle
// ---------------------------------------------------------------------------

pub struct Data {
    pub pitches: Vec<PitchDef>,
    pub pitch_info: Vec<PitchInfo>,   // parallel to pitches
    pub physics: PhysicsCfg,
    pub game: GameCfg,
    pub batter_user: BatterCfg,
    pub batter_ai: BatterCfg,
    pub pitcher_user: PitcherCfg,
    pub pitcher_ai_cfg: PitcherCfg,
    pub ai_pitcher: AiPitcher,
    pub ai_batter: AiBatter,
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// Parser helpers
// ---------------------------------------------------------------------------

fn parse_blocks(text: &str) -> Vec<(String, HashMap<String, String>)> {
    let mut blocks: Vec<(String, HashMap<String, String>)> = Vec::new();
    let mut section = String::new();
    let mut map: HashMap<String, String> = HashMap::new();

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        if line.starts_with('[') && line.ends_with(']') {
            if !section.is_empty() {
                blocks.push((section.clone(), map.clone()));
                map.clear();
            }
            section = line[1..line.len()-1].to_lowercase();
        } else if let Some(eq) = line.find('=') {
            let key = line[..eq].trim().to_lowercase();
            let val = line[eq+1..].trim().to_string();
            // strip inline comment
            let val = val.split('#').next().unwrap_or("").trim().to_string();
            map.insert(key, val);
        }
    }
    if !section.is_empty() {
        blocks.push((section, map));
    }
    blocks
}

fn g(map: &HashMap<String, String>, key: &str, def: f32) -> f32 {
    map.get(key).and_then(|v| v.parse().ok()).unwrap_or(def)
}

fn parse_weights(s: &str) -> Vec<(String, f32)> {
    s.split(',').filter_map(|part| {
        let mut it = part.trim().splitn(2, ':');
        let k = it.next()?.trim().to_string();
        let v: f32 = it.next()?.trim().parse().ok()?;
        Some((k, v))
    }).collect()
}

// ---------------------------------------------------------------------------
// Embedded defaults (compile-time)
// ---------------------------------------------------------------------------

const DEF_PITCHES: &str = include_str!("../data/pitches.txt");
const DEF_PLAYERS: &str = include_str!("../data/players.txt");
const DEF_PHYSICS: &str = include_str!("../data/physics.txt");
const DEF_GAME:    &str = include_str!("../data/game.txt");
const DEF_AI:      &str = include_str!("../data/ai.txt");

fn load_file(path: &str, default: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|_| default.to_string())
}

/// Write data files to disk if they don't already exist.
pub fn ensure_data_files() {
    let _ = std::fs::create_dir_all("data");
    let pairs = [
        ("data/pitches.txt", DEF_PITCHES),
        ("data/players.txt", DEF_PLAYERS),
        ("data/physics.txt", DEF_PHYSICS),
        ("data/game.txt",    DEF_GAME),
        ("data/ai.txt",      DEF_AI),
    ];
    for (path, content) in pairs {
        if !std::path::Path::new(path).exists() {
            let _ = std::fs::write(path, content);
        }
    }
}

// ---------------------------------------------------------------------------
// Main loader
// ---------------------------------------------------------------------------

pub fn load() -> Data {
    let pitches_txt = load_file("data/pitches.txt", DEF_PITCHES);
    let players_txt = load_file("data/players.txt", DEF_PLAYERS);
    let physics_txt = load_file("data/physics.txt", DEF_PHYSICS);
    let game_txt    = load_file("data/game.txt",    DEF_GAME);
    let ai_txt      = load_file("data/ai.txt",      DEF_AI);

    let mut warnings = Vec::new();

    // -- pitches --
    let mut pitches: Vec<PitchDef> = Vec::new();
    for (sec, map) in parse_blocks(&pitches_txt) {
        if sec == "pitch" {
            let key = map.get("key").cloned().unwrap_or_default();
            if key.is_empty() { warnings.push("pitch block missing key".into()); continue; }
            pitches.push(PitchDef {
                name: map.get("name").cloned().unwrap_or_else(|| key.clone()),
                key,
                velocity_mph: g(&map, "velocity_mph", 90.0),
                backspin_rpm: g(&map, "backspin_rpm", 1800.0),
                sidespin_rpm: g(&map, "sidespin_rpm", 0.0),
                gyrospin_rpm: g(&map, "gyrospin_rpm", 0.0),
            });
        }
    }
    if pitches.is_empty() {
        warnings.push("No pitches loaded — using built-in FF".into());
        pitches.push(PitchDef {
            name: "4-Seam".into(), key: "FF".into(),
            velocity_mph: 93.0, backspin_rpm: 2000.0,
            sidespin_rpm: 0.0, gyrospin_rpm: 0.0,
        });
    }

    // -- players --
    let mut batter_user   = BatterCfg::default();
    let mut batter_ai     = BatterCfg::default();
    let mut pitcher_user  = PitcherCfg { command_sigma: 0.35 };
    let mut pitcher_ai_cfg= PitcherCfg { command_sigma: 0.55 };
    batter_user.power_ev  = 107.0;
    batter_user.pci_rx    = 1.05;
    batter_user.pci_rz    = 0.55;

    for (sec, map) in parse_blocks(&players_txt) {
        match sec.as_str() {
            "batter_user" => {
                let side = map.get("side").and_then(|s| s.chars().next())
                    .unwrap_or('R').to_ascii_uppercase();
                batter_user.side      = side;
                batter_user.side_sign = if side == 'L' { -1.0 } else { 1.0 };
                batter_user.power_ev  = g(&map, "power_ev", 107.0);
                batter_user.pci_rx    = g(&map, "pci_rx", 1.05);
                batter_user.pci_rz    = g(&map, "pci_rz", 0.55);
                batter_user.trait_vert_sign  = parse_trait_sign(map.get("trait_vert"), "up", "down");
                batter_user.trait_horiz_sign = parse_trait_sign(map.get("trait_horiz"), "push", "pull");
                batter_user.trait_bonus      = g(&map, "trait_bonus", 0.12);
            }
            "batter_ai" => {
                let side = map.get("side").and_then(|s| s.chars().next())
                    .unwrap_or('R').to_ascii_uppercase();
                batter_ai.side      = side;
                batter_ai.side_sign = if side == 'L' { -1.0 } else { 1.0 };
                batter_ai.power_ev  = g(&map, "power_ev", 104.0);
                batter_ai.pci_rx    = g(&map, "pci_rx", 1.0);
                batter_ai.pci_rz    = g(&map, "pci_rz", 0.5);
                batter_ai.trait_vert_sign  = parse_trait_sign(map.get("trait_vert"), "up", "down");
                batter_ai.trait_horiz_sign = parse_trait_sign(map.get("trait_horiz"), "push", "pull");
                batter_ai.trait_bonus      = g(&map, "trait_bonus", 0.10);
            }
            "pitcher_user" => {
                pitcher_user.command_sigma = g(&map, "command_sigma", 0.35);
            }
            "pitcher_ai" => {
                pitcher_ai_cfg.command_sigma = g(&map, "command_sigma", 0.55);
            }
            _ => {}
        }
    }

    // -- physics --
    let mut phys = PhysicsCfg::default();
    for (sec, map) in parse_blocks(&physics_txt) {
        match sec.as_str() {
            "physics" => {
                phys.air_density  = g(&map, "air_density", 0.00238);
                phys.ball_mass    = g(&map, "ball_mass_oz", 5.125) * 0.001_941;
                phys.ball_radius  = g(&map, "ball_radius_in", 1.45) / 12.0;
                phys.cd           = g(&map, "cd", 0.35);
                phys.cl_slope     = g(&map, "cl_slope", 1.18);
                phys.cl_break     = g(&map, "cl_break", 0.10);
                phys.cl_a         = g(&map, "cl_a", 0.07);
                phys.cl_b         = g(&map, "cl_b", 0.48);
                phys.gravity      = g(&map, "gravity", 32.174);
                phys.sim_dt       = g(&map, "sim_dt", 0.0005);
            }
            "release" => {
                phys.release_x = g(&map, "x", -1.4);
                phys.release_y = g(&map, "y", 55.0);
                phys.release_z = g(&map, "z", 5.9);
            }
            _ => {}
        }
    }

    // -- game --
    let mut game = GameCfg::default();
    for (sec, map) in parse_blocks(&game_txt) {
        match sec.as_str() {
            "zone" => {
                game.zone_half_w = g(&map, "half_width", 0.708);
                game.zone_bottom = g(&map, "bottom", 1.55);
                game.zone_top    = g(&map, "top", 3.45);
            }
            "batting" => {
                game.timing_full            = g(&map, "timing_full", 0.100);
                game.timing_whiff           = g(&map, "timing_whiff", 0.135);
                game.pci_whiff              = g(&map, "pci_whiff", 1.60);
                game.foul_tip_band          = g(&map, "foul_tip_band", 1.30);
                game.la_center              = g(&map, "la_center", 13.0);
                game.la_per_vert            = g(&map, "la_per_vert", 55.0);
                game.la_per_late            = g(&map, "la_per_late", 140.0);
                game.spray_per_late         = g(&map, "spray_per_late", 800.0);
                game.spray_noise            = g(&map, "spray_noise", 6.0);
                game.la_noise               = g(&map, "la_noise", 4.0);
                game.ev_min                 = g(&map, "ev_min", 34.0);
                game.foul_angle             = g(&map, "foul_angle", 45.0);
                game.bat_backspin_base      = g(&map, "bat_backspin_base", 500.0);
                game.bat_backspin_per_la    = g(&map, "bat_backspin_per_la", 60.0);
                game.bat_sidespin_per_spray = g(&map, "bat_sidespin_per_spray", 25.0);
                game.reach_penalty_per_ft   = g(&map, "reach_penalty_per_ft", 0.30);
                game.reach_penalty_cap      = g(&map, "reach_penalty_cap", 0.55);
                game.auto_pitch_default     = g(&map, "auto_pitch_default", 1.0);
                game.auto_pitch_delay       = g(&map, "auto_pitch_delay", 3.0);
            }
            "park" => {
                game.fence_line  = g(&map, "fence_line", 330.0);
                game.fence_cf    = g(&map, "fence_cf", 404.0);
                game.wall_height = g(&map, "wall_height", 9.0);
            }
            "view" => {
                game.time_scale = g(&map, "time_scale", 1.0);
                game.cam_back   = g(&map, "cam_back", 13.0);
                game.cam_height = g(&map, "cam_height", 4.6);
                game.focal      = g(&map, "focal", 1250.0);
            }
            "fx" => {
                game.enable_outcome_fx = g(&map, "enable_outcome_fx", 1.0);
            }
            _ => {}
        }
    }

    // -- AI --
    let mut ai_pitcher = AiPitcher::default();
    let mut ai_batter  = AiBatter::default();

    for (sec, map) in parse_blocks(&ai_txt) {
        match sec.as_str() {
            "pitcher_ai" => {
                let wkeys = ["weights_even","weights_ahead","weights_behind","weights_two_strikes"];
                for (i, k) in wkeys.iter().enumerate() {
                    if let Some(v) = map.get(*k) {
                        ai_pitcher.weights[i] = parse_weights(v);
                    }
                }
                let zkeys = ["zone_rate_even","zone_rate_ahead","zone_rate_behind","zone_rate_two_strikes"];
                let zdefs  = [0.58f32, 0.38, 0.78, 0.34];
                for (i, k) in zkeys.iter().enumerate() {
                    ai_pitcher.zone_rates[i] = g(&map, k, zdefs[i]);
                }
            }
            "batter_ai" => {
                ai_batter.recognition_sigma       = g(&map, "recognition_sigma", 0.35);
                ai_batter.timing_sigma            = g(&map, "timing_sigma", 0.045);
                ai_batter.pci_sigma               = g(&map, "pci_sigma", 0.35);
                ai_batter.zone_swing              = g(&map, "zone_swing", 0.66);
                ai_batter.zone_swing_two_strikes  = g(&map, "zone_swing_two_strikes", 0.92);
                ai_batter.chase                   = g(&map, "chase", 0.32);
                ai_batter.chase_falloff           = g(&map, "chase_falloff", 0.45);
                ai_batter.two_strike_zone_expand  = g(&map, "two_strike_zone_expand", 0.22);
            }
            _ => {}
        }
    }

    // Defer pitch_info computation (requires physics) — filled by caller
    let pitch_info = vec![PitchInfo::default(); pitches.len()];

    Data { pitches, pitch_info, physics: phys, game, batter_user, batter_ai,
           pitcher_user, pitcher_ai_cfg, ai_pitcher, ai_batter, warnings }
}
