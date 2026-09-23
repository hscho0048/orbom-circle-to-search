use crate::art::smoothstep;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Area {
    pub fn between(a: Point, b: Point, width: i32, height: i32) -> Self {
        let x = a.x.min(b.x).clamp(0, width);
        let y = a.y.min(b.y).clamp(0, height);
        Self {
            x,
            y,
            w: a.x.max(b.x).clamp(0, width) - x,
            h: a.y.max(b.y).clamp(0, height) - y,
        }
    }

    pub fn valid(self) -> bool {
        self.w >= 8 && self.h >= 8
    }

    pub fn contains(self, p: Point) -> bool {
        p.x >= self.x && p.x < self.x + self.w && p.y >= self.y && p.y < self.y + self.h
    }

    pub fn adjust(&mut self, dx: i32, dy: i32, resize: bool, width: i32, height: i32) {
        if resize {
            self.w = (self.w + dx).clamp(8, width - self.x);
            self.h = (self.h + dy).clamp(8, height - self.y);
        } else {
            self.x = (self.x + dx).clamp(0, width - self.w);
            self.y = (self.y + dy).clamp(0, height - self.h);
        }
    }
}

pub fn bounds(points: &[Point], width: i32, height: i32) -> Option<Area> {
    let first = *points.first()?;
    let mut a = first;
    let mut b = first;
    for p in points {
        a.x = a.x.min(p.x);
        a.y = a.y.min(p.y);
        b.x = b.x.max(p.x);
        b.y = b.y.max(p.y);
    }
    Some(Area::between(a, b, width, height))
}

/// Handwriting-style correction for the displayed stroke: resample by arc length, then apply a
/// gaussian along the path. Crop bounds keep using the raw points. Open strokes keep both
/// endpoints (the live end stays under the cursor); closed strokes are smoothed across the seam.
pub fn smooth_stroke(points: &[Point], closed: bool, sigma: f32) -> Vec<(f32, f32)> {
    const SPACING: f32 = 3.0;
    let raw: Vec<(f32, f32)> = points.iter().map(|p| (p.x as f32, p.y as f32)).collect();
    if raw.len() < 3 {
        return raw;
    }
    let mut samples = vec![raw[0]];
    let mut carry = 0.0;
    let segments = raw.len() - 1 + closed as usize;
    for i in 0..segments {
        let a = raw[i];
        let b = raw[(i + 1) % raw.len()];
        let length = (b.0 - a.0).hypot(b.1 - a.1);
        let mut at = SPACING - carry;
        while at <= length {
            let t = at / length;
            samples.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
            at += SPACING;
        }
        carry = length - (at - SPACING);
    }
    if closed {
        if samples.len() > 1 && carry < SPACING * 0.5 {
            samples.pop();
        }
    } else if carry > 0.01 {
        samples.push(*raw.last().unwrap());
    }
    let n = samples.len();
    let steps = sigma / SPACING;
    if n < 3 || steps < 0.1 {
        return samples;
    }
    let reach = (steps * 3.0).ceil() as usize;
    let kernel: Vec<f32> = (0..=reach)
        .map(|k| (-(k as f32 * k as f32) / (2.0 * steps * steps)).exp())
        .collect();
    (0..n)
        .map(|i| {
            // Symmetric truncation keeps open ends pinned without pulling them inward.
            let r = if closed {
                reach.min((n - 1) / 2)
            } else {
                reach.min(i).min(n - 1 - i)
            };
            let (mut x, mut y, mut total) = (samples[i].0, samples[i].1, 1.0);
            for k in 1..=r {
                let w = kernel[k];
                let before = samples[(i + n - k) % n];
                let after = samples[(i + k) % n];
                x += (before.0 + after.0) * w;
                y += (before.1 + after.1) * w;
                total += 2.0 * w;
            }
            (x / total, y / total)
        })
        .collect()
}

/// Cyclic palette shared with the orb, eased between stops so no hue band looks harder than another.
pub fn ink_rgb(phase: f32) -> (u8, u8, u8) {
    let [r, g, b] = crate::art::palette_at(phase);
    (r.round() as u8, g.round() as u8, b.round() as u8)
}

/// Paints a neon stroke into a top-down BGRA buffer within `clip` (left, top, right, bottom).
/// Every pixel is shaded from its exact distance to the path, so the core is anti-aliased and
/// the glow falls off as a continuous gaussian instead of stacked hard-edged pens.
pub fn render_ink(
    pixels: &mut [u8],
    width: i32,
    height: i32,
    clip: (i32, i32, i32, i32),
    points: &[(f32, f32)],
    scale: f32,
    phase: f32,
) {
    if points.len() < 2 || pixels.len() != width as usize * height as usize * 4 {
        return;
    }
    let reach = 20.0 * scale;
    // Longer chords cut the overlapping per-segment work; on a smoothed curve the chord error at
    // this spacing stays well under a pixel.
    let step = 7.0 * scale;
    let mut sparse = vec![points[0]];
    for &p in &points[1..points.len() - 1] {
        let last = *sparse.last().unwrap();
        if (p.0 - last.0).hypot(p.1 - last.1) >= step {
            sparse.push(p);
        }
    }
    sparse.push(points[points.len() - 1]);
    let points = &sparse[..];
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for &(x, y) in points {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    let left = clip.0.max(0).max((min_x - reach).floor() as i32);
    let top = clip.1.max(0).max((min_y - reach).floor() as i32);
    let right = clip.2.min(width).min((max_x + reach).ceil() as i32 + 1);
    let bottom = clip.3.min(height).min((max_y + reach).ceil() as i32 + 1);
    if left >= right || top >= bottom {
        return;
    }
    let w = (right - left) as usize;
    let reach2 = reach * reach;
    thread_local! {
        static SCRATCH: std::cell::RefCell<(Vec<f32>, Vec<f32>)> = Default::default();
    }
    SCRATCH.with_borrow_mut(|(distance, along)| {
        let cells = w * (bottom - top) as usize;
        distance.clear();
        distance.resize(cells, reach2);
        along.clear();
        along.resize(cells, 0.0);
        shade_ink(
            pixels,
            width,
            points,
            scale,
            phase,
            (left, top, right, bottom),
            reach,
            distance,
            along,
        );
    });
}

#[allow(clippy::too_many_arguments)]
fn shade_ink(
    pixels: &mut [u8],
    width: i32,
    points: &[(f32, f32)],
    scale: f32,
    phase: f32,
    (left, top, right, bottom): (i32, i32, i32, i32),
    reach: f32,
    distance: &mut [f32],
    along: &mut [f32],
) {
    let w = (right - left) as usize;
    let reach2 = reach * reach;
    let mut lengths = Vec::with_capacity(points.len());
    let mut total = 0.0;
    lengths.push(0.0);
    for pair in points.windows(2) {
        total += (pair[1].0 - pair[0].0).hypot(pair[1].1 - pair[0].1);
        lengths.push(total);
    }
    let total = total.max(1.0);
    for (i, pair) in points.windows(2).enumerate() {
        let (a, b) = (pair[0], pair[1]);
        let x0 = left.max((a.0.min(b.0) - reach).floor() as i32);
        let x1 = right.min((a.0.max(b.0) + reach).ceil() as i32 + 1);
        let y0 = top.max((a.1.min(b.1) - reach).floor() as i32);
        let y1 = bottom.min((a.1.max(b.1) + reach).ceil() as i32 + 1);
        if x0 >= x1 || y0 >= y1 {
            continue;
        }
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let inverse = 1.0 / (dx * dx + dy * dy).max(1e-6);
        let (ua, ub) = (lengths[i] / total, lengths[i + 1] / total);
        for y in y0..y1 {
            let py = y as f32 + 0.5 - a.1;
            let row = (y - top) as usize * w;
            for x in x0..x1 {
                let px = x as f32 + 0.5 - a.0;
                let t = ((px * dx + py * dy) * inverse).clamp(0.0, 1.0);
                let (ex, ey) = (px - dx * t, py - dy * t);
                let d2 = ex * ex + ey * ey;
                let cell = row + (x - left) as usize;
                if d2 < distance[cell] {
                    distance[cell] = d2;
                    along[cell] = ua + (ub - ua) * t;
                }
            }
        }
    }
    let core = 2.0 * scale;
    let feather = 0.85 * scale.max(1.0);
    let inner = 5.0 * scale;
    let outer = 12.0 * scale;
    let hot = 1.1 * scale;
    for y in top..bottom {
        let row = (y - top) as usize * w;
        for x in left..right {
            let cell = row + (x - left) as usize;
            if distance[cell] >= reach2 {
                continue;
            }
            let d = distance[cell].sqrt();
            let body = 1.0 - smoothstep(core - feather, core + feather, d);
            let glow = (0.46 * (-(d / inner).powi(2)).exp() + 0.20 * (-(d / outer).powi(2)).exp())
                * (1.0 - smoothstep(reach * 0.6, reach, d));
            let alpha = (body + glow * (1.0 - body)).min(1.0);
            if alpha < 0.003 {
                continue;
            }
            // A slight drift along the stroke, like the orb's blobs, around one shared color.
            let (r, g, b) = ink_rgb(phase + along[cell] * 0.1);
            let white = 0.5 * (-(d / hot).powi(2)).exp();
            let index = (y as usize * width as usize + x as usize) * 4;
            for (offset, value) in [(0, b), (1, g), (2, r)] {
                let lit = value as f32 + (255.0 - value as f32) * white;
                let dst = pixels[index + offset] as f32;
                pixels[index + offset] = (dst + (lit - dst) * alpha).round() as u8;
            }
        }
    }
}

/// Adaptive low-pass filter: quiet hand jitter is reduced, fast gestures stay responsive.
pub struct StrokeSmoother {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
}

impl StrokeSmoother {
    pub fn new(start: Point) -> Self {
        Self {
            x: start.x as f32,
            y: start.y as f32,
            vx: 0.0,
            vy: 0.0,
        }
    }

    pub fn update(&mut self, target: Point, elapsed_seconds: f32) -> Point {
        let dt = elapsed_seconds.clamp(1.0 / 240.0, 0.05);
        let alpha = |cutoff: f32| 1.0 - (-std::f32::consts::TAU * cutoff * dt).exp();
        let derivative_alpha = alpha(1.0);
        self.vx += (target.x as f32 - self.x) / dt * derivative_alpha - self.vx * derivative_alpha;
        self.vy += (target.y as f32 - self.y) / dt * derivative_alpha - self.vy * derivative_alpha;
        let speed = self.vx.hypot(self.vy);
        let position_alpha = alpha(2.2 + 0.045 * speed);
        self.x += (target.x as f32 - self.x) * position_alpha;
        self.y += (target.y as f32 - self.y) * position_alpha;
        Point {
            x: self.x.round() as i32,
            y: self.y.round() as i32,
        }
    }
}

/// A top-down 32-bit DIB, suitable for CF_DIB and BMP file output.
pub fn crop_dib(pixels: &[u8], width: i32, height: i32, area: Area) -> Option<Vec<u8>> {
    if !area.valid()
        || area.x < 0
        || area.y < 0
        || area.x + area.w > width
        || area.y + area.h > height
        || pixels.len() != width as usize * height as usize * 4
    {
        return None;
    }
    let size = area.w as usize * area.h as usize * 4;
    let mut out = vec![0u8; 40 + size];
    out[0..4].copy_from_slice(&40u32.to_le_bytes());
    out[4..8].copy_from_slice(&area.w.to_le_bytes());
    out[8..12].copy_from_slice(&(-area.h).to_le_bytes());
    out[12..14].copy_from_slice(&1u16.to_le_bytes());
    out[14..16].copy_from_slice(&32u16.to_le_bytes());
    out[20..24].copy_from_slice(&(size as u32).to_le_bytes());
    for row in 0..area.h as usize {
        let from = ((area.y as usize + row) * width as usize + area.x as usize) * 4;
        let to = 40 + row * area.w as usize * 4;
        out[to..to + area.w as usize * 4]
            .copy_from_slice(&pixels[from..from + area.w as usize * 4]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reverse_drag_clamps_to_screen() {
        assert_eq!(
            Area::between(Point { x: 120, y: 80 }, Point { x: -20, y: 10 }, 100, 100),
            Area {
                x: 0,
                y: 10,
                w: 100,
                h: 70
            }
        );
    }
    #[test]
    fn lasso_covers_all_points() {
        assert_eq!(
            bounds(
                &[
                    Point { x: 10, y: 20 },
                    Point { x: 90, y: 40 },
                    Point { x: 40, y: 80 }
                ],
                100,
                100
            ),
            Some(Area {
                x: 10,
                y: 20,
                w: 80,
                h: 60
            })
        );
        assert_eq!(bounds(&[], 100, 100), None);
    }
    #[test]
    fn movement_and_resize_stay_inside_frame() {
        let mut a = Area {
            x: 80,
            y: 80,
            w: 20,
            h: 20,
        };
        a.adjust(100, 100, false, 100, 100);
        assert_eq!(a.x, 80);
        a.adjust(-200, -200, true, 100, 100);
        assert_eq!((a.w, a.h), (8, 8));
        a.adjust(-200, -200, false, 100, 100);
        assert_eq!((a.x, a.y), (0, 0));
    }
    #[test]
    fn crop_has_correct_rows_and_orientation() {
        let pixels: Vec<u8> = (0..16 * 16 * 4).map(|i| (i % 251) as u8).collect();
        let dib = crop_dib(
            &pixels,
            16,
            16,
            Area {
                x: 3,
                y: 4,
                w: 8,
                h: 9,
            },
        )
        .unwrap();
        assert_eq!(&dib[8..12], &(-9i32).to_le_bytes());
        assert_eq!(&dib[40..72], &pixels[(4 * 16 + 3) * 4..(4 * 16 + 11) * 4]);
        assert_eq!(dib.len(), 40 + 8 * 9 * 4);
    }
    #[test]
    fn crop_rejects_invalid_bounds_and_buffer() {
        assert!(
            crop_dib(
                &[],
                16,
                16,
                Area {
                    x: 0,
                    y: 0,
                    w: 8,
                    h: 8
                }
            )
            .is_none()
        );
        assert!(
            crop_dib(
                &vec![0; 1024],
                16,
                16,
                Area {
                    x: 12,
                    y: 0,
                    w: 8,
                    h: 8
                }
            )
            .is_none()
        );
        assert!(
            crop_dib(
                &vec![0; 1024],
                16,
                16,
                Area {
                    x: 0,
                    y: 0,
                    w: 1,
                    h: 8
                }
            )
            .is_none()
        );
    }

    #[test]
    fn smoothed_stroke_keeps_cursor_and_endpoints() {
        let points = [
            Point { x: 0, y: 0 },
            Point { x: 30, y: 0 },
            Point { x: 30, y: 30 },
        ];
        let smooth = smooth_stroke(&points, false, 8.0);
        assert_eq!(smooth.first(), Some(&(0.0, 0.0)));
        assert_eq!(smooth.last(), Some(&(30.0, 30.0)));
        assert!(
            smooth
                .windows(2)
                .all(|p| (p[1].0 - p[0].0).abs() <= 4.0 && (p[1].1 - p[0].1).abs() <= 4.0)
        );
    }

    #[test]
    fn smoothing_removes_hand_wobble() {
        let wobbly: Vec<Point> = (0..=100)
            .map(|x| Point {
                x: x * 3,
                y: if x % 2 == 0 { 4 } else { -4 },
            })
            .collect();
        let smooth = smooth_stroke(&wobbly, false, 10.0);
        let middle = &smooth[20..smooth.len() - 20];
        assert!(middle.iter().all(|p| p.1.abs() < 1.0));
    }

    #[test]
    fn ink_has_soft_falloff() {
        let (width, height) = (64, 48);
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        render_ink(
            &mut pixels,
            width,
            height,
            (0, 0, width, height),
            &[(4.0, 24.0), (60.0, 24.0)],
            1.0,
            0.0,
        );
        let brightness = |y: usize| {
            let i = (y * width as usize + 32) * 4;
            pixels[i] as u32 + pixels[i + 1] as u32 + pixels[i + 2] as u32
        };
        let profile: Vec<u32> = (24..48).map(brightness).collect();
        assert!(profile.windows(2).all(|p| p[0] >= p[1]));
        assert!(profile[0] > 600);
        assert!(profile[6] > 0 && profile[6] < profile[0] / 3);
        assert_eq!(brightness(0), 0);

        #[cfg(debug_assertions)]
        {
            // Preview: a jittery hand-drawn loop, raw on the left and corrected on the right.
            let (width, height) = (720, 360);
            let mut pixels = vec![24u8; width as usize * height as usize * 4];
            let hand: Vec<Point> = (0..240)
                .map(|i| {
                    let t = i as f32 / 240.0 * std::f32::consts::TAU * 1.04;
                    let wobble = 9.0 * (t * 11.0).sin() + 5.0 * (t * 23.0).cos();
                    Point {
                        x: (180.0 + (120.0 + wobble) * t.cos()) as i32,
                        y: (180.0 + (130.0 + wobble) * t.sin()) as i32,
                    }
                })
                .collect();
            let raw: Vec<(f32, f32)> = hand.iter().map(|p| (p.x as f32, p.y as f32)).collect();
            render_ink(
                &mut pixels,
                width,
                height,
                (0, 0, width, height),
                &raw,
                1.0,
                0.0,
            );
            let shifted: Vec<Point> = hand
                .iter()
                .map(|p| Point {
                    x: p.x + 360,
                    y: p.y,
                })
                .collect();
            let mut smooth = smooth_stroke(&shifted, true, 16.0);
            smooth.push(smooth[0]);
            render_ink(
                &mut pixels,
                width,
                height,
                (0, 0, width, height),
                &smooth,
                1.0,
                0.0,
            );
            let rgba: Vec<u8> = pixels
                .chunks_exact(4)
                .flat_map(|p| [p[2], p[1], p[0], 255])
                .collect();
            let _ = std::fs::create_dir_all("target");
            if let Ok(file) = std::fs::File::create("target/ink-preview.png") {
                let mut encoder = png::Encoder::new(file, width as u32, height as u32);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                if let Ok(mut writer) = encoder.write_header() {
                    let _ = writer.write_image_data(&rgba);
                }
            }
        }
    }

    #[test]
    fn adaptive_filter_quiets_small_motion_but_follows_fast_motion() {
        let mut still = StrokeSmoother::new(Point { x: 100, y: 100 });
        for x in [101, 99, 101, 99, 101, 99] {
            let point = still.update(Point { x, y: 100 }, 1.0 / 60.0);
            assert!((point.x - 100).abs() <= 1);
        }
        let mut fast = StrokeSmoother::new(Point { x: 0, y: 0 });
        let followed = fast.update(Point { x: 100, y: 0 }, 1.0 / 60.0);
        assert!(followed.x >= 70);
    }

    #[test]
    fn ink_palette_wraps_without_a_color_jump() {
        assert_eq!(ink_rgb(0.0), ink_rgb(1.0));
        assert_eq!(ink_rgb(0.0), (60, 220, 255));
        let (a, b) = (ink_rgb(0.999), ink_rgb(0.0));
        assert!((a.0 as i32 - b.0 as i32).abs() < 4 && (a.2 as i32 - b.2 as i32).abs() < 4);
    }
}
