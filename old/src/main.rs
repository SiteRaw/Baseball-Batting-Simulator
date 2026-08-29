// Baseball Duel — macroquad + egui-macroquad frontend.
// You control both sides; press Tab to switch roles.

use macroquad::prelude::*;
use egui_macroquad::egui;

mod util;
mod data;
mod physics;
mod outcomes;
mod ai;
mod render;

use util::{gauss, clampf};
use data::Data;
use physics::{PitchFlight, BipFlight, simulate_pitch, simulate_pitch_release_aim, pitch_movement};
use outcomes::{PitchOutcome, BipType, SwingResult, resolve};
use ai::{SwingPlan, ai_select_pitch, ai_swing_decision};
use render::{View, draw_scene, draw_aim_crosshair, draw_release_fx, draw_release_fx_warning,
             draw_swing_animation, draw_minimap, draw_predicted_landing, draw_strikeout_fx, draw_walk_fx};

// ---------------------------------------------------------------------------
// Tunables
// ---------------------------------------------------------------------------

// Windup is short for manual pitcher-role throws (no pre-existing countdown
// to "borrow" FX lead-time from, so speed matters most there). Batting-mode
// auto-pitch keeps the full 1s windup — its FX still starts up to
// FX_WARMUP_LEAD seconds early during the existing "Next pitch" countdown,
// so the extra length is felt as anticipation, not latency.
const WINDUP_DURATION_PITCHER: f32 = 0.5;
const WINDUP_DURATION_BATTER: f32 = 1.0;
const RELEASE_FX_WINDOW: f32 = 0.18;   // stage 2 (release-imminent) solo period
const CROSSFADE_START: f32 = 0.30;     // stage 1 solo before this; crossfades into stage 2 below it
const SWING_ANIM_DURATION: f32 = 0.30;

// Batting-mode auto-pitch already makes the player wait out a "Next pitch in
// Xs" countdown before the AI throws. Rather than ALSO stretching the windup
// to fit a full-length warning pulse, the pulse starts up to FX_WARMUP_LEAD
// seconds early — during the tail of that pre-existing wait — so it never
// feels like extra latency (matches "starts on the 1 of a 3-2-1-0 countdown").
const FX_WARMUP_LEAD: f32 = 1.0;
const FX_WARMUP_FADE: f32 = 0.15;
const OUTCOME_FX_DURATION: f32 = 1.3;

// ---------------------------------------------------------------------------
// Core enums
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum Role { Pitcher, Batter }

#[derive(Clone, Copy, PartialEq, Debug)]
enum OutcomeFxKind { Strikeout, Walk }

#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    PrePitch,
    Windup     { timer: f32 },
    InFlight   { t: f32 },
    BallInPlay { t: f32 },
    Result     { timer: f32 },
}

// ---------------------------------------------------------------------------
// Stats
// ---------------------------------------------------------------------------

#[derive(Default, Clone, Copy)]
struct Stats {
    pub pa: u32, pub ab: u32,
    pub h: u32, pub doubles: u32, pub triples: u32, pub hr: u32,
    pub bb: u32, pub k: u32,
    pub bip: u32, pub total_ev: f32, pub max_dist: f32,
}

impl Stats {
    fn avg(&self) -> f32 {
        if self.ab == 0 { 0.0 } else { self.h as f32 / self.ab as f32 }
    }
    fn obp(&self) -> f32 {
        if self.pa == 0 { 0.0 } else { (self.h + self.bb) as f32 / self.pa as f32 }
    }
    fn slg(&self) -> f32 {
        let tb = self.h + self.doubles + 2 * self.triples + 3 * self.hr;
        if self.ab == 0 { 0.0 } else { tb as f32 / self.ab as f32 }
    }
    fn avg_ev(&self) -> f32 {
        if self.bip == 0 { 0.0 } else { self.total_ev / self.bip as f32 }
    }
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

struct App {
    data: Data,
    role: Role,

    // Count
    balls: u8, strikes: u8, outs: u8,

    // Phase
    phase: Phase,
    pre_pitch_timer: f32,   // seconds spent in Phase::PrePitch (drives auto-pitch)

    // Pre-pitch
    sel_pitch: usize,
    aim_x: f32,
    aim_z: f32,

    // In-flight state
    flight: Option<PitchFlight>,
    ai_plan: SwingPlan,           // AI batter's pre-computed plan
    user_swung_at: Option<f32>,   // time user pressed Space (batter role)
    pci_x: f32, pci_z: f32,       // current PCI / aim (world space at plate plane)

    // After contact
    bip: Option<BipFlight>,
    last_result: SwingResult,
    last_pitch_name: String,

    // Trail for result display (pre-contact pitch path)
    trail: Vec<util::Vec3>,

    // Swing bat animation
    swing_anim_t: Option<f32>,

    // Batting-mode pacing (auto-pitch)
    auto_pitch: bool,
    auto_pitch_delay: f32,

    // Aerial mini-map (batting mode): shows where the most recent ball in
    // play FIRST landed. Revealed only once the on-field animation actually
    // reaches that point (never "forward-looking"), and only replaced —
    // never cleared early — once the next ball in play similarly lands.
    minimap_landing: Option<(f32, f32)>,
    pending_minimap: Option<(f32, f32, f32)>,  // (land_x, land_y, reveal_time)

    // Cosmetic strikeout/walk FX (toggleable via game.txt [fx])
    outcome_fx: Option<(OutcomeFxKind, f32)>,

    // Stats (user as batter, user as pitcher)
    stats_batting: Stats,
    stats_pitching: Stats,   // from user-pitcher's perspective (AI batting)

    // Log
    log: Vec<String>,

    // UI
    over_ui: bool,
    show_movement_chart: bool,
    movement_cache: Vec<(f32, f32)>,   // (hb, ivb) per pitch

    // Reload
    reload_req: bool,
}

impl App {
    fn new() -> Self {
        data::ensure_data_files();
        let mut d = data::load();
        // Compute movement chart data
        let mc: Vec<(f32, f32)> = d.pitches.iter()
            .map(|p| pitch_movement(p, &d.physics))
            .collect();
        for (i, (h, v)) in mc.iter().enumerate() {
            d.pitch_info[i].hb_in  = *h;
            d.pitch_info[i].ivb_in = *v;
        }
        let auto_pitch = d.game.auto_pitch_default > 0.5;
        let auto_pitch_delay = d.game.auto_pitch_delay;

        App {
            movement_cache: mc,
            data: d,
            role: Role::Pitcher,
            balls: 0, strikes: 0, outs: 0,
            phase: Phase::PrePitch,
            pre_pitch_timer: 0.0,
            sel_pitch: 0,
            aim_x: 0.0,
            aim_z: 2.5,
            flight: None,
            ai_plan: SwingPlan { will_swing: false, swing_t: 0.0, pci_x: 0.0, pci_z: 0.0 },
            user_swung_at: None,
            pci_x: 0.0, pci_z: 2.5,
            bip: None,
            last_result: SwingResult::default(),
            last_pitch_name: String::new(),
            trail: Vec::new(),
            swing_anim_t: None,
            auto_pitch,
            auto_pitch_delay,
            minimap_landing: None,
            pending_minimap: None,
            outcome_fx: None,
            stats_batting: Stats::default(),
            stats_pitching: Stats::default(),
            log: Vec::new(),
            over_ui: false,
            show_movement_chart: false,
            reload_req: false,
        }
    }

    fn reload(&mut self) {
        data::ensure_data_files();
        let mut d = data::load();
        let mc: Vec<(f32, f32)> = d.pitches.iter()
            .map(|p| pitch_movement(p, &d.physics))
            .collect();
        for (i, (h, v)) in mc.iter().enumerate() {
            d.pitch_info[i].hb_in = *h;
            d.pitch_info[i].ivb_in = *v;
        }
        self.movement_cache = mc;
        self.sel_pitch = self.sel_pitch.min(d.pitches.len().saturating_sub(1));
        self.data = d;
        self.log.push("[Reloaded data files]".into());
        self.reload_req = false;
    }

    fn goto_pre_pitch(&mut self) {
        self.phase = Phase::PrePitch;
        self.pre_pitch_timer = 0.0;
    }

    fn windup_duration(&self) -> f32 {
        match self.role {
            Role::Pitcher => WINDUP_DURATION_PITCHER,
            Role::Batter  => WINDUP_DURATION_BATTER,
        }
    }

    // -----------------------------------------------------------------------
    // Start a pitch (Windup → InFlight transition)
    // -----------------------------------------------------------------------

    fn launch_pitch(&mut self) {
        let pidx = self.sel_pitch.min(self.data.pitches.len() - 1);
        let def  = &self.data.pitches[pidx];
        self.last_pitch_name = def.name.clone();

        let flight = match self.role {
            Role::Pitcher => {
                // The human pitcher aims a GENERIC straight fastball at the
                // reticle; the actually-selected pitch is released along that
                // same direction and drifts away from it due to its own
                // movement (see physics::simulate_pitch_release_aim).
                let sigma = self.data.pitcher_user.command_sigma;
                let ax = self.aim_x + gauss(sigma);
                let az = self.aim_z + gauss(sigma);
                let def_for_sim = &self.data.pitches[self.sel_pitch];
                simulate_pitch_release_aim(def_for_sim, ax, az, &self.data.physics)
            }
            Role::Batter => {
                // AI pitcher targets its true intended location.
                let (ai_idx, tx, tz) = ai_select_pitch(
                    &self.data.pitches,
                    &self.data.ai_pitcher,
                    self.balls, self.strikes,
                    &self.data.game,
                );
                self.sel_pitch = ai_idx;
                let def2 = &self.data.pitches[ai_idx];
                self.last_pitch_name = def2.name.clone();
                let sig = self.data.pitcher_ai_cfg.command_sigma;
                let ax = tx + gauss(sig);
                let az = tz + gauss(sig);
                simulate_pitch(def2, ax, az, &self.data.physics)
            }
        };

        // AI batter pre-plans swing
        if self.role == Role::Pitcher {
            self.ai_plan = ai_swing_decision(
                &flight,
                &self.data.ai_batter,
                &self.data.batter_ai,
                &self.data.game,
                self.balls, self.strikes,
            );
        }

        self.flight = Some(flight);
        self.user_swung_at = None;
        self.swing_anim_t = None;
        self.bip = None;
        self.trail.clear();
        self.phase = Phase::InFlight { t: 0.0 };
    }

    // -----------------------------------------------------------------------
    // Resolve a completed pitch
    // -----------------------------------------------------------------------

    fn resolve_pitch(&mut self, t_flight: f32) {
        let flight = match &self.flight { Some(f) => f, None => return };

        let (swing_t, pci_x, pci_z) = match self.role {
            Role::Batter => {
                // User swung (or didn't) — PCI is wherever mouse was
                (self.user_swung_at, self.pci_x, self.pci_z)
            }
            Role::Pitcher => {
                // AI batter
                if self.ai_plan.will_swing {
                    (Some(self.ai_plan.swing_t), self.ai_plan.pci_x, self.ai_plan.pci_z)
                } else {
                    (None, 0.0, 0.0)
                }
            }
        };

        let batter_cfg = match self.role {
            Role::Batter   => &self.data.batter_user,
            Role::Pitcher  => &self.data.batter_ai,
        };

        let result = resolve(
            flight, swing_t, pci_x, pci_z,
            batter_cfg, &self.data.game, &self.data.physics,
        );

        // Pre-contact pitch trail (shown during Result on takes/whiffs/fouls)
        let total_t = t_flight.min(flight.plate_t + 0.05);
        let step = total_t / 30.0;
        self.trail = (0..30).map(|i| flight.pos_at(i as f32 * step)).collect();

        self.bip = result.bip.clone();

        if self.role == Role::Batter {
            if let Some(b) = &self.bip {
                self.pending_minimap = Some((b.land_x, b.land_y, b.first_landing_time));
            }
        }

        self.update_count_and_stats(&result);
        self.log.push(format!("{} | {} | {}",
            pitch_count_str(self.balls, self.strikes),
            self.last_pitch_name,
            result.description,
        ));

        let was_in_play = matches!(result.outcome, PitchOutcome::InPlay);
        self.last_result = result;

        if was_in_play {
            self.phase = Phase::BallInPlay { t: 0.0 };
        } else {
            self.phase = Phase::Result { timer: 0.0 };
        }
    }

    fn update_count_and_stats(&mut self, r: &SwingResult) {
        use PitchOutcome::*;
        let stats = match self.role {
            Role::Batter  => &mut self.stats_batting,
            Role::Pitcher => &mut self.stats_pitching,
        };

        let fx_enabled = self.data.game.enable_outcome_fx > 0.5;

        match &r.outcome {
            Ball => {
                self.balls += 1;
                if self.balls >= 4 {
                    // Walk
                    stats.pa += 1; stats.bb += 1;
                    self.log.push("Walk (BB)".into());
                    if fx_enabled { self.outcome_fx = Some((OutcomeFxKind::Walk, 0.0)); }
                    self.reset_count(false);
                }
            }
            CalledStrike | SwingingStrike | FoulTip => {
                let add_k = if self.strikes < 2 {
                    self.strikes += 1; false
                } else {
                    // K
                    stats.pa += 1; stats.ab += 1; stats.k += 1;
                    self.log.push("Strikeout (K)".into());
                    if fx_enabled { self.outcome_fx = Some((OutcomeFxKind::Strikeout, 0.0)); }
                    self.reset_count(true);
                    true
                };
                let _ = add_k;
            }
            Foul | FoulBack => {
                if self.strikes < 2 { self.strikes += 1; }
                // Can't strike out on a foul (unless foul tip with 2 strikes, handled as FoulTip above)
            }
            InPlay => {
                stats.pa += 1; stats.ab += 1;
                if r.ev > 1.0 {
                    stats.bip  += 1;
                    stats.total_ev += r.ev;
                }
                if let Some(bt) = &r.bip_type {
                    match bt {
                        BipType::Hit(ht) => {
                            stats.h += 1;
                            use outcomes::HitType::*;
                            match ht {
                                Double  => { stats.doubles += 1; }
                                Triple  => { stats.triples += 1; }
                                HomeRun => { stats.hr += 1; }
                                Single  => {}
                            }
                            if let Some(b) = &self.bip {
                                if b.distance > stats.max_dist { stats.max_dist = b.distance; }
                            }
                        }
                        _ => {
                            // Out
                            self.outs += 1;
                            if self.outs >= 3 {
                                self.log.push("--- Side retired ---".into());
                                self.outs = 0;
                            }
                        }
                    }
                }
                self.reset_count(false);
            }
        }
    }

    fn reset_count(&mut self, _is_out: bool) {
        self.balls   = 0;
        self.strikes = 0;
    }

    // -----------------------------------------------------------------------
    // Update (called every frame)
    // -----------------------------------------------------------------------

    fn update(&mut self) {
        if self.reload_req { self.reload(); }
        let raw_dt = get_frame_time();
        let dt = raw_dt * self.data.game.time_scale;

        // Swing bat animation ticks independent of phase (follow-through
        // can spill past InFlight into the Result banner).
        if let Some(t) = self.swing_anim_t {
            let nt = t + raw_dt;
            self.swing_anim_t = if nt > SWING_ANIM_DURATION { None } else { Some(nt) };
        }

        // Strikeout/walk celebration FX ticks independent of phase too.
        if let Some((kind, t)) = self.outcome_fx {
            let nt = t + raw_dt;
            self.outcome_fx = if nt > OUTCOME_FX_DURATION { None } else { Some((kind, nt)) };
        }

        match self.phase {
            Phase::PrePitch => {
                self.pre_pitch_timer += raw_dt;
                if self.role == Role::Batter && self.auto_pitch
                    && self.pre_pitch_timer >= self.auto_pitch_delay {
                    self.phase = Phase::Windup { timer: self.windup_duration() };
                }
            }

            Phase::Windup { timer } => {
                let new_t = timer - raw_dt;
                if new_t <= 0.0 {
                    self.launch_pitch();
                } else {
                    self.phase = Phase::Windup { timer: new_t };
                }
            }

            Phase::InFlight { t } => {
                let new_t = t + dt;

                // User swing input (batter role only, when UI not captured)
                if self.role == Role::Batter && !self.over_ui {
                    if is_key_pressed(KeyCode::Space) && self.user_swung_at.is_none() {
                        self.user_swung_at = Some(new_t);
                        self.swing_anim_t = Some(0.0);
                    }
                }

                // Check if flight is complete
                if let Some(flight) = &self.flight {
                    let plate_t = flight.plate_t;

                    let ai_swing_done = self.role == Role::Pitcher
                        && self.ai_plan.will_swing
                        && new_t >= self.ai_plan.swing_t + 0.05;
                    let user_swung    = self.user_swung_at.is_some() && new_t > self.user_swung_at.unwrap() + 0.05;
                    let past_plate    = new_t >= plate_t + 0.12;

                    if past_plate || user_swung || ai_swing_done {
                        self.phase = Phase::InFlight { t: new_t };
                        self.resolve_pitch(new_t);
                        return;
                    }
                }

                self.phase = Phase::InFlight { t: new_t };
            }

            Phase::BallInPlay { t } => {
                let new_t = t + dt;

                // Reveal the mini-map landing dot only once the on-field
                // animation actually reaches it — never ahead of what the
                // player can see.
                if let Some((lx, ly, reveal_t)) = self.pending_minimap {
                    if new_t >= reveal_t {
                        self.minimap_landing = Some((lx, ly));
                        self.pending_minimap = None;
                    }
                }

                let total = self.bip.as_ref().map(|b| b.duration).unwrap_or(0.0);
                if new_t >= total || new_t > 9.0 {
                    self.phase = Phase::Result { timer: 0.0 };
                } else {
                    self.phase = Phase::BallInPlay { t: new_t };
                }
            }

            Phase::Result { timer } => {
                let new_t = timer + raw_dt;
                let mut advance = false;

                if !self.over_ui {
                    if is_key_pressed(KeyCode::Space) || is_key_pressed(KeyCode::Enter)
                        || is_mouse_button_pressed(MouseButton::Left) {
                        advance = true;
                    }
                }
                if self.role == Role::Batter && self.auto_pitch && new_t >= self.auto_pitch_delay {
                    advance = true;
                }

                if advance {
                    match self.role {
                        // Batting mode flows straight into the next pitch —
                        // no click required between at-bats when auto-pitch is on.
                        Role::Batter  => self.phase = Phase::Windup { timer: self.windup_duration() },
                        // Pitching mode returns to pitch-selection so the user
                        // can pick their next pitch and aim before throwing.
                        Role::Pitcher => self.goto_pre_pitch(),
                    }
                } else {
                    self.phase = Phase::Result { timer: new_t };
                }
            }
        }

        // Tab: switch role
        if is_key_pressed(KeyCode::Tab) {
            self.role = match self.role { Role::Pitcher => Role::Batter, Role::Batter => Role::Pitcher };
            self.goto_pre_pitch();
        }
    }

    // -----------------------------------------------------------------------
    // Draw
    // -----------------------------------------------------------------------

    fn draw(&mut self, view: &View) {
        let game = &self.data.game;

        // How far into the batted-ball animation are we (for trail/bounce display)?
        let bip_display_t = match self.phase {
            Phase::BallInPlay { t } => Some(t),
            Phase::Result { .. } if matches!(self.last_result.outcome, PitchOutcome::InPlay) => {
                self.bip.as_ref().map(|b| b.duration)
            }
            _ => None,
        };

        let ball_pos = match self.phase {
            Phase::InFlight { t } => self.flight.as_ref().map(|f| f.pos_at(t)),
            Phase::BallInPlay { t } => self.bip.as_ref().map(|b| b.pos_at(t)),
            Phase::Result { .. } => {
                if matches!(self.last_result.outcome, PitchOutcome::InPlay) {
                    self.bip.as_ref().map(|b| b.rest)
                } else {
                    self.flight.as_ref().map(|f| f.pos_at(f.plate_t))
                }
            }
            _ => None,
        };

        let (bip_trail, bounce_points): (Vec<util::Vec3>, Vec<util::Vec3>) =
            if let (Some(bip), Some(disp_t)) = (self.bip.as_ref(), bip_display_t) {
                let n = bip.samples.len();
                let trail_pts = if n > 0 {
                    let idx_max = ((disp_t * physics::BIP_SAMPLE) as usize).min(n - 1);
                    let step = (idx_max / 50).max(1);
                    (0..=idx_max).step_by(step).map(|i| bip.samples[i]).collect()
                } else {
                    Vec::new()
                };
                let bounces = bip.bounces.iter()
                    .filter(|(bt, _)| *bt <= disp_t)
                    .map(|(_, p)| *p)
                    .collect();
                (trail_pts, bounces)
            } else {
                (Vec::new(), Vec::new())
            };

        let pci_world = if self.role == Role::Batter {
            Some((self.pci_x, self.pci_z))
        } else {
            None
        };

        let landing = if matches!(self.phase, Phase::Result { .. })
            && matches!(self.last_result.outcome, PitchOutcome::InPlay) {
            self.bip.as_ref().map(|b| b.rest)
        } else {
            None
        };

        let show_zone = true;

        draw_scene(
            view, game, ball_pos, pci_world,
            self.data.batter_user.pci_rx,
            self.data.batter_user.pci_rz,
            &self.trail, &bip_trail, &bounce_points,
            landing, show_zone,
        );

        // Pitching aim crosshair
        if self.role == Role::Pitcher && matches!(self.phase, Phase::PrePitch | Phase::Windup { .. }) {
            draw_aim_crosshair(view, self.aim_x, self.aim_z);

            // Light-green preview: where THIS pitch's own movement actually
            // carries it, given the same release aim (no noise — a clean
            // "if executed perfectly" preview). Subtle by design (low alpha).
            if let Some(def) = self.data.pitches.get(self.sel_pitch) {
                let preview = simulate_pitch_release_aim(def, self.aim_x, self.aim_z, &self.data.physics);
                let pt = preview.plate_t;
                let start_t = (pt - 0.07).max(0.0);
                let preview_trail: Vec<util::Vec3> = (0..12)
                    .map(|i| {
                        let f = i as f32 / 11.0;
                        preview.pos_at(start_t + (pt - start_t) * f)
                    })
                    .collect();
                draw_predicted_landing(
                    view, &preview_trail,
                    util::Vec3::new(preview.plate_x, 0.8, preview.plate_z),
                );
            }
        }

        // Release-point FX: an early "get ready" pulse crossfades into a
        // sharp multicolor pulse in the final instant before release. For
        // batting-mode auto-pitch, the warning pulse starts during the tail
        // of the existing "Next pitch in Xs" wait rather than the windup
        // itself, so it never adds perceived delay.
        let (stage1_alpha, stage2_alpha) = match self.phase {
            Phase::Windup { timer } => {
                let s1 = clampf(
                    (timer - RELEASE_FX_WINDOW) / (CROSSFADE_START - RELEASE_FX_WINDOW),
                    0.0, 1.0,
                );
                (s1, 1.0 - s1)
            }
            Phase::PrePitch if self.role == Role::Batter && self.auto_pitch => {
                (warmup_alpha(self.auto_pitch_delay - self.pre_pitch_timer), 0.0)
            }
            Phase::Result { timer } if self.role == Role::Batter && self.auto_pitch => {
                (warmup_alpha(self.auto_pitch_delay - timer), 0.0)
            }
            _ => (0.0, 0.0),
        };
        if stage1_alpha > 0.0 || stage2_alpha > 0.0 {
            let rp = util::Vec3::new(
                self.data.physics.release_x,
                self.data.physics.release_y,
                self.data.physics.release_z,
            );
            if stage1_alpha > 0.0 {
                draw_release_fx_warning(view, rp, stage1_alpha);
            }
            if stage2_alpha > 0.0 {
                draw_release_fx(view, rp, stage2_alpha);
            }
        }

        // Swing bat animation overlay
        if let Some(t) = self.swing_anim_t {
            draw_swing_animation((t / SWING_ANIM_DURATION).min(1.0));
        }

        // HUD count (top of screen)
        let count_text = format!("{}-{}   {} outs", self.balls, self.strikes, self.outs);
        draw_text(&count_text, screen_width() * 0.5 - 60.0, 24.0, 22.0, WHITE);

        // Auto-pitch countdown (batting mode)
        if self.role == Role::Batter && self.auto_pitch {
            let remaining = match self.phase {
                Phase::PrePitch        => Some(self.auto_pitch_delay - self.pre_pitch_timer),
                Phase::Result { timer } => Some(self.auto_pitch_delay - timer),
                _ => None,
            };
            if let Some(r) = remaining {
                if r > 0.0 {
                    let txt = format!("Next pitch in {:.1}s", r);
                    draw_text(&txt, screen_width() * 0.5 - 55.0, 48.0, 16.0,
                        Color::from_rgba(200, 220, 255, 220));
                }
            }
        }

        // Phase hint
        let hint = match self.phase {
            Phase::PrePitch => match self.role {
                Role::Pitcher => "[Space] throw   [Scroll / 1-9] pick pitch   [Mouse] aim",
                Role::Batter  => "[LMB] throw AI pitch   [Space] swing",
            },
            Phase::Windup { .. } => "",
            Phase::InFlight { .. } => if self.role == Role::Batter { "[ SPACE ] to swing!" } else { "" },
            Phase::BallInPlay { .. } => "",
            Phase::Result { .. } => "[ Space / Enter / LMB ] → next pitch",
        };
        if !hint.is_empty() {
            draw_text(hint, 10.0, screen_height() - 12.0, 16.0, Color::from_rgba(200, 200, 200, 200));
        }

        // Result banner
        if let Phase::Result { timer } = self.phase {
            let alpha = ((1.0 - (timer / 2.5).min(1.0)) * 255.0) as u8;
            let col   = outcome_color(&self.last_result.outcome);
            let banr  = format!("{} — {}  ({})",
                self.last_pitch_name, self.last_result.description,
                pitch_count_str(self.balls, self.strikes));
            draw_text_centered(&banr, screen_width() * 0.5, screen_height() * 0.25, 26.0,
                Color { r: col.r, g: col.g, b: col.b, a: alpha as f32 / 255.0 });
        }

        // Strikeout (fire) / walk (ice storm) celebration FX
        if let Some((kind, t)) = self.outcome_fx {
            let progress = (t / OUTCOME_FX_DURATION).min(1.0);
            match kind {
                OutcomeFxKind::Strikeout => draw_strikeout_fx(progress),
                OutcomeFxKind::Walk      => draw_walk_fx(progress),
            }
        }
    }

    // -----------------------------------------------------------------------
    // egui side panel
    // -----------------------------------------------------------------------

    fn draw_ui(&mut self, ctx: &egui::Context) {
        self.over_ui = ctx.wants_pointer_input();

        egui::SidePanel::right("panel").default_width(240.0).show(ctx, |ui| {
            ui.heading(match self.role {
                Role::Pitcher => "⚾ Pitching",
                Role::Batter  => "🏏 Batting",
            });
            ui.label(egui::RichText::new("[Tab] switch role").small().color(egui::Color32::GRAY));
            ui.separator();

            // --- Pitcher: pitch selection ---
            if self.role == Role::Pitcher {
                ui.label("Pitch Arsenal  [1-9 or Scroll]");
                let pitches = &self.data.pitches;
                let info    = &self.data.pitch_info;
                let sel     = &mut self.sel_pitch;
                let phase   = self.phase;

                egui::ScrollArea::vertical().id_source("pitch_list")
                    .max_height(220.0).show(ui, |ui| {
                    for (i, p) in pitches.iter().enumerate() {
                        let label = format!("[{}] {}  {:.0} mph  HB{:+.1}\" IVB{:+.1}\"",
                            i + 1, p.key, p.velocity_mph,
                            info[i].hb_in, info[i].ivb_in);
                        let resp = ui.selectable_label(*sel == i, &label);
                        if resp.clicked() && matches!(phase, Phase::PrePitch) {
                            *sel = i;
                        }
                    }
                });

                ui.separator();
                if matches!(self.phase, Phase::PrePitch) {
                    if ui.button("⚾ THROW  [Space]").clicked() {
                        self.phase = Phase::Windup { timer: self.windup_duration() };
                    }
                }
                ui.label(egui::RichText::new(
                    "Aim shows where a straight fastball would cross — \
                     your actual pitch will break away from it.")
                    .small().color(egui::Color32::GRAY));
            }

            // --- Batter: swing hint + pacing ---
            if self.role == Role::Batter {
                ui.label("Move mouse to position PCI.");
                ui.label("Press [Space] to swing.");
                ui.separator();

                ui.checkbox(&mut self.auto_pitch, "Auto-pitch");
                ui.add(egui::Slider::new(&mut self.auto_pitch_delay, 1.0..=6.0).text("Delay (s)"));
                if self.auto_pitch {
                    let remaining = match self.phase {
                        Phase::PrePitch         => Some(self.auto_pitch_delay - self.pre_pitch_timer),
                        Phase::Result { timer } => Some(self.auto_pitch_delay - timer),
                        _ => None,
                    };
                    if let Some(r) = remaining {
                        ui.label(format!("Next pitch in {:.1}s", r.max(0.0)));
                    }
                }

                ui.separator();
                if matches!(self.phase, Phase::PrePitch) {
                    if ui.button("▶ Trigger AI pitch").clicked() {
                        self.phase = Phase::Windup { timer: self.windup_duration() };
                    }
                }
            }

            ui.separator();

            // --- Stats panels ---
            ui.collapsing("📊 Batting Stats (you bat)", |ui| {
                stats_grid(ui, &self.stats_batting);
            });
            ui.collapsing("📊 Pitching Stats (AI bats)", |ui| {
                stats_grid(ui, &self.stats_pitching);
            });

            ui.separator();
            // --- Log ---
            ui.collapsing("📝 Pitch Log", |ui| {
                egui::ScrollArea::vertical().id_source("log")
                    .max_height(150.0).stick_to_bottom(true).show(ui, |ui| {
                    for line in self.log.iter().rev().take(40) {
                        ui.label(egui::RichText::new(line).small().monospace());
                    }
                });
            });

            ui.separator();
            if ui.button("↺ Reload data files").clicked() {
                self.reload_req = true;
            }
        });
    }

    // -----------------------------------------------------------------------
    // Input: mouse aim / PCI update
    // -----------------------------------------------------------------------

    fn handle_input(&mut self, view: &View) {
        if self.over_ui { return; }

        let (mx, my) = mouse_position();
        let (wx, wz) = view.screen_to_world(mx, my, 0.8);

        match self.role {
            Role::Pitcher => {
                // Update aim location (this is the "generic straight fastball"
                // reticle — see simulate_pitch_release_aim)
                self.aim_x = clampf(wx, -1.8, 1.8);
                self.aim_z = clampf(wz, 0.5, 5.5);

                if matches!(self.phase, Phase::PrePitch) {
                    // Space throws — kept off the mouse entirely so aiming
                    // and pitch-selection clicks can never misfire a pitch.
                    if is_key_pressed(KeyCode::Space) {
                        self.phase = Phase::Windup { timer: self.windup_duration() };
                    }
                    // Scroll wheel cycles the selected pitch.
                    let (_, wheel_y) = mouse_wheel();
                    if wheel_y.abs() > 0.01 {
                        let n = self.data.pitches.len();
                        if n > 0 {
                            let dir: isize = if wheel_y > 0.0 { -1 } else { 1 };
                            let cur = self.sel_pitch as isize;
                            self.sel_pitch = (cur + dir).rem_euclid(n as isize) as usize;
                        }
                    }
                }
                // Pitch hotkeys 1-9
                for (i, key) in [KeyCode::Key1,KeyCode::Key2,KeyCode::Key3,
                                  KeyCode::Key4,KeyCode::Key5,KeyCode::Key6,
                                  KeyCode::Key7,KeyCode::Key8,KeyCode::Key9].iter().enumerate() {
                    if is_key_pressed(*key) && i < self.data.pitches.len() {
                        if matches!(self.phase, Phase::PrePitch) {
                            self.sel_pitch = i;
                        }
                    }
                }
            }
            Role::Batter => {
                // Move PCI
                self.pci_x = clampf(wx, -2.5, 2.5);
                self.pci_z = clampf(wz, 0.3, 5.5);
                // Trigger AI pitch on LMB at PrePitch (no conflict with Space,
                // which is reserved for swinging)
                if is_mouse_button_pressed(MouseButton::Left)
                    && matches!(self.phase, Phase::PrePitch) {
                    self.phase = Phase::Windup { timer: self.windup_duration() };
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn pitch_count_str(b: u8, s: u8) -> String { format!("{}-{}", b, s) }

/// Fades the stage-1 "get ready" pulse in during the last FX_WARMUP_LEAD
/// seconds before a countdown reaches zero (used for auto-pitch pre-windup).
fn warmup_alpha(remaining: f32) -> f32 {
    if remaining <= 0.0 { 1.0 }
    else if remaining >= FX_WARMUP_LEAD { 0.0 }
    else { clampf((FX_WARMUP_LEAD - remaining) / FX_WARMUP_FADE, 0.0, 1.0) }
}

fn outcome_color(o: &PitchOutcome) -> Color {
    match o {
        PitchOutcome::Ball             => BLUE,
        PitchOutcome::CalledStrike     => ORANGE,
        PitchOutcome::SwingingStrike   => RED,
        PitchOutcome::FoulTip          => YELLOW,
        PitchOutcome::Foul | PitchOutcome::FoulBack => YELLOW,
        PitchOutcome::InPlay           => GREEN,
    }
}

fn draw_text_centered(text: &str, x: f32, y: f32, size: f32, color: Color) {
    let dims = measure_text(text, None, size as u16, 1.0);
    draw_text(text, x - dims.width * 0.5, y, size, color);
}

fn stats_grid(ui: &mut egui::Ui, s: &Stats) {
    egui::Grid::new(ui.next_auto_id()).num_columns(2).striped(true).show(ui, |ui| {
        let row = |ui: &mut egui::Ui, k: &str, v: String| {
            ui.label(k); ui.label(v); ui.end_row();
        };
        row(ui, "PA",      s.pa.to_string());
        row(ui, "AB",      s.ab.to_string());
        row(ui, "H",       s.h.to_string());
        row(ui, "2B",      s.doubles.to_string());
        row(ui, "3B",      s.triples.to_string());
        row(ui, "HR",      s.hr.to_string());
        row(ui, "BB",      s.bb.to_string());
        row(ui, "K",       s.k.to_string());
        row(ui, "AVG",     format!(".{:03}", (s.avg() * 1000.0).min(999.0) as u32));
        row(ui, "OBP",     format!(".{:03}", (s.obp() * 1000.0).min(999.0) as u32));
        row(ui, "SLG",     format!(".{:03}", (s.slg() * 1000.0).min(999.0) as u32));
        row(ui, "avgEV",   format!("{:.1} mph", s.avg_ev()));
        row(ui, "maxDist", format!("{:.0} ft", s.max_dist));
    });
}

// ---------------------------------------------------------------------------
// macroquad window config
// ---------------------------------------------------------------------------

fn window_conf() -> Conf {
    Conf {
        window_title: "Baseball Duel".to_owned(),
        window_width: 1280,
        window_height: 800,
        high_dpi: true,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Main loop
// ---------------------------------------------------------------------------

#[macroquad::main(window_conf)]
async fn main() {
    let mut app = App::new();

    loop {
        clear_background(Color::from_rgba(20, 25, 30, 255));

        let view = View::new(&app.data.game);

        // Input + physics update
        app.handle_input(&view);
        app.update();

        // Scene draw
        app.draw(&view);

        // egui UI
        egui_macroquad::ui(|ctx| {
            app.draw_ui(ctx);
        });
        egui_macroquad::draw();

        // Aerial mini-map (batting mode) — drawn last so it sits ON TOP of
        // the egui side panel instead of being hidden behind it.
        if app.role == Role::Batter {
            draw_minimap(&app.data.game, app.minimap_landing);
        }

        next_frame().await;
    }
}
