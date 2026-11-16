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
        let status = Command::new("windres")
            .current_dir(&manifest_dir)
            .args([
                rc_path.to_str().unwrap(),
                "-O",
                "coff",
                "-o",
                obj_path.to_str().unwrap(),
            ])
            .status();
        if let Ok(s) = status {
            if s.success() && obj_path.exists() {
                println!("cargo:rustc-link-arg-bins={}", obj_path.display());
            }
        }
    }
}
