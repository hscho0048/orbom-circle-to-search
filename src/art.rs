//! Platform-free orb artwork, shared by the runtime and by build.rs for the executable icon.

pub const PALETTE: [[f32; 3]; 6] = [
    [60.0, 220.0, 255.0],
    [103.0, 250.0, 170.0],
    [255.0, 217.0, 98.0],
    [255.0, 137.0, 183.0],
    [159.0, 91.0, 250.0],
    [85.0, 134.0, 255.0],
];

/// Palette position (0..1 wraps) of the orb's shared color at `phase`.
pub fn orb_hue(phase: f32) -> f32 {
    phase / std::f32::consts::TAU * 0.155
}

/// Cyclic, eased walk through the palette: 0..1 visits every color once and wraps smoothly.
pub fn palette_at(t: f32) -> [f32; 3] {
    let position = t.rem_euclid(1.0) * PALETTE.len() as f32;
    let i = position as usize % PALETTE.len();
    let k = smoothstep(0.0, 1.0, position.fract());
    let (a, b) = (PALETTE[i], PALETTE[(i + 1) % PALETTE.len()]);
    [0, 1, 2].map(|c| a[c] + (b[c] - a[c]) * k)
}

/// exp(-x2), cut off before it underflows into slow denormals.
fn gauss_squared(x2: f32) -> f32 {
    if x2 > 60.0 { 0.0 } else { (-x2).exp() }
}

fn gauss(x: f32) -> f32 {
    gauss_squared(x * x)
}

pub fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Premultiplied BGRA orb. `zoom` is the body radius as a fraction of `side`; the halo always
/// fades out before the bitmap border. `glass` (0..1) makes the middle see-through while the rim
/// stays dense, like a glass bead. `phase` advances TAU per nominal cycle; the blobs use
/// incommensurate speeds so the pattern never visibly repeats.
pub fn orb_pixels(side: usize, zoom: f32, phase: f32, glass: f32) -> Vec<u8> {
    let mut pixels = vec![0; side * side * 4];
    orb_into(&mut pixels, side, zoom, phase, glass);
    pixels
}

/// Renders the orb into an existing premultiplied BGRA buffer of `side * side` pixels.
pub fn orb_into(pixels: &mut [u8], side: usize, zoom: f32, phase: f32, glass: f32) {
    // Blobs drift at different speeds and directions so the colors swirl rather than rotate rigidly.
    let speeds = [1.0, -1.31, 1.73, 0.87, -1.57, 1.19];
    let wobble = [2.0, 1.43, 2.61, 1.87, 2.29, 1.61];
    let centers: Vec<_> = (0..6)
        .map(|i| {
            let angle = i as f32 * std::f32::consts::TAU / 6.0 + phase * speeds[i];
            let radius = 0.50 + 0.14 * (wobble[i] * phase + i as f32 * 1.7).sin();
            (angle.cos() * radius, angle.sin() * radius)
        })
        .collect();
    // The whole orb shares one hue that travels around the palette (a lap every ~16 seconds);
    // blobs only differ by a small hue offset, which keeps some depth without mixing rival colors.
    let hue = orb_hue(phase);
    let tints: Vec<[f32; 3]> = (0..6).map(|i| palette_at(hue + i as f32 * 0.018)).collect();
    let limit = 0.5 / zoom * 0.99;
    let scale = side as f32 * zoom;

    // Everything except the color depends only on the radius, so tabulate it once per frame.
    const STEPS: usize = 1024;
    let aa = 1.5 / scale;
    let radial: Vec<(f32, f32)> = (0..=STEPS)
        .map(|i| {
            let radius = i as f32 / STEPS as f32 * limit;
            // Crisp, anti-aliased body edge with a tight colored glow just outside it.
            let body = 1.0 - smoothstep(1.0 - aa, 1.0 + aa, radius);
            let halo = 0.24 * gauss((radius - 1.0) / 0.14);
            let edge = 1.0 - smoothstep(limit * 0.76, limit, radius);
            let rim = 0.14 * gauss((radius - 1.0) / 0.04);
            let white = rim * body;
            let density = 1.0 - glass * (1.0 - 0.3 * radius.min(1.0).powi(4));
            let body_alpha = body * (density + (1.0 - density) * white);
            ((body_alpha + (1.0 - body) * halo) * edge, white)
        })
        .collect();

    // The blob colors vary slowly, so evaluate them on a coarse grid and interpolate.
    const GRID: usize = 4;
    let cells = side / GRID + 2;
    let colors: Vec<[f32; 3]> = (0..cells * cells)
        .map(|i| {
            let nx = ((i % cells * GRID) as f32 + 0.5 - side as f32 / 2.0) / scale;
            let ny = ((i / cells * GRID) as f32 + 0.5 - side as f32 / 2.0) / scale;
            let mut rgb = [0.0; 3];
            let mut total = 0.0;
            for (c, &(cx, cy)) in centers.iter().enumerate() {
                let weight = gauss_squared(((nx - cx).powi(2) + (ny - cy).powi(2)) / 0.32);
                for channel in 0..3 {
                    rgb[channel] += tints[c][channel] * weight;
                }
                total += weight;
            }
            rgb.map(|v| v / f32::max(total, 0.0001))
        })
        .collect();

    pixels.fill(0);
    for y in 0..side {
        let ny = (y as f32 + 0.5 - side as f32 / 2.0) / scale;
        let (gy, ty) = (y / GRID, (y % GRID) as f32 / GRID as f32);
        for x in 0..side {
            let nx = (x as f32 + 0.5 - side as f32 / 2.0) / scale;
            let position = (nx * nx + ny * ny).sqrt() / limit * STEPS as f32;
            if position >= STEPS as f32 {
                continue;
            }
            let (i, t) = (position as usize, position.fract());
            let (a0, w0) = radial[i];
            let (a1, w1) = radial[i + 1];
            let alpha = a0 + (a1 - a0) * t;
            if alpha < 0.002 {
                continue;
            }
            let white = w0 + (w1 - w0) * t;
            let (gx, tx) = (x / GRID, (x % GRID) as f32 / GRID as f32);
            let at = |dx: usize, dy: usize| colors[(gy + dy) * cells + gx + dx];
            let (c00, c10, c01, c11) = (at(0, 0), at(1, 0), at(0, 1), at(1, 1));
            let index = (y * side + x) * 4;
            for channel in 0..3 {
                let top = c00[channel] + (c10[channel] - c00[channel]) * tx;
                let bottom = c01[channel] + (c11[channel] - c01[channel]) * tx;
                let value = top + (bottom - top) * ty;
                let lit = value + (255.0 - value) * white;
                pixels[index + 2 - channel] = (lit * alpha + 0.5).min(255.0) as u8;
            }
            pixels[index + 3] = (alpha * 255.0 + 0.5) as u8;
        }
    }
}

/// Straight-alpha BGRA icon image, supersampled so small sizes stay round.
#[allow(dead_code)] // Used by build.rs.
pub fn icon_pixels(size: usize) -> Vec<u8> {
    let factor = (128 / size).clamp(1, 4);
    let big = orb_pixels(size * factor, 0.42, 0.0, 0.0);
    let mut out = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let mut sum = [0u32; 4];
            for sy in 0..factor {
                for sx in 0..factor {
                    let i = ((y * factor + sy) * size * factor + x * factor + sx) * 4;
                    for c in 0..4 {
                        sum[c] += big[i + c] as u32;
                    }
                }
            }
            let n = (factor * factor) as u32;
            let alpha = sum[3] / n;
            let o = (y * size + x) * 4;
            for c in 0..3 {
                out[o + c] = (sum[c] / n * 255).checked_div(alpha).unwrap_or(0).min(255) as u8;
            }
            out[o + 3] = alpha as u8;
        }
    }
    out
}

#[allow(dead_code)] // Used by build.rs.
pub const ICON_SIZES: [usize; 8] = [16, 20, 24, 32, 40, 48, 64, 256];

/// A 32-bit DIB icon image (BITMAPINFOHEADER + bottom-up BGRA + AND mask) as stored in ICO/RT_ICON.
#[allow(dead_code)] // Used by build.rs.
pub fn icon_dib(size: usize) -> Vec<u8> {
    let pixels = icon_pixels(size);
    let mask_row = size.div_ceil(32) * 4;
    let mut out = Vec::with_capacity(40 + pixels.len() + mask_row * size);
    for v in [40u32, size as u32, 2 * size as u32] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    for y in (0..size).rev() {
        out.extend_from_slice(&pixels[y * size * 4..(y + 1) * size * 4]);
    }
    for y in (0..size).rev() {
        let mut row = vec![0u8; mask_row];
        for x in 0..size {
            if pixels[(y * size + x) * 4 + 3] == 0 {
                row[x / 8] |= 0x80 >> (x % 8);
            }
        }
        out.extend_from_slice(&row);
    }
    out
}

/// A compiled resource (.res) file holding icon group 1, linkable directly by MSVC link.exe.
#[allow(dead_code)] // Used by build.rs.
pub fn icon_res() -> Vec<u8> {
    fn entry(out: &mut Vec<u8>, kind: u16, id: u16, flags: u16, data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&32u32.to_le_bytes());
        for v in [0xFFFF, kind, 0xFFFF, id] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&0x0409u16.to_le_bytes());
        out.extend_from_slice(&[0; 8]);
        out.extend_from_slice(data);
        while !out.len().is_multiple_of(4) {
            out.push(0);
        }
    }
    let mut out = Vec::new();
    entry(&mut out, 0, 0, 0, &[]);
    let mut group = Vec::new();
    for v in [0u16, 1, ICON_SIZES.len() as u16] {
        group.extend_from_slice(&v.to_le_bytes());
    }
    for (i, &size) in ICON_SIZES.iter().enumerate() {
        let image = icon_dib(size);
        entry(&mut out, 3, i as u16 + 1, 0x1010, &image);
        group.extend_from_slice(&[size as u8, size as u8, 0, 0]);
        group.extend_from_slice(&1u16.to_le_bytes());
        group.extend_from_slice(&32u16.to_le_bytes());
        group.extend_from_slice(&(image.len() as u32).to_le_bytes());
        group.extend_from_slice(&(i as u16 + 1).to_le_bytes());
    }
    entry(&mut out, 14, 1, 0x1030, &group);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn icon_resource_has_every_size_and_a_group() {
        let res = icon_res();
        assert_eq!(res.len() % 4, 0);
        let mut at = 0;
        let mut kinds = Vec::new();
        while at < res.len() {
            let size = u32::from_le_bytes(res[at..at + 4].try_into().unwrap()) as usize;
            kinds.push(u16::from_le_bytes(
                res[at + 10..at + 12].try_into().unwrap(),
            ));
            at += (32 + size).div_ceil(4) * 4;
        }
        assert_eq!(at, res.len());
        assert_eq!(kinds.iter().filter(|&&k| k == 3).count(), ICON_SIZES.len());
        assert_eq!(kinds.last(), Some(&14));
        #[cfg(debug_assertions)]
        {
            // Preview sheet for eyeballing the icon: every size side by side on dark and light.
            let width: usize = ICON_SIZES.iter().map(|s| s + 8).sum();
            let mut sheet = vec![0u8; width * 2 * 264 * 4];
            let mut left = 0;
            for &size in &ICON_SIZES {
                let pixels = icon_pixels(size);
                for (band, bg) in [(0usize, 32.0f32), (264, 236.0)] {
                    for y in 0..size {
                        for x in 0..size {
                            let p = &pixels[(y * size + x) * 4..][..4];
                            let a = p[3] as f32 / 255.0;
                            let o = ((band + y) * width + left + x) * 4;
                            for c in 0..3 {
                                sheet[o + c] = (p[2 - c] as f32 * a + bg * (1.0 - a)) as u8;
                            }
                            sheet[o + 3] = 255;
                        }
                    }
                }
                left += size + 8;
            }
            let _ = std::fs::create_dir_all("target");
            if let Ok(file) = std::fs::File::create("target/icon-preview.png") {
                let mut encoder = png::Encoder::new(file, width as u32, 528);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                if let Ok(mut writer) = encoder.write_header() {
                    let _ = writer.write_image_data(&sheet);
                }
            }
        }
        let small = icon_pixels(16);
        assert_eq!(small[3], 0);
        assert!(small[(8 * 16 + 8) * 4 + 3] > 250);
    }
}
