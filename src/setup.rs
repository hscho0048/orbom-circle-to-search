//! Per-user install, uninstall and "start with Windows", shared by the app and the installer.
//! Nothing here needs administrator rights: files go to %LOCALAPPDATA%\Programs\Orbom and all
//! registration lives under HKEY_CURRENT_USER.
#![allow(unsafe_op_in_unsafe_fn, dead_code)]

use crate::i18n::t;
use std::{
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    System::Registry::*,
    UI::WindowsAndMessaging::{FindWindowW, IsWindow, PostMessageW, WM_COMMAND},
};

pub const APP_NAME: &str = "Orbom";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Orbom";
/// Menu command id of "Quit" in the running app (win.rs EXIT).
pub const EXIT_COMMAND: usize = 112;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn local_app_data() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
}

pub fn install_dir() -> Option<PathBuf> {
    local_app_data().map(|p| p.join("Programs").join(APP_NAME))
}

pub fn installed_exe() -> Option<PathBuf> {
    install_dir().map(|p| p.join("Orbom.exe"))
}

/// Settings and the WebView2 profile.
pub fn data_dir() -> Option<PathBuf> {
    local_app_data().map(|p| p.join(APP_NAME))
}

fn shortcut_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|p| {
        PathBuf::from(p)
            .join(r"Microsoft\Windows\Start Menu\Programs")
            .join("Orbom.lnk")
    })
}

unsafe fn set_string(key: HKEY, name: &str, value: &str) -> bool {
    let value = wide(value);
    RegSetValueExW(
        key,
        wide(name).as_ptr(),
        0,
        REG_SZ,
        value.as_ptr().cast(),
        (value.len() * 2) as u32,
    ) == ERROR_SUCCESS
}

unsafe fn set_dword(key: HKEY, name: &str, value: u32) -> bool {
    RegSetValueExW(
        key,
        wide(name).as_ptr(),
        0,
        REG_DWORD,
        (&value as *const u32).cast(),
        4,
    ) == ERROR_SUCCESS
}

unsafe fn open_key(path: &str) -> Option<HKEY> {
    let mut key = null_mut();
    (RegCreateKeyExW(
        HKEY_CURRENT_USER,
        wide(path).as_ptr(),
        0,
        null(),
        0,
        KEY_SET_VALUE | KEY_QUERY_VALUE,
        null(),
        &mut key,
        null_mut(),
    ) == ERROR_SUCCESS)
        .then_some(key)
}

fn quoted(path: &Path) -> String {
    format!("\"{}\"", path.display())
}

pub fn startup_enabled() -> bool {
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide(RUN_KEY).as_ptr(),
            wide(APP_NAME).as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            null_mut(),
            null_mut(),
        ) == ERROR_SUCCESS
    }
}

/// Registers (or removes) `exe` to start when the user signs in.
pub fn set_startup(on: bool, exe: &Path) -> bool {
    unsafe {
        if !on {
            let status = RegDeleteKeyValueW(
                HKEY_CURRENT_USER,
                wide(RUN_KEY).as_ptr(),
                wide(APP_NAME).as_ptr(),
            );
            return status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND;
        }
        let Some(key) = open_key(RUN_KEY) else {
            return false;
        };
        let ok = set_string(key, APP_NAME, &format!("{} --background", quoted(exe)));
        RegCloseKey(key);
        ok
    }
}

pub fn is_installed() -> bool {
    installed_exe().is_some_and(|p| p.is_file())
}

/// Asks a running Orbom to quit and waits briefly for it to go away.
pub fn close_running_app() {
    unsafe {
        let class = wide("OrbomMain");
        let window = FindWindowW(class.as_ptr(), null());
        if window.is_null() {
            return;
        }
        PostMessageW(window, WM_COMMAND, EXIT_COMMAND, 0);
        let start = Instant::now();
        while IsWindow(window) != 0 && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(100));
        }
        // Give the process a moment to release its executable after the window is gone.
        std::thread::sleep(Duration::from_millis(300));
    }
}

fn create_shortcut(target: &Path, link: &Path) -> Result<(), String> {
    use windows::{
        Win32::{
            System::Com::{
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoUninitialize, IPersistFile,
            },
            UI::Shell::{IShellLinkW, ShellLink},
        },
        core::{HSTRING, Interface},
    };
    unsafe {
        let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let result = (|| -> windows::core::Result<()> {
            let shell_link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            shell_link.SetPath(&HSTRING::from(target.as_os_str()))?;
            shell_link.SetIconLocation(&HSTRING::from(target.as_os_str()), 0)?;
            shell_link.SetDescription(&HSTRING::from(t(
                "동그라미로 화면을 검색합니다",
                "Circle anything on screen to search it",
            )))?;
            if let Some(dir) = target.parent() {
                shell_link.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()))?;
            }
            shell_link
                .cast::<IPersistFile>()?
                .Save(&HSTRING::from(link.as_os_str()), true)
        })();
        if initialized {
            CoUninitialize();
        }
        result.map_err(|e| e.message().to_string())
    }
}

/// Copies `payload` (the app executable) and `extras` (name, contents) into place and registers
/// it with Windows.
pub fn install(payload: &[u8], version: &str, extras: &[(&str, &str)]) -> Result<PathBuf, String> {
    let dir = install_dir().ok_or("LOCALAPPDATA is not set")?;
    let exe = installed_exe().ok_or("LOCALAPPDATA is not set")?;
    close_running_app();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // A just-closed instance can hold the file for a moment; retry briefly.
    let mut attempt = 0;
    loop {
        match std::fs::write(&exe, payload) {
            Ok(()) => break,
            Err(_) if attempt < 20 => {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(e) => {
                return Err(format!(
                    "{}\n{e}",
                    t(
                        "프로그램 파일을 쓰지 못했습니다. 실행 중인 Orbom을 종료한 뒤 다시 시도해 주세요.",
                        "Couldn't write the program file. Quit Orbom and try again.",
                    )
                ));
            }
        }
    }
    for (name, contents) in extras {
        std::fs::write(dir.join(name), contents).map_err(|e| e.to_string())?;
    }
    if let Some(link) = shortcut_path() {
        create_shortcut(&exe, &link)?;
    }
    set_startup(true, &exe);
    unsafe {
        let key = open_key(UNINSTALL_KEY).ok_or("Couldn't register the uninstaller")?;
        let command = format!("{} --uninstall", quoted(&exe));
        set_string(key, "DisplayName", APP_NAME);
        set_string(key, "DisplayVersion", version);
        set_string(key, "Publisher", APP_NAME);
        set_string(key, "DisplayIcon", &exe.display().to_string());
        set_string(key, "InstallLocation", &dir.display().to_string());
        set_string(key, "UninstallString", &command);
        set_string(key, "QuietUninstallString", &format!("{command} --quiet"));
        set_dword(key, "EstimatedSize", (payload.len() / 1024) as u32);
        set_dword(key, "NoModify", 1);
        set_dword(key, "NoRepair", 1);
        RegCloseKey(key);
    }
    Ok(exe)
}

/// Removes everything `install` created plus the user's settings, then deletes the program
/// folder from a short-lived helper once this process (which may be that program) has exited.
pub fn uninstall() {
    close_running_app();
    if let Some(exe) = installed_exe() {
        set_startup(false, &exe);
    }
    unsafe {
        RegDeleteTreeW(HKEY_CURRENT_USER, wide(UNINSTALL_KEY).as_ptr());
    }
    if let Some(link) = shortcut_path() {
        let _ = std::fs::remove_file(link);
    }
    let folders: Vec<PathBuf> = [install_dir(), data_dir()].into_iter().flatten().collect();
    // WebView2 helpers may still be releasing the profile, so data is removed by the helper too.
    let targets: Vec<String> = folders
        .iter()
        .map(|p| format!("rmdir /s /q {}", quoted(p)))
        .collect();
    let _ = std::process::Command::new("cmd.exe")
        .raw_arg(format!(
            "/d /c ping -n 3 127.0.0.1 >nul & {}",
            targets.join(" & ")
        ))
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}
