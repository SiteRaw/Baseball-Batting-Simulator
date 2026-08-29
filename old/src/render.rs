// 3D perspective renderer (behind-catcher view via pinhole projection).
//
// World: +x = catcher's right, +y = toward pitcher (plate=0, mound≈60.5), +z = up.
// Camera: sits at (0, -cam_back, cam_z), looking toward +y.
// Projection: sx = cx + x * focal/(y - cam_y), sy = horizon - (z - cam_z) * focal/(y - cam_y)

use macroquad::prelude::*;
use crate::util::Vec3 as V3;
use crate::data::GameCfg;

const PI2: f32 = std::f32::consts::PI * 2.0;

// ---------------------------------------------------------------------------
// View (recomputed each frame from screen size)
// ---------------------------------------------------------------------------

pub struct View {
    pub cam_y:   f32,
    pub cam_z:   f32,
    pub focal:   f32,
    pub horizon: f32,
    pub cx:      f32,
}

impl View {
    pub fn new(game: &GameCfg) -> Self {
        let h = screen_height();
        let w = screen_width();
        Self {
            cam_y:   -game.cam_back,
            cam_z:   game.cam_height,
            focal:   game.focal,
            horizon: h * 0.40,
            cx:      w * 0.50,
        }
    }

    /// Project a world point to screen (sx, sy).
    #[inline]
    pub fn project(&self, p: V3) -> (f32, f32) {
        let s  = self.focal / (p.y - self.cam_y);
        let sx = self.cx + p.x * s;
        let sy = self.horizon - (p.z - self.cam_z) * s;
        (sx, sy)
    }

    /// Project and also return scale factor s (useful for sizing circles).
    #[inline]
    pub fn project_s(&self, p: V3) -> (f32, f32, f32) {
        let s  = self.focal / (p.y - self.cam_y);
        (self.cx + p.x * s, self.horizon - (p.z - self.cam_z) * s, s)
    }

    /// Convert screen coordinates back to world (x, z) at given world_y plane.
    pub fn screen_to_world(&self, mx: f32, my: f32, world_y: f32) -> (f32, f32) {
        let s  = self.focal / (world_y - self.cam_y);
        let wx = (mx - self.cx) / s;
        let wz = self.cam_z - (my - self.horizon) / s;
        (wx, wz)
    }
}

// ---------------------------------------------------------------------------
// Line and polygon helpers
// ---------------------------------------------------------------------------

fn line3(v: &View, a: V3, b: V3, color: Color, thick: f32) {
    let (ax, ay) = v.project(a);
    let (bx, by) = v.project(b);
    draw_line(ax, ay, bx, by, thick, color);
}

fn fill_poly3_ground(v: &View, pts_w: &[V3], color: Color) {
    // Fan triangulation from first point
    if pts_w.len() < 3 { return; }
    let a = pts_w[0];
    let (ax, ay) = v.project(a);
    let pa = Vec2::new(ax, ay);
    for i in 1..pts_w.len().saturating_sub(1) {
        let (bx, by) = v.project(pts_w[i]);
        let (cx, cy) = v.project(pts_w[i + 1]);
        draw_triangle(pa, Vec2::new(bx, by), Vec2::new(cx, cy), color);
    }
}

fn ground_disk(v: &View, cx_w: f32, cy_w: f32, r_w: f32, color: Color, segs: usize) {
    let pts: Vec<V3> = (0..segs)
        .map(|i| {
            let a = PI2 * i as f32 / segs as f32;
            V3::new(cx_w + r_w * a.cos(), cy_w + r_w * a.sin(), 0.0)
        })
        .collect();
    let (scx, scy) = v.project(V3::new(cx_w, cy_w, 0.0));
    let ctr = Vec2::new(scx, scy);
    for i in 0..segs {
        let (ax, ay) = v.project(pts[i]);
        let (bx, by) = v.project(pts[(i + 1) % segs]);
        draw_triangle(ctr, Vec2::new(ax, ay), Vec2::new(bx, by), color);
    }
}

/// Draw an ellipse in screen space (for PCI and crosshair).
pub fn screen_ellipse(cx: f32, cy: f32, rx: f32, ry: f32, thick: f32, color: Color) {
    let segs = 40;
    let mut prev = (cx + rx, cy);
    for i in 1..=segs {
        let a = PI2 * i as f32 / segs as f32;
        let pt = (cx + rx * a.cos(), cy + ry * a.sin());
        draw_line(prev.0, prev.1, pt.0, pt.1, thick, color);
        prev = pt;
    }
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let h = h.rem_euclid(1.0) * 6.0;
    let i = h.floor() as i32;
    let f = h - h.floor();
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    match i.rem_euclid(6) {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    }
}

// ---------------------------------------------------------------------------
// Public draw_scene
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub fn draw_scene(
    view: &View,
    game: &GameCfg,
    ball_pos: Option<V3>,
    pci_world: Option<(f32, f32)>,        // (x, z) in world space on plate plane
    batter_rx: f32,
    batter_rz: f32,
    trail: &[V3],
    bip_trail: &[V3],
    bounce_points: &[V3],
    landing: Option<V3>,
    show_zone: bool,
) {
    let w = screen_width();
    let h = screen_height();

    // Sky
    let sky = Color::from_rgba(80, 120, 190, 255);
    draw_rectangle(0.0, 0.0, w, view.horizon, sky);

    // Ground zoning, near-to-far isn't the point here — it's concentric from
    // home plate outward: infield dirt (drawn below) sits inside a green
    // outfield fan that runs out to the fence, and everything beyond the
    // fence (foul territory corners, behind the wall) reverts to a plain
    // dirt/soil tone so the fence line reads as a clear boundary.
    let outside_dirt = Color::from_rgba(120, 95, 65, 255);
    draw_rectangle(0.0, view.horizon, w, h - view.horizon, outside_dirt);

    // Outfield grass — filled fan from home plate out to the fence contour.
    {
        let segs = 32usize;
        let mut pts: Vec<V3> = Vec::with_capacity(segs + 2);
        pts.push(V3::ZERO);
        for i in 0..=segs {
            let frac = i as f32 / segs as f32;
            let angle = (-45.0f32 + 90.0 * frac).to_radians();
            let fd = game.fence_cf - (game.fence_cf - game.fence_line) * (angle.abs() / (45.0f32.to_radians()));
            pts.push(V3::new(fd * angle.sin(), fd * angle.cos(), 0.0));
        }
        fill_poly3_ground(view, &pts, Color::from_rgba(40, 110, 45, 255));
    }

    // Infield dirt — big circle around mound
    let dirt = Color::from_rgba(180, 135, 75, 255);
    ground_disk(view, 0.0, 30.5, 100.0, dirt, 48);

    // Pitcher's mound
    let mound = Color::from_rgba(210, 160, 90, 255);
    ground_disk(view, 0.0, 60.5, 9.5, mound, 32);

    // Foul lines
    {
        let len = 360.0f32;
        let ang = 45.0f32.to_radians();
        let rf = V3::new( len * ang.sin(), len * ang.cos(), 0.0);
        let lf = V3::new(-len * ang.sin(), len * ang.cos(), 0.0);
        let hp = V3::ZERO;
        line3(view, hp, rf, WHITE, 1.0);
        line3(view, hp, lf, WHITE, 1.0);
    }

    // Outfield wall arc + vertical face
    {
        let segs = 24usize;
        let mut prev_top: Option<V3> = None;
        let mut prev_bot: Option<V3> = None;
        for i in 0..=segs {
            let frac = i as f32 / segs as f32;
            let angle = (-45.0f32 + 90.0 * frac).to_radians();
            let d = game.fence_cf - (game.fence_cf - game.fence_line) * (angle.abs() / (45.0f32.to_radians()));
            let wx = d * angle.sin();
            let wy = d * angle.cos();
            let top = V3::new(wx, wy, game.wall_height);
            let bot = V3::new(wx, wy, 0.0);
            let wall_col = Color::from_rgba(45, 60, 175, 255);
            if let (Some(pt), Some(pb)) = (prev_top, prev_bot) {
                line3(view, pt, top, wall_col, 2.0);
                // Simple 2-triangle face
                let (ax, ay) = view.project(pt);
                let (bx, by) = view.project(top);
                let (cx, cy) = view.project(bot);
                let (dx, dy) = view.project(pb);
                let face = Color::from_rgba(30, 40, 140, 200);
                draw_triangle(Vec2::new(ax, ay), Vec2::new(bx, by), Vec2::new(cx, cy), face);
                draw_triangle(Vec2::new(ax, ay), Vec2::new(cx, cy), Vec2::new(dx, dy), face);
            }
            line3(view, bot, top, Color::from_rgba(45, 60, 175, 255), 2.0);
            prev_top = Some(top);
            prev_bot = Some(bot);
        }
    }

    // Home plate (pentagon, lying flat)
    {
        const PW: f32 = 0.708;
        let plate_pts = [
            V3::new(-PW, 0.0, 0.01), V3::new(-PW, 1.42, 0.01),
            V3::new(  0.0, 1.85, 0.01), V3::new( PW, 1.42, 0.01),
            V3::new( PW, 0.0, 0.01),
        ];
        fill_poly3_ground(view, &plate_pts, Color::from_rgba(235, 235, 235, 255));
        for i in 0..plate_pts.len() {
            line3(view, plate_pts[i], plate_pts[(i + 1) % plate_pts.len()], GRAY, 1.0);
        }
    }

    // Infield bases — simple squares at their real 90-ft-spacing positions,
    // with a faint baseline diamond, so ground balls / liners are easy to
    // place relative to 1B/2B/3B.
    draw_bases(view);

    // Strike zone box, drawn at its true configured dimensions. The called-
    // strike decision (outcomes::in_zone) allows the ball to touch the zone,
    // not just have its center inside it — expanding the DECISION by one
    // ball radius rather than shrinking the drawn box keeps the visual at
    // least as generous as the real call, so a pitch that looks like it
    // clearly missed can never be called a strike.
    if show_zone {
        let zw = game.zone_half_w;
        let zb = game.zone_bottom;
        let zt = game.zone_top;
        let y0 = 1.0f32;    // slightly in front of plate
        let zone = [
            V3::new(-zw, y0, zb), V3::new( zw, y0, zb),
            V3::new( zw, y0, zt), V3::new(-zw, y0, zt),
        ];
        // Filled semi-transparent quad
        let fill = Color::from_rgba(255, 255, 255, 22);
        let (ax, ay) = view.project(zone[0]);
        let (bx, by) = view.project(zone[1]);
        let (cx, cy) = view.project(zone[2]);
        let (dx, dy) = view.project(zone[3]);
        draw_triangle(Vec2::new(ax, ay), Vec2::new(bx, by), Vec2::new(cx, cy), fill);
        draw_triangle(Vec2::new(ax, ay), Vec2::new(cx, cy), Vec2::new(dx, dy), fill);
        // Outline
        let line_col = Color::from_rgba(200, 200, 200, 160);
        for i in 0..4 {
            line3(view, zone[i], zone[(i + 1) % 4], line_col, 1.5);
        }
    }

    // Pitch trail (pre-contact, shown after result on a take/whiff/foul)
    for (i, &pt) in trail.iter().enumerate() {
        let alpha = ((i as f32 / trail.len().max(1) as f32) * 200.0) as u8;
        let col = Color::from_rgba(255, 200, 50, alpha);
        let (sx, sy) = view.project(pt);
        draw_circle(sx, sy, 2.5, col);
    }

    // Batted-ball trail (post-contact flight/bounce path)
    for (i, &pt) in bip_trail.iter().enumerate() {
        let alpha = (60.0 + (i as f32 / bip_trail.len().max(1) as f32) * 170.0) as u8;
        let col = Color::from_rgba(235, 235, 215, alpha);
        let (sx, sy) = view.project(pt);
        draw_circle(sx, sy, 2.2, col);
    }

    // Bounce markers (small dust puffs where the ball has hit the ground)
    for &pt in bounce_points {
        let (sx, sy) = view.project(pt);
        draw_circle(sx, sy, 5.0, Color::from_rgba(200, 180, 140, 130));
        draw_circle_lines(sx, sy, 7.0, 1.2, Color::from_rgba(220, 200, 160, 90));
    }

    // Ball
    if let Some(bpos) = ball_pos {
        let (sx, sy, s) = view.project_s(bpos);
        let r = (s * 0.145).max(3.0).min(14.0);
        // Ground shadow
        let (gx, gy) = view.project(V3::new(bpos.x, bpos.y, 0.0));
        draw_circle(gx, gy, r * 0.55, Color::from_rgba(0, 0, 0, 70));
        // Ball body
        draw_circle(sx, sy, r, Color::from_rgba(230, 230, 220, 255));
        draw_circle_lines(sx, sy, r, 1.2, Color::from_rgba(190, 50, 40, 200));
    }

    // PCI (batting cursor) — tints toward orange/red the further it reaches
    // outside the strike zone (mirrors the power/reach penalty in outcomes.rs).
    if let Some((px_w, pz_w)) = pci_world {
        let y_face = 0.8f32; // slightly in front of plate
        let (cx2, cy2) = view.project(V3::new(px_w, y_face, pz_w));
        let (rx_sx, rx_sy) = view.project(V3::new(px_w + batter_rx, y_face, pz_w));
        let (rz_sx, rz_sy) = view.project(V3::new(px_w, y_face, pz_w + batter_rz));
        let scr_rx = ((rx_sx - cx2).powi(2) + (rx_sy - cy2).powi(2)).sqrt();
        let scr_rz = ((rz_sx - cx2).powi(2) + (rz_sy - cy2).powi(2)).sqrt();

        let dx = (px_w.abs() - game.zone_half_w).max(0.0);
        let dz = ((game.zone_bottom - pz_w).max(0.0)).max((pz_w - game.zone_top).max(0.0));
        let reach = (dx * dx + dz * dz).sqrt().min(2.5) / 2.5; // 0..1

        let pci_col = Color::from_rgba(
            (50.0 + 180.0 * reach) as u8,
            (255.0 - 140.0 * reach) as u8,
            (110.0 - 70.0 * reach) as u8,
            210,
        );
        screen_ellipse(cx2, cy2, scr_rx, scr_rz, 2.0, pci_col);
        let ghost = Color::from_rgba(pci_col.r as u8, pci_col.g as u8, pci_col.b as u8, 90);
        draw_line(cx2 - scr_rx * 1.4, cy2, cx2 + scr_rx * 1.4, cy2, 1.0, ghost);
        draw_line(cx2, cy2 - scr_rz * 1.6, cx2, cy2 + scr_rz * 1.6, 1.0, ghost);
    }

    // Landing / rest marker (X ring)
    if let Some(land) = landing {
        let (lx, ly) = view.project(land);
        draw_circle_lines(lx, ly, 9.0, 2.5, Color::from_rgba(255, 90, 40, 220));
    }
}

fn draw_bases(view: &View) {
    let ang = 45.0f32.to_radians();
    let d = 90.0f32; // MLB base spacing, feet
    let first  = V3::new( d * ang.sin(),  d * ang.cos(), 0.02);
    let third  = V3::new(-d * ang.sin(),  d * ang.cos(), 0.02);
    let second = V3::new(0.0, d * std::f32::consts::SQRT_2, 0.02);
    let home   = V3::new(0.0, 0.0, 0.02);

    // Faint baseline diamond
    let line_col = Color::from_rgba(225, 225, 225, 130);
    line3(view, home, first, line_col, 1.5);
    line3(view, first, second, line_col, 1.5);
    line3(view, second, third, line_col, 1.5);
    line3(view, third, home, line_col, 1.5);

    // Base markers (small diamonds, like real bases)
    let base_col = Color::from_rgba(240, 240, 235, 255);
    for &pos in &[first, second, third] {
        draw_base_square(view, pos, base_col);
    }
}

fn draw_base_square(view: &View, center: V3, color: Color) {
    let half = 0.75f32; // ~1.5 ft base, corner-to-corner along the baseline
    let pts = [
        center + V3::new(half, 0.0, 0.0),
        center + V3::new(0.0, half, 0.0),
        center + V3::new(-half, 0.0, 0.0),
        center + V3::new(0.0, -half, 0.0),
    ];
    let (ax, ay) = view.project(pts[0]);
    let (bx, by) = view.project(pts[1]);
    let (cx, cy) = view.project(pts[2]);
    let (dx, dy) = view.project(pts[3]);
    draw_triangle(Vec2::new(ax, ay), Vec2::new(bx, by), Vec2::new(cx, cy), color);
    draw_triangle(Vec2::new(ax, ay), Vec2::new(cx, cy), Vec2::new(dx, dy), color);
}

// ---------------------------------------------------------------------------
// Aim crosshair (when user is pitching) — shows where a generic straight
// fastball would cross; the actually-selected pitch will drift from this
// point according to its own movement.
// ---------------------------------------------------------------------------

pub fn draw_aim_crosshair(view: &View, aim_x: f32, aim_z: f32) {
    let y_face = 0.8f32;
    let (cx2, cy2) = view.project(V3::new(aim_x, y_face, aim_z));
    let col = Color::from_rgba(255, 240, 60, 220);
    draw_circle_lines(cx2, cy2, 8.0, 2.0, col);
    draw_line(cx2 - 14.0, cy2, cx2 + 14.0, cy2, 1.5, col);
    draw_line(cx2, cy2 - 14.0, cx2, cy2 + 14.0, 1.5, col);
}

// ---------------------------------------------------------------------------
// Predicted-landing preview — a light, low-opacity green marker + short
// approach trail showing where THIS pitch's own movement will actually carry
// it, alongside the yellow "generic straight fastball" aim reticle. Helps
// the pitcher gauge how much a breaking ball will drift off their aim point
// without being visually overwhelming (kept subtle via low alpha).
// ---------------------------------------------------------------------------

pub fn draw_predicted_landing(view: &View, trail_pts: &[V3], landing: V3) {
    let n = trail_pts.len().max(1);
    for (i, &pt) in trail_pts.iter().enumerate() {
        let alpha = (25.0 + (i as f32 / n as f32) * 85.0) as u8;
        let (sx, sy) = view.project(pt);
        draw_circle(sx, sy, 2.0, Color::from_rgba(80, 230, 120, alpha));
    }
    let (lx, ly) = view.project(landing);
    draw_circle_lines(lx, ly, 7.0, 1.6, Color::from_rgba(80, 230, 120, 140));
    draw_circle(lx, ly, 2.2, Color::from_rgba(130, 255, 160, 160));
}

// ---------------------------------------------------------------------------
// Release-point FX — two stages. A calm "get ready" pulse appears early in
// the windup (distinct blue/steady style), then crossfades into the sharp
// multicolor pulse right before release. Caller controls timing/blending by
// passing independent alphas for each stage.
// ---------------------------------------------------------------------------

/// Stage 1: early "get ready" warning — a slow breathing cyan ring.
pub fn draw_release_fx_warning(view: &View, release_pos: V3, alpha: f32) {
    if alpha <= 0.0 { return; }
    let (sx, sy, s) = view.project_s(release_pos);
    let base_r = (s * 0.5).max(5.0).min(30.0);
    let t = get_time() as f32;
    let pulse = 0.5 + 0.5 * (t * 3.0).sin();
    let ring_r = base_r * (0.85 + 0.5 * pulse);
    let col = Color::new(0.35, 0.75, 1.0, alpha * (0.35 + 0.25 * pulse));
    draw_circle_lines(sx, sy, ring_r, 2.5, col);
    draw_circle(sx, sy, base_r * 0.3, Color::new(0.6, 0.85, 1.0, alpha * 0.35));
}

/// Stage 2: release-imminent — a sharp multicolor pulse in the final instant.
pub fn draw_release_fx(view: &View, release_pos: V3, progress: f32) {
    if progress <= 0.0 { return; }
    let (sx, sy, s) = view.project_s(release_pos);
    let base_r = (s * 0.5).max(5.0).min(30.0);
    let t = get_time() as f32;

    for i in 0..5 {
        let hue = (t * 0.5 + i as f32 * 0.2).fract();
        let (r, g, b) = hsv_to_rgb(hue, 0.85, 1.0);
        let ang = t * 8.0 + i as f32 * 1.2566;
        let off = 3.0 + 9.0 * progress;
        let ox = sx + off * ang.cos();
        let oy = sy + off * ang.sin();
        draw_circle(ox, oy, base_r * 0.35, Color::new(r, g, b, 0.55 * progress));
    }
    draw_circle(sx, sy, base_r * 0.55, Color::new(1.0, 1.0, 1.0, 0.30 * progress));
}

// ---------------------------------------------------------------------------
// Swing animation — quick screen-space bat swing overlay triggered on swing.
// ---------------------------------------------------------------------------

pub fn draw_swing_animation(progress: f32) {
    let w = screen_width();
    let h = screen_height();
    let pivot = Vec2::new(w * 0.5 + 70.0, h - 90.0);
    let ease = 1.0 - (1.0 - progress).powi(3);
    let start_deg = -80.0f32;
    let end_deg   = 100.0f32;
    let bat_len   = 140.0f32;

    // Motion-blur trailing copies
    for i in 1..=3 {
        let back = i as f32 * 0.09;
        let pe = (progress - back).max(0.0);
        let pease = 1.0 - (1.0 - pe).powi(3);
        let a2 = (start_deg + (end_deg - start_deg) * pease).to_radians();
        let tip2 = pivot + Vec2::new(a2.cos(), a2.sin()) * bat_len;
        let alpha = (0.22 - i as f32 * 0.06).max(0.0);
        draw_line(pivot.x, pivot.y, tip2.x, tip2.y, 6.0, Color::new(1.0, 1.0, 1.0, alpha));
    }

    let angle = (start_deg + (end_deg - start_deg) * ease).to_radians();
    let tip = pivot + Vec2::new(angle.cos(), angle.sin()) * bat_len;

    draw_line(pivot.x, pivot.y, tip.x, tip.y, 7.0, Color::from_rgba(175, 128, 70, 255));
    draw_circle(tip.x, tip.y, 6.0, Color::from_rgba(205, 160, 100, 255));
    draw_circle(pivot.x, pivot.y, 5.5, Color::from_rgba(50, 50, 55, 255));
}

// ---------------------------------------------------------------------------
// Aerial mini-map (batting mode) — bottom-right corner, shows a spray chart
// of where batted balls have ended up. Pure screen-space, independent of the
// main behind-catcher View.
// ---------------------------------------------------------------------------

pub fn draw_minimap(game: &GameCfg, landing: Option<(f32, f32)>) {
    let size = 150.0f32;
    let margin = 14.0f32;
    let w = screen_width();
    let h = screen_height();
    let x0 = w - size - margin;
    let y0 = h - size - margin;

    draw_rectangle(x0, y0, size, size, Color::from_rgba(15, 20, 15, 190));
    draw_rectangle_lines(x0, y0, size, size, 2.0, Color::from_rgba(200, 200, 200, 160));

    let plate = Vec2::new(x0 + size * 0.5, y0 + size * 0.92);
    let max_dist = game.fence_cf * 1.05;
    let scale = (size * 0.85) / max_dist;
    let ang = 45.0f32.to_radians();

    // Foul lines
    let line_col = Color::from_rgba(210, 210, 210, 150);
    let lf = plate + Vec2::new(-ang.sin(), -ang.cos()) * (max_dist * scale);
    let rf = plate + Vec2::new( ang.sin(), -ang.cos()) * (max_dist * scale);
    draw_line(plate.x, plate.y, lf.x, lf.y, 1.0, line_col);
    draw_line(plate.x, plate.y, rf.x, rf.y, 1.0, line_col);

    // Fence arc
    let segs = 16;
    let mut prev: Option<Vec2> = None;
    for i in 0..=segs {
        let frac = i as f32 / segs as f32;
        let a = (-45.0 + 90.0 * frac).to_radians();
        let fd = game.fence_cf - (game.fence_cf - game.fence_line) * (a.abs() / ang);
        let p = plate + Vec2::new(fd * a.sin(), -fd * a.cos()) * scale;
        if let Some(pp) = prev {
            draw_line(pp.x, pp.y, p.x, p.y, 1.0, Color::from_rgba(120, 150, 220, 180));
        }
        prev = Some(p);
    }

    // Home plate marker
    draw_circle(plate.x, plate.y, 2.5, WHITE);

    // Landing dot for the most recent ball in play — clamped to the box so
    // a very deep home run (now tracked to its true landing spot) still
    // shows at the edge rather than rendering outside the panel.
    if let Some((wx, wy)) = landing {
        let d = (wx * wx + wy * wy).sqrt().max(0.001);
        let clamp_scale = (max_dist / d).min(1.0);
        let p = plate + Vec2::new(wx, -wy) * (scale * clamp_scale);
        draw_circle(p.x, p.y, 3.5, Color::from_rgba(235, 60, 60, 235));
        draw_circle_lines(p.x, p.y, 5.5, 1.2, Color::from_rgba(255, 140, 140, 160));
    }

    draw_text("Last Ball In Play", x0 + 4.0, y0 + 12.0, 12.0, Color::from_rgba(210, 210, 210, 200));
}

// ---------------------------------------------------------------------------
// Outcome celebration FX — full-screen overlays for strikeout (fire) and
// walk (ice storm). Purely cosmetic; toggle via game.txt [fx] section.
// progress: 0..1 over the effect's lifetime (quick fade-in, slower fade-out).
// ---------------------------------------------------------------------------

fn fx_fade(progress: f32) -> f32 {
    if progress < 0.15 { progress / 0.15 } else { ((1.0 - progress) / 0.85).max(0.0) }
}

pub fn draw_strikeout_fx(progress: f32) {
    let fade = fx_fade(progress);
    if fade <= 0.0 { return; }

    let w = screen_width();
    let h = screen_height();
    let t = get_time() as f32;

    draw_rectangle(0.0, 0.0, w, h, Color::new(0.85, 0.22, 0.05, 0.10 * fade));

    let n = 22;
    for i in 0..n {
        let seed = i as f32 * 12.9898;
        let x0 = (seed.sin() * 0.5 + 0.5) * w;
        let phase = (seed * 1.7).fract() * PI2;
        let speed = 60.0 + (seed * 3.1).fract() * 45.0;
        let y = h - ((t * speed + phase * 25.0) % (h + 80.0));
        let flicker = 0.5 + 0.5 * (t * 9.0 + phase).sin();
        let size = (5.0 + 9.0 * flicker) * (0.4 + 0.6 * fade);
        let hue = 0.02 + 0.07 * flicker; // red -> orange flicker
        let (r, g, b) = hsv_to_rgb(hue, 0.92, 1.0);
        let dx = (t * 2.2 + phase).sin() * 10.0;
        draw_circle(x0 + dx, y, size, Color::new(r, g, b, 0.55 * fade * (0.5 + 0.5 * flicker)));
    }

    if progress < 0.2 {
        let flash = (1.0 - progress / 0.2).max(0.0);
        draw_rectangle(0.0, 0.0, w, h, Color::new(1.0, 0.6, 0.2, 0.18 * flash));
    }
}

pub fn draw_walk_fx(progress: f32) {
    let fade = fx_fade(progress);
    if fade <= 0.0 { return; }

    let w = screen_width();
    let h = screen_height();
    let t = get_time() as f32;

    draw_rectangle(0.0, 0.0, w, h, Color::new(0.55, 0.75, 1.0, 0.10 * fade));

    let n = 26;
    for i in 0..n {
        let seed = i as f32 * 7.233;
        let x0 = (seed.sin() * 0.5 + 0.5) * w;
        let phase = (seed * 2.3).fract() * PI2;
        let speed = 40.0 + (seed * 1.7).fract() * 50.0;
        let y = (t * speed + phase * 30.0) % (h + 60.0) - 30.0;
        let drift = (t * 1.5 + phase).sin() * 14.0;
        let size = 2.5 + (seed * 5.1).fract() * 3.0;
        draw_circle(x0 + drift, y, size, Color::new(0.85, 0.93, 1.0, 0.7 * fade));
    }

    // Frost creeping in from top and bottom edges
    let band = 40.0 * fade;
    let frost = Color::new(0.8, 0.92, 1.0, 0.14 * fade);
    draw_rectangle(0.0, 0.0, w, band, frost);
    draw_rectangle(0.0, h - band, w, band, frost);
}
