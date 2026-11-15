//! Compile-time integrity pins for model files.
//!
//! The expected SHA-256 values live in this binary. A sidecar manifest next to the
//! model directory is not consulted and cannot authorize a load. A directory that
//! merely exists is not a verified model.

use crate::catalog::{extract_tar_bz2_safely, find_model_spec};
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct FilePin {
    pub relative: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

const PARAKEET_V2: &[FilePin] = &[
    FilePin { relative: "decoder.int8.onnx", size: 7_257_753, sha256: "4d097bae3bac5c96018d5293ba39f1b657296102b142affda40378f38b020f41" },
    FilePin { relative: "encoder.int8.onnx", size: 652_282_309, sha256: "92a10bd9c23936ea0566c29f963bc57c2a5df4261b6a8921c39abea66557ca8b" },
    FilePin { relative: "joiner.int8.onnx", size: 1_739_080, sha256: "77a29f3e5ac7d37074e0af465391b80f314015d5b97f0f90c6c7767f1d56f428" },
    FilePin { relative: "tokens.txt", size: 9_384, sha256: "ec182b70dd42113aff6c5372c75cac58c952443eb22322f57bbd7f53977d497d" },
];

const MOONSHINE_TINY: &[FilePin] = &[
    FilePin { relative: "cached_decode.int8.onnx", size: 45_264_830, sha256: "2aff28bba6a03d8dcf5c9feac45462629bae37317442299f28115ad09da773f6" },
    FilePin { relative: "encode.int8.onnx", size: 18_249_187, sha256: "8774dfba578de027ec6595c2c654a0836434489bc963a0db124a7f181f571acb" },
    FilePin { relative: "preprocess.onnx", size: 6_800_738, sha256: "f33addce61a143460fe753b5ee5b7db255e5140b5b779c065b94f6c83ff0bf4e" },
    FilePin { relative: "tokens.txt", size: 436_688, sha256: "1165c2aeb9f72f457a83be2d459a09054f27490acd9b41bd43794dfd25e296ea" },
    FilePin { relative: "uncached_decode.int8.onnx", size: 53_216_096, sha256: "216737000dd5881a17aa043f6bbd286add33e4c3b0ae257153e2ec15438bdc41" },
];

const MOONSHINE_BASE: &[FilePin] = &[
    FilePin { relative: "cached_decode.int8.onnx", size: 99_983_837, sha256: "2db74e51cedf64a8b1be3c8192e0bb5e4923af0e90bd9e87f8e8771873f8ea03" },
    FilePin { relative: "encode.int8.onnx", size: 50_311_494, sha256: "7e38770f776f2e5583a53b052936005df2ba5c833d7e09c2a5fd796b94bf73e2" },
    FilePin { relative: "preprocess.onnx", size: 14_077_290, sha256: "ffa630d395c5ccf76f5d4954be5b882df76aaf6491519ec01fd82ea7a3819fb2" },
    FilePin { relative: "tokens.txt", size: 436_688, sha256: "1165c2aeb9f72f457a83be2d459a09054f27490acd9b41bd43794dfd25e296ea" },
    FilePin { relative: "uncached_decode.int8.onnx", size: 122_120_451, sha256: "c01f4b35093bcac20d352d23a75a539e772964579f9d024a90e5e6f09cae9987" },
];

const PARAKEET_V3: &[FilePin] = &[
    FilePin { relative: "decoder.int8.onnx", size: 11_845_274, sha256: "0222a86147299d4dee90edf95b6d13827edb42ad8b7c9cf0b2582f030937c0b5" },
    FilePin { relative: "encoder.int8.onnx", size: 652_282_294, sha256: "1afbe69c1ffdabee94755f42e4daf1d93549131a3da11356bda84fcef0f2c479" },
    FilePin { relative: "joiner.int8.onnx", size: 6_355_277, sha256: "cbd02f3696e9944970111fb439d9c3fd365a80374565f13f2b9c7007d253fc98" },
    FilePin { relative: "tokens.txt", size: 93_939, sha256: "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d" },
];

const SENSE_VOICE: &[FilePin] = &[
    FilePin { relative: "model.int8.onnx", size: 239_233_841, sha256: "c71f0ce00bec95b07744e116345e33d8cbbe08cef896382cf907bf4b51a2cd51" },
    FilePin { relative: "tokens.txt", size: 315_894, sha256: "f449eb28dc567533d7fa59be34e2abca8784f771850c78a47fb731a31429a1dc" },
];

const WHISPER_SMALL_EN: &[FilePin] = &[
    FilePin { relative: "small.en-decoder.int8.onnx", size: 262_223_042, sha256: "710ccf890e10f3faa15f51ec346081a2723c9f3adb6e4da81c6573a5a6f877fb" },
    FilePin { relative: "small.en-encoder.int8.onnx", size: 112_442_483, sha256: "8bdac288f369aa94ee2194059238c465ed82ea9d47ee8fa4a8c0a891873e462f" },
    FilePin { relative: "small.en-tokens.txt", size: 835_554, sha256: "306cd27f03c1a714eca7108e03d66b7dc042abe8c258b44c199a7ed9838dd930" },
];

pub const SILERO_VAD_SIZE: u64 = 643_854;
pub const SILERO_VAD_SHA256: &str = "9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6";

pub const QWEN35_GGUF_NAME: &str = "Qwen3.5-0.8B-Q4_K_M.gguf";
pub const QWEN35_GGUF_SIZE: u64 = 532_517_120;
pub const QWEN35_GGUF_SHA256: &str = "bd258782e35f7f458f8aced1adc053e6e92e89bc735ba3be89d38a06121dc517";

pub fn pins_for(model_id: &str) -> Option<&'static [FilePin]> {
    let pins = match model_id {
        "parakeet-tdt-0.6b-v2-int8" | "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8" => PARAKEET_V2,
        "parakeet-tdt-0.6b-v3-int8" | "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8" => PARAKEET_V3,
        "moonshine-tiny-en-int8" | "sherpa-onnx-moonshine-tiny-en-int8" => MOONSHINE_TINY,
        "moonshine-base-en-int8" | "sherpa-onnx-moonshine-base-en-int8" => MOONSHINE_BASE,
        "sense-voice-small-int8" | "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17" => SENSE_VOICE,
        "whisper-small.en-int8" | "sherpa-onnx-whisper-small.en" => WHISPER_SMALL_EN,
        _ => return None,
    };
    if pins.is_empty() {
        None
    } else {
        Some(pins)
    }
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub fn verify_named_files(dir: &Path, files: &[(&str, u64, &str)]) -> Result<()> {
    if files.is_empty() {
        bail!("refusing to verify {} with an empty pin list", dir.display());
    }
    if !dir.is_dir() {
        bail!("model path is not a directory: {}", dir.display());
    }
    for (rel, size, sha) in files {
        let path = dir.join(rel);
        let meta = fs::metadata(&path).with_context(|| format!("missing required file {}", path.display()))?;
        if !meta.is_file() {
            bail!("required model path is not a file: {}", path.display());
        }
        if meta.len() != *size {
            bail!(
                "size mismatch for {}: expected {} bytes, got {}",
                path.display(),
                size,
                meta.len()
            );
        }
        let digest = sha256_file(&path)?;
        if !digest.eq_ignore_ascii_case(sha) {
            bail!("SHA-256 mismatch for {}: expected {}, got {}", path.display(), sha, digest);
        }
    }
    Ok(())
}

pub fn verify_model_dir(dir: &Path, model_id: &str) -> Result<()> {
    let pins = pins_for(model_id).with_context(|| {
        format!("no compile-time file pins for '{model_id}'; refusing to load an unpinned model")
    })?;
    let borrowed: Vec<(&str, u64, &str)> = pins.iter().map(|p| (p.relative, p.size, p.sha256)).collect();
    verify_named_files(dir, &borrowed)
}

pub fn verify_silero(path: &Path) -> Result<()> {
    verify_named_files(
        path.parent().unwrap_or_else(|| Path::new(".")),
        &[(
            path.file_name().and_then(|s| s.to_str()).unwrap_or("silero_vad.onnx"),
            SILERO_VAD_SIZE,
            SILERO_VAD_SHA256,
        )],
    )
}

pub fn verify_gguf(path: &Path) -> Result<()> {
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    if name != QWEN35_GGUF_NAME {
        bail!("refusing unpinned weight file {}", path.display());
    }
    verify_named_files(
        path.parent().unwrap_or_else(|| Path::new(".")),
        &[(QWEN35_GGUF_NAME, QWEN35_GGUF_SIZE, QWEN35_GGUF_SHA256)],
    )
}

fn rejected_root(models_root: &Path) -> PathBuf {
    models_root.join(".rejected")
}

pub fn quarantine_dir(dir: &Path) -> Result<PathBuf> {
    let parent = dir.parent().context("model dir has no parent")?;
    let name = dir.file_name().context("model dir has no name")?;
    let dest_root = rejected_root(parent);
    fs::create_dir_all(&dest_root)?;
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dest = dest_root.join(format!("{}-{}", name.to_string_lossy(), nanos));
    fs::rename(dir, &dest).with_context(|| format!("quarantine rename {} -> {}", dir.display(), dest.display()))?;
    Ok(dest)
}

fn pid_path(models_root: &Path) -> PathBuf {
    models_root.join(".incoming").join("install.pid")
}

fn staging_path(models_root: &Path) -> PathBuf {
    models_root.join(".staging")
}

fn read_install_pid(models_root: &Path) -> Option<u32> {
    let text = fs::read_to_string(pid_path(models_root)).ok()?;
    text.trim().parse().ok()
}

fn process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        Path::new(&format!("/proc/{pid}")).exists()
    }
    #[cfg(windows)]
    {
        extern "system" {
            fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut core::ffi::c_void;
            fn GetExitCodeProcess(handle: *mut core::ffi::c_void, code: *mut u32) -> i32;
            fn CloseHandle(handle: *mut core::ffi::c_void) -> i32;
        }
        const QUERY: u32 = 0x1000;
        const STILL_ACTIVE: u32 = 259;
        unsafe {
            let handle = OpenProcess(QUERY, 0, pid);
            if handle.is_null() {
                return false;
            }
            let mut code = 0u32;
            let ok = GetExitCodeProcess(handle, &mut code);
            CloseHandle(handle);
            ok != 0 && code == STILL_ACTIVE
        }
    }
}

fn cmdline_is_keyreed(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let raw = fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
        let text = String::from_utf8_lossy(&raw).to_ascii_lowercase();
        text.contains("keyreed")
    }
    #[cfg(windows)]
    {
        let _ = pid;
        false
    }
}

pub fn write_install_pid(models_root: &Path) -> Result<()> {
    let incoming = models_root.join(".incoming");
    fs::create_dir_all(&incoming)?;
    fs::write(pid_path(models_root), format!("{}", std::process::id()))?;
    Ok(())
}

pub fn clear_install_pid(models_root: &Path) -> Result<()> {
    let _ = fs::remove_file(pid_path(models_root));
    Ok(())
}

/// Deletes staging trees and partial downloads left by a crash or a kill.
/// Does not touch a live install whose pid is still running.
pub fn sweep_incomplete(models_root: &Path) -> Result<()> {
    if let Some(pid) = read_install_pid(models_root) {
        if pid != std::process::id() && process_alive(pid) {
            return Ok(());
        }
    }
    let _ = fs::remove_dir_all(staging_path(models_root));
    let incoming = models_root.join(".incoming");
    if incoming.is_dir() {
        if let Ok(rd) = fs::read_dir(&incoming) {
            for ent in rd.flatten() {
                let p = ent.path();
                let name = ent.file_name().to_string_lossy().to_string();
                if name.ends_with(".part") || name.ends_with(".partial") || name == "install.pid" {
                    let _ = if p.is_dir() { fs::remove_dir_all(&p) } else { fs::remove_file(&p) };
                }
            }
        }
    }
    Ok(())
}

/// Stop a Keyreed install process recorded in the pid file, then sweep leftovers.
/// Never signals a pid whose command line is not Keyreed.
pub fn abort_incomplete_install(models_root: &Path) -> Result<()> {
    if let Some(pid) = read_install_pid(models_root) {
        if pid != std::process::id() && process_alive(pid) && cmdline_is_keyreed(pid) {
            #[cfg(unix)]
            unsafe {
                extern "C" {
                    fn kill(pid: i32, sig: i32) -> i32;
                }
                kill(pid as i32, 15);
            }
        }
    }
    // Force the next sweep to treat the install as dead by clearing a dead pid.
    if let Some(pid) = read_install_pid(models_root) {
        if !process_alive(pid) {
            let _ = fs::remove_file(pid_path(models_root));
        }
    }
    sweep_incomplete(models_root)
}

pub fn prepare_installed_model(models_root: &Path, model_id: &str) -> Result<PathBuf> {
    sweep_incomplete(models_root)?;
    let spec = find_model_spec(model_id).with_context(|| format!("unknown model '{model_id}'"))?;
    let dir = models_root.join(spec.folder_name);
    if !dir.exists() {
        bail!("model '{}' is not installed at {}", spec.id, dir.display());
    }
    if let Err(err) = verify_model_dir(&dir, spec.id) {
        let quarantined = quarantine_dir(&dir)?;
        bail!(
            "model '{}' failed integrity and was quarantined at {}: {err}",
            spec.id,
            quarantined.display()
        );
    }
    Ok(dir)
}

/// Extract into `.staging`, verify compile-time pins, then rename into the active folder.
/// The active folder is created only after verification succeeds.
pub fn install_verified_archive(archive: &Path, models_root: &Path, model_id: &str) -> Result<PathBuf> {
    let spec = find_model_spec(model_id).with_context(|| format!("unknown model '{model_id}'"))?;
    if pins_for(spec.id).is_none() {
        bail!("refusing to install '{}': no compile-time file pins", spec.id);
    }
    sweep_incomplete(models_root)?;
    write_install_pid(models_root)?;
    let staging = staging_path(models_root);
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;
    let extract_res = extract_tar_bz2_safely(archive, &staging);
    if let Err(err) = extract_res {
        let _ = fs::remove_dir_all(&staging);
        clear_install_pid(models_root)?;
        return Err(err);
    }
    let extracted = staging.join(spec.folder_name);
    if let Err(err) = verify_model_dir(&extracted, spec.id) {
        let _ = fs::remove_dir_all(&staging);
        clear_install_pid(models_root)?;
        return Err(err);
    }
    let final_dir = models_root.join(spec.folder_name);
    if final_dir.exists() {
        let _ = fs::remove_dir_all(&staging);
        clear_install_pid(models_root)?;
        bail!("refusing to replace existing model directory {}", final_dir.display());
    }
    fs::rename(&extracted, &final_dir).context("atomic promote failed")?;
    if let Err(err) = verify_model_dir(&final_dir, spec.id) {
        let _ = quarantine_dir(&final_dir);
        clear_install_pid(models_root)?;
        return Err(err);
    }
    let _ = fs::remove_dir_all(&staging);
    clear_install_pid(models_root)?;
    Ok(final_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn pin_slice() -> [(&'static str, u64, &'static str); 1] {
        [("model.bin", 4, "88d4266fd4e6338d13b845fcf289579d209c897823b9217da3e161936f031589")]
    }

    fn write_good(dir: &Path) {
        fs::create_dir_all(dir).unwrap();
        let mut f = File::create(dir.join("model.bin")).unwrap();
        f.write_all(b"abcd").unwrap();
    }

    #[test]
    fn rejects_modified_file() {
        let root = std::env::temp_dir().join(format!("kr-mod-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        write_good(&root);
        fs::write(root.join("model.bin"), b"abce").unwrap();
        assert!(verify_named_files(&root, &pin_slice()).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_deleted_file() {
        let root = std::env::temp_dir().join(format!("kr-del-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        assert!(verify_named_files(&root, &pin_slice()).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_truncated_file() {
        let root = std::env::temp_dir().join(format!("kr-trunc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("model.bin"), b"ab").unwrap();
        let err = verify_named_files(&root, &pin_slice()).unwrap_err().to_string();
        assert!(err.contains("size mismatch"), "{err}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_empty_pin_list() {
        let root = std::env::temp_dir().join(format!("kr-empty-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        assert!(verify_named_files(&root, &[]).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn sweep_removes_staging_and_partial_after_dead_pid() {
        let root = std::env::temp_dir().join(format!("kr-sweep-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let staging = root.join(".staging").join("sherpa-onnx-moonshine-tiny-en-int8");
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("encode.int8.onnx"), b"partial").unwrap();
        fs::create_dir_all(root.join(".incoming")).unwrap();
        fs::write(root.join(".incoming").join("model.tar.bz2.part"), b"partial-archive").unwrap();
        fs::write(root.join(".incoming").join("install.pid"), "999999").unwrap();
        sweep_incomplete(&root).unwrap();
        assert!(!root.join(".staging").exists());
        assert!(!root.join(".incoming").join("model.tar.bz2.part").exists());
        assert!(!root.join("sherpa-onnx-moonshine-tiny-en-int8").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn prepare_quarantines_unverified_directory() {
        let root = std::env::temp_dir().join(format!("kr-unv-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let dir = root.join("sherpa-onnx-moonshine-tiny-en-int8");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("tokens.txt"), b"not-the-model").unwrap();
        let err = prepare_installed_model(&root, "moonshine-tiny-en-int8").unwrap_err().to_string();
        assert!(err.contains("quarantined"), "{err}");
        assert!(!dir.exists(), "active dir must not remain");
        assert!(root.join(".rejected").read_dir().unwrap().next().is_some());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn corrupt_archive_does_not_activate() {
        let root = std::env::temp_dir().join(format!("kr-arch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let archive = root.join("bad.tar.bz2");
        fs::write(&archive, b"this is not a bzip2 archive").unwrap();
        let err = install_verified_archive(&archive, &root, "moonshine-tiny-en-int8");
        assert!(err.is_err());
        assert!(!root.join("sherpa-onnx-moonshine-tiny-en-int8").exists());
        assert!(!root.join(".staging").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn known_models_have_nonempty_pins() {
        for id in [
            "parakeet-tdt-0.6b-v2-int8",
            "moonshine-tiny-en-int8",
            "moonshine-base-en-int8",
        ] {
            let pins = pins_for(id).unwrap_or_else(|| panic!("missing pins for {id}"));
            assert!(!pins.is_empty());
        }
    }

    #[test]
    fn all_six_catalog_models_have_file_pins() {
        for id in [
            "parakeet-tdt-0.6b-v2-int8",
            "parakeet-tdt-0.6b-v3-int8",
            "moonshine-base-en-int8",
            "moonshine-tiny-en-int8",
            "sense-voice-small-int8",
            "whisper-small.en-int8",
        ] {
            let pins = pins_for(id).unwrap_or_else(|| panic!("missing pins for {id}"));
            assert!(pins.len() >= 2, "{id}");
        }
    }

    #[test]
    fn extracted_archives_match_pins() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cache/pin-archives/extracted");
        let cases = [
            ("parakeet-tdt-0.6b-v3-int8", "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8"),
            ("sense-voice-small-int8", "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17"),
            ("whisper-small.en-int8", "sherpa-onnx-whisper-small.en"),
        ];
        for (id, folder) in cases {
            let dir = base.join(folder);
            if !dir.exists() {
                continue;
            }
            verify_model_dir(&dir, id).unwrap_or_else(|err| panic!("{id}: {err}"));
        }
        let tiny = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench/models/sherpa-onnx-moonshine-tiny-en-int8");
        if tiny.exists() {
            verify_model_dir(&tiny, "moonshine-tiny-en-int8").unwrap();
        }
    }
}
