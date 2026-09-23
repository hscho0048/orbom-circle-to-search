//! Shared animated, premultiplied-alpha rainbow orb for the launcher and loading view.
#![allow(unsafe_op_in_unsafe_fn)]

use std::{
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
};
use windows_sys::Win32::{
    Foundation::*, Graphics::Gdi::*, System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::*,
};

/// Seconds per nominal color cycle; the motion itself never exactly repeats.
const PERIOD: f32 = 2.5;
const ZOOM: f32 = 0.335;
/// Glass amount in thousandths, shared by every orb and changed from the tray menu.
static GLASS: AtomicU32 = AtomicU32::new(320);

pub fn set_glass(glass: f32) {
    GLASS.store((glass.clamp(0.0, 0.9) * 1000.0) as u32, Ordering::Relaxed);
}

/// Whether orbs animate their colors. Independent of Windows' "animation effects" switch, since
/// the moving color is the app's identity; users turn it off from the tray menu instead.
static MOTION: AtomicBool = AtomicBool::new(true);

pub fn set_motion(on: bool) {
    MOTION.store(on, Ordering::Relaxed);
}

/// Seconds on the clock every orb and the ink share, so they always show the same color.
pub fn clock() -> f32 {
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_secs_f32()
}

pub fn motion() -> bool {
    MOTION.load(Ordering::Relaxed)
}

fn glass() -> f32 {
    GLASS.load(Ordering::Relaxed) as f32 / 1000.0
}

fn phase_at(seconds: f32) -> f32 {
    // Wrap to keep f32 precision over long uptimes; the jump happens once every 30 minutes.
    (seconds % 1800.0) * std::f32::consts::TAU / PERIOD
}

/// The orb's shared palette position after `seconds` on its clock, for drawing matching ink.
pub fn hue_at(seconds: f32) -> f32 {
    crate::art::orb_hue(phase_at(seconds))
}

/// The orb icon embedded by build.rs, at the system's large or small icon size.
pub unsafe fn app_icon(small: bool) -> HICON {
    let (cx, cy) = if small {
        (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON))
    } else {
        (GetSystemMetrics(SM_CXICON), GetSystemMetrics(SM_CYICON))
    };
    let icon = LoadImageW(
        GetModuleHandleW(null()),
        1 as _,
        IMAGE_ICON,
        cx,
        cy,
        LR_DEFAULTCOLOR | LR_SHARED,
    );
    if icon.is_null() {
        LoadIconW(null_mut(), IDI_APPLICATION)
    } else {
        icon
    }
}

pub struct Surface {
    pub dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    data: *mut u8,
    pub width: i32,
    pub height: i32,
}

impl Surface {
    pub unsafe fn new(width: i32, height: i32) -> Result<Self, String> {
        if width <= 0 || height <= 0 {
            return Err(
                crate::i18n::t("잘못된 그리기 영역입니다.", "Invalid drawing area.").into(),
            );
        }
        let dc = CreateCompatibleDC(null_mut());
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = width;
        info.bmiHeader.biHeight = -height;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        let mut bits = null_mut();
        let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if dc.is_null() || bitmap.is_null() || bits.is_null() {
            if !bitmap.is_null() {
                DeleteObject(bitmap);
            }
            if !dc.is_null() {
                DeleteDC(dc);
            }
            return Err(crate::i18n::t(
                "그리기 버퍼를 만들지 못했습니다.",
                "Couldn't create the drawing buffer.",
            )
            .into());
        }
        let old = SelectObject(dc, bitmap);
        Ok(Self {
            dc,
            bitmap,
            old,
            data: bits.cast(),
            width,
            height,
        })
    }

    unsafe fn bytes(&mut self) -> &mut [u8] {
        GdiFlush();
        std::slice::from_raw_parts_mut(self.data, self.width as usize * self.height as usize * 4)
    }

    #[cfg(debug_assertions)]
    pub unsafe fn save_png(&mut self, path: &str, transparent: bool) -> Result<(), String> {
        let width = self.width as u32;
        let height = self.height as u32;
        let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
        for p in self.bytes().chunks_exact(4) {
            let alpha = if transparent { p[3] } else { 255 };
            let expand = |v: u8| {
                if transparent && alpha > 0 {
                    ((v as u32 * 255) / alpha as u32).min(255) as u8
                } else {
                    v
                }
            };
            rgba.extend_from_slice(&[expand(p[2]), expand(p[1]), expand(p[0]), alpha]);
        }
        let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
        let mut encoder = png::Encoder::new(file, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .map_err(|e| e.to_string())?
            .write_image_data(&rgba)
            .map_err(|e| e.to_string())
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

/// Real-time orb: every frame is computed at exactly the size it is shown, so there is no
/// frame cache and no scaling blur.
pub struct Glow {
    phase: f32,
    source: Option<Surface>,
    rendered: Option<(i32, f32, f32)>,
    layer: Option<Surface>,
}

const BLEND: BLENDFUNCTION = BLENDFUNCTION {
    BlendOp: AC_SRC_OVER as u8,
    BlendFlags: 0,
    SourceConstantAlpha: 255,
    AlphaFormat: AC_SRC_ALPHA as u8,
};

impl Glow {
    pub unsafe fn new() -> Result<Self, String> {
        Ok(Self {
            phase: 0.0,
            source: None,
            rendered: None,
            layer: None,
        })
    }

    pub unsafe fn update(&mut self, seconds: f32) {
        self.phase = phase_at(seconds);
    }

    /// Renders the current phase at `size` pixels unless the source surface already holds it.
    unsafe fn render(&mut self, size: i32) -> Option<&Surface> {
        let size = size.max(1);
        if self.source.as_ref().is_none_or(|s| s.width != size) {
            self.source = Some(Surface::new(size, size).ok()?);
            self.rendered = None;
        }
        let glass = glass();
        let source = self.source.as_mut()?;
        if self.rendered != Some((size, self.phase, glass)) {
            crate::art::orb_into(source.bytes(), size as usize, ZOOM, self.phase, glass);
            self.rendered = Some((size, self.phase, glass));
        }
        self.source.as_ref()
    }

    pub unsafe fn draw(&mut self, dc: HDC, x: i32, y: i32, size: i32) {
        if let Some(source) = self.render(size) {
            AlphaBlend(dc, x, y, size, size, source.dc, 0, 0, size, size, BLEND);
        }
    }

    pub unsafe fn present(&mut self, hwnd: HWND, width: i32, height: i32, emphasis: f32) {
        if self
            .layer
            .as_ref()
            .is_none_or(|b| b.width != width || b.height != height)
        {
            self.layer = Surface::new(width, height).ok();
        }
        let inset = ((1.0 - emphasis) * 2.0).round() as i32;
        let size = width.min(height) - 2 * inset;
        let Some(mut layer) = self.layer.take() else {
            return;
        };
        layer.bytes().fill(0);
        self.draw(layer.dc, inset, inset, size);
        UpdateLayeredWindow(
            hwnd,
            null_mut(),
            null(),
            &SIZE {
                cx: width,
                cy: height,
            },
            layer.dc,
            &POINT { x: 0, y: 0 },
            0,
            &BLEND,
            ULW_ALPHA,
        );
        self.layer = Some(layer);
    }

    #[cfg(debug_assertions)]
    pub unsafe fn save_png(&mut self, path: &str) -> Result<(), String> {
        self.render(192);
        self.source.as_mut().ok_or("no orb")?.save_png(path, true)
    }

    #[cfg(debug_assertions)]
    pub unsafe fn save_animation_preview(&mut self) -> Result<(), String> {
        std::fs::create_dir_all("target/orb-frames").map_err(|e| e.to_string())?;
        for i in 0..120 {
            self.update(i as f32 * PERIOD / 120.0);
            self.save_png(&format!("target/orb-frames/{i:03}.png"))?;
        }
        self.update(0.0);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SIDE: usize = 192;

    fn frame(phase: f32) -> Vec<u8> {
        crate::art::orb_pixels(SIDE, ZOOM, phase, 0.32)
    }

    #[test]
    fn orb_has_transparent_corners_glass_center_and_premultiplied_color() {
        let image = frame(0.0);
        assert_eq!(image[3], 0);
        let center = image[((SIDE / 2) * SIDE + SIDE / 2) * 4 + 3];
        assert!(
            (120..250).contains(&center),
            "glass center should be see-through: {center}"
        );
        let rim = image[((SIDE / 2) * SIDE + SIDE / 2 + 62) * 4 + 3];
        // Transparency stays nearly even to the edge, so the rim does not read as a thick ring.
        assert!(
            rim >= center && rim < center + 60,
            "rim {rim} vs center {center}"
        );
        assert!(
            image
                .chunks_exact(4)
                .all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3])
        );
        assert_ne!(image, frame(1.0));
        let solid = crate::art::orb_pixels(SIDE, ZOOM, 0.0, 0.0);
        assert_eq!(solid[((SIDE / 2) * SIDE + SIDE / 2) * 4 + 3], 255);
    }

    #[test]
    fn realtime_orb_renders_at_display_size_and_keeps_moving() {
        unsafe {
            let mut glow = Glow::new().unwrap();
            let mut layer = Surface::new(84, 84).unwrap();
            let mut snapshot = |glow: &mut Glow, seconds: f32| {
                glow.update(seconds);
                layer.bytes().fill(0);
                glow.draw(layer.dc, 0, 0, 84);
                layer.bytes().to_vec()
            };
            let start = snapshot(&mut glow, 0.0);
            assert_eq!(start[3], 0);
            assert!(start[(42 * 84 + 42) * 4 + 3] > 120);
            assert_eq!(glow.source.as_ref().unwrap().width, 84);
            assert_ne!(snapshot(&mut glow, PERIOD / 4.0), start);
            // No fixed loop: one nominal period later the colors are somewhere new.
            assert_ne!(snapshot(&mut glow, PERIOD), start);
        }
    }
}
