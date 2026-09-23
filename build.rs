#[allow(dead_code)]
#[path = "src/art.rs"]
mod art;

fn main() {
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-changed=src/art.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        let manifest = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
            .join("app.manifest");
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
        // The executable and window icon are the same orb the app draws, rendered at build time.
        let res = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("icon.res");
        std::fs::write(&res, art::icon_res()).unwrap();
        println!("cargo:rustc-link-arg-bins={}", res.display());
    }
}
