//! Path segments, clip snapshots, gradient paint and style enums.

/// Max `save()` depth per onPaint call.
pub const MAX_SAVE_DEPTH: usize = 16;
/// Max path segments copied into one fill/stroke/clip op.
pub const MAX_PATH_SEGS_PER_OP: usize = 256;
/// Max path segments copied into recorded ops for one frame.
pub const MAX_PATH_SEGS_PER_FRAME: usize = 2048;
/// Max live gradient handles per onPaint call.
pub const MAX_GRADIENTS: usize = 32;
/// Max color stops on one gradient handle / snapshot.
pub const MAX_GRADIENT_STOPS: usize = 16;
/// Max intersecting clip paths snapshotted onto one draw.
pub const MAX_CLIP_PATHS: usize = 8;
/// Max accepted `lineWidth` (assignment above this fails the frame).
pub const MAX_LINE_WIDTH: f32 = 256.0;
/// Max accepted `shadowBlur` (assignment above this fails the frame).
pub const MAX_SHADOW_BLUR: f32 = 32.0;

/// One path command. Coordinates are applet-space `f32` (finite at record).
#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum PathSeg {
    MoveTo {
        x: f32,
        y: f32,
    },
    LineTo {
        x: f32,
        y: f32,
    },
    QuadTo {
        cx: f32,
        cy: f32,
        x: f32,
        y: f32,
    },
    CubicTo {
        c1x: f32,
        c1y: f32,
        c2x: f32,
        c2y: f32,
        x: f32,
        y: f32,
    },
    Close,
}

impl Eq for PathSeg {}

/// One clip path in a per-draw intersection list.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ClipPath {
    pub segs: Vec<PathSeg>,
}

impl Eq for ClipPath {}

/// Gradient color stop. `offset` is in `[0, 1]`; `color` is `0xRRGGBBAA`.
#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct GradStop {
    pub offset: f32,
    pub color: u32,
}

impl Eq for GradStop {}

/// Snapshotted fill that is not the op's solid `color` field.
#[derive(Debug, Clone, PartialEq, Default, serde::Deserialize, serde::Serialize)]
pub enum FillPaint {
    #[default]
    Solid,
    Linear {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        stops: Vec<GradStop>,
    },
    Radial {
        x0: f32,
        y0: f32,
        r0: f32,
        x1: f32,
        y1: f32,
        r1: f32,
        stops: Vec<GradStop>,
    },
}

impl Eq for FillPaint {}

/// Drop-shadow snapshot. Alpha-0 `color` means the shadow is inactive.
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Deserialize, serde::Serialize)]
pub struct Shadow {
    pub color: u32,
    pub blur: f32,
    pub offset_x: f32,
    pub offset_y: f32,
}

impl Eq for Shadow {}

impl Shadow {
    pub fn active(self) -> bool {
        (self.color & 0xff) != 0
    }
}

/// Per-draw clip, shadow and non-solid fill. Default matches old buffers.
#[derive(Debug, Clone, PartialEq, Default, serde::Deserialize, serde::Serialize)]
pub struct DrawExtras {
    #[serde(default)]
    pub clips: Vec<ClipPath>,
    #[serde(default)]
    pub shadow: Shadow,
    #[serde(default)]
    pub fill: FillPaint,
}

impl Eq for DrawExtras {}

impl DrawExtras {
    pub fn is_default(&self) -> bool {
        self.clips.is_empty() && !self.shadow.active() && matches!(self.fill, FillPaint::Solid)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize, serde::Serialize)]
#[repr(u8)]
pub enum LineJoinKind {
    #[default]
    Miter = 0,
    Round = 1,
    Bevel = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize, serde::Serialize)]
#[repr(u8)]
pub enum TextAlign {
    #[default]
    Left = 0,
    Center = 1,
    Right = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize, serde::Serialize)]
#[repr(u8)]
pub enum TextBaseline {
    #[default]
    Alphabetic = 0,
    Top = 1,
    Middle = 2,
    Bottom = 3,
}

pub fn finite_f32(v: f64) -> Option<f32> {
    if v.is_finite() {
        Some(v as f32)
    } else {
        None
    }
}

/// Conservative axis-aligned bounds of path points and control points.
pub fn path_bounds(segs: &[PathSeg]) -> Option<(f32, f32, f32, f32)> {
    let mut minx = f32::MAX;
    let mut miny = f32::MAX;
    let mut maxx = f32::MIN;
    let mut maxy = f32::MIN;
    let mut any = false;
    let mut include = |x: f32, y: f32| {
        if !x.is_finite() || !y.is_finite() {
            return;
        }
        any = true;
        minx = minx.min(x);
        miny = miny.min(y);
        maxx = maxx.max(x);
        maxy = maxy.max(y);
    };
    for seg in segs {
        match *seg {
            PathSeg::MoveTo { x, y } | PathSeg::LineTo { x, y } => include(x, y),
            PathSeg::QuadTo { cx, cy, x, y } => {
                include(cx, cy);
                include(x, y);
            }
            PathSeg::CubicTo {
                c1x,
                c1y,
                c2x,
                c2y,
                x,
                y,
            } => {
                include(c1x, c1y);
                include(c2x, c2y);
                include(x, y);
            }
            PathSeg::Close => {}
        }
    }
    if any {
        Some((minx, miny, maxx, maxy))
    } else {
        None
    }
}

/// Append an HTML `arc`. Full circles (`|end-start| >= 2π`) use four cubics.
/// Partial arcs use cubic approximations in the requested direction.
/// Returns `false` if a segment would exceed `max_len` (path stays ≤ `max_len`).
pub fn append_arc(
    path: &mut Vec<PathSeg>,
    cx: f32,
    cy: f32,
    r: f32,
    start: f32,
    end: f32,
    ccw: bool,
    max_len: usize,
) -> bool {
    if !(cx.is_finite() && cy.is_finite() && r.is_finite() && start.is_finite() && end.is_finite())
    {
        return true;
    }
    if r <= 0.0 {
        return true;
    }
    let push = |path: &mut Vec<PathSeg>, seg: PathSeg| -> bool {
        if path.len() >= max_len {
            return false;
        }
        path.push(seg);
        true
    };
    let tau = std::f32::consts::TAU;
    let sx = cx + r * start.cos();
    let sy = cy + r * start.sin();
    let start_seg = if path.is_empty() {
        PathSeg::MoveTo { x: sx, y: sy }
    } else {
        PathSeg::LineTo { x: sx, y: sy }
    };
    if !push(path, start_seg) {
        return false;
    }
    let mut sweep = if ccw {
        let mut d = end - start;
        if d >= 0.0 {
            d -= tau * ((d / tau).floor() + 1.0);
        }
        if (end - start).abs() >= tau - 1e-4 {
            tau
        } else {
            d + tau
        }
    } else {
        let mut d = end - start;
        if d <= 0.0 {
            d += tau * ((-d / tau).floor() + 1.0);
        }
        if (end - start).abs() >= tau - 1e-4 {
            tau
        } else {
            d
        }
    };
    if ccw {
        sweep = -sweep.abs();
        if (end - start).abs() >= tau - 1e-4 {
            sweep = -tau;
        }
    } else if (end - start).abs() >= tau - 1e-4 {
        sweep = tau;
    }
    // Split into <=90° cubic segments.
    let n = ((sweep.abs() / (std::f32::consts::FRAC_PI_2) - 1e-4).ceil() as i32).clamp(1, 8);
    let step = sweep / n as f32;
    let mut a = start;
    for _ in 0..n {
        let a1 = a + step;
        if !append_arc_cubic(path, cx, cy, r, a, a1, max_len) {
            return false;
        }
        a = a1;
    }
    true
}

fn append_arc_cubic(
    path: &mut Vec<PathSeg>,
    cx: f32,
    cy: f32,
    r: f32,
    a0: f32,
    a1: f32,
    max_len: usize,
) -> bool {
    let da = a1 - a0;
    // 4/3 * tan(da/4)
    let k = (4.0 / 3.0) * (da / 4.0).tan();
    let (s0, c0) = (a0.sin(), a0.cos());
    let (s1, c1) = (a1.sin(), a1.cos());
    let x0 = cx + r * c0;
    let y0 = cy + r * s0;
    let x1 = cx + r * c1;
    let y1 = cy + r * s1;
    let c1x = x0 - k * r * s0;
    let c1y = y0 + k * r * c0;
    let c2x = x1 + k * r * s1;
    let c2y = y1 - k * r * c1;
    let _ = x0;
    let _ = y0;
    if path.len() >= max_len {
        return false;
    }
    path.push(PathSeg::CubicTo {
        c1x,
        c1y,
        c2x,
        c2y,
        x: x1,
        y: y1,
    });
    true
}

/// Two-circle conical parameter `t` for point `(px,py)`.
///
/// Circle(t) has centre `C0 + t (C1-C0)` and radius `r0 + t (r1-r0)`.
/// tiny-skia's `RadialGradient` drops the start radius; this keeps it.
pub fn conical_t(px: f32, py: f32, x0: f32, y0: f32, r0: f32, x1: f32, y1: f32, r1: f32) -> f32 {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let dr = r1 - r0;
    let fx = px - x0;
    let fy = py - y0;
    let a = dx * dx + dy * dy - dr * dr;
    let b = -2.0 * (fx * dx + fy * dy + r0 * dr);
    let c = fx * fx + fy * fy - r0 * r0;
    const EPS: f32 = 1e-6;
    let r_at = |t: f32| r0 + t * dr;
    let pick = |t0: f32, t1: f32| -> f32 {
        let v0 = r_at(t0) >= -EPS;
        let v1 = r_at(t1) >= -EPS;
        match (v0, v1) {
            (true, true) => {
                let in0 = (0.0..=1.0).contains(&t0);
                let in1 = (0.0..=1.0).contains(&t1);
                if in0 && !in1 {
                    t0
                } else if in1 && !in0 {
                    t1
                } else if dr >= 0.0 {
                    t0.max(t1)
                } else {
                    t0.min(t1)
                }
            }
            (true, false) => t0,
            (false, true) => t1,
            (false, false) => 0.0,
        }
    };
    if a.abs() < EPS {
        if b.abs() < EPS {
            return 0.0;
        }
        return -c / b;
    }
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return 0.0;
    }
    let s = disc.sqrt();
    let t0 = (-b - s) / (2.0 * a);
    let t1 = (-b + s) / (2.0 * a);
    pick(t0, t1)
}

pub fn sample_stops(stops: &[GradStop], t: f32) -> u32 {
    if stops.is_empty() {
        return 0;
    }
    let t = t.clamp(0.0, 1.0);
    let mut idx: Vec<usize> = (0..stops.len()).collect();
    idx.sort_by(|&a, &b| {
        stops[a]
            .offset
            .partial_cmp(&stops[b].offset)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let first = stops[idx[0]];
    if t <= first.offset {
        return first.color;
    }
    let last = stops[*idx.last().expect("non-empty")];
    if t >= last.offset {
        return last.color;
    }
    for w in idx.windows(2) {
        let a = stops[w[0]];
        let b = stops[w[1]];
        if t >= a.offset && t <= b.offset {
            let span = b.offset - a.offset;
            let u = if span.abs() < 1e-8 {
                0.0
            } else {
                (t - a.offset) / span
            };
            return lerp_color(a.color, b.color, u);
        }
    }
    last.color
}

fn lerp_color(a: u32, b: u32, u: f32) -> u32 {
    let ua = u.clamp(0.0, 1.0);
    let ch = |shift: u32| -> u8 {
        let ca = ((a >> shift) & 0xff) as f32;
        let cb = ((b >> shift) & 0xff) as f32;
        (ca + (cb - ca) * ua).round().clamp(0.0, 255.0) as u8
    };
    (u32::from(ch(24)) << 24)
        | (u32::from(ch(16)) << 16)
        | (u32::from(ch(8)) << 8)
        | u32::from(ch(0))
}

pub fn segs_to_skia(segs: &[PathSeg]) -> Option<tiny_skia::Path> {
    let mut b = tiny_skia::PathBuilder::new();
    let mut started = false;
    for seg in segs {
        match *seg {
            PathSeg::MoveTo { x, y } => {
                b.move_to(x, y);
                started = true;
            }
            PathSeg::LineTo { x, y } => {
                if started {
                    b.line_to(x, y);
                } else {
                    b.move_to(x, y);
                    started = true;
                }
            }
            PathSeg::QuadTo { cx, cy, x, y } => {
                if started {
                    b.quad_to(cx, cy, x, y);
                }
            }
            PathSeg::CubicTo {
                c1x,
                c1y,
                c2x,
                c2y,
                x,
                y,
            } => {
                if started {
                    b.cubic_to(c1x, c1y, c2x, c2y, x, y);
                }
            }
            PathSeg::Close => {
                if started {
                    b.close();
                }
            }
        }
    }
    b.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conical_inner_radius_is_not_dropped() {
        // Seal geometry: inner circle r=6 offset from outer.
        let x0 = 10.0;
        let y0 = 10.0;
        let r0 = 6.0;
        let x1 = 14.0;
        let y1 = 14.0;
        let r1 = 40.0;
        let t_inner = conical_t(x0 + r0, y0, x0, y0, r0, x1, y1, r1);
        let t_outer = conical_t(x1 + r1, y1, x0, y0, r0, x1, y1, r1);
        assert!(
            t_inner < 0.35,
            "point on inner circle must stay near t=0, got {t_inner}"
        );
        assert!(
            t_outer > 0.7,
            "point on outer circle must stay near t=1, got {t_outer}"
        );
        assert!((t_outer - t_inner).abs() > 0.3);
    }

    #[test]
    fn sample_stops_interpolates() {
        let stops = [
            GradStop {
                offset: 0.0,
                color: 0xff0000ff,
            },
            GradStop {
                offset: 1.0,
                color: 0x00ff00ff,
            },
        ];
        let mid = sample_stops(&stops, 0.5);
        let r = (mid >> 24) as u8;
        let g = (mid >> 16) as u8;
        assert!(r > 80 && r < 180, "r={r}");
        assert!(g > 80 && g < 180, "g={g}");
    }
}
