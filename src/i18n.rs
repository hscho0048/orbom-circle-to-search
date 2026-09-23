//! Korean/English UI strings. Every user-facing string is written as `t("한국어", "English")`.

use std::sync::atomic::{AtomicBool, Ordering};

static ENGLISH: AtomicBool = AtomicBool::new(false);

pub fn set_english(on: bool) {
    ENGLISH.store(on, Ordering::Relaxed);
}

pub fn english() -> bool {
    ENGLISH.load(Ordering::Relaxed)
}

/// Picks the string for the current UI language.
pub fn t(ko: &'static str, en: &'static str) -> &'static str {
    if english() { en } else { ko }
}

/// English unless Windows' display language is Korean.
pub fn system_prefers_english() -> bool {
    const LANG_KOREAN: u16 = 0x12;
    let language = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() };
    language & 0x3ff != LANG_KOREAN
}

/// Applies the saved choice ("ko"/"en") or falls back to the system language.
pub fn init(saved: Option<&str>) {
    set_english(match saved.map(str::trim) {
        Some("en") => true,
        Some("ko") => false,
        _ => system_prefers_english(),
    });
}
