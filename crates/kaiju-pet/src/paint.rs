//! Procedural kaiju: drawn from egui shapes so it can blink, wiggle, breathe and
//! react to state — a flat sprite can't animate its parts. Uses only circles,
//! convex polygons and line segments (no version-fragile rounded-rect API).

use std::f64::consts::PI;

use egui::{pos2, vec2, Color32, Painter, Pos2, Rect, Shape, Stroke};

use crate::reducer::VisualState;

struct Palette {
    body: Color32,
    body_dk: Color32,
    belly: Color32,
    spike: Color32,
    glint: Color32,
}

fn palette(v: VisualState) -> Palette {
    // Base kaiju green, shifted by state. Color is supplemental to the text label.
    let (body, spike, glint) = match v {
        VisualState::Error => (
            Color32::from_rgb(206, 96, 96),
            Color32::from_rgb(255, 170, 120),
            Color32::from_rgb(255, 210, 180),
        ),
        VisualState::Limited => (
            Color32::from_rgb(150, 122, 220),
            Color32::from_rgb(200, 180, 255),
            Color32::from_rgb(225, 215, 255),
        ),
        VisualState::Disconnected => (
            Color32::from_rgb(120, 130, 140),
            Color32::from_rgb(160, 170, 180),
            Color32::from_rgb(200, 208, 214),
        ),
        VisualState::Celebrate => (
            Color32::from_rgb(104, 214, 138),
            Color32::from_rgb(150, 245, 200),
            Color32::from_rgb(210, 255, 240),
        ),
        _ => (
            Color32::from_rgb(88, 186, 120),
            Color32::from_rgb(120, 224, 200),
            Color32::from_rgb(150, 240, 240),
        ),
    };
    Palette {
        body,
        body_dk: darken(body, 0.78),
        belly: lighten(body, 1.35),
        spike,
        glint,
    }
}

fn darken(c: Color32, f: f32) -> Color32 {
    Color32::from_rgb(
        (c.r() as f32 * f) as u8,
        (c.g() as f32 * f) as u8,
        (c.b() as f32 * f) as u8,
    )
}

fn lighten(c: Color32, f: f32) -> Color32 {
    Color32::from_rgb(
        (c.r() as f32 * f).min(245.0) as u8,
        (c.g() as f32 * f).min(248.0) as u8,
        (c.b() as f32 * f).min(245.0) as u8,
    )
}

fn tri(painter: &Painter, a: Pos2, b: Pos2, c: Pos2, fill: Color32) {
    painter.add(Shape::convex_polygon(vec![a, b, c], fill, Stroke::NONE));
}

/// Draw the animated kaiju centered in `rect`. `time` is elapsed seconds.
pub fn paint_kaiju(painter: &Painter, rect: Rect, time: f64, visual: VisualState, reduced: bool) {
    let s = rect.width() / 120.0;
    let p = palette(visual);
    let calm = visual.is_calm();
    let t = if reduced { 0.0 } else { time };

    // Whole-body idle motion.
    let (bob_amp, bob_speed) = if calm { (3.0, 1.6) } else { (6.0, 3.4) };
    let bob = (t * bob_speed).sin() * bob_amp;
    let breathe = ((t * 2.0).sin() * 2.0) as f32 * s;
    let shake_x = if visual == VisualState::Error {
        ((t * 40.0).sin() * 1.6) as f32 * s
    } else {
        0.0
    };
    let cx = rect.center().x + shake_x;
    let cy = rect.center().y + bob as f32 * s;
    let c = |dx: f32, dy: f32| pos2(cx + dx * s, cy + dy * s);

    // Eye blink: open factor 1 normally, dips to 0 briefly and periodically.
    let blink = {
        let period = if calm { 4.5 } else { 2.6 };
        let ph = (t / period).fract();
        if ph > 0.92 {
            let b = (ph - 0.92) / 0.08;
            (1.0 - (b * PI).sin()).max(0.0)
        } else {
            1.0
        }
    };
    // Arm wiggle and spike shimmer phases.
    let arm = ((t * if calm { 3.0 } else { 6.5 }).sin() * 3.0) as f32 * s;

    // --- back to front ---
    // tail
    tri(painter, c(30.0, 22.0), c(52.0, 30.0), c(30.0, 2.0), p.body_dk);
    painter.circle_filled(c(30.0, 18.0), 10.0 * s, p.body_dk);
    // legs
    painter.circle_filled(c(-14.0, 40.0), 11.0 * s, p.body_dk);
    painter.circle_filled(c(14.0, 40.0), 11.0 * s, p.body_dk);
    // body + breathing
    painter.circle_filled(pos2(cx, cy + 12.0 * s), 40.0 * s + breathe, p.body);
    painter.circle_filled(c(0.0, 22.0), 24.0 * s, p.belly);
    // arms (wiggle in opposite phase)
    painter.circle_filled(c(-34.0, 6.0 + arm / s), 9.0 * s, p.body_dk);
    painter.circle_filled(c(34.0, 6.0 - arm / s), 9.0 * s, p.body_dk);
    // head
    painter.circle_filled(c(0.0, -20.0), 34.0 * s, p.body);
    // dorsal + head spikes (shimmer color, slight wiggle)
    for (i, dx) in [-30.0f32, -8.0, 14.0, 34.0].iter().enumerate() {
        let w = ((t * 3.0 + i as f64).sin() * 1.5) as f32 * s;
        let base_y = if *dx < 6.0 { -46.0 } else { -30.0 };
        let sc = lerp_color(p.spike, p.glint, (0.5 + 0.5 * (t * 4.0 + i as f64).sin()) as f32);
        tri(
            painter,
            c(*dx, base_y),
            c(*dx + 16.0, base_y + 14.0),
            c(*dx - 2.0 + w, base_y + 16.0),
            sc,
        );
    }
    // eyes
    let eye_dark = Color32::from_rgb(28, 40, 34);
    for (ex, look) in [(-14.0f32, 1.0f32), (14.0, 1.0)] {
        let eye = c(ex, -22.0);
        if blink > 0.32 {
            painter.circle_filled(eye, 12.0 * s, Color32::from_rgb(240, 248, 240));
            // pupil drifts slightly toward the viewer when attention is needed.
            let drift = if calm { 0.0 } else { (t * 1.5).sin() as f32 * 2.0 };
            let pupil = pos2(eye.x + (drift + look - 1.0) * s, eye.y + 1.0 * s);
            painter.circle_filled(pupil, 6.0 * s, eye_dark);
            painter.circle_filled(pos2(pupil.x - 2.0 * s, pupil.y - 2.0 * s), 2.2 * s, p.glint);
        } else {
            // closed eye: a short dark line.
            painter.line_segment(
                [pos2(eye.x - 9.0 * s, eye.y), pos2(eye.x + 9.0 * s, eye.y)],
                Stroke::new(2.5 * s, eye_dark),
            );
        }
    }
    // mouth: a small smile (or flat when error/disconnected)
    let mouth_y = -6.0;
    if matches!(visual, VisualState::Error | VisualState::Disconnected) {
        painter.line_segment(
            [c(-8.0, mouth_y), c(8.0, mouth_y)],
            Stroke::new(2.2 * s, eye_dark),
        );
    } else {
        painter.add(Shape::line(
            vec![c(-9.0, mouth_y), c(0.0, mouth_y + 4.0), c(9.0, mouth_y)],
            Stroke::new(2.2 * s, eye_dark),
        ));
    }
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

// Kept so callers can size the paint box consistently.
pub const PET_BOX: (f32, f32) = (112.0, 120.0);

#[allow(unused_imports)]
use vec2 as _vec2; // vec2 reserved for future limb transforms
