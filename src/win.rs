#![allow(unsafe_op_in_unsafe_fn)]

use crate::orb::Glow;
use crate::search::{self, Panel};
use crate::selection::{
    Area, Point, StrokeSmoother, bounds, crop_dib, ink_rgb, render_ink, smooth_stroke,
};
use std::{
    ffi::c_void,
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    time::Instant,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Dwm::DwmFlush, Gdi::*, GdiPlus as gp},
    System::{LibraryLoader::GetModuleHandleW, Threading::CreateMutexW},
    UI::{
        HiDpi::*,
        Input::{KeyboardAndMouse::*, Pointer::*},
        Shell::*,
        WindowsAndMessaging::*,
    },
};

const CAPTURE: usize = 101;
const RECTANGLE: usize = 102;
const LASSO: usize = 103;
const RESET: usize = 104;
const SEARCH: usize = 105;
const TOGGLE_BUBBLE: usize = 109;
const TOGGLE_MOTION: usize = 110;
const TOGGLE_STARTUP: usize = 111;
const EXIT: usize = crate::setup::EXIT_COMMAND;
const TRAY: u32 = WM_APP + 1;
const KEYBOARD_AREA: usize = 113;
const RUN_SEARCH: u32 = WM_APP + 2;
const MOUSE_LEAVE: u32 = 0x02A3;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn color(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}
fn argb(alpha: u8, r: u8, g: u8, b: u8) -> u32 {
    (alpha as u32) << 24 | (r as u32) << 16 | (g as u32) << 8 | b as u32
}

/// Where the touch magnifier is painted for a finger at `cursor`.
fn magnifier_rect(cursor: Point, width: i32) -> RECT {
    const SIZE: i32 = 144;
    let x = (cursor.x - SIZE / 2).clamp(0, (width - SIZE).max(0));
    let y = (cursor.y - SIZE - 48).max(0);
    RECT {
        left: x,
        top: y,
        right: x + SIZE,
        bottom: y + SIZE,
    }
}

fn selection_dirty(area: Option<Area>, cursor: Point, touch: bool, width: i32) -> RECT {
    let mut rect = if let Some(a) = area {
        RECT {
            left: a.x - 64,
            top: a.y - 64,
            right: a.x + a.w + 64,
            bottom: a.y + a.h + 64,
        }
    } else {
        RECT {
            left: cursor.x - 64,
            top: cursor.y - 64,
            right: cursor.x + 64,
            bottom: cursor.y + 64,
        }
    };
    if touch {
        let lens = magnifier_rect(cursor, width);
        rect = union_rect(
            rect,
            RECT {
                left: lens.left - 2,
                top: lens.top - 2,
                right: lens.right + 2,
                bottom: lens.bottom + 2,
            },
        );
    }
    rect
}

fn union_rect(a: RECT, b: RECT) -> RECT {
    RECT {
        left: a.left.min(b.left),
        top: a.top.min(b.top),
        right: a.right.max(b.right),
        bottom: a.bottom.max(b.bottom),
    }
}

fn overlay_header(s: &Session, dpi: i32) -> (RECT, RECT) {
    let scale = |n| n * dpi / 96;
    let available = s.monitor.right - s.monitor.left;
    let width = scale(390).min(available - scale(24));
    let left = s.monitor.left - s.origin.x + (available - width) / 2;
    let top = s.monitor.top - s.origin.y + scale(24);
    let header = RECT {
        left,
        top,
        right: left + width,
        bottom: top + scale(72),
    };
    let close = RECT {
        left: header.right - scale(54),
        top: header.top + scale(12),
        right: header.right - scale(12),
        bottom: header.bottom - scale(12),
    };
    (header, close)
}

fn inside(p: Point, r: RECT) -> bool {
    p.x >= r.left && p.x < r.right && p.y >= r.top && p.y < r.bottom
}

struct Bitmap {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    data: *mut u8,
    width: i32,
    height: i32,
}

impl Bitmap {
    unsafe fn new(width: i32, height: i32) -> Result<Self, String> {
        if width <= 0 || height <= 0 || width as i64 * height as i64 > 150_000_000 {
            return Err(crate::i18n::t(
                "지원할 수 없는 화면 크기입니다.",
                "This screen size is not supported.",
            )
            .into());
        }
        let dc = CreateCompatibleDC(null_mut());
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = width;
        info.bmiHeader.biHeight = -height;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        let mut data: *mut c_void = null_mut();
        let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut data, null_mut(), 0);
        if dc.is_null() || bitmap.is_null() || data.is_null() {
            if !bitmap.is_null() {
                DeleteObject(bitmap);
            }
            if !dc.is_null() {
                DeleteDC(dc);
            }
            return Err(crate::i18n::t(
                "화면 이미지 메모리를 만들지 못했습니다.",
                "Couldn't allocate memory for the screen image.",
            )
            .into());
        }
        let old = SelectObject(dc, bitmap);
        Ok(Self {
            dc,
            bitmap,
            old,
            data: data.cast(),
            width,
            height,
        })
    }

    unsafe fn pixels(&self) -> &[u8] {
        std::slice::from_raw_parts(self.data, self.width as usize * self.height as usize * 4)
    }

    #[allow(clippy::mut_from_ref)]
    unsafe fn pixels_mut(&self) -> &mut [u8] {
        GdiFlush();
        std::slice::from_raw_parts_mut(self.data, self.width as usize * self.height as usize * 4)
    }
}

impl Drop for Bitmap {
    fn drop(&mut self) {
        unsafe {
            GdiFlush();
            // Wipe the captured image; the fence keeps the wipe from being optimized out.
            std::ptr::write_bytes(self.data, 0, self.width as usize * self.height as usize * 4);
            std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
            SelectObject(self.dc, self.old);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

#[derive(Clone, Copy)]
enum Drag {
    Draw(Point),
    Move(Point, Area),
    Resize(Point, Area, bool, bool),
}

struct Session {
    original: Bitmap,
    dim: Bitmap,
    frame: Bitmap,
    origin: Point,
    monitor: RECT,
    area: Option<Area>,
    path: Vec<Point>,
    drag: Option<Drag>,
    pointer: Option<u32>,
    touch: bool,
    cursor: Point,
    smoother: Option<StrokeSmoother>,
    stroke_at: Option<Instant>,
    started: Instant,
    completed: Option<Instant>,
}

struct App {
    main: HWND,
    overlay: HWND,
    previous: HWND,
    session: Option<Session>,
    lasso: bool,
    explicit_mode: bool,
    pending: bool,
    bubble: HWND,
    bubble_enabled: bool,
    glass: usize,
    dim: usize,
    bubble_drag: Option<(POINT, RECT)>,
    bubble_moved: bool,
    bubble_hovered: bool,
    bubble_emphasis: f32,
    bubble_glow: Glow,
    animations: bool,
    gdiplus_ready: bool,
    hotkey: usize,
    hotkey_registered: bool,
    tray_added: bool,
    result: Option<Box<Panel>>,
    search_generation: usize,
    searching: bool,
}

unsafe fn state(hwnd: HWND) -> *mut App {
    GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App
}

unsafe fn message(hwnd: HWND, text: &str) {
    MessageBoxW(
        hwnd,
        wide(text).as_ptr(),
        wide("Orbom").as_ptr(),
        MB_OK | MB_ICONINFORMATION,
    );
}

fn settings_path(name: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|p| std::path::PathBuf::from(p).join("Orbom").join(name))
}

/// Carries settings and the WebView2 profile over from the app's former name, Cirque.
fn migrate_settings() {
    if let Some(base) = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from) {
        let (old, new) = (base.join("Cirque"), base.join("Orbom"));
        if old.is_dir() && !new.exists() {
            let _ = std::fs::rename(old, new);
        }
    }
}

fn save_setting(name: &str, value: &str) {
    if let Some(path) = settings_path(name) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, value);
    }
}

/// Tray menu choices for how see-through the orb's middle is.
const GLASS_LEVELS: [(&str, &str, f32); 3] = [
    ("불투명", "Solid", 0.0),
    ("보통", "Medium", 0.32),
    ("투명", "Clear", 0.6),
];

/// Tray menu choices for how much the screen darkens (percent brightness kept) while selecting.
const DIM_LEVELS: [(&str, &str, u16); 3] = [
    ("없음", "Off", 100),
    ("약하게", "Light", 78),
    ("보통", "Medium", 48),
];

fn load_choice(name: &str, count: usize, default: usize) -> usize {
    settings_path(name)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v < count)
        .unwrap_or(default)
}

const SHORTCUTS: [(&str, u32, u32); 3] = [
    (
        "Ctrl + Shift + Space",
        MOD_CONTROL | MOD_SHIFT,
        VK_SPACE as u32,
    ),
    ("Ctrl + Alt + S", MOD_CONTROL | MOD_ALT, b'S' as u32),
    ("Alt + Shift + S", MOD_ALT | MOD_SHIFT, b'S' as u32),
];

impl App {
    unsafe fn create_bubble(&mut self) {
        let name = wide("OrbomBubble");
        let mut class: WNDCLASSW = zeroed();
        class.lpfnWndProc = Some(bubble_proc);
        class.hInstance = GetModuleHandleW(null());
        class.lpszClassName = name.as_ptr();
        class.hCursor = LoadCursorW(null_mut(), IDC_HAND);
        RegisterClassW(&class);
        let mut monitor: MONITORINFO = zeroed();
        monitor.cbSize = size_of::<MONITORINFO>() as u32;
        let mut point = zeroed();
        GetCursorPos(&mut point);
        GetMonitorInfoW(
            MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST),
            &mut monitor,
        );
        let size = 84 * GetDpiForSystem() as i32 / 96;
        self.bubble = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED,
            name.as_ptr(),
            wide(bubble_title()).as_ptr(),
            WS_POPUP,
            monitor.rcWork.right - size - 24,
            monitor.rcWork.top + 160,
            size,
            size,
            null_mut(),
            null_mut(),
            class.hInstance,
            null(),
        );
        SetWindowLongPtrW(self.bubble, GWLP_USERDATA, self as *mut Self as isize);
        self.show_bubble();
        UpdateWindow(self.bubble);
        self.redraw_bubble();
        #[cfg(debug_assertions)]
        if std::env::args().any(|arg| arg == "--preview-overlay" || arg == "--preview-loading") {
            let _ = self.bubble_glow.save_png("target/bubble-preview.png");
        }
    }

    /// Shows the floating orb unless the user turned it off from the tray menu.
    unsafe fn show_bubble(&self) {
        if self.bubble_enabled {
            ShowWindow(self.bubble, SW_SHOWNOACTIVATE);
            // Keep the color animation running even if WM_SHOWWINDOW was not delivered.
            if crate::orb::motion() {
                SetTimer(self.bubble, 3, 33, None);
            }
        }
    }

    unsafe fn redraw_bubble(&mut self) {
        let mut rect: RECT = zeroed();
        GetClientRect(self.bubble, &mut rect);
        if rect.right <= 0 || rect.bottom <= 0 {
            return;
        }
        self.bubble_glow.update(if crate::orb::motion() {
            crate::orb::clock()
        } else {
            0.0
        });
        self.bubble_glow
            .present(self.bubble, rect.right, rect.bottom, self.bubble_emphasis);
    }

    unsafe fn menu(&mut self) {
        let menu = CreatePopupMenu();
        AppendMenuW(
            menu,
            MF_STRING,
            CAPTURE,
            wide(crate::i18n::t("동그라미로 검색", "Circle to search")).as_ptr(),
        );
        AppendMenuW(
            menu,
            MF_STRING | if self.bubble_enabled { MF_CHECKED } else { 0 },
            TOGGLE_BUBBLE,
            wide(crate::i18n::t(
                "바탕화면에 오브 버튼 표시",
                "Show orb button on desktop",
            ))
            .as_ptr(),
        );
        AppendMenuW(
            menu,
            MF_STRING | if crate::orb::motion() { MF_CHECKED } else { 0 },
            TOGGLE_MOTION,
            wide(crate::i18n::t("오브 색 변화", "Animate orb colors")).as_ptr(),
        );
        let glass_menu = CreatePopupMenu();
        for (i, (ko, en, _)) in GLASS_LEVELS.iter().enumerate() {
            AppendMenuW(
                glass_menu,
                MF_STRING | if self.glass == i { MF_CHECKED } else { 0 },
                400 + i,
                wide(crate::i18n::t(ko, en)).as_ptr(),
            );
        }
        AppendMenuW(
            menu,
            MF_POPUP,
            glass_menu as usize,
            wide(crate::i18n::t("오브 투명도", "Orb transparency")).as_ptr(),
        );
        let dim_menu = CreatePopupMenu();
        for (i, (ko, en, _)) in DIM_LEVELS.iter().enumerate() {
            AppendMenuW(
                dim_menu,
                MF_STRING | if self.dim == i { MF_CHECKED } else { 0 },
                500 + i,
                wide(crate::i18n::t(ko, en)).as_ptr(),
            );
        }
        AppendMenuW(
            menu,
            MF_POPUP,
            dim_menu as usize,
            wide(crate::i18n::t(
                "선택할 때 화면 어둡게",
                "Dim screen while selecting",
            ))
            .as_ptr(),
        );
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        for (i, (text, _, _)) in SHORTCUTS.iter().enumerate() {
            AppendMenuW(
                menu,
                MF_STRING
                    | if self.hotkey_registered && self.hotkey == i {
                        MF_CHECKED
                    } else {
                        0
                    },
                300 + i,
                wide(text).as_ptr(),
            );
        }
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        let language_menu = CreatePopupMenu();
        for (i, text) in ["한국어", "English"].iter().enumerate() {
            AppendMenuW(
                language_menu,
                MF_STRING
                    | if crate::i18n::english() == (i == 1) {
                        MF_CHECKED
                    } else {
                        0
                    },
                600 + i,
                wide(text).as_ptr(),
            );
        }
        AppendMenuW(
            menu,
            MF_POPUP,
            language_menu as usize,
            wide("언어 / Language").as_ptr(),
        );
        AppendMenuW(
            menu,
            MF_STRING
                | if crate::setup::startup_enabled() {
                    MF_CHECKED
                } else {
                    0
                },
            TOGGLE_STARTUP,
            wide(crate::i18n::t("Windows 시작 시 실행", "Start with Windows")).as_ptr(),
        );
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        AppendMenuW(
            menu,
            MF_STRING,
            EXIT,
            wide(crate::i18n::t("종료", "Quit")).as_ptr(),
        );
        let mut p = zeroed();
        GetCursorPos(&mut p);
        SetForegroundWindow(self.main);
        let id = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            p.x,
            p.y,
            0,
            self.main,
            null(),
        ) as usize;
        PostMessageW(self.main, WM_NULL, 0, 0);
        DestroyMenu(menu);
        command(self, id);
    }

    unsafe fn install_hotkey(&mut self, index: usize) {
        if self.hotkey_registered && index == self.hotkey {
            return;
        }
        let (_, modifiers, key) = SHORTCUTS[index];
        // Register the new shortcut before releasing the working one.
        if RegisterHotKey(self.main, 10 + index as i32, modifiers | MOD_NOREPEAT, key) == 0 {
            message(
                self.main,
                crate::i18n::t(
                    "이 단축키는 다른 앱에서 사용 중입니다. 다른 조합을 선택해 주세요.\n화면의 원형 버튼은 계속 사용할 수 있습니다.",
                    "This shortcut is already used by another app. Please choose a different one.\nThe on-screen orb button still works.",
                ),
            );
            return;
        }
        if self.hotkey_registered {
            UnregisterHotKey(self.main, 10 + self.hotkey as i32);
        }
        self.hotkey = index;
        self.hotkey_registered = true;
        save_setting("hotkey.txt", &index.to_string());
    }

    unsafe fn begin(&mut self) {
        if self.session.is_some() || self.pending {
            return;
        }
        self.previous = GetForegroundWindow();
        if let Some(panel) = &self.result {
            panel.hide();
        }
        self.lasso = true;
        self.searching = false;
        ShowWindow(self.bubble, SW_HIDE);
        self.pending = true;
        // Let DWM remove our floating button before capturing.
        SetTimer(self.main, 1, 160, None);
    }

    unsafe fn capture(&mut self) -> Result<(), String> {
        self.pending = false;
        let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let h = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        DwmFlush();
        let original = Bitmap::new(w, h)?;
        let screen = GetDC(null_mut());
        let ok = BitBlt(original.dc, 0, 0, w, h, screen, x, y, SRCCOPY | CAPTUREBLT);
        ReleaseDC(null_mut(), screen);
        if ok == 0 {
            return Err(crate::i18n::t(
                "화면을 캡처하지 못했습니다. 일반 데스크톱에서 다시 시도하세요.",
                "Couldn't capture the screen. Try again from the regular desktop.",
            )
            .into());
        }
        GdiFlush();
        let dim = Bitmap::new(w, h)?;
        let frame = Bitmap::new(w, h)?;
        let keep = DIM_LEVELS[self.dim].2;
        for (i, value) in original.pixels().iter().enumerate() {
            *dim.data.add(i) = (*value as u16 * keep / 100) as u8;
        }
        let mut pointer: POINT = zeroed();
        GetCursorPos(&mut pointer);
        let mut info: MONITORINFO = zeroed();
        info.cbSize = size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(
            MonitorFromPoint(pointer, MONITOR_DEFAULTTONEAREST),
            &mut info,
        );
        self.session = Some(Session {
            original,
            dim,
            frame,
            origin: Point { x, y },
            monitor: info.rcWork,
            area: None,
            path: vec![],
            drag: None,
            pointer: None,
            touch: false,
            cursor: Point::default(),
            smoother: None,
            stroke_at: None,
            started: Instant::now(),
            completed: None,
        });
        let overlay = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_APPWINDOW,
            wide("OrbomOverlay").as_ptr(),
            wide(crate::i18n::t("Orbom — 영역 선택", "Orbom — Select area")).as_ptr(),
            WS_POPUP | WS_CLIPCHILDREN,
            x,
            y,
            w,
            h,
            null_mut(),
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        );
        if overlay.is_null() {
            self.session = None;
            return Err(crate::i18n::t(
                "선택 창을 만들지 못했습니다.",
                "Couldn't create the selection window.",
            )
            .into());
        }
        self.overlay = overlay;
        SetWindowLongPtrW(overlay, GWLP_USERDATA, self as *mut Self as isize);
        ShowWindow(overlay, SW_SHOW);
        SetForegroundWindow(overlay);
        SetFocus(overlay);
        if self.animations {
            SetTimer(overlay, 2, 16, None);
        }
        Ok(())
    }

    unsafe fn refresh(&self) {
        InvalidateRect(self.overlay, null(), 0);
    }

    unsafe fn end(&mut self) {
        let hwnd = self.overlay;
        self.overlay = null_mut();
        if !hwnd.is_null() {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            DestroyWindow(hwnd);
        }
        self.session = None;
        self.searching = false;
        self.show_bubble();
        if let Some(panel) = &self.result {
            panel.show();
        }
        if !self.previous.is_null() && IsWindow(self.previous) != 0 {
            SetForegroundWindow(self.previous);
        }
    }

    unsafe fn reset(&mut self) {
        if let Some(s) = &mut self.session {
            s.area = None;
            s.drag = None;
            s.pointer = None;
            s.path.clear();
            s.completed = None;
            s.smoother = None;
            s.stroke_at = None;
        }
        self.searching = false;
        ReleaseCapture();
        self.refresh();
        SetFocus(self.overlay);
    }

    unsafe fn keyboard_area(&mut self) {
        self.lasso = false;
        if let Some(s) = &mut self.session {
            let w = (s.monitor.right - s.monitor.left).min(320);
            let h = (s.monitor.bottom - s.monitor.top).min(220);
            s.area = Some(Area {
                x: s.monitor.left - s.origin.x + (s.monitor.right - s.monitor.left - w) / 2,
                y: s.monitor.top - s.origin.y + (s.monitor.bottom - s.monitor.top - h) / 3,
                w,
                h,
            });
            s.path.clear();
        }
        self.refresh();
        SetFocus(self.overlay);
    }

    unsafe fn down(&mut self, p: Point, touch: bool) {
        if self.searching {
            return;
        }
        if touch && !self.explicit_mode {
            self.lasso = true;
        }
        let s = self.session.as_mut().unwrap();
        if s.drag.is_some() {
            return;
        }
        s.touch = touch;
        s.cursor = p;
        let radius = if touch { 28 } else { 12 } * GetDpiForWindow(self.overlay) as i32 / 96;
        let mut drag = Drag::Draw(p);
        if let Some(a) = s.area {
            for right in [false, true] {
                for bottom in [false, true] {
                    let x = if right { a.x + a.w } else { a.x };
                    let y = if bottom { a.y + a.h } else { a.y };
                    if (p.x - x).abs() <= radius && (p.y - y).abs() <= radius {
                        drag = Drag::Resize(p, a, right, bottom);
                    }
                }
            }
            if matches!(drag, Drag::Draw(_)) && a.contains(p) {
                drag = Drag::Move(p, a);
            }
        }
        if matches!(drag, Drag::Draw(_)) {
            s.area = None;
            s.path = vec![p];
            s.smoother = Some(StrokeSmoother::new(p));
            s.stroke_at = Some(Instant::now());
        } else {
            s.path.clear();
        }
        s.drag = Some(drag);
        if crate::orb::motion() && self.lasso && matches!(drag, Drag::Draw(_)) {
            SetTimer(self.overlay, 4, 33, None);
        }
        self.refresh();
    }

    unsafe fn movement(&mut self, p: Point) {
        let s = self.session.as_mut().unwrap();
        let old = selection_dirty(s.area, s.cursor, s.touch, s.original.width);
        let p = Point {
            x: p.x.clamp(0, s.original.width),
            y: p.y.clamp(0, s.original.height),
        };
        s.cursor = p;
        match s.drag {
            Some(Drag::Draw(start)) => {
                if self.lasso {
                    let now = Instant::now();
                    let dt = s
                        .stroke_at
                        .map_or(1.0 / 60.0, |last| now.duration_since(last).as_secs_f32());
                    let corrected = s.smoother.as_mut().map_or(p, |filter| filter.update(p, dt));
                    s.stroke_at = Some(now);
                    if s.path.last().is_none_or(|last| {
                        (last.x - corrected.x).abs() + (last.y - corrected.y).abs() >= 2
                    }) {
                        s.path.push(corrected);
                    }
                    s.area = bounds(&s.path, s.original.width, s.original.height);
                } else {
                    s.area = Some(Area::between(start, p, s.original.width, s.original.height));
                }
            }
            Some(Drag::Move(start, mut a)) => {
                a.adjust(
                    p.x - start.x,
                    p.y - start.y,
                    false,
                    s.original.width,
                    s.original.height,
                );
                s.area = Some(a);
            }
            Some(Drag::Resize(start, a, right, bottom)) => {
                let anchor = Point {
                    x: if right { a.x } else { a.x + a.w },
                    y: if bottom { a.y } else { a.y + a.h },
                };
                let moving = Point {
                    x: if right {
                        a.x + a.w + p.x - start.x
                    } else {
                        a.x + p.x - start.x
                    },
                    y: if bottom {
                        a.y + a.h + p.y - start.y
                    } else {
                        a.y + p.y - start.y
                    },
                };
                s.area = Some(Area::between(
                    anchor,
                    moving,
                    s.original.width,
                    s.original.height,
                ));
            }
            None => return,
        }
        let new = selection_dirty(s.area, s.cursor, s.touch, s.original.width);
        InvalidateRect(self.overlay, &union_rect(old, new), 0);
    }

    unsafe fn up(&mut self, p: Point) {
        let was_drawing = self.session.as_ref().is_some_and(|s| s.drag.is_some());
        if !was_drawing {
            return;
        }
        self.movement(p);
        let s = self.session.as_mut().unwrap();
        s.drag = None;
        s.pointer = None;
        KillTimer(self.overlay, 4);
        if s.area.is_some_and(|a| a.valid()) {
            self.searching = true;
            s.completed = Some(Instant::now());
            if self.animations {
                SetTimer(self.overlay, 2, 16, None);
            } else {
                PostMessageW(self.main, RUN_SEARCH, 0, 0);
            }
        } else {
            s.area = None;
            s.path.clear();
        }
        self.refresh();
    }

    unsafe fn search(&mut self) {
        let Some(s) = &self.session else {
            return;
        };
        let Some(area) = s.area else {
            return;
        };
        if s.drag.is_some() {
            return;
        }
        let monitor = s.monitor;
        let Some(mut dib) = crop_dib(
            s.original.pixels(),
            s.original.width,
            s.original.height,
            area,
        ) else {
            return;
        };
        let image = search::png_from_dib(&dib);
        for byte in &mut dib {
            std::ptr::write_volatile(byte, 0);
        }
        match image {
            Ok(png) => {
                self.result = None;
                self.end();
                self.search_generation += 1;
                match Panel::new(png, monitor, self.main, self.search_generation) {
                    Ok(panel) => self.result = Some(panel),
                    Err(e) => {
                        self.show_bubble();
                        message(self.main, &e);
                    }
                }
            }
            Err(e) => {
                self.searching = false;
                self.refresh();
                message(self.overlay, &e);
            }
        }
    }
}

unsafe fn paint_header(dc: HDC, s: &Session, dpi: i32, searching: bool) {
    let (header, close) = overlay_header(s, dpi);
    let bg = CreateSolidBrush(color(22, 30, 39));
    let edge = CreatePen(PS_SOLID, 1, color(63, 78, 87));
    let old_bg = SelectObject(dc, bg);
    let old_edge = SelectObject(dc, edge);
    RoundRect(
        dc,
        header.left,
        header.top,
        header.right,
        header.bottom,
        24,
        24,
    );
    SelectObject(dc, old_bg);
    SelectObject(dc, old_edge);
    DeleteObject(bg);
    DeleteObject(edge);
    let close_bg = CreateSolidBrush(color(43, 58, 65));
    let close_edge = CreatePen(PS_SOLID, 1, color(79, 100, 106));
    let old_bg = SelectObject(dc, close_bg);
    let old_edge = SelectObject(dc, close_edge);
    RoundRect(dc, close.left, close.top, close.right, close.bottom, 18, 18);
    SelectObject(dc, old_bg);
    SelectObject(dc, old_edge);
    DeleteObject(close_bg);
    DeleteObject(close_edge);
    let scale = |n: i32| n * dpi / 96;
    let title = CreateFontW(
        -scale(13),
        0,
        0,
        0,
        700,
        0,
        0,
        0,
        DEFAULT_CHARSET as u32,
        0,
        0,
        CLEARTYPE_QUALITY as u32,
        0,
        wide(crate::i18n::t("맑은 고딕", "Segoe UI")).as_ptr(),
    );
    let body = CreateFontW(
        -scale(14),
        0,
        0,
        0,
        500,
        0,
        0,
        0,
        DEFAULT_CHARSET as u32,
        0,
        0,
        CLEARTYPE_QUALITY as u32,
        0,
        wide(crate::i18n::t("맑은 고딕", "Segoe UI")).as_ptr(),
    );
    let old_font = SelectObject(dc, title);
    SetBkMode(dc, TRANSPARENT as i32);
    SetTextColor(dc, color(117, 244, 207));
    let mut text = RECT {
        left: header.left + scale(20),
        top: header.top + scale(10),
        right: close.left - scale(6),
        bottom: header.top + scale(31),
    };
    DrawTextW(
        dc,
        wide("ORBOM").as_ptr(),
        -1,
        &mut text,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER,
    );
    SelectObject(dc, body);
    SetTextColor(dc, color(235, 245, 244));
    text.top = header.top + scale(33);
    text.bottom = header.bottom - scale(9);
    let instruction = if searching {
        crate::i18n::t("이미지 검색 중…", "Searching image…")
    } else {
        crate::i18n::t(
            "동그라미를 그리고 손을 떼세요",
            "Draw a circle, then let go",
        )
    };
    DrawTextW(
        dc,
        wide(instruction).as_ptr(),
        -1,
        &mut text,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
    );
    SelectObject(dc, old_font);
    DeleteObject(title);
    DeleteObject(body);
    let close_pen = CreatePen(PS_SOLID, 2, color(215, 234, 229));
    let old_pen = SelectObject(dc, close_pen);
    let cx = (close.left + close.right) / 2;
    let cy = (close.top + close.bottom) / 2;
    let arm = scale(6);
    MoveToEx(dc, cx - arm, cy - arm, null_mut());
    LineTo(dc, cx + arm, cy + arm);
    MoveToEx(dc, cx - arm, cy + arm, null_mut());
    LineTo(dc, cx + arm, cy - arm);
    SelectObject(dc, old_pen);
    DeleteObject(close_pen);
}

unsafe fn draw_ink(s: &Session, dpi: i32, hue: f32, clip: RECT) {
    if s.path.len() < 2 {
        return;
    }
    let scale = dpi as f32 / 96.0;
    let closed = s.drag.is_none() && s.path.len() > 2;
    let mut points = smooth_stroke(&s.path, closed, 16.0 * scale);
    if closed && let Some(&first) = points.first() {
        points.push(first);
    }
    render_ink(
        s.frame.pixels_mut(),
        s.frame.width,
        s.frame.height,
        (clip.left, clip.top, clip.right, clip.bottom),
        &points,
        scale,
        hue,
    );
}

unsafe fn draw_completion_ring(dc: HDC, s: &Session, dpi: i32, hue: f32) {
    let (Some(completed), Some(a)) = (s.completed, s.area) else {
        return;
    };
    let mut graphics: *mut gp::GpGraphics = null_mut();
    GdiFlush();
    if gp::GdipCreateFromHDC(dc, &mut graphics) != gp::Ok {
        return;
    }
    gp::GdipSetSmoothingMode(graphics, gp::SmoothingModeAntiAlias);
    let scale = dpi as f32 / 96.0;
    let t = (completed.elapsed().as_secs_f32() / 0.18).clamp(0.0, 1.0);
    let eased = 1.0 - (1.0 - t).powi(3);
    let (r, g, b) = ink_rgb(hue + 0.05);
    // Two concentric pens give the expanding ring a soft glow instead of a single hard line.
    for (width, strength) in [(7.0, 50.0), (2.0, 170.0)] {
        let mut pen: *mut gp::GpPen = null_mut();
        if gp::GdipCreatePen1(
            argb((strength * (1.0 - t)) as u8, r, g, b),
            width * scale,
            gp::UnitPixel,
            &mut pen,
        ) == gp::Ok
        {
            let expand = 26.0 * eased * scale;
            gp::GdipDrawEllipse(
                graphics,
                pen,
                a.x as f32 - expand,
                a.y as f32 - expand,
                a.w as f32 + 2.0 * expand,
                a.h as f32 + 2.0 * expand,
            );
            gp::GdipDeletePen(pen);
        }
    }
    gp::GdipDeleteGraphics(graphics);
}

unsafe fn paint_overlay(hwnd: HWND, app: &App) {
    let mut ps: PAINTSTRUCT = zeroed();
    let output_dc = BeginPaint(hwnd, &mut ps);
    if let Some(s) = &app.session {
        let dc = s.frame.dc;
        let r = ps.rcPaint;
        BitBlt(
            dc,
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            s.dim.dc,
            r.left,
            r.top,
            SRCCOPY,
        );
        if app.animations && s.started.elapsed().as_millis() < 140 {
            let t = (s.started.elapsed().as_secs_f32() / 0.14).clamp(0.0, 1.0);
            let eased = 1.0 - (1.0 - t).powi(3);
            AlphaBlend(
                dc,
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                s.original.dc,
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: ((1.0 - eased) * 255.0) as u8,
                    AlphaFormat: 0,
                },
            );
        }
        let pen = CreatePen(PS_SOLID, 3, color(78, 239, 188));
        let oldpen = SelectObject(dc, pen);
        let oldbrush = SelectObject(dc, GetStockObject(NULL_BRUSH));
        if let Some(a) = s.area.filter(|_| !app.lasso || s.path.len() < 3) {
            BitBlt(dc, a.x, a.y, a.w, a.h, s.original.dc, a.x, a.y, SRCCOPY);
            Rectangle(dc, a.x, a.y, a.x + a.w, a.y + a.h);
            let size = (GetDpiForWindow(hwnd) as i32 * 7 / 96).max(6);
            let brush = CreateSolidBrush(color(78, 239, 188));
            for x in [a.x, a.x + a.w] {
                for y in [a.y, a.y + a.h] {
                    FillRect(
                        dc,
                        &RECT {
                            left: x - size,
                            top: y - size,
                            right: x + size,
                            bottom: y + size,
                        },
                        brush,
                    );
                }
            }
            DeleteObject(brush);
        }
        if s.path.len() > 2 {
            let points: Vec<POINT> = s.path.iter().map(|p| POINT { x: p.x, y: p.y }).collect();
            let saved = SaveDC(dc);
            let region = CreatePolygonRgn(points.as_ptr(), points.len() as i32, ALTERNATE);
            ExtSelectClipRgn(dc, region, RGN_AND);
            if let Some(a) = s.area {
                BitBlt(dc, a.x, a.y, a.w, a.h, s.original.dc, a.x, a.y, SRCCOPY);
            }
            RestoreDC(dc, saved);
            DeleteObject(region);
        }
        // Ink follows the orbs' shared clock so the stroke matches the orb's current color.
        let hue = if crate::orb::motion() {
            crate::orb::hue_at(crate::orb::clock())
        } else {
            0.0
        };
        draw_ink(s, GetDpiForWindow(hwnd) as i32, hue, r);
        if app.gdiplus_ready {
            draw_completion_ring(dc, s, GetDpiForWindow(hwnd) as i32, hue);
        }
        if s.touch && s.drag.is_some() {
            let p = s.cursor;
            let lens = magnifier_rect(p, s.original.width);
            let (x, y, size) = (lens.left, lens.top, lens.right - lens.left);
            let sx = (p.x - 24).clamp(0, (s.original.width - 48).max(0));
            let sy = (p.y - 24).clamp(0, (s.original.height - 48).max(0));
            StretchBlt(dc, x, y, size, size, s.original.dc, sx, sy, 48, 48, SRCCOPY);
            Rectangle(dc, x, y, x + size, y + size);
            MoveToEx(dc, x + size / 2 - 8, y + size / 2, null_mut());
            LineTo(dc, x + size / 2 + 8, y + size / 2);
            MoveToEx(dc, x + size / 2, y + size / 2 - 8, null_mut());
            LineTo(dc, x + size / 2, y + size / 2 + 8);
        }
        SelectObject(dc, oldpen);
        SelectObject(dc, oldbrush);
        DeleteObject(pen);
        paint_header(dc, s, GetDpiForWindow(hwnd) as i32, app.searching);
        BitBlt(
            output_dc,
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            dc,
            r.left,
            r.top,
            SRCCOPY,
        );
    }
    EndPaint(hwnd, &ps);
}

unsafe fn command(app: &mut App, id: usize) {
    match id {
        CAPTURE => app.begin(),
        RECTANGLE | LASSO => {
            app.lasso = id == LASSO;
            app.explicit_mode = true;
            app.reset();
        }
        RESET => app.reset(),
        KEYBOARD_AREA => app.keyboard_area(),
        SEARCH => app.search(),
        TOGGLE_BUBBLE => {
            app.bubble_enabled = !app.bubble_enabled;
            save_setting("bubble.txt", if app.bubble_enabled { "1" } else { "0" });
            if app.bubble_enabled {
                app.show_bubble();
            } else {
                ShowWindow(app.bubble, SW_HIDE);
            }
        }
        TOGGLE_MOTION => {
            let on = !crate::orb::motion();
            crate::orb::set_motion(on);
            save_setting("motion.txt", if on { "1" } else { "0" });
            // Both orbs re-read the switch on a settings change and start or stop their timers.
            SendMessageW(app.bubble, WM_SETTINGCHANGE, 0, 0);
            if let Some(panel) = &app.result {
                SendMessageW(panel.hwnd, WM_SETTINGCHANGE, 0, 0);
            }
            app.redraw_bubble();
        }
        TOGGLE_STARTUP => {
            if let Ok(exe) = std::env::current_exe() {
                crate::setup::set_startup(!crate::setup::startup_enabled(), &exe);
            }
        }
        600..=601 => {
            crate::i18n::set_english(id == 601);
            save_setting("lang.txt", if id == 601 { "en" } else { "ko" });
            update_tray_tip(app);
            SetWindowTextW(app.bubble, wide(bubble_title()).as_ptr());
        }
        300..=302 => app.install_hotkey(id - 300),
        400..=402 => {
            app.glass = id - 400;
            crate::orb::set_glass(GLASS_LEVELS[app.glass].2);
            save_setting("glass.txt", &app.glass.to_string());
            app.redraw_bubble();
        }
        500..=502 => {
            app.dim = id - 500;
            save_setting("dim.txt", &app.dim.to_string());
        }
        EXIT => {
            DestroyWindow(app.main);
        }
        _ => {}
    }
}

unsafe extern "system" fn overlay_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let ptr = state(hwnd);
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let app = &mut *ptr;
    match msg {
        WM_TIMER if wp == 2 => {
            if let Some(s) = &app.session {
                if s.completed.is_some_and(|t| t.elapsed().as_millis() >= 180) {
                    KillTimer(hwnd, 2);
                    PostMessageW(app.main, RUN_SEARCH, 0, 0);
                } else if s.completed.is_none() && s.started.elapsed().as_millis() >= 140 {
                    KillTimer(hwnd, 2);
                }
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_TIMER if wp == 4 => {
            if let Some(s) = &app.session {
                if s.drag.is_some() && s.path.len() > 1 {
                    InvalidateRect(
                        hwnd,
                        &selection_dirty(s.area, s.cursor, s.touch, s.original.width),
                        0,
                    );
                } else {
                    KillTimer(hwnd, 4);
                }
            }
            0
        }
        WM_PAINT => {
            paint_overlay(hwnd, app);
            0
        }
        WM_ERASEBKGND => 1,
        WM_COMMAND => {
            command(app, wp & 0xffff);
            0
        }
        WM_CLOSE => {
            app.end();
            0
        }
        WM_DISPLAYCHANGE => {
            app.end();
            message(
                app.main,
                crate::i18n::t(
                    "화면 구성이 변경되어 선택을 취소했습니다. 다시 시작해 주세요.",
                    "The display layout changed, so the selection was canceled. Please start again.",
                ),
            );
            0
        }
        WM_CTLCOLORSTATIC => {
            SetTextColor(wp as HDC, color(244, 247, 250));
            SetBkColor(wp as HDC, color(28, 36, 44));
            GetStockObject(BLACK_BRUSH) as isize
        }
        WM_LBUTTONDOWN | WM_MOUSEMOVE | WM_LBUTTONUP => {
            // Touch/pen can synthesize mouse events. Handle the pointer stream only once.
            if GetMessageExtraInfo() as usize & 0xffffff00 == 0xff515700 {
                return 0;
            }
            let p = Point {
                x: lp as i16 as i32,
                y: (lp >> 16) as i16 as i32,
            };
            if msg == WM_LBUTTONDOWN {
                if let Some(session) = &app.session {
                    let (header, close) = overlay_header(session, GetDpiForWindow(hwnd) as i32);
                    if inside(p, close) {
                        app.end();
                        return 0;
                    }
                    if inside(p, header) {
                        return 0;
                    }
                }
                SetFocus(hwnd);
                SetCapture(hwnd);
                app.down(p, false);
            } else if msg == WM_MOUSEMOVE {
                app.movement(p);
            } else {
                app.up(p);
                ReleaseCapture();
            }
            0
        }
        WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP => {
            let id = (wp & 0xffff) as u32;
            let mut info: POINTER_INFO = zeroed();
            if GetPointerInfo(id, &mut info) != 0 {
                let s = app.session.as_ref().unwrap();
                if s.pointer.is_some_and(|active| active != id) {
                    return 0;
                }
                if info.pointerFlags & POINTER_FLAG_CANCELED != 0 {
                    app.reset();
                    return 0;
                }
                let p = Point {
                    x: info.ptPixelLocation.x - s.origin.x,
                    y: info.ptPixelLocation.y - s.origin.y,
                };
                if msg == WM_POINTERDOWN {
                    let (header, close) = overlay_header(s, GetDpiForWindow(hwnd) as i32);
                    if inside(p, close) {
                        app.end();
                        return 0;
                    }
                    if inside(p, header) {
                        return 0;
                    }
                    SetFocus(hwnd);
                    app.down(p, info.pointerType == PT_TOUCH);
                    app.session.as_mut().unwrap().pointer = Some(id);
                } else if s.pointer == Some(id) {
                    if msg == WM_POINTERUPDATE {
                        app.movement(p);
                    } else {
                        app.up(p);
                    }
                }
            }
            0
        }
        WM_POINTERCAPTURECHANGED | WM_CAPTURECHANGED => {
            if app.session.as_ref().is_some_and(|s| s.drag.is_some()) {
                app.reset();
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe extern "system" fn bubble_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let ptr = state(hwnd);
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let app = &mut *ptr;
    match msg {
        WM_TIMER if wp == 3 => {
            if IsWindowVisible(hwnd) == 0 {
                KillTimer(hwnd, 3);
                return 0;
            }
            let target = if app.bubble_hovered { 1.0 } else { 0.0 };
            app.bubble_emphasis += (target - app.bubble_emphasis) * 0.22;
            app.redraw_bubble();
            0
        }
        WM_PAINT => {
            let mut ps = zeroed();
            BeginPaint(hwnd, &mut ps);
            EndPaint(hwnd, &ps);
            app.redraw_bubble();
            0
        }
        WM_ERASEBKGND => 1,
        WM_SHOWWINDOW => {
            if wp != 0 && crate::orb::motion() {
                SetTimer(hwnd, 3, 33, None);
            } else {
                KillTimer(hwnd, 3);
            }
            0
        }
        WM_NCHITTEST => {
            let mut r: RECT = zeroed();
            GetWindowRect(hwnd, &mut r);
            let x = lp as i16 as i32 - (r.left + r.right) / 2;
            let y = (lp >> 16) as i16 as i32 - (r.top + r.bottom) / 2;
            let radius = (r.right - r.left) * 43 / 100;
            if x * x + y * y > radius * radius {
                HTTRANSPARENT as isize
            } else {
                HTCLIENT as isize
            }
        }
        WM_SETTINGCHANGE => {
            let mut animate = 1i32;
            SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                (&mut animate as *mut i32).cast(),
                0,
            );
            app.animations = animate != 0;
            if crate::orb::motion() && IsWindowVisible(hwnd) != 0 {
                SetTimer(hwnd, 3, 33, None);
            } else {
                KillTimer(hwnd, 3);
            }
            app.redraw_bubble();
            0
        }
        WM_LBUTTONDOWN => {
            let mut p = zeroed();
            let mut r = zeroed();
            GetCursorPos(&mut p);
            GetWindowRect(hwnd, &mut r);
            app.bubble_drag = Some((p, r));
            app.bubble_moved = false;
            SetCapture(hwnd);
            0
        }
        WM_MOUSEMOVE => {
            if !app.bubble_hovered {
                app.bubble_hovered = true;
                let mut track = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                TrackMouseEvent(&mut track);
                if crate::orb::motion() {
                    SetTimer(hwnd, 3, 33, None);
                } else {
                    app.bubble_emphasis = 1.0;
                    InvalidateRect(hwnd, null(), 0);
                }
            }
            if let Some((start, r)) = app.bubble_drag {
                let mut p = zeroed();
                GetCursorPos(&mut p);
                if (p.x - start.x).abs() + (p.y - start.y).abs() > 6 {
                    app.bubble_moved = true;
                }
                if app.bubble_moved {
                    SetWindowPos(
                        hwnd,
                        null_mut(),
                        r.left + p.x - start.x,
                        r.top + p.y - start.y,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER,
                    );
                }
            }
            0
        }
        MOUSE_LEAVE => {
            app.bubble_hovered = false;
            if crate::orb::motion() {
                SetTimer(hwnd, 3, 33, None);
            } else {
                app.bubble_emphasis = 0.0;
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_LBUTTONUP => {
            let clicked = app.bubble_drag.take().is_some() && !app.bubble_moved;
            ReleaseCapture();
            if clicked {
                app.begin();
            }
            0
        }
        WM_CAPTURECHANGED => {
            app.bubble_drag = None;
            0
        }
        WM_CONTEXTMENU => {
            app.menu();
            0
        }
        WM_MOUSEACTIVATE => MA_NOACTIVATE as isize,
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe extern "system" fn main_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let ptr = state(hwnd);
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let app = &mut *ptr;
    if msg == taskbar_created() && app.tray_added {
        let mut tray: NOTIFYICONDATAW = zeroed();
        tray.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        tray.hWnd = hwnd;
        tray.uID = 1;
        tray.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        tray.uCallbackMessage = TRAY;
        tray.hIcon = crate::orb::app_icon(true);
        set_tip(&mut tray);
        Shell_NotifyIconW(NIM_ADD, &tray);
        return 0;
    }
    match msg {
        WM_COMMAND => {
            command(app, wp & 0xffff);
            0
        }
        RUN_SEARCH => {
            app.search();
            0
        }
        search::REDRAW => {
            app.begin();
            0
        }
        search::CLOSE_PANEL => {
            app.result = None;
            0
        }
        WM_HOTKEY => {
            app.begin();
            0
        }
        WM_TIMER if wp == 1 => {
            KillTimer(hwnd, 1);
            if let Err(e) = app.capture() {
                app.show_bubble();
                message(hwnd, &e);
            }
            0
        }
        #[cfg(debug_assertions)]
        WM_TIMER if wp == 9 => {
            DestroyWindow(hwnd);
            0
        }
        TRAY => {
            if lp as u32 == WM_LBUTTONUP {
                app.begin();
            } else if lp as u32 == WM_RBUTTONUP {
                app.menu();
            }
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe fn overlay_key(app: &mut App, msg: &MSG) -> bool {
    if msg.message != WM_KEYDOWN {
        return false;
    }
    let key = msg.wParam as u16;
    if key == VK_ESCAPE {
        if app
            .session
            .as_ref()
            .is_some_and(|s| s.area.is_some() || s.drag.is_some())
        {
            app.reset();
        } else {
            app.end();
        }
        return true;
    }
    if GetFocus() != app.overlay {
        return false;
    }
    match key {
        VK_RETURN => {
            app.search();
            true
        }
        VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN => {
            if app.session.as_ref().unwrap().area.is_none() {
                app.keyboard_area();
            }
            let s = app.session.as_mut().unwrap();
            if s.drag.is_some() {
                return true;
            }
            let old = selection_dirty(s.area, s.cursor, false, s.original.width);
            let step = if GetKeyState(VK_CONTROL as i32) < 0 {
                10
            } else {
                1
            };
            let dx = if key == VK_LEFT {
                -step
            } else if key == VK_RIGHT {
                step
            } else {
                0
            };
            let dy = if key == VK_UP {
                -step
            } else if key == VK_DOWN {
                step
            } else {
                0
            };
            if let Some(a) = &mut s.area {
                a.adjust(
                    dx,
                    dy,
                    GetKeyState(VK_SHIFT as i32) < 0,
                    s.original.width,
                    s.original.height,
                );
            }
            s.path.clear();
            let new = selection_dirty(s.area, s.cursor, false, s.original.width);
            InvalidateRect(app.overlay, &union_rect(old, new), 0);
            true
        }
        0x52 => {
            command(app, RECTANGLE);
            true
        }
        0x4c => {
            command(app, LASSO);
            true
        }
        0x4b => {
            command(app, KEYBOARD_AREA);
            true
        }
        0x4e => {
            command(app, RESET);
            true
        }
        _ => false,
    }
}

fn tray_tip() -> &'static str {
    crate::i18n::t("Orbom — 화면 검색", "Orbom — Screen search")
}

fn bubble_title() -> &'static str {
    crate::i18n::t("Orbom — 동그라미로 검색", "Orbom — Circle to search")
}

fn set_tip(tray: &mut NOTIFYICONDATAW) {
    let tip: Vec<u16> = tray_tip()
        .encode_utf16()
        .take(tray.szTip.len() - 1)
        .collect();
    tray.szTip[..tip.len()].copy_from_slice(&tip);
    tray.szTip[tip.len()] = 0;
}

/// The message Explorer broadcasts when the taskbar is recreated (e.g. after it restarts).
fn taskbar_created() -> u32 {
    static MESSAGE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *MESSAGE.get_or_init(|| unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) })
}

unsafe fn update_tray_tip(app: &App) {
    if !app.tray_added {
        return;
    }
    let mut tray: NOTIFYICONDATAW = zeroed();
    tray.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    tray.hWnd = app.main;
    tray.uID = 1;
    tray.uFlags = NIF_TIP;
    set_tip(&mut tray);
    Shell_NotifyIconW(NIM_MODIFY, &tray);
}

/// `Orbom.exe --uninstall [--quiet]`, as registered in Windows' installed apps list.
unsafe fn run_uninstall() {
    let quiet = std::env::args().any(|arg| arg == "--quiet");
    if !quiet
        && MessageBoxW(
            null_mut(),
            wide(crate::i18n::t(
                "Orbom을 제거할까요?\n설정과 검색 창 로그인 정보도 함께 삭제됩니다.",
                "Uninstall Orbom?\nIts settings and search sign-in data will be removed too.",
            ))
            .as_ptr(),
            wide("Orbom").as_ptr(),
            MB_YESNO | MB_ICONQUESTION,
        ) != IDYES
    {
        return;
    }
    crate::setup::uninstall();
    if !quiet {
        message(
            null_mut(),
            crate::i18n::t("Orbom을 제거했습니다.", "Orbom has been uninstalled."),
        );
    }
}

pub fn run() {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        migrate_settings();
        crate::i18n::init(
            settings_path("lang.txt")
                .and_then(|p| std::fs::read_to_string(p).ok())
                .as_deref(),
        );
        if std::env::args().any(|arg| arg == "--uninstall") {
            run_uninstall();
            return;
        }
        #[cfg(debug_assertions)]
        let test_mode = std::env::args().any(|arg| {
            matches!(
                arg.as_str(),
                "--preview-overlay"
                    | "--preview-loading"
                    | "--preview-bubble"
                    | "--demo-search"
                    | "--smoke-search"
            )
        });
        #[cfg(not(debug_assertions))]
        let test_mode = false;
        // The mutex closes the race where two quick launches both miss each other's window.
        // Its handle is intentionally kept open for the life of the process.
        CreateMutexW(null(), 0, wide(r"Local\OrbomSingleInstance").as_ptr());
        let already_running = GetLastError() == ERROR_ALREADY_EXISTS;
        if !test_mode && already_running {
            let existing = FindWindowW(wide("OrbomMain").as_ptr(), null());
            if !existing.is_null() {
                PostMessageW(existing, WM_COMMAND, CAPTURE, 0);
            }
            return;
        }
        let mut gdiplus_token = 0usize;
        let gdiplus_ready = gp::GdiplusStartup(
            &mut gdiplus_token,
            &gp::GdiplusStartupInput {
                GdiplusVersion: 1,
                ..Default::default()
            },
            null_mut(),
        ) == gp::Ok;
        let instance = GetModuleHandleW(null());
        for (name, proc) in [
            (
                "OrbomMain",
                main_proc as unsafe extern "system" fn(_, _, _, _) -> _,
            ),
            (
                "OrbomOverlay",
                overlay_proc as unsafe extern "system" fn(_, _, _, _) -> _,
            ),
        ] {
            let mut class: WNDCLASSW = zeroed();
            class.lpfnWndProc = Some(proc);
            class.hInstance = instance;
            class.hCursor = LoadCursorW(
                null_mut(),
                if name == "OrbomOverlay" {
                    IDC_CROSS
                } else {
                    IDC_ARROW
                },
            );
            class.hIcon = crate::orb::app_icon(false);
            // Keep the class name allocation alive across registration.
            let name = wide(name);
            class.lpszClassName = name.as_ptr();
            RegisterClassW(&class);
        }
        let main = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            wide("OrbomMain").as_ptr(),
            wide("Orbom").as_ptr(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if main.is_null() {
            message(
                null_mut(),
                crate::i18n::t("Orbom을 시작하지 못했습니다.", "Orbom couldn't start."),
            );
            return;
        }
        let mut animate: i32 = 1;
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&mut animate as *mut i32).cast(),
            0,
        );
        let mut app = Box::new(App {
            main,
            overlay: null_mut(),
            previous: null_mut(),
            session: None,
            lasso: true,
            explicit_mode: false,
            pending: false,
            bubble: null_mut(),
            glass: load_choice("glass.txt", GLASS_LEVELS.len(), 1),
            dim: load_choice("dim.txt", DIM_LEVELS.len(), 1),
            bubble_enabled: settings_path("bubble.txt")
                .and_then(|p| std::fs::read_to_string(p).ok())
                .is_none_or(|v| v.trim() != "0"),
            bubble_drag: None,
            bubble_moved: false,
            bubble_hovered: false,
            bubble_emphasis: 0.0,
            bubble_glow: match Glow::new() {
                Ok(glow) => glow,
                Err(e) => {
                    message(main, &e);
                    DestroyWindow(main);
                    return;
                }
            },
            animations: animate != 0,
            gdiplus_ready,
            hotkey: 0,
            hotkey_registered: false,
            tray_added: false,
            result: None,
            search_generation: 0,
            searching: false,
        });
        SetWindowLongPtrW(main, GWLP_USERDATA, &mut *app as *mut App as isize);
        crate::orb::set_glass(GLASS_LEVELS[app.glass].2);
        crate::orb::set_motion(load_choice("motion.txt", 2, 1) == 1);
        app.create_bubble();
        let mut tray: NOTIFYICONDATAW = zeroed();
        tray.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        tray.hWnd = main;
        tray.uID = 1;
        tray.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        tray.uCallbackMessage = TRAY;
        tray.hIcon = crate::orb::app_icon(true);
        set_tip(&mut tray);
        if !test_mode {
            app.tray_added = Shell_NotifyIconW(NIM_ADD, &tray) != 0;
        }

        let saved = settings_path("hotkey.txt")
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|v| v.trim().parse::<usize>().ok())
            .filter(|v| *v < 3)
            .unwrap_or(0);

        if !test_mode {
            app.install_hotkey(saved);
        }
        // A launch by the user (Start menu, pen button shortcut) goes straight to selecting, like a
        // second launch does; sign-in startup and the installer pass --background to stay in the tray.
        if !test_mode && !std::env::args().any(|arg| arg == "--background") {
            PostMessageW(main, WM_COMMAND, CAPTURE, 0);
        }
        #[cfg(debug_assertions)]
        if std::env::args().any(|arg| arg == "--preview-loading") {
            ShowWindow(app.bubble, SW_HIDE);
            app.bubble_enabled = false;
            let mut monitor: MONITORINFO = zeroed();
            monitor.cbSize = size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(
                MonitorFromWindow(main, MONITOR_DEFAULTTONEAREST),
                &mut monitor,
            );
            match Panel::new(Vec::new(), monitor.rcWork, main, 1) {
                Ok(panel) => app.result = Some(panel),
                Err(e) => {
                    message(main, &e);
                    PostQuitMessage(1);
                }
            }
        }
        #[cfg(debug_assertions)]
        if std::env::args().any(|arg| arg == "--preview-overlay") {
            ShowWindow(app.bubble, SW_HIDE);
            app.bubble_enabled = false;
            let mut monitor: MONITORINFO = zeroed();
            monitor.cbSize = size_of::<MONITORINFO>() as u32;
            let mut cursor = zeroed();
            GetCursorPos(&mut cursor);
            GetMonitorInfoW(
                MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST),
                &mut monitor,
            );
            let width = 960.min(monitor.rcWork.right - monitor.rcWork.left);
            let height = 640.min(monitor.rcWork.bottom - monitor.rcWork.top);
            let x = monitor.rcWork.left + (monitor.rcWork.right - monitor.rcWork.left - width) / 2;
            let y = monitor.rcWork.top;
            let original = Bitmap::new(width, height).unwrap();
            let pixels =
                std::slice::from_raw_parts_mut(original.data, width as usize * height as usize * 4);
            for row in 0..height as usize {
                for col in 0..width as usize {
                    let i = (row * width as usize + col) * 4;
                    let warm = (col as i32 - width / 2).pow(2) + (row as i32 - height / 2).pow(2)
                        < 120 * 120;
                    pixels[i..i + 4].copy_from_slice(if warm {
                        &[34, 136, 241, 255]
                    } else {
                        &[232, 237, 245, 255]
                    });
                }
            }
            let dim = Bitmap::new(width, height).unwrap();
            for (i, value) in pixels.iter().enumerate() {
                *dim.data.add(i) = (*value as u16 * 48 / 100) as u8;
            }
            let frame = Bitmap::new(width, height).unwrap();
            let mut path = Vec::new();
            for step in 0..=64 {
                let angle = step as f32 * std::f32::consts::TAU / 64.0;
                path.push(Point {
                    x: width / 2 + (155.0 * angle.cos()) as i32,
                    y: height / 2 + (148.0 * angle.sin()) as i32,
                });
            }
            let area = bounds(&path, width, height);
            app.session = Some(Session {
                original,
                dim,
                frame,
                origin: Point { x, y },
                monitor: monitor.rcWork,
                area,
                path,
                drag: None,
                pointer: None,
                touch: false,
                cursor: Point::default(),
                smoother: None,
                stroke_at: None,
                started: Instant::now() - std::time::Duration::from_millis(200),
                completed: None,
            });
            let overlay = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_APPWINDOW,
                wide("OrbomOverlay").as_ptr(),
                wide("Orbom — design preview").as_ptr(),
                WS_POPUP,
                x,
                y,
                width,
                height,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            app.overlay = overlay;
            SetWindowLongPtrW(overlay, GWLP_USERDATA, &mut *app as *mut App as isize);
            ShowWindow(overlay, SW_SHOW);
            SetForegroundWindow(overlay);
            UpdateWindow(overlay);
            if let Some(session) = &app.session
                && let Some(dib) = crop_dib(
                    session.frame.pixels(),
                    width,
                    height,
                    Area {
                        x: 0,
                        y: 0,
                        w: width,
                        h: height,
                    },
                )
                && let Ok(png) = search::png_from_dib(&dib)
            {
                let _ = std::fs::write("target/design-preview.png", png);
            }
            SetTimer(main, 9, 30_000, None);
        }
        #[cfg(debug_assertions)]
        if std::env::args().any(|arg| arg == "--demo-search" || arg == "--smoke-search") {
            ShowWindow(app.bubble, SW_HIDE);
            // Development smoke test: generated artwork only, never a desktop screenshot.
            let mut pixels = vec![255u8; 256 * 256 * 4];
            for y in 0i32..256 {
                for x in 0i32..256 {
                    let p = ((y * 256 + x) * 4) as usize;
                    if (x - 128).pow(2) + (y - 142).pow(2) < 82 * 82 {
                        pixels[p..p + 4].copy_from_slice(&[18, 140, 245, 255]);
                    }
                    if (90..140).contains(&x) && (45..70).contains(&y) {
                        pixels[p..p + 4].copy_from_slice(&[50, 125, 40, 255]);
                    }
                }
            }
            let mut monitor: MONITORINFO = zeroed();
            monitor.cbSize = size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(
                MonitorFromWindow(main, MONITOR_DEFAULTTONEAREST),
                &mut monitor,
            );
            app.bubble_enabled = false;
            // Exercise the actual gesture-completion timer and search path on a hidden synthetic frame.
            let original = Bitmap::new(256, 256).unwrap();
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), original.data, pixels.len());
            let dim = Bitmap::new(256, 256).unwrap();
            let frame = Bitmap::new(256, 256).unwrap();
            app.session = Some(Session {
                original,
                dim,
                frame,
                origin: Point::default(),
                monitor: monitor.rcWork,
                area: None,
                path: vec![],
                drag: None,
                pointer: None,
                touch: false,
                cursor: Point::default(),
                smoother: None,
                stroke_at: None,
                started: Instant::now(),
                completed: None,
            });
            app.overlay = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                wide("OrbomOverlay").as_ptr(),
                wide("Orbom test").as_ptr(),
                WS_POPUP,
                0,
                0,
                256,
                256,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            SetWindowLongPtrW(app.overlay, GWLP_USERDATA, &mut *app as *mut App as isize);
            let start = Point { x: 238, y: 128 };
            app.down(start, false);
            for step in 1..=48 {
                let angle = step as f32 * std::f32::consts::TAU / 48.0;
                app.movement(Point {
                    x: 128 + (110.0 * angle.cos()) as i32,
                    y: 128 + (110.0 * angle.sin()) as i32,
                });
            }
            app.up(start);
        }
        let mut msg: MSG = zeroed();
        loop {
            let result = GetMessageW(&mut msg, null_mut(), 0, 0);
            if result <= 0 {
                break;
            }
            if !app.overlay.is_null() {
                if overlay_key(&mut app, &msg) {
                    continue;
                }
                if IsDialogMessageW(app.overlay, &msg) != 0 {
                    continue;
                }
            } else if app
                .result
                .as_ref()
                .is_some_and(|p| msg.hwnd == p.hwnd || IsChild(p.hwnd, msg.hwnd) != 0)
                && msg.message == WM_KEYDOWN
                && msg.wParam == VK_ESCAPE as usize
            {
                app.result = None;
                continue;
            }
            if app
                .result
                .as_ref()
                .is_some_and(|panel| panel.keyboard_navigation(&msg))
            {
                continue;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        app.result = None;
        if app.hotkey_registered {
            UnregisterHotKey(main, 10 + app.hotkey as i32);
        }
        if app.tray_added {
            Shell_NotifyIconW(NIM_DELETE, &tray);
        }
        if !app.overlay.is_null() {
            app.end();
        }
        SetWindowLongPtrW(app.bubble, GWLP_USERDATA, 0);
        DestroyWindow(app.bubble);
        if gdiplus_ready {
            gp::GdiplusShutdown(gdiplus_token);
        }
    }
}
