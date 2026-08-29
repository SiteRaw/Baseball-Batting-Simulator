// Physics simulation: pitch trajectory and batted-ball flight.
//
// Coordinate system (catcher's POV, Statcast-style):
//   +x = catcher's right, +y = toward pitcher (plate = y 0, release y ≈ 55 ft), +z = up
//
// Spin vector:  ω = ( -backspin, -gyro, sidespin ) × RPM_TO_RADS
//   → +backspin lifts;  +sidespin moves right (RHP glove-side)
//   Gyro component is along the flight axis and produces no Magnus lift.
//
// Aerodynamic model (Sawicki & Hubbard 2003):
//   S  = r · |ω_perp| / |v|          (spin parameter; ω_perp = ω minus gyro)
//   Cl = cl_slope · S                 if S < cl_break
//      = cl_a + cl_b · S              otherwise
//   F_drag   = -Cd · (ρA/2) · |v|² · v̂
//   F_magnus =  Cl · (ρA/2) · |v|² · (ω_perp_hat × v̂)
//
// Knuckleball wobble: real knuckleball movement comes from seam-induced
// turbulence that this simple aero model doesn't capture. Pitches with very
// low total spin (< KNUCKLE_RPM_THRESHOLD) get a small extra "flutter"
// acceleration (see Wobble) applied only to the FINAL rendered flight — the
// release-direction solver always runs wobble-free so a pitcher can still aim
// a knuckleball, they just can't control exactly where it drifts.

use crate::util::{Vec3, MPH_TO_FPS, RPM_TO_RADS, frand};
use crate::data::{PitchDef, PhysicsCfg, GameCfg};

const SAMPLE_RATE: f32 = 240.0; // samples/second for pitch
pub const BIP_SAMPLE: f32 = 120.0; // samples/second for batted ball

const KNUCKLE_RPM_THRESHOLD: f32 = 350.0;
const KNUCKLE_WOBBLE_AMP: f32 = 5.5; // ft/s^2

// ---------------------------------------------------------------------------
// Wobble (knuckleball flutter)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct Wobble {
    pub amp: f32,
    pub seed_x: f32,
    pub seed_z: f32,
}

impl Wobble {
    pub const NONE: Wobble = Wobble { amp: 0.0, seed_x: 0.0, seed_z: 0.0 };

    /// Random wobble profile for pitches with very low total spin.
    /// Deterministic pitches (normal spin) get Wobble::NONE (zero amplitude).
    pub fn for_pitch(def: &PitchDef) -> Wobble {
        let total_rpm = (def.backspin_rpm.powi(2)
            + def.sidespin_rpm.powi(2)
            + def.gyrospin_rpm.powi(2)).sqrt();
        if total_rpm < KNUCKLE_RPM_THRESHOLD {
            Wobble { amp: KNUCKLE_WOBBLE_AMP, seed_x: frand(0.0, 6.283), seed_z: frand(0.0, 6.283) }
        } else {
            Wobble::NONE
        }
    }
}

// ---------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct PitchFlight {
    pub samples: Vec<Vec3>,   // positions at 1/240 s intervals from release
    pub plate_x: f32,         // crossing position at plate (y = 0)
    pub plate_z: f32,
    pub plate_t: f32,         // time from release to plate plane
    pub speed_mph: f32,
}

impl PitchFlight {
    /// Interpolated world position at time t seconds after release.
    pub fn pos_at(&self, t: f32) -> Vec3 {
        if self.samples.is_empty() { return Vec3::ZERO; }
        let n = self.samples.len();
        let idx_f = (t * SAMPLE_RATE).max(0.0);
        let idx = (idx_f as usize).min(n.saturating_sub(2));
        let frac = idx_f - idx as f32;
        if idx + 1 < n {
            self.samples[idx].lerp(self.samples[idx + 1], frac.min(1.0))
        } else {
            self.samples[n - 1]
        }
    }
}

#[derive(Clone, Debug)]
pub struct BipFlight {
    pub samples: Vec<Vec3>,
    pub land_x: f32,   // feet: + = RF side (first-bounce / catch point; for HRs, the true landing spot, not just the fence-crossing point)
    pub land_y: f32,   // feet from plate toward CF
    pub distance: f32, // distance from plate to first-bounce/landing point (HRs can exceed the fence distance)
    pub hang_time: f32,
    pub is_hr: bool,
    pub is_wall_ball: bool,
    pub bounces: Vec<(f32, Vec3)>, // (time, position) of each bounce after the first
    pub rest: Vec3,                // final resting position (or wall-ball exit point)
    pub duration: f32,             // total animation length in seconds
    pub first_landing_time: f32,   // seconds from contact until land_x/land_y is reached
}

impl BipFlight {
    pub fn pos_at(&self, t: f32) -> Vec3 {
        if self.samples.is_empty() { return Vec3::ZERO; }
        let n = self.samples.len();
        let idx = ((t * BIP_SAMPLE) as usize).min(n - 1);
        self.samples[idx]
    }
}

// ---------------------------------------------------------------------------
// Spin vector helper
// ---------------------------------------------------------------------------

fn spin_vec(def: &PitchDef) -> Vec3 {
    Vec3::new(
        -def.backspin_rpm * RPM_TO_RADS,
        -def.gyrospin_rpm * RPM_TO_RADS,
         def.sidespin_rpm * RPM_TO_RADS,
    )
}

// ---------------------------------------------------------------------------
// Core acceleration function
// ---------------------------------------------------------------------------

fn accel(vel: Vec3, omega: Vec3, phys: &PhysicsCfg, t: f32, wobble: Wobble) -> Vec3 {
    let area = std::f32::consts::PI * phys.ball_radius * phys.ball_radius;
    let rho_a2 = 0.5 * phys.air_density * area;       // ρA/2
    let v_len = vel.length();

    let gravity = Vec3::new(0.0, 0.0, -phys.gravity);

    if v_len < 1e-4 { return gravity; }

    let v_hat = vel * (1.0 / v_len);

    // Remove gyro component of spin (along flight axis → no lift)
    let gyro = omega.dot(v_hat);
    let omega_perp = omega - v_hat * gyro;
    let op_len = omega_perp.length();

    // Drag (opposes velocity)
    let a_drag = vel * (-(rho_a2 * phys.cd / phys.ball_mass) * v_len);

    // Magnus lift
    let a_magnus = if op_len > 1e-4 {
        let s = phys.ball_radius * op_len / v_len;
        let cl = if s < phys.cl_break {
            phys.cl_slope * s
        } else {
            phys.cl_a + phys.cl_b * s
        };
        let op_hat = omega_perp * (1.0 / op_len);
        // ω_perp_hat × v̂  gives correct lift direction (backspin → up)
        let lift_dir = op_hat.cross(v_hat);
        lift_dir * (rho_a2 * cl * v_len * v_len / phys.ball_mass)
    } else {
        Vec3::ZERO
    };

    // Knuckleball flutter: a slow, unpredictable extra acceleration for
    // very-low-spin pitches. Zero-cost (amp = 0) for normal pitches.
    let a_wobble = if wobble.amp > 0.0 {
        Vec3::new(
            wobble.amp * (2.6 * t + wobble.seed_x).sin(),
            0.0,
            wobble.amp * 0.6 * (3.3 * t + wobble.seed_z).sin(),
        )
    } else {
        Vec3::ZERO
    };

    gravity + a_drag + a_magnus + a_wobble
}

// Single Euler step with ground bounce.
fn step(pos: &mut Vec3, vel: &mut Vec3, omega: Vec3, dt: f32, phys: &PhysicsCfg, t: f32, wobble: Wobble) {
    let a = accel(*vel, omega, phys, t, wobble);
    *vel += a * dt;
    *pos += *vel * dt;
    if pos.z < 0.0 && vel.z < 0.0 {
        pos.z = 0.0;
        vel.z *= -0.38;
    }
}

// ---------------------------------------------------------------------------
// Aim solver — iterate to find initial velocity that crosses (aim_x, aim_z)
// ---------------------------------------------------------------------------

/// Quick sim returning (plate_t, plate_x, plate_z); no sample collection.
fn sim_to_plate(v0: Vec3, pos0: Vec3, omega: Vec3, phys: &PhysicsCfg, wobble: Wobble) -> (f32, f32, f32) {
    let dt = phys.sim_dt;
    let mut pos = pos0;
    let mut vel = v0;
    let mut t = 0.0f32;

    while t < 1.5 {
        let prev_pos = pos;
        step(&mut pos, &mut vel, omega, dt, phys, t, wobble);
        t += dt;

        // Detect crossing of plate plane (y = 0)
        if prev_pos.y > 0.0 && pos.y <= 0.0 {
            let frac = prev_pos.y / (prev_pos.y - pos.y);
            return (
                t - dt + dt * frac,
                prev_pos.x + (pos.x - prev_pos.x) * frac,
                prev_pos.z + (pos.z - prev_pos.z) * frac,
            );
        }
    }
    (t, pos.x, pos.z)
}

/// Full sim with sample collection.
fn sim_full(v0: Vec3, pos0: Vec3, omega: Vec3, phys: &PhysicsCfg, wobble: Wobble) -> PitchFlight {
    let dt = phys.sim_dt;
    let sample_dt = 1.0 / SAMPLE_RATE;
    let mut pos = pos0;
    let mut vel = v0;
    let mut t = 0.0f32;
    let mut next_s = 0.0f32;
    let mut samples = Vec::with_capacity(150);
    let mut plate_t = 0.5f32;
    let mut plate_x = 0.0f32;
    let mut plate_z = 2.5f32;
    let mut crossed = false;

    while t < 1.5 {
        let prev_pos = pos;
        step(&mut pos, &mut vel, omega, dt, phys, t, wobble);
        t += dt;

        if t >= next_s { samples.push(pos); next_s += sample_dt; }

        if !crossed && prev_pos.y > 0.0 && pos.y <= 0.0 {
            let frac = prev_pos.y / (prev_pos.y - pos.y);
            plate_t = t - dt + dt * frac;
            plate_x = prev_pos.x + (pos.x - prev_pos.x) * frac;
            plate_z = prev_pos.z + (pos.z - prev_pos.z) * frac;
            crossed = true;
        }

        // Stop collecting once sufficiently past the plate or bounced a while
        if crossed && t > plate_t + 0.4 { break; }
    }

    PitchFlight { samples, plate_x, plate_z, plate_t, speed_mph: v0.length() / MPH_TO_FPS }
}

// ---------------------------------------------------------------------------
// Public simulation entry points
// ---------------------------------------------------------------------------

/// Simulate a pitch aimed EXACTLY at (aim_x, aim_z) — i.e. the solver uses the
/// pitch's own movement, so it always crosses right where aimed (used by the
/// AI pitcher, which "knows" its true intended location).
pub fn simulate_pitch(def: &PitchDef, aim_x: f32, aim_z: f32, phys: &PhysicsCfg) -> PitchFlight {
    let speed = def.velocity_mph * MPH_TO_FPS;
    let omega = spin_vec(def);
    let pos0  = Vec3::new(phys.release_x, phys.release_y, phys.release_z);

    // 3-iteration aim solver: adjust virtual target until ball crosses near (aim_x, aim_z)
    let mut virt = Vec3::new(aim_x, 0.0, aim_z);
    for _ in 0..3 {
        let dir = (virt - pos0).normalize();
        let v0  = dir * speed;
        let (_, cx, cz) = sim_to_plate(v0, pos0, omega, phys, Wobble::NONE);
        virt.x += aim_x - cx;
        virt.z += aim_z - cz;
    }

    let dir = (virt - pos0).normalize();
    let v0  = dir * speed;
    sim_full(v0, pos0, omega, phys, Wobble::for_pitch(def))
}

/// Simulate a pitch released toward (aim_x, aim_z) as if it were a GENERIC
/// straight fastball (90 mph / 2000 rpm backspin, no side/gyro spin). The
/// actual selected pitch is then thrown along that same release direction —
/// its own movement makes it drift away from the aim reticle, just like a
/// real batter/pitcher has to account for break. This is what the user
/// (human pitcher) sees and aims with.
const REF_VELOCITY_MPH: f32 = 90.0;
const REF_BACKSPIN: f32 = 2000.0;

pub fn simulate_pitch_release_aim(def: &PitchDef, aim_x: f32, aim_z: f32, phys: &PhysicsCfg) -> PitchFlight {
    let pos0 = Vec3::new(phys.release_x, phys.release_y, phys.release_z);
    let ref_omega = Vec3::new(-REF_BACKSPIN * RPM_TO_RADS, 0.0, 0.0);
    let ref_speed = REF_VELOCITY_MPH * MPH_TO_FPS;

    // Solve release direction so a GENERIC fastball would cross at (aim_x, aim_z)
    let mut virt = Vec3::new(aim_x, 0.0, aim_z);
    for _ in 0..3 {
        let dir = (virt - pos0).normalize();
        let v0  = dir * ref_speed;
        let (_, cx, cz) = sim_to_plate(v0, pos0, ref_omega, phys, Wobble::NONE);
        virt.x += aim_x - cx;
        virt.z += aim_z - cz;
    }
    let release_dir = (virt - pos0).normalize();

    // Throw the ACTUAL pitch along that same release direction.
    let actual_speed = def.velocity_mph * MPH_TO_FPS;
    let v0_actual = release_dir * actual_speed;
    let actual_omega = spin_vec(def);
    sim_full(v0_actual, pos0, actual_omega, phys, Wobble::for_pitch(def))
}

/// Compute (HB, IVB) in inches for a pitch definition.
/// HB+ = moves right; IVB+ = lifts vs gravity.
pub fn pitch_movement(def: &PitchDef, phys: &PhysicsCfg) -> (f32, f32) {
    let speed = def.velocity_mph * MPH_TO_FPS;
    let pos0  = Vec3::new(phys.release_x, phys.release_y, phys.release_z);
    let target = Vec3::new(0.0, 0.0, 2.5); // aim mid-zone for comparison

    // Solve aim for spin version
    let omega = spin_vec(def);
    let mut virt = target;
    for _ in 0..3 {
        let dir = (virt - pos0).normalize();
        let (_, cx, cz) = sim_to_plate(dir * speed, pos0, omega, phys, Wobble::NONE);
        virt.x += 0.0 - cx;  // aim at x=0
        virt.z += 2.5 - cz;
    }
    let v_spin = (virt - pos0).normalize() * speed;
    let (_, sx, sz) = sim_to_plate(v_spin, pos0, omega, phys, Wobble::NONE);

    // Spinless version (same initial velocity)
    let (_, nx, nz) = sim_to_plate(v_spin, pos0, Vec3::ZERO, phys, Wobble::NONE);

    ((sx - nx) * 12.0, (sz - nz) * 12.0)
}

// ---------------------------------------------------------------------------
// Batted-ball simulation
// ---------------------------------------------------------------------------

const MAX_BOUNCES: usize = 4;
const BOUNCE_RESTITUTION: f32 = 0.40;
const BOUNCE_FRICTION: f32 = 0.55;
const ROLL_STOP_SPEED: f32 = 5.0; // ft/s

/// Simulate a batted ball in play, including bounces and roll-out.
/// Field coords: +x = RF side, +y = toward CF, +z = up.
/// spray_deg: 0 = straight center; positive = RF; negative = LF.
pub fn simulate_bip(
    ev_mph: f32,
    la_deg: f32,
    spray_deg: f32,
    backspin_rpm: f32,
    sidespin_rpm: f32,
    contact_z: f32,
    phys: &PhysicsCfg,
    game: &GameCfg,
) -> BipFlight {
    let ev_fps   = ev_mph * MPH_TO_FPS;
    let la_rad   = la_deg.to_radians();
    let spr_rad  = spray_deg.to_radians();

    // Initial velocity: y = toward CF, x = RF side, z = up
    let vx = ev_fps * la_rad.cos() * spr_rad.sin();
    let vy = ev_fps * la_rad.cos() * spr_rad.cos();
    let vz = ev_fps * la_rad.sin();
    let mut vel = Vec3::new(vx, vy, vz);

    // Spin: backspin in BIP coords → +x axis topspin/backspin
    let mut omega = Vec3::new(
        -backspin_rpm * RPM_TO_RADS,
         0.0,
         sidespin_rpm * RPM_TO_RADS,
    );

    let pos0 = Vec3::new(0.0, 1.5, contact_z);
    let mut pos = pos0;

    let dt = phys.sim_dt;
    let sample_dt = 1.0 / BIP_SAMPLE;
    let mut t   = 0.0f32;
    let mut next_s = 0.0f32;
    let mut samples: Vec<Vec3> = Vec::with_capacity(400);

    // Hang time: how long above launch height (for la > 8)
    let mut hang_start = 0.0f32;
    let mut above_launch = false;
    let mut hang_time = 0.0f32;
    let launch_z = contact_z;

    let mut land_x = 0.0f32;
    let mut land_y = 0.0f32;
    let mut distance = 0.0f32;
    let mut first_landing_time = 0.0f32;
    let mut is_hr = false;
    let mut is_wall_ball = false;
    let mut first_landing_recorded = false;
    let mut bounces: Vec<(f32, Vec3)> = Vec::new();
    let mut rest = pos0;
    let mut done = false;

    while t < 9.0 && !done {
        let prev_pos = pos;

        let a = accel(vel, omega, phys, t, Wobble::NONE);
        vel += a * dt;
        pos += vel * dt;
        t   += dt;

        if t >= next_s { samples.push(pos); next_s += sample_dt; }

        // Hang time tracking (only meaningful before first bounce)
        if !first_landing_recorded && la_deg > 8.0 {
            if !above_launch && pos.z > launch_z + 0.5 {
                above_launch = true;
                hang_start = t;
            }
            if above_launch && pos.z <= launch_z && vel.z < 0.0 {
                hang_time = t - hang_start;
                above_launch = false;
            }
        }

        // Fence check — only needs to fire once. A ball that clears the wall
        // is flagged as a home run but keeps flying: its recorded distance
        // reflects where it would ACTUALLY land (potentially well past the
        // fence), not just the fence-crossing point. A ball that hits the
        // wall below home-run height stops there (wall ball).
        if !first_landing_recorded && !is_hr {
            let r_xy = (pos.x * pos.x + pos.y * pos.y).sqrt();
            if r_xy > 5.0 {
                let angle_deg = (pos.x.abs().atan2(pos.y.max(0.001))).to_degrees();
                let frac_to_line = (angle_deg / 45.0).min(1.0);
                let fence_d = game.fence_cf - (game.fence_cf - game.fence_line) * frac_to_line;
                if r_xy >= fence_d {
                    if pos.z >= game.wall_height {
                        is_hr = true;
                        // No break — let it keep flying to its true landing spot.
                    } else {
                        is_wall_ball = true;
                        land_x = pos.x;
                        land_y = pos.y;
                        distance = r_xy;
                        first_landing_time = t;
                        rest = pos;
                        first_landing_recorded = true;
                        done = true;
                        break;
                    }
                }
            }
        }

        // Ground contact (z crossed 0): bounce, don't stop. No minimum-time
        // guard here — a hard-topped ball can legitimately hit the dirt
        // within the first few milliseconds of contact, and skipping that
        // check let such balls "sink" through the ground undetected.
        if prev_pos.z >= 0.0 && pos.z < 0.0 {
            let frac = prev_pos.z / (prev_pos.z - pos.z);
            let contact_pt = Vec3::new(
                prev_pos.x + (pos.x - prev_pos.x) * frac,
                prev_pos.y + (pos.y - prev_pos.y) * frac,
                0.0,
            );
            pos = contact_pt;

            if !first_landing_recorded {
                land_x = contact_pt.x;
                land_y = contact_pt.y;
                distance = (land_x * land_x + land_y * land_y).sqrt();
                first_landing_time = t;
                first_landing_recorded = true;
            } else {
                bounces.push((t, contact_pt));
            }

            // Bounce dynamics: energy loss on impact
            vel.z = -vel.z * BOUNCE_RESTITUTION;
            vel.x *= BOUNCE_FRICTION;
            vel.y *= BOUNCE_FRICTION;
            omega *= 0.5; // spin bleeds off each bounce so it doesn't keep "lifting"

            let horiz_speed = (vel.x * vel.x + vel.y * vel.y).sqrt();
            let bounce_count = bounces.len();

            if bounce_count >= MAX_BOUNCES || (horiz_speed < ROLL_STOP_SPEED && vel.z.abs() < 4.0) {
                rest = Vec3::new(pos.x, pos.y, 0.0);
                done = true;
                break;
            }
        }
    }

    if !done {
        rest = Vec3::new(pos.x, pos.y, 0.0);
        if !first_landing_recorded {
            land_x = pos.x; land_y = pos.y;
            distance = (land_x * land_x + land_y * land_y).sqrt();
            first_landing_time = t;
        }
    }

    let duration = samples.len() as f32 / BIP_SAMPLE;

    BipFlight {
        samples, land_x, land_y, distance, hang_time, is_hr, is_wall_ball,
        bounces, rest, duration, first_landing_time,
    }
}
