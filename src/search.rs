//! A user-initiated upload through the ordinary Google Lens web UI.
//! No private HTTP endpoints, clipboard, temporary host, or filesystem image is used.
#![allow(unsafe_op_in_unsafe_fn)]

use crate::orb::{Glow, Surface};
use base64::{Engine, engine::general_purpose::STANDARD};
use raw_window_handle::{HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle};
use std::{
    mem::{size_of, zeroed},
    num::NonZeroIsize,
    ptr::{null, null_mut},
    time::Instant,
};
use webview2_com::{
    Microsoft::Web::WebView2::Win32::{COREWEBVIEW2_PERMISSION_STATE_DENY, ICoreWebView2Settings2},
    PermissionRequestedEventHandler,
};
use windows::core::{Interface, PCWSTR};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Controls::{DRAWITEMSTRUCT, ODS_FOCUS, ODS_SELECTED, WM_MOUSELEAVE},
        HiDpi::{GetDpiForMonitor, GetDpiForWindow, GetSystemMetricsForDpi, MDT_EFFECTIVE_DPI},
        Input::KeyboardAndMouse::{
            ReleaseCapture, SetCapture, SetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
        },
        Shell::ShellExecuteW,
        WindowsAndMessaging::*,
    },
};
use wry::{NewWindowResponse, WebContext, WebView, WebViewBuilder, WebViewExtWindows};

pub const CLOSE_PANEL: u32 = WM_APP + 20;
pub const REDRAW: u32 = WM_APP + 21;
const READY: u32 = WM_APP + 22;
const SUBMITTED: u32 = WM_APP + 23;
const RESULT_PAGE: u32 = WM_APP + 24;
const FAILED: u32 = WM_APP + 25;
const STATUS: i32 = 201;
const RETRY: usize = 202;
const DRAW: usize = 203;
const CLOSE: usize = 204;
const SUBTITLE: i32 = 205;
/// Height of the panel's own title bar, which replaces the system caption.
const BAR: i32 = 36;
/// Hit box of each caption orb (minimize, maximize, close).
const BUTTON: i32 = 28;
/// Segoe MDL2 Assets glyphs shown on a hovered caption orb.
const GLYPH_MINIMIZE: u16 = 0xE921;
const GLYPH_MAXIMIZE: u16 = 0xE922;
const GLYPH_RESTORE: u16 = 0xE923;
const GLYPH_CLOSE: u16 = 0xE8BB;

const OBSERVE_UPLOAD: &str = r#"
(() => {
  if (window.top !== window || location.protocol !== 'https:' ||
      !['www.google.com', 'google.com', 'lens.google.com'].includes(location.hostname)) return;
  let attempts = 0;
  const timer = setInterval(() => {
    if (document.readyState !== 'complete') return;
    if (location.pathname === '/search' && new URLSearchParams(location.search).has('vsrid')) {
      clearInterval(timer);
      window.ipc.postMessage('orbom:result-page');
      return;
    }
    const input = document.querySelector('input[type="file"][name="encoded_image"]');
    if (input && location.pathname === '/') {
      clearInterval(timer);
      window.ipc.postMessage('orbom:ready');
    } else if (++attempts > 200) { clearInterval(timer); }
  }, 150);
})();
"#;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// Convert the selected top-down BGRA DIB to an opaque PNG for the web form.
pub fn png_from_dib(dib: &[u8]) -> Result<Vec<u8>, String> {
    if dib.len() < 40 {
        return Err(crate::i18n::t(
            "선택 이미지가 비어 있습니다.",
            "The selected image is empty.",
        )
        .into());
    }
    let w = i32::from_le_bytes(dib[4..8].try_into().unwrap());
    let h = -i32::from_le_bytes(dib[8..12].try_into().unwrap());
    if w <= 0 || h <= 0 || dib.len() != 40 + w as usize * h as usize * 4 {
        return Err(crate::i18n::t(
            "선택 이미지 크기가 잘못되었습니다.",
            "The selected image size is invalid.",
        )
        .into());
    }
    let mut rgb = Vec::with_capacity(w as usize * h as usize * 3);
    for pixel in dib[40..].chunks_exact(4) {
        rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
    }
    let mut bytes = Vec::new();
    let result = (|| {
        let mut encoder = png::Encoder::new(&mut bytes, w as u32, h as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&rgb).map_err(|e| e.to_string())
    })();
    wipe(&mut rgb);
    result.map(|_| bytes)
}

fn wipe(bytes: &mut [u8]) {
    for b in bytes {
        unsafe {
            std::ptr::write_volatile(b, 0);
        }
    }
}

fn google_origin(uri: &wry::http::Uri) -> bool {
    uri.scheme_str() == Some("https")
        && matches!(
            uri.host(),
            Some("google.com" | "www.google.com" | "lens.google.com")
        )
}

struct NativeWindow(HWND);
impl HasWindowHandle for NativeWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, raw_window_handle::HandleError> {
        let raw = Win32WindowHandle::new(NonZeroIsize::new(self.0 as isize).unwrap());
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(raw)) })
    }
}

pub struct Panel {
    pub hwnd: HWND,
    main: HWND,
    webview: Option<WebView>,
    _context: WebContext,
    image: String,
    submitted: bool,
    image_tab_requested: bool,
    showing_results: bool,
    showing_web: bool,
    generation: usize,
    font: HFONT,
    heading: HFONT,
    background: HBRUSH,
    glow: Glow,
    canvas: Option<Surface>,
    started: Instant,
    animations: bool,
    caption: [Glow; 3],
    glyphs: HFONT,
    hovered: Option<usize>,
    pressed: Option<usize>,
}

impl Panel {
    pub unsafe fn new(
        mut png: Vec<u8>,
        monitor: RECT,
        main: HWND,
        generation: usize,
    ) -> Result<Box<Self>, String> {
        let instance = GetModuleHandleW(null());
        let name = wide("OrbomResults");
        let mut class: WNDCLASSW = zeroed();
        class.lpfnWndProc = Some(window_proc);
        class.hInstance = instance;
        class.lpszClassName = name.as_ptr();
        class.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
        class.hIcon = crate::orb::app_icon(false);
        class.hbrBackground = GetStockObject(WHITE_BRUSH);
        RegisterClassW(&class);
        let mut dpi_x = 96;
        let mut dpi_y = 96;
        GetDpiForMonitor(
            MonitorFromRect(&monitor, MONITOR_DEFAULTTONEAREST),
            MDT_EFFECTIVE_DPI,
            &mut dpi_x,
            &mut dpi_y,
        );
        let width = (monitor.right - monitor.left).min(440 * dpi_x as i32 / 96);
        let height = monitor.bottom - monitor.top;
        let hwnd = CreateWindowExW(
            WS_EX_APPWINDOW,
            name.as_ptr(),
            wide(crate::i18n::t(
                "Orbom · 이미지 검색",
                "Orbom · Image search",
            ))
            .as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            monitor.right - width,
            monitor.top,
            width,
            height,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            wipe(&mut png);
            return Err(crate::i18n::t(
                "검색 창을 만들지 못했습니다.",
                "Couldn't create the search window.",
            )
            .into());
        }
        let image = STANDARD.encode(&png);
        wipe(&mut png);
        let path = std::env::var_os("LOCALAPPDATA")
            .map(|p| std::path::PathBuf::from(p).join("Orbom").join("WebView2"));
        let font = CreateFontW(
            -(14 * GetDpiForWindow(hwnd) as i32 / 96),
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            0,
            wide("Malgun Gothic").as_ptr(),
        );
        let heading = CreateFontW(
            -(18 * GetDpiForWindow(hwnd) as i32 / 96),
            0,
            0,
            0,
            600,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            0,
            wide("Malgun Gothic").as_ptr(),
        );
        let glyphs = CreateFontW(
            -(8 * GetDpiForWindow(hwnd) as i32 / 96),
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            0,
            wide("Segoe MDL2 Assets").as_ptr(),
        );
        let glows = (|| Ok::<_, String>((Glow::new()?, [Glow::new()?, Glow::new()?, Glow::new()?])))();
        let (glow, mut caption) = match glows {
            Ok(glows) => glows,
            Err(error) => {
                DestroyWindow(hwnd);
                DeleteObject(font);
                DeleteObject(heading);
                DeleteObject(glyphs);
                return Err(error);
            }
        };
        // Still pastel beads from the orb's palette: yellow, mint and pink, in caption order.
        for (orb, hue) in caption.iter_mut().zip([2.0 / 6.0, 1.0 / 6.0, 3.0 / 6.0]) {
            orb.set_hue(hue);
        }
        let mut panel = Box::new(Self {
            hwnd,
            main,
            webview: None,
            _context: WebContext::new(path),
            image,
            submitted: false,
            image_tab_requested: false,
            showing_results: false,
            showing_web: false,
            generation,
            font,
            heading,
            background: CreateSolidBrush(0x00FCF9F7),
            glow,
            canvas: None,
            started: Instant::now(),
            animations: crate::orb::motion(),
            caption,
            glyphs,
            hovered: None,
            pressed: None,
        });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, &mut *panel as *mut Self as isize);
        // Re-run WM_NCCALCSIZE now that the window proc can see the panel and drop the caption.
        SetWindowPos(
            hwnd,
            null_mut(),
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
        for (id, text, button) in [
            (STATUS, crate::i18n::t("이미지 찾는 중", "Searching"), false),
            (
                SUBTITLE,
                crate::i18n::t(
                    "화면 속 궁금한 것을 찾고 있어요",
                    "Looking up what's on your screen",
                ),
                false,
            ),
            (DRAW as i32, crate::i18n::t("다시 그리기", "Redraw"), true),
            (RETRY as i32, crate::i18n::t("재시도", "Retry"), true),
            (CLOSE as i32, crate::i18n::t("취소", "Cancel"), true),
        ] {
            let child = CreateWindowExW(
                0,
                wide(if button { "BUTTON" } else { "STATIC" }).as_ptr(),
                wide(text).as_ptr(),
                WS_CHILD
                    | if id == DRAW as i32 || id == RETRY as i32 {
                        0
                    } else {
                        WS_VISIBLE
                    }
                    | if button {
                        WS_TABSTOP | BS_OWNERDRAW as u32
                    } else {
                        1
                    },
                0,
                0,
                10,
                10,
                hwnd,
                id as usize as HMENU,
                instance,
                null(),
            );
            SendMessageW(
                child,
                WM_SETFONT,
                if id == STATUS { heading } else { font } as usize,
                1,
            );
        }
        panel.resize();
        let smoke = cfg!(debug_assertions) && std::env::args().any(|a| a == "--smoke-search");
        if !smoke {
            ShowWindow(hwnd, SW_SHOW);
        }
        UpdateWindow(hwnd);
        if panel.animations {
            SetTimer(hwnd, 2, 33, None);
        }
        #[cfg(debug_assertions)]
        if std::env::args().any(|a| a == "--preview-loading") {
            panel.save_preview("target/loading-preview.png");
            let _ = panel.glow.save_png("target/orb-preview.png");
            let _ = panel.glow.save_animation_preview();
            SetTimer(hwnd, 3, 1500, None);
            return Ok(panel);
        }
        let handle = hwnd as usize;
        let view = WebViewBuilder::new_with_web_context(&mut panel._context)
            .with_url("https://lens.google.com/")
            .with_initialization_script(OBSERVE_UPLOAD)
            .with_visible(false)
            .with_focused(false)
            .with_ipc_handler(move |request| {
                if !google_origin(request.uri()) {return;}
                #[cfg(debug_assertions)]
                if smoke && request.body().starts_with("orbom:test:") {
                    let _=std::fs::write("target/smoke-search.json",&request.body()[12..]);
                    PostQuitMessage(0);
                    return;
                }
                let event = match request.body().as_str() {
                    "orbom:ready"=>READY, "orbom:submitted"=>SUBMITTED, "orbom:failed"=>FAILED, "orbom:result-page"=>RESULT_PAGE, _=>return,
                };
                PostMessageW(handle as HWND,event,generation,0);
            })
            .with_new_window_req_handler(|url,_| {
                if url.starts_with("https://") || url.starts_with("http://") {
                    ShellExecuteW(null_mut(),wide("open").as_ptr(),wide(&url).as_ptr(),null(),null(),SW_SHOWNORMAL);
                }
                NewWindowResponse::Deny
            })
            .build_as_child(&NativeWindow(hwnd))
            .map_err(|e| format!("{}\n{e}", crate::i18n::t("이미지 검색 창을 시작하지 못했습니다. Microsoft Edge WebView2 Runtime 설치를 확인해 주세요.", "Couldn't start the image search window. Please check that Microsoft Edge WebView2 Runtime is installed.")))?;
        // Lens offers live camera search; Orbom only uploads the capture, so answer every
        // permission prompt (camera, microphone, location...) with no instead of asking the user.
        if let Ok(core) = view.controller().CoreWebView2() {
            let mut token = 0;
            let _ = core.add_PermissionRequested(
                &PermissionRequestedEventHandler::create(Box::new(|_, args| {
                    if let Some(args) = args {
                        args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)?;
                    }
                    Ok(())
                })),
                &mut token,
            );
        }
        panel.webview = Some(view);
        panel.resize();
        SetTimer(hwnd, 1, 20_000, None);
        if !smoke {
            SetForegroundWindow(hwnd);
            SetFocus(GetDlgItem(hwnd, CLOSE as i32));
        }
        Ok(panel)
    }

    unsafe fn status(&self, text: &str) {
        SetWindowTextW(GetDlgItem(self.hwnd, STATUS), wide(text).as_ptr());
    }

    unsafe fn orb_rect(&self) -> RECT {
        let mut r: RECT = zeroed();
        GetClientRect(self.hwnd, &mut r);
        let d = |n: i32| n * GetDpiForWindow(self.hwnd) as i32 / 96;
        let bar = d(BAR);
        if self.showing_web {
            return RECT {
                left: d(12),
                top: bar + d(8),
                right: d(80),
                bottom: bar + d(76),
            };
        }
        let size = d(220)
            .min(r.right - d(32))
            .min((r.bottom - bar - d(180)).max(d(96)))
            .max(32);
        let top = bar + ((r.bottom - bar - size - d(148)) / 2).max(d(20));
        RECT {
            left: (r.right - size) / 2,
            top,
            right: (r.right + size) / 2,
            bottom: top + size,
        }
    }

    unsafe fn resize(&mut self) {
        let mut r: RECT = zeroed();
        GetClientRect(self.hwnd, &mut r);
        if r.right <= 0 || r.bottom <= 0 {
            return;
        }
        if self
            .canvas
            .as_ref()
            .is_none_or(|c| c.width != r.right || c.height != r.bottom)
        {
            self.canvas = Surface::new(r.right, r.bottom).ok();
        }
        let d = |n: i32| n * GetDpiForWindow(self.hwnd) as i32 / 96;
        let orb = self.orb_rect();
        let bar = d(BAR);
        let (left, width, title_y, body_y, buttons_y) = if self.showing_web {
            (d(88), r.right - d(104), bar + d(12), bar + d(46), bar + d(100))
        } else {
            (
                d(20),
                r.right - d(40),
                orb.bottom,
                orb.bottom + d(38),
                orb.bottom + d(88),
            )
        };
        MoveWindow(
            GetDlgItem(self.hwnd, STATUS),
            left,
            title_y,
            width,
            d(30),
            1,
        );
        MoveWindow(
            GetDlgItem(self.hwnd, SUBTITLE),
            left,
            body_y,
            width,
            d(34),
            1,
        );
        if self.showing_web {
            let button_width = ((r.right - d(48)) / 3).min(d(110));
            let start = (r.right - button_width * 3 - d(16)) / 2;
            for (i, id) in [RETRY, DRAW, CLOSE].iter().enumerate() {
                MoveWindow(
                    GetDlgItem(self.hwnd, *id as i32),
                    start + i as i32 * (button_width + d(8)),
                    buttons_y,
                    button_width,
                    d(44),
                    1,
                );
            }
        } else {
            MoveWindow(
                GetDlgItem(self.hwnd, CLOSE as i32),
                (r.right - d(112)) / 2,
                buttons_y,
                d(112),
                d(44),
                1,
            );
        }
        let top = if self.showing_web && !self.showing_results {
            bar + d(164)
        } else {
            bar
        };
        if let Some(view) = &self.webview {
            let _ = view.set_bounds(wry::Rect {
                position: wry::dpi::PhysicalPosition::new(0, top).into(),
                size: wry::dpi::PhysicalSize::new(r.right as u32, (r.bottom - top).max(1) as u32)
                    .into(),
            });
        }
        InvalidateRect(self.hwnd, null(), 0);
    }

    unsafe fn paint(&mut self) {
        let mut ps: PAINTSTRUCT = zeroed();
        let dc = BeginPaint(self.hwnd, &mut ps);
        let rect = self.orb_rect();
        if let Some(canvas) = self.canvas.as_ref().map(|c| c.dc) {
            FillRect(canvas, &ps.rcPaint, self.background);
            if !self.showing_results {
                self.glow.update(if self.animations && !self.showing_web {
                    crate::orb::clock()
                } else {
                    0.0
                });
                self.glow
                    .draw(canvas, rect.left, rect.top, rect.right - rect.left);
            }
            self.draw_caption(canvas);
            let r = ps.rcPaint;
            BitBlt(
                dc,
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                canvas,
                r.left,
                r.top,
                SRCCOPY,
            );
        }
        EndPaint(self.hwnd, &ps);
    }

    unsafe fn d(&self, n: i32) -> i32 {
        n * GetDpiForWindow(self.hwnd) as i32 / 96
    }

    /// Hit box of caption orb `i` (0 minimize, 1 maximize, 2 close), right-aligned like Windows.
    unsafe fn caption_button(&self, i: usize) -> RECT {
        let mut r: RECT = zeroed();
        GetClientRect(self.hwnd, &mut r);
        let size = self.d(BUTTON);
        // Same margin on the top and right, so the close orb sits on the corner's 45° diagonal.
        let top = (self.d(BAR) - size) / 2;
        let right = r.right - top - (2 - i as i32) * size;
        RECT {
            left: right - size,
            top,
            right,
            bottom: top + size,
        }
    }

    unsafe fn caption_hit(&self, x: i32, y: i32) -> Option<usize> {
        (0..3).find(|&i| PtInRect(&self.caption_button(i), POINT { x, y }) != 0)
    }

    unsafe fn invalidate_caption(&self) {
        let mut r: RECT = zeroed();
        GetClientRect(self.hwnd, &mut r);
        r.bottom = self.d(BAR);
        InvalidateRect(self.hwnd, &r, 0);
    }

    /// The title and the three pastel orbs that stand in for minimize, maximize and close.
    unsafe fn draw_caption(&mut self, dc: HDC) {
        let bar = self.d(BAR);
        let mut title = [0u16; 128];
        GetWindowTextW(self.hwnd, title.as_mut_ptr(), title.len() as i32);
        let old = SelectObject(dc, self.font);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, 0x00908078);
        let mut text = RECT {
            left: self.d(14),
            top: 0,
            right: self.caption_button(0).left,
            bottom: bar,
        };
        DrawTextW(
            dc,
            title.as_ptr(),
            -1,
            &mut text,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        );
        SelectObject(dc, self.glyphs);
        SetTextColor(dc, 0x00463832);
        let maximize = if IsZoomed(self.hwnd) != 0 {
            GLYPH_RESTORE
        } else {
            GLYPH_MAXIMIZE
        };
        for (i, glyph) in [GLYPH_MINIMIZE, maximize, GLYPH_CLOSE].into_iter().enumerate() {
            let mut b = self.caption_button(i);
            let size = b.right - b.left;
            // The bead swells a little under the pointer and shrinks while pressed.
            let side = if self.pressed == Some(i) {
                size * 2 / 3
            } else if self.hovered == Some(i) {
                size * 7 / 8
            } else {
                size * 3 / 4
            };
            let offset = (size - side) / 2;
            self.caption[i].draw(dc, b.left + offset, b.top + offset, side);
            if self.hovered == Some(i) || self.pressed == Some(i) {
                DrawTextW(
                    dc,
                    [glyph, 0].as_ptr(),
                    -1,
                    &mut b,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                );
            }
        }
        SelectObject(dc, old);
    }

    unsafe fn show_error(&mut self, title: &str, detail: &str) {
        self.showing_web = true;
        KillTimer(self.hwnd, 2);
        self.status(title);
        SetWindowTextW(GetDlgItem(self.hwnd, SUBTITLE), wide(detail).as_ptr());
        SetWindowTextW(
            GetDlgItem(self.hwnd, CLOSE as i32),
            wide(crate::i18n::t("닫기", "Close")).as_ptr(),
        );
        for id in [STATUS, SUBTITLE, RETRY as i32, DRAW as i32, CLOSE as i32] {
            ShowWindow(GetDlgItem(self.hwnd, id), SW_SHOW);
        }
        self.resize();
        if let Some(view) = &self.webview {
            let _ = view.set_visible(true);
        }
    }

    unsafe fn draw_button(&self, item: &DRAWITEMSTRUCT) {
        let dc = item.hDC;
        let r = item.rcItem;
        FillRect(dc, &r, self.background);
        let fill = CreateSolidBrush(if item.itemState & ODS_SELECTED != 0 {
            0x00E9E1DA
        } else {
            0x00F3EDE8
        });
        let border = CreatePen(
            PS_SOLID,
            1,
            if item.itemState & ODS_FOCUS != 0 {
                0x00C7969A
            } else {
                0x00E0D5CF
            },
        );
        let old_brush = SelectObject(dc, fill);
        let old_pen = SelectObject(dc, border);
        RoundRect(dc, r.left + 1, r.top + 1, r.right - 1, r.bottom - 1, 20, 20);
        SelectObject(dc, old_brush);
        SelectObject(dc, old_pen);
        DeleteObject(fill);
        DeleteObject(border);
        let mut text = [0u16; 64];
        GetWindowTextW(item.hwndItem, text.as_mut_ptr(), text.len() as i32);
        let old_font = SelectObject(dc, self.font);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, 0x00594D47);
        let mut text_rect = r;
        DrawTextW(
            dc,
            text.as_ptr(),
            -1,
            &mut text_rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
        SelectObject(dc, old_font);
    }

    #[cfg(debug_assertions)]
    unsafe fn save_preview(&mut self, path: &str) {
        let Some(canvas) = &self.canvas else {
            return;
        };
        let dc = canvas.dc;
        FillRect(
            dc,
            &RECT {
                left: 0,
                top: 0,
                right: canvas.width,
                bottom: canvas.height,
            },
            self.background,
        );
        let orb = self.orb_rect();
        self.glow.update(0.0);
        self.glow.draw(dc, orb.left, orb.top, orb.right - orb.left);
        // Show the close orb hovered so the preview has both caption states.
        self.hovered = Some(2);
        self.draw_caption(dc);
        self.hovered = None;
        for id in [STATUS, SUBTITLE, CLOSE as i32] {
            let child = GetDlgItem(self.hwnd, id);
            let mut rect: RECT = zeroed();
            GetWindowRect(child, &mut rect);
            let mut p = POINT {
                x: rect.left,
                y: rect.top,
            };
            ScreenToClient(self.hwnd, &mut p);
            rect = RECT {
                left: p.x,
                top: p.y,
                right: p.x + rect.right - rect.left,
                bottom: p.y + rect.bottom - rect.top,
            };
            if id == CLOSE as i32 {
                let item = DRAWITEMSTRUCT {
                    hDC: dc,
                    hwndItem: child,
                    rcItem: rect,
                    ..zeroed()
                };
                self.draw_button(&item);
            } else {
                let mut text = [0u16; 128];
                GetWindowTextW(child, text.as_mut_ptr(), 128);
                let old = SelectObject(
                    dc,
                    if id == STATUS {
                        self.heading
                    } else {
                        self.font
                    },
                );
                SetBkMode(dc, TRANSPARENT as i32);
                SetTextColor(dc, if id == STATUS { 0x00463832 } else { 0x00908078 });
                DrawTextW(
                    dc,
                    text.as_ptr(),
                    -1,
                    &mut rect,
                    DT_CENTER | DT_SINGLELINE | DT_VCENTER,
                );
                SelectObject(dc, old);
            }
        }
        if let Some(canvas) = &mut self.canvas {
            let _ = canvas.save_png(path, false);
        }
    }

    unsafe fn submit(&mut self) {
        if self.submitted {
            return;
        }
        let Some(view) = &self.webview else {
            return;
        };
        // The base64 alphabet cannot terminate a JS string; it contains no user-authored code.
        let script = format!(
            r#"(() => {{
          if (location.protocol !== 'https:' || !['www.google.com','google.com','lens.google.com'].includes(location.hostname)) return;
          const input = document.querySelector('input[type="file"][name="encoded_image"]');
          if (!input) {{ window.ipc.postMessage('orbom:failed'); return; }}
          try {{
            const bytes = Uint8Array.from(atob('{}'), c => c.charCodeAt(0));
            const transfer = new DataTransfer();
            transfer.items.add(new File([bytes], 'orbom.png', {{type:'image/png'}}));
            input.files = transfer.files;
            input.dispatchEvent(new Event('change', {{bubbles:true}}));
            window.ipc.postMessage('orbom:submitted');
          }} catch (_) {{ window.ipc.postMessage('orbom:failed'); }}
        }})();"#,
            self.image
        );
        self.submitted = true;
        self.status(crate::i18n::t("이미지 찾는 중", "Searching"));
        if view.evaluate_script(&script).is_err() {
            self.submitted = false;
            self.show_error(
                crate::i18n::t("연결하지 못했어요", "Couldn't connect"),
                crate::i18n::t("잠시 후 다시 시도해 주세요", "Please try again in a moment"),
            );
        }
    }

    pub unsafe fn hide(&self) {
        ShowWindow(self.hwnd, SW_HIDE);
    }

    pub unsafe fn show(&self) {
        ShowWindow(self.hwnd, SW_SHOW);
    }

    pub unsafe fn keyboard_navigation(&self, msg: &MSG) -> bool {
        !self.showing_results
            && matches!(msg.message, WM_KEYDOWN | WM_SYSKEYDOWN)
            && (msg.hwnd == self.hwnd || IsChild(self.hwnd, msg.hwnd) != 0)
            && IsDialogMessageW(self.hwnd, msg) != 0
    }

    unsafe fn set_mobile(&self, mobile: bool) -> windows::core::Result<()> {
        if let Some(view) = &self.webview {
            let settings: ICoreWebView2Settings2 =
                view.controller().CoreWebView2()?.Settings()?.cast()?;
            let version = wry::webview_version().unwrap_or_else(|_| "140.0.0.0".into());
            let agent = if mobile {
                format!(
                    "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{version} Mobile Safari/537.36"
                )
            } else {
                String::new()
            };
            settings.SetUserAgent(PCWSTR(wide(&agent).as_ptr()))?;
        }
        Ok(())
    }
}

impl Drop for Panel {
    fn drop(&mut self) {
        unsafe {
            KillTimer(self.hwnd, 1);
            KillTimer(self.hwnd, 2);
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            self.webview = None;
            wipe(self.image.as_bytes_mut());
            DestroyWindow(self.hwnd);
            DeleteObject(self.font);
            DeleteObject(self.heading);
            DeleteObject(self.glyphs);
            DeleteObject(self.background);
        }
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Panel;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let panel = &mut *ptr;
    let point = || POINT {
        x: (lp & 0xffff) as i16 as i32,
        y: ((lp >> 16) & 0xffff) as i16 as i32,
    };
    match msg {
        WM_NCCALCSIZE if wp != 0 => {
            // Keep the resizable side and bottom borders but give the caption to the client area,
            // where the panel draws its own. A maximized window hangs its frame off-screen, so
            // push the top back in by that much.
            let params = &mut *(lp as *mut NCCALCSIZE_PARAMS);
            let top = params.rgrc[0].top;
            DefWindowProcW(hwnd, msg, wp, lp);
            params.rgrc[0].top = top;
            if IsZoomed(hwnd) != 0 {
                let dpi = GetDpiForWindow(hwnd);
                params.rgrc[0].top += GetSystemMetricsForDpi(SM_CYFRAME, dpi)
                    + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
            }
            0
        }
        WM_NCHITTEST => {
            let hit = DefWindowProcW(hwnd, msg, wp, lp);
            if hit != HTCLIENT as isize {
                return hit;
            }
            let mut p = point();
            ScreenToClient(hwnd, &mut p);
            if IsZoomed(hwnd) == 0
                && p.y < GetSystemMetricsForDpi(SM_CYFRAME, GetDpiForWindow(hwnd))
            {
                HTTOP as isize
            } else if p.y < panel.d(BAR) && panel.caption_hit(p.x, p.y).is_none() {
                HTCAPTION as isize
            } else {
                HTCLIENT as isize
            }
        }
        WM_MOUSEMOVE => {
            let p = point();
            let hit = panel.caption_hit(p.x, p.y);
            if hit != panel.hovered {
                panel.hovered = hit;
                panel.invalidate_caption();
                let mut track = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                TrackMouseEvent(&mut track);
            }
            0
        }
        WM_MOUSELEAVE => {
            panel.hovered = None;
            panel.invalidate_caption();
            0
        }
        WM_LBUTTONDOWN => {
            let p = point();
            if let Some(i) = panel.caption_hit(p.x, p.y) {
                panel.pressed = Some(i);
                SetCapture(hwnd);
                panel.invalidate_caption();
            }
            0
        }
        WM_LBUTTONUP => {
            if let Some(i) = panel.pressed.take() {
                ReleaseCapture();
                panel.invalidate_caption();
                let p = point();
                if panel.caption_hit(p.x, p.y) == Some(i) {
                    let command = match i {
                        0 => SC_MINIMIZE,
                        1 if IsZoomed(hwnd) != 0 => SC_RESTORE,
                        1 => SC_MAXIMIZE,
                        _ => SC_CLOSE,
                    };
                    PostMessageW(hwnd, WM_SYSCOMMAND, command as usize, 0);
                }
            }
            0
        }
        WM_PAINT => {
            panel.paint();
            0
        }
        WM_ERASEBKGND => 1,
        WM_CTLCOLORSTATIC => {
            SetBkColor(wp as HDC, 0x00FCF9F7);
            SetTextColor(
                wp as HDC,
                if GetDlgCtrlID(lp as HWND) == STATUS {
                    0x00463832
                } else {
                    0x00908078
                },
            );
            panel.background as isize
        }
        WM_DRAWITEM => {
            panel.draw_button(&*(lp as *const DRAWITEMSTRUCT));
            1
        }
        WM_SIZE => {
            if wp == SIZE_MINIMIZED as usize {
                KillTimer(hwnd, 2);
                return 0;
            }
            panel.resize();
            if panel.animations && !panel.showing_web {
                SetTimer(hwnd, 2, 33, None);
            }
            0
        }
        WM_SHOWWINDOW => {
            if wp != 0 && panel.animations && !panel.showing_web {
                SetTimer(hwnd, 2, 33, None);
            } else {
                KillTimer(hwnd, 2);
            }
            0
        }
        WM_SETTINGCHANGE => {
            panel.animations = crate::orb::motion();
            if panel.animations && !panel.showing_web {
                SetTimer(hwnd, 2, 33, None);
            } else {
                KillTimer(hwnd, 2);
            }
            InvalidateRect(hwnd, &panel.orb_rect(), 0);
            0
        }
        WM_GETMINMAXINFO => {
            let info = &mut *(lp as *mut MINMAXINFO);
            let dpi = GetDpiForWindow(hwnd) as i32;
            info.ptMinTrackSize = POINT {
                x: 380 * dpi / 96,
                y: 320 * dpi / 96,
            };
            0
        }
        WM_CLOSE => {
            PostMessageW(panel.main, CLOSE_PANEL, 0, 0);
            0
        }
        WM_COMMAND => {
            match wp & 0xffff {
                DRAW => {
                    PostMessageW(panel.main, REDRAW, 0, 0);
                }
                CLOSE => {
                    PostMessageW(panel.main, CLOSE_PANEL, 0, 0);
                }
                RETRY => {
                    let _ = panel.set_mobile(false);
                    panel.submitted = false;
                    panel.image_tab_requested = false;
                    panel.showing_results = false;
                    panel.showing_web = false;
                    panel.started = Instant::now();
                    for id in [DRAW, RETRY] {
                        ShowWindow(GetDlgItem(hwnd, id as i32), SW_HIDE);
                    }
                    for id in [STATUS, SUBTITLE, CLOSE as i32] {
                        ShowWindow(GetDlgItem(hwnd, id), SW_SHOW);
                    }
                    SetWindowTextW(
                        GetDlgItem(hwnd, SUBTITLE),
                        wide(crate::i18n::t(
                            "화면 속 궁금한 것을 찾고 있어요",
                            "Looking up what's on your screen",
                        ))
                        .as_ptr(),
                    );
                    SetWindowTextW(
                        GetDlgItem(hwnd, CLOSE as i32),
                        wide(crate::i18n::t("취소", "Cancel")).as_ptr(),
                    );
                    panel.resize();
                    panel.status(crate::i18n::t("이미지 찾는 중", "Searching"));
                    if let Some(view) = &panel.webview {
                        let _ = view.set_visible(false);
                        let _ = view.load_url("https://lens.google.com/");
                    }
                    SetTimer(hwnd, 1, 20_000, None);
                    if panel.animations {
                        SetTimer(hwnd, 2, 33, None);
                    }
                }
                _ => {}
            }
            0
        }
        READY if wp == panel.generation => {
            panel.submit();
            0
        }
        SUBMITTED if wp == panel.generation => {
            panel.status(crate::i18n::t("이미지 찾는 중", "Searching"));
            0
        }
        FAILED if wp == panel.generation => {
            panel.submitted = false;
            panel.show_error(
                crate::i18n::t("연결하지 못했어요", "Couldn't connect"),
                crate::i18n::t(
                    "아래 화면을 확인하거나 다시 시도해 주세요",
                    "Check the page below or try again",
                ),
            );
            0
        }
        RESULT_PAGE if wp == panel.generation => {
            if panel.submitted {
                if !panel.image_tab_requested {
                    panel.image_tab_requested = true;
                    let _ = panel.set_mobile(true);
                    if let Some(view) = &panel.webview {
                        let _ = view.evaluate_script(r#"(() => {
                            const link = Array.from(document.querySelectorAll('a[href]')).find(a => {
                                const url = new URL(a.href, location.href);
                                return url.origin === location.origin && url.pathname === '/search' && url.searchParams.get('udm') === '44';
                            });
                            if (link && new URLSearchParams(location.search).get('udm') !== '44') link.click();
                            else location.reload();
                        })();"#);
                        return 0;
                    }
                }
                KillTimer(hwnd, 1);
                KillTimer(hwnd, 2);
                panel.showing_results = true;
                panel.showing_web = true;
                for id in [STATUS, SUBTITLE, DRAW as i32, RETRY as i32, CLOSE as i32] {
                    ShowWindow(GetDlgItem(hwnd, id), SW_HIDE);
                }
                panel.resize();
                SetWindowTextW(
                    hwnd,
                    wide(crate::i18n::t(
                        "Orbom · Google 이미지 검색",
                        "Orbom · Google image search",
                    ))
                    .as_ptr(),
                );
                panel.invalidate_caption();
                if let Some(view) = &panel.webview {
                    let _ = view.set_visible(true);
                    let _ = view.focus();
                    #[cfg(debug_assertions)]
                    if std::env::args().any(|a| a == "--smoke-search") {
                        let _=view.evaluate_script("window.ipc.postMessage('orbom:test:' + JSON.stringify({userAgent:navigator.userAgent, mobile:navigator.userAgent.includes('Mobile'), imageTab:new URLSearchParams(location.search).get('udm'), text:document.body.innerText.slice(0,1800)}))");
                    }
                }
            }
            0
        }
        WM_TIMER if wp == 2 => {
            if !panel.showing_web && IsIconic(hwnd) == 0 && IsWindowVisible(hwnd) != 0 {
                InvalidateRect(hwnd, &panel.orb_rect(), 0);
            }
            0
        }
        #[cfg(debug_assertions)]
        WM_TIMER if wp == 3 => {
            PostQuitMessage(0);
            0
        }
        WM_TIMER if wp == 1 => {
            KillTimer(hwnd, 1);
            panel.show_error(
                crate::i18n::t("조금 오래 걸리고 있어요", "This is taking a while"),
                crate::i18n::t(
                    "아래 화면을 확인하거나 다시 시도해 주세요",
                    "Check the page below or try again",
                ),
            );
            #[cfg(debug_assertions)]
            if std::env::args().any(|a| a == "--smoke-search") {
                let _ = std::fs::write("target/smoke-search.json", "{\"error\":\"timeout\"}");
                PostQuitMessage(1);
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn png_preserves_crop_colors() {
        let pixels = [19, 40, 230, 0].repeat(64);
        let dib = crate::selection::crop_dib(
            &pixels,
            8,
            8,
            crate::selection::Area {
                x: 0,
                y: 0,
                w: 8,
                h: 8,
            },
        )
        .unwrap();
        let png = png_from_dib(&dib).unwrap();
        let decoder = png::Decoder::new(std::io::Cursor::new(png));
        let mut reader = decoder.read_info().unwrap();
        let mut output = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut output).unwrap();
        assert_eq!((info.width, info.height), (8, 8));
        assert_eq!(&output[..3], &[230, 40, 19]);
    }
    #[test]
    fn upload_only_targets_exact_google_https_origins() {
        for value in ["https://www.google.com/?olud", "https://lens.google.com/"] {
            assert!(google_origin(&value.parse().unwrap()));
        }
        for value in [
            "http://www.google.com/",
            "https://www.google.com.evil.test/",
            "https://example.com/",
            "https://accounts.google.com/",
        ] {
            assert!(!google_origin(&value.parse().unwrap()));
        }
    }
}
