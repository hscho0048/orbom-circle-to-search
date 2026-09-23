//! Orbom setup: installs the embedded app for the current user, no administrator rights needed.
#![windows_subsystem = "windows"]

#[path = "../../src/i18n.rs"]
mod i18n;
#[path = "../../src/setup.rs"]
mod setup;

use i18n::t;
use std::ptr::null_mut;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    IDYES, MB_ICONERROR, MB_ICONINFORMATION, MB_ICONQUESTION, MB_OK, MB_YESNO, MessageBoxW,
};

static APP: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/orbom.exe"));
/// License notices installed next to the app, as the licenses require for binary distribution.
static NOTICES: [(&str, &str); 3] = [
    ("LICENSE-MIT.txt", include_str!("../../LICENSE-MIT")),
    ("LICENSE-APACHE.txt", include_str!("../../LICENSE-APACHE")),
    (
        "THIRD_PARTY_LICENSES.txt",
        include_str!("../../THIRD_PARTY_LICENSES.txt"),
    ),
];

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn ask(text: &str, style: u32) -> i32 {
    unsafe {
        MessageBoxW(
            null_mut(),
            wide(text).as_ptr(),
            wide(t("Orbom 설치", "Orbom Setup")).as_ptr(),
            style,
        )
    }
}

fn main() {
    let saved = setup::data_dir().and_then(|p| std::fs::read_to_string(p.join("lang.txt")).ok());
    i18n::init(saved.as_deref());
    if APP.is_empty() {
        ask(
            "This installer was built without the app (ORBOM_EXE was not set).",
            MB_OK | MB_ICONERROR,
        );
        return;
    }
    let quiet = std::env::args().any(|arg| arg == "/S" || arg == "--quiet");
    let location = setup::install_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let prompt = if setup::is_installed() {
        format!(
            "{}\n\n{location}",
            t(
                "Orbom이 이미 설치되어 있습니다. 이 버전으로 업데이트할까요?",
                "Orbom is already installed. Update it to this version?",
            )
        )
    } else {
        format!(
            "{}\n\n{location}\n\n{}",
            t("Orbom을 설치할까요?", "Install Orbom?"),
            t(
                "• 시작 메뉴에 바로가기를 추가합니다\n• Windows 시작 시 자동으로 실행합니다 (트레이 메뉴에서 끌 수 있어요)\n• 제거는 Windows 설정 > 앱 > 설치된 앱에서 할 수 있습니다",
                "• Adds a Start menu shortcut\n• Starts with Windows (you can turn this off from the tray menu)\n• Uninstall any time from Settings > Apps > Installed apps",
            )
        )
    };
    if !quiet && ask(&prompt, MB_YESNO | MB_ICONQUESTION) != IDYES {
        return;
    }
    match setup::install(APP, env!("CARGO_PKG_VERSION"), &NOTICES) {
        Ok(exe) => {
            if !quiet {
                ask(
                    t(
                        "설치가 끝났습니다. Orbom을 시작합니다.\n트레이 아이콘이나 화면의 오브 버튼으로 사용할 수 있어요.",
                        "Setup is complete. Starting Orbom.\nUse it from the tray icon or the orb button on your screen.",
                    ),
                    MB_OK | MB_ICONINFORMATION,
                );
            }
            let _ = std::process::Command::new(exe).arg("--background").spawn();
        }
        Err(e) => {
            ask(
                &format!("{}\n\n{e}", t("설치하지 못했습니다.", "Setup failed.")),
                MB_OK | MB_ICONERROR,
            );
        }
    }
}
