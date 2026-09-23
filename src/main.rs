#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod art;
#[cfg(windows)]
mod i18n;
#[cfg(windows)]
mod orb;
#[cfg(windows)]
mod search;
mod selection;
#[cfg(windows)]
mod setup;
#[cfg(windows)]
mod win;

fn main() {
    #[cfg(windows)]
    win::run();
    #[cfg(not(windows))]
    eprintln!("Orbom requires Windows 10/11.");
}
