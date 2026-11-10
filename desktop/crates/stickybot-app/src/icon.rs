//! The menu bar icon: the dashboard's brand mark (the stick with two eyes), drawn as a template
//! image so it follows light / dark mode; the eyes show the state. The same shapes as the Python
//! app's `menubar.py` `_draw_mark` (AppKit points, y up, 18 x 18 pt), rasterised here at 2x.

/// How the mark looks (menubar.py `STYLES`' values).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Style {
    /// ready: the outline and two eyes
    Eyes,
    /// listening: a filled stick, the eyes cut out
    Filled,
    /// transcribing / thinking / working: eyes looking up
    Up,
    /// error: two little crosses
    X,
    /// paused: eyes closed (two dashes)
    Closed,
    /// no stick / searching / starting: a dashed outline, no eyes
    Dashed,
    /// a permission prompt waits: the eyes and a dot top right
    Badge,
}

/// menubar.py `STYLES`: the agent's state (or `approval` while a prompt waits) -> the mark.
pub fn style(state: &str) -> Style {
    match state {
        "ready" => Style::Eyes,
        "listening" => Style::Filled,
        "transcribing" | "thinking" | "working" => Style::Up,
        "error" => Style::X,
        "paused" => Style::Closed,
        "approval" => Style::Badge,
        _ => Style::Dashed, // no-stick, searching, starting, anything new
    }
}

pub const SCALE: usize = 2;
pub const SIZE: usize = 18 * SCALE;
const SUB: usize = 4; // 4 x 4 samples a pixel

// the body: a rounded rect (4.6, 1.2, 8.8 x 15.6), radius 3.4, line 1.4
const BODY: (f64, f64, f64, f64) = (4.6, 1.2, 8.8, 15.6);
const BODY_R: f64 = 3.4;
const LINE: f64 = 1.4;
const DASH: (f64, f64) = (2.2, 1.6);

/// Signed distance to a rounded rect `(x, y, w, h)` with radius `r` (< 0 inside).
fn rrect(px: f64, py: f64, (x, y, w, h): (f64, f64, f64, f64), r: f64) -> f64 {
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let qx = (px - cx).abs() - (w / 2.0 - r);
    let qy = (py - cy).abs() - (h / 2.0 - r);
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

/// How far along the body's outline the point nearest (px, py) is (counter-clockwise from the
/// bottom of the right side), for the dashes.
fn along_body(px: f64, py: f64) -> f64 {
    use std::f64::consts::{FRAC_PI_2, PI};
    let (x, y, w, h) = BODY;
    let r = BODY_R;
    let (qx, qy) = (px - (x + w / 2.0), py - (y + h / 2.0));
    let (ix, iy) = (w / 2.0 - r, h / 2.0 - r);
    let (side, arc, top) = (2.0 * iy, r * FRAC_PI_2, 2.0 * ix);
    let angle = |cx: f64, cy: f64| {
        let a = (qy - cy).atan2(qx - cx);
        if a < 0.0 { a + 2.0 * PI } else { a }
    };
    match (qx.abs() > ix, qy.abs() > iy) {
        (true, true) => {
            let a = angle(ix.copysign(qx), iy.copysign(qy));
            match (qx > 0.0, qy > 0.0) {
                (true, true) => side + r * a,
                (false, true) => side + arc + top + r * (a - FRAC_PI_2),
                (false, false) => 2.0 * side + 2.0 * arc + top + r * (a - PI),
                (true, false) => 2.0 * side + 3.0 * arc + 2.0 * top + r * (a - 3.0 * FRAC_PI_2),
            }
        }
        (true, false) if qx > 0.0 => qy + iy,
        (true, false) => side + 2.0 * arc + top + (iy - qy), // the left side, top to bottom
        (false, _) if qy > 0.0 => side + arc + (ix - qx),
        (false, _) => 2.0 * side + 3.0 * arc + top + (qx + ix),
    }
}

/// Distance from (px, py) to the segment a-b, `None` past its ends (butt caps).
fn segment(px: f64, py: f64, a: (f64, f64), b: (f64, f64)) -> Option<f64> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = ((px - a.0) * dx + (py - a.1) * dy) / (dx * dx + dy * dy);
    (0.0..=1.0).contains(&t).then(|| (px - a.0 - t * dx).hypot(py - a.1 - t * dy))
}

/// Is the point (AppKit points, y up) ink?
fn ink(style: Style, px: f64, py: f64) -> bool {
    let d = rrect(px, py, BODY, BODY_R);
    let body = match style {
        Style::Filled => d <= 0.0,
        Style::Dashed => d.abs() <= LINE / 2.0 && along_body(px, py).rem_euclid(DASH.0 + DASH.1) < DASH.0,
        _ => d.abs() <= LINE / 2.0,
    };
    let eye_y = if style == Style::Up { 10.6 } else { 8.6 };
    let eyes = matches!(style, Style::Eyes | Style::Filled | Style::Up | Style::Badge)
        && [6.9, 9.6].iter().any(|&x| rrect(px, py, (x, eye_y, 1.6, 3.6), 0.8) <= 0.0);
    let closed = style == Style::Closed
        && [6.6, 9.4].iter().any(|&x| (x..=x + 2.2).contains(&px) && (10.0..=11.1).contains(&py));
    let cross = style == Style::X
        && [7.7, 10.4].iter().any(|&cx| {
            [-1.0, 1.0].iter().any(|&dx: &f64| {
                segment(px, py, (cx - 1.0, 9.4 - dx), (cx + 1.0, 9.4 + dx)).is_some_and(|d| d <= 0.5)
            })
        });
    let badge = style == Style::Badge && (px - 15.0).hypot(py - 15.0) <= 2.6;
    if style == Style::Filled {
        return body && !eyes; // the eyes are cut out of the filled stick
    }
    body || eyes || closed || cross || badge
}

/// The mark as `SIZE` x `SIZE` RGBA (black, the shape in the alpha; a template image).
pub fn rgba(style: Style) -> Vec<u8> {
    let mut out = vec![0u8; SIZE * SIZE * 4];
    for row in 0..SIZE {
        for col in 0..SIZE {
            let mut hits = 0;
            for j in 0..SUB {
                for i in 0..SUB {
                    let x = (col as f64 + (i as f64 + 0.5) / SUB as f64) / SCALE as f64;
                    let y = 18.0 - (row as f64 + (j as f64 + 0.5) / SUB as f64) / SCALE as f64;
                    hits += usize::from(ink(style, x, y));
                }
            }
            out[(row * SIZE + col) * 4 + 3] = (hits * 255 / (SUB * SUB)) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The alpha at a point given in AppKit points (y up).
    fn alpha(img: &[u8], x: f64, y: f64) -> u8 {
        let (col, row) = ((x * SCALE as f64) as usize, ((18.0 - y) * SCALE as f64) as usize);
        img[(row * SIZE + col) * 4 + 3]
    }

    #[test]
    fn states_map_to_the_python_marks() {
        assert_eq!(style("ready"), Style::Eyes);
        assert_eq!(style("listening"), Style::Filled);
        assert_eq!(style("thinking"), Style::Up);
        assert_eq!(style("paused"), Style::Closed);
        assert_eq!(style("approval"), Style::Badge);
        for s in ["no-stick", "searching", "starting", "something-new"] {
            assert_eq!(style(s), Style::Dashed);
        }
    }

    #[test]
    fn the_marks_show_the_state() {
        let eyes = rgba(Style::Eyes);
        assert_eq!(eyes.len(), SIZE * SIZE * 4);
        assert_eq!(alpha(&eyes, 7.7, 10.4), 255, "an eye");
        assert_eq!(alpha(&eyes, 9.0, 4.0), 0, "inside the outline is empty");
        assert_eq!(alpha(&eyes, 4.6, 9.0), 255, "the outline");
        let filled = rgba(Style::Filled);
        assert_eq!((alpha(&filled, 9.0, 4.0), alpha(&filled, 7.7, 10.4)), (255, 0), "filled, eyes cut out");
        let up = rgba(Style::Up);
        assert_eq!((alpha(&up, 7.7, 9.0), alpha(&up, 7.7, 13.0)), (0, 255), "eyes up");
        let closed = rgba(Style::Closed);
        assert_eq!((alpha(&closed, 7.7, 10.5), alpha(&closed, 7.7, 12.0)), (255, 0));
        let badge = rgba(Style::Badge);
        assert_eq!(alpha(&badge, 15.0, 15.0), 255);
        assert_eq!(alpha(&eyes, 15.5, 15.5), 0);
        let x = rgba(Style::X);
        assert_eq!(alpha(&x, 7.7, 9.4), 255, "a cross's middle");
    }

    #[test]
    fn the_dashed_outline_has_gaps() {
        let img = rgba(Style::Dashed);
        // walk up the left side of the outline: some ink, some gaps
        let on: Vec<bool> = (0..40).map(|i| alpha(&img, 4.6, 5.0 + i as f64 * 0.2) > 128).collect();
        assert!(on.iter().any(|&b| b) && on.iter().any(|&b| !b));
        assert_eq!(alpha(&img, 7.7, 10.4), 0, "no eyes");
        let solid = rgba(Style::Eyes);
        assert!((0..40).all(|i| alpha(&solid, 4.6, 5.0 + i as f64 * 0.2) > 128));
    }

    #[test]
    fn the_dash_position_runs_round_the_outline() {
        // each side continues where the corner before it ended
        let (x, y, w, h) = BODY;
        let (l, r, b, t) = (x, x + w, y, y + h);
        let pts = [
            (r, y + h / 2.0),
            (x + w / 2.0, t),
            (l, y + h / 2.0),
            (x + w / 2.0, b),
        ];
        let s: Vec<f64> = pts.iter().map(|&(px, py)| along_body(px, py)).collect();
        assert!(s.windows(2).all(|p| p[0] < p[1]), "{s:?}");
        let total = 2.0 * (w - 2.0 * BODY_R) + 2.0 * (h - 2.0 * BODY_R) + 2.0 * std::f64::consts::PI * BODY_R;
        assert!(s[3] < total);
    }
}
