#[allow(dead_code)]
#[path = "../src/art.rs"]
mod art;

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=setup.manifest");
    println!("cargo:rerun-if-changed=../src/art.rs");
    println!("cargo:rerun-if-env-changed=ORBOM_EXE");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    // The installer carries the already-built app; build.ps1 points ORBOM_EXE at it.
    let payload = out.join("orbom.exe");
    match std::env::var_os("ORBOM_EXE") {
        Some(path) => {
            println!("cargo:rerun-if-changed={}", PathBuf::from(&path).display());
            std::fs::copy(&path, &payload).expect("ORBOM_EXE must point to the built orbom.exe");
        }
        None => {
            println!("cargo:warning=ORBOM_EXE is not set; building an installer without an app");
            std::fs::write(&payload, []).unwrap();
        }
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        let manifest =
            PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("setup.manifest");
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
        let res = out.join("icon.res");
        std::fs::write(&res, art::icon_res()).unwrap();
        println!("cargo:rustc-link-arg-bins={}", res.display());
    }
}
