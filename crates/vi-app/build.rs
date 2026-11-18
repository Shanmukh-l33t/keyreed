use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=keyreed.rc");
    println!("cargo:rerun-if-changed=keyreed.manifest");
    println!("cargo:rerun-if-changed=../../assets/keyreed.ico");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os != "windows" {
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let rc_path = manifest_dir.join("keyreed.rc");

    if target_env == "gnu" {
        let obj_path = out_dir.join("keyreed_res.o");
        // A binary without the embedded manifest/resources (common controls v6, DPI awareness, UTF-8
        // code page, icon) is broken, so a missing windres must fail the build, never skip silently.
        let status = Command::new("windres")
            .current_dir(&manifest_dir)
            .args([rc_path.to_str().unwrap(), "-O", "coff", "-o", obj_path.to_str().unwrap()])
            .status()
            .expect("windres not found on PATH: build with scripts/build-release.sh (llvm-mingw UCRT toolchain)");
        assert!(status.success() && obj_path.exists(), "windres failed to compile keyreed.rc");
        println!("cargo:rustc-link-arg-bins={}", obj_path.display());
    } else if target_env == "msvc" {
        embed_msvc(&rc_path);
    }
}

// Without the embedded manifest/resources the binary is broken, so an MSVC build embeds them too.
// The build dependency only exists on Windows hosts; cross-building to MSVC is not supported.
#[cfg(windows)]
fn embed_msvc(rc_path: &std::path::Path) {
    embed_resource::compile(rc_path, embed_resource::NONE)
        .manifest_required()
        .expect("embedding keyreed.rc (manifest, icon) failed");
}

#[cfg(not(windows))]
fn embed_msvc(_rc_path: &std::path::Path) {
    panic!("an MSVC build of keyreed needs a Windows host (it embeds keyreed.rc with embed-resource)");
}
