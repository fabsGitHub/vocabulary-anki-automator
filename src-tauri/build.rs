use std::{env, path::PathBuf};

fn main() {
    let mut attributes = tauri_build::Attributes::new();
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        // The Tauri runtime imports `TaskDialogIndirect`, which only Common
        // Controls v6 has, and Windows loads v6 only for an executable whose
        // manifest asks for it. tauri-build embeds that manifest into the
        // app's binary alone, so the test executables could not even start
        // (STATUS_ENTRYPOINT_NOT_FOUND). The linker embeds it into every
        // executable of this crate instead, tests included, as Tauri's own
        // build script does for its tests.
        attributes = attributes
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
        let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("set by cargo"))
            .join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
    if let Err(error) = tauri_build::try_build(attributes) {
        panic!("tauri-build failed: {error:#}");
    }
}
