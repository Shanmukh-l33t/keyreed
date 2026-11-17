//! Compile-time integrity pins for model files.
//!
//! The expected SHA-256 values live in this binary. A sidecar manifest next to the
//! model directory is not consulted and cannot authorize a load. A directory that
//! merely exists is not a verified model.

use crate::catalog::{extract_tar_bz2_filtered, find_model_spec};
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static ACTIVE_IN_PROCESS_INSTALLS: AtomicUsize = AtomicUsize::new(0);
static VERIFIED_CACHE: Mutex<Vec<(PathBuf, u64, u128, String)>> = Mutex::new(Vec::new());

pub struct InProcessInstallGuard;

impl InProcessInstallGuard {
    pub fn acquire() -> Self {
        ACTIVE_IN_PROCESS_INSTALLS.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for InProcessInstallGuard {
    fn drop(&mut self) {
        ACTIVE_IN_PROCESS_INSTALLS.fetch_sub(1, Ordering::SeqCst);
    }
}

pub fn has_active_in_process_install() -> bool {
    ACTIVE_IN_PROCESS_INSTALLS.load(Ordering::SeqCst) > 0
}

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

const PARAKEET_110M_EN: &[FilePin] = &[
    FilePin { relative: "decoder.int8.onnx", size: 3_955_863, sha256: "f7c331c5504c2e593c76ed22b728e3f554af6c4a383dde862e719ced08b1da19" },
    FilePin { relative: "encoder.int8.onnx", size: 131_113_202, sha256: "0f35509ddeb9b39002fb077d979a9fe74f06eb0bc4dd5c34f512f82e5111d657" },
    FilePin { relative: "joiner.int8.onnx", size: 1_411_403, sha256: "bf7dff69e9f2cdbe9943d70da358f38b361c115ba0105bae7e908e0d6ec782f6" },
    FilePin { relative: "tokens.txt", size: 9_953, sha256: "450e56bd2f036fe5b6aa821865838cc5aa9d8b0106134ce9a9ba0664abe6cd10" },
];

const PARAKEET_ULTRA: &[FilePin] = &[
    FilePin { relative: "decoder.int8.onnx", size: 11_845_274, sha256: "1fab98fe6c12aded87d2da66272cc9e148d0a0044ce3850a12fe56302ec4a922" },
    FilePin { relative: "encoder.int8.onnx", size: 652_282_345, sha256: "b43072e1b721f18a3df03fcfeb763ff7c4debdf29843598a3aba2f703e2780a3" },
    FilePin { relative: "joiner.int8.onnx", size: 6_355_277, sha256: "8a71aaccfdba3d451775507a889afd2257d2e754463fd81095a8c8f290f924c2" },
    FilePin { relative: "tokens.txt", size: 93_939, sha256: "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d" },
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

const GECTOR_BERT_EN_INT8: &[FilePin] = &[
    FilePin { relative: "LICENSE", size: 11_358, sha256: "cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30" },
    FilePin { relative: "NOTICE", size: 381, sha256: "5aa2310e15ea2f48e7f36ac1c581a6f8109dfc3ebff6c9745f60bfef7124a269" },
    FilePin { relative: "config.json", size: 157_469, sha256: "d008874db157abe63587a49efe92422cffbbbca88df80da46b21a8fae39eefeb" },
    FilePin { relative: "gector-guards.json", size: 6_703, sha256: "6e565ee87f0ecc99859342321175737ced5d9d0cc9475644d995da19d8adcdf8" },
    FilePin { relative: "gector-meta.json", size: 986, sha256: "1be9bc98d5a9e8b0c24b24490b488878b2f709b696f1cb18d50ee48560f1b30d" },
    FilePin { relative: "language-rules.json", size: 1_074, sha256: "11e0c0a7501db23e1a67193236cc377b57a883693fabe5e6d3fa85a996b95228" },
    FilePin { relative: "model.onnx", size: 112_377_233, sha256: "f57c654b8f4d65925501de51520618a9f28da65fe685472e2ac7a4029586659d" },
    FilePin { relative: "tokenizer.json", size: 668_923, sha256: "343989712a36cd8b253efeaf8baf6a08b9d2583f78e395e83832e8ee9f8d8ee1" },
    FilePin { relative: "tokenizer_config.json", size: 352, sha256: "9cd988573a9588a6085cd353eeefa09b803dba75b03f58d0c1d302aad3078193" },
    FilePin { relative: "verb-form-vocab.txt", size: 4_390_076, sha256: "5906804ec8a1acd8cbfc2918522bf429ce5edded8bbc46ffc2be861d280d3abf" },
    FilePin { relative: "vocab.txt", size: 213_450, sha256: "eeaa9875b23b04b4c54ef759d03db9d1ba1554838f8fb26c5d96fa551df93d02" },
];

pub const SILERO_VAD_SIZE: u64 = 643_854;
pub const SILERO_VAD_SHA256: &str = "9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6";

pub const QWEN35_GGUF_NAME: &str = "Qwen3.5-0.8B-Q4_K_M.gguf";
pub const QWEN35_GGUF_SIZE: u64 = 532_517_120;
pub const QWEN35_GGUF_SHA256: &str = "bd258782e35f7f458f8aced1adc053e6e92e89bc735ba3be89d38a06121dc517";

/// A pinned writing-model weight file: only these exact names, sizes and hashes are loaded.
#[derive(Debug, Clone, Copy)]
pub struct GgufPin {
    pub name: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

/// Qwen3.5 Q4_K_M files (Apache-2.0), figures from the Hugging Face file listing of the
/// `unsloth/Qwen3.5-*-GGUF` repositories at the revisions in `catalog::WRITING_MODELS`.
pub const GGUF_PINS: &[GgufPin] = &[
    GgufPin { name: QWEN35_GGUF_NAME, size: QWEN35_GGUF_SIZE, sha256: QWEN35_GGUF_SHA256 },
    GgufPin { name: "Qwen3.5-2B-Q4_K_M.gguf", size: 1_280_835_840, sha256: "aaf42c8b7c3cab2bf3d69c355048d4a0ee9973d48f16c731c0520ee914699223" },
    GgufPin { name: "Qwen3.5-4B-Q4_K_M.gguf", size: 2_740_937_888, sha256: "00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4" },
];

pub fn gguf_pin(file_name: &str) -> Option<&'static GgufPin> {
    GGUF_PINS.iter().find(|p| p.name.eq_ignore_ascii_case(file_name))
}

pub fn pins_for(model_id: &str) -> Option<&'static [FilePin]> {
    let pins = match model_id {
        "parakeet-tdt-0.6b-v2-int8" | "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8" => PARAKEET_V2,
        "parakeet-tdt-0.6b-v3-int8" | "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8" => PARAKEET_V3,
        "parakeet-tdt-110m-en-int8" | "sherpa-onnx-nemo-parakeet_tdt_transducer_110m-en-36000-int8" => PARAKEET_110M_EN,
        "parakeet-tdt-0.6b-ultra-int8" | "sherpa-onnx-nemo-parakeet-tdt-0.6b-ultra-int8" => PARAKEET_ULTRA,
        "moonshine-tiny-en-int8" | "sherpa-onnx-moonshine-tiny-en-int8" => MOONSHINE_TINY,
        "moonshine-base-en-int8" | "sherpa-onnx-moonshine-base-en-int8" => MOONSHINE_BASE,
        "sense-voice-small-int8" | "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17" => SENSE_VOICE,
        "whisper-small.en-int8" | "sherpa-onnx-whisper-small.en" => WHISPER_SMALL_EN,
        "gector-bert-en-int8" => GECTOR_BERT_EN_INT8,
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
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn verify_single_file(dir: &Path, rel: &str, size: u64, sha: &str) -> Result<()> {
    let path = dir.join(rel);
    let meta = fs::metadata(&path).with_context(|| format!("missing required file {}", path.display()))?;
    if !meta.is_file() {
        bail!("required model path is not a file: {}", path.display());
    }
    if meta.len() != size {
        bail!(
            "size mismatch for {}: expected {} bytes, got {}",
            path.display(),
            size,
            meta.len()
        );
    }
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let sha_low = sha.to_ascii_lowercase();
    if mtime_ns > 0 {
        if let Ok(cache) = VERIFIED_CACHE.lock() {
            if cache.iter().any(|(p, sz, mt, s)| p == &path && *sz == size && *mt == mtime_ns && s == &sha_low) {
                return Ok(());
            }
        }
    }
    let digest = sha256_file(&path)?;
    if !digest.eq_ignore_ascii_case(sha) {
        bail!("SHA-256 mismatch for {}: expected {}, got {}", path.display(), sha, digest);
    }
    if mtime_ns > 0 {
        if let Ok(mut cache) = VERIFIED_CACHE.lock() {
            cache.push((path, size, mtime_ns, sha_low));
        }
    }
    Ok(())
}

pub fn verify_named_files(dir: &Path, files: &[(&str, u64, &str)]) -> Result<()> {
    if files.is_empty() {
        bail!("refusing to verify {} with an empty pin list", dir.display());
    }
    if !dir.is_dir() {
        bail!("model path is not a directory: {}", dir.display());
    }
    if files.len() == 1 {
        let (rel, size, sha) = files[0];
        return verify_single_file(dir, rel, size, sha);
    }
    std::thread::scope(|s| {
        let handles: Vec<_> = files
            .iter()
            .map(|&(rel, size, sha)| s.spawn(move || verify_single_file(dir, rel, size, sha)))
            .collect();
        for h in handles {
            h.join().map_err(|_| anyhow::anyhow!("verification thread panicked"))??;
        }
        Ok(())
    })
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
    let Some(pin) = gguf_pin(name) else {
        bail!("refusing unpinned weight file {}", path.display());
    };
    verify_named_files(path.parent().unwrap_or_else(|| Path::new(".")), &[(name, pin.size, pin.sha256)])
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
/// Does not touch a live install whose pid is still running or an active in-process install.
pub fn sweep_incomplete(models_root: &Path) -> Result<()> {
    if has_active_in_process_install() {
        return Ok(());
    }
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
    let pins = pins_for(spec.id).context("missing compile-time file pins")?;
    let allowed_names: Vec<&str> = pins.iter().map(|p| p.relative).collect();
    let extract_res = extract_tar_bz2_filtered(archive, &staging, Some(&allowed_names));
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
    // Prune unpinned extra files (e.g. duplicate FP32 .onnx weights or test_wavs) so only verified pinned files remain
    if let Some(pins) = pins_for(spec.id) {
        if let Ok(rd) = fs::read_dir(&extracted) {
            for ent in rd.flatten() {
                let fname = ent.file_name().to_string_lossy().to_string();
                let keep = pins.iter().any(|p| p.relative == fname);
                if !keep {
                    let p = ent.path();
                    let _ = if p.is_dir() {
                        fs::remove_dir_all(&p)
                    } else {
                        fs::remove_file(&p)
                    };
                }
            }
        }
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

    #[test]
    fn only_listed_weight_files_pass_and_an_altered_one_is_refused() {
        let dir = std::env::temp_dir().join(format!("kr-gguf-pins-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for name in ["Qwen3.5-2B-Q4_K_M.gguf", "qwen3.5-4b-q4_k_m.GGUF", "Qwen3.5-0.8B-Q4_K_M.gguf"] {
            assert!(gguf_pin(name).is_some(), "{name}");
        }
        // A listed name with the wrong content is refused (the size differs, so nothing is hashed).
        for name in ["Qwen3.5-2B-Q4_K_M.gguf", "Qwen3.5-4B-Q4_K_M.gguf", "Qwen3.5-0.8B-Q4_K_M.gguf"] {
            fs::write(dir.join(name), b"not the real weights").unwrap();
            assert!(verify_gguf(&dir.join(name)).is_err(), "{name}");
        }
        // An unlisted name is refused before the file is read.
        fs::write(dir.join("Qwen3.5-9B-Q4_K_M.gguf"), b"x").unwrap();
        let e = verify_gguf(&dir.join("Qwen3.5-9B-Q4_K_M.gguf")).unwrap_err().to_string();
        assert!(e.contains("unpinned"));
        assert!(verify_gguf(&dir.join("missing.gguf")).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn grammar_model_is_pinned_with_its_licence_and_kept_out_of_speech_models() {
        let pins = pins_for("gector-bert-en-int8").unwrap();
        for f in ["LICENSE", "NOTICE", "model.onnx", "vocab.txt", "config.json", "verb-form-vocab.txt"] {
            assert!(pins.iter().any(|p| p.relative == f), "{f}");
        }
        assert!(find_model_spec("gector-bert-en-int8").is_some());
        assert!(!crate::catalog::ASR_CATALOG.iter().any(|m| m.id == "gector-bert-en-int8"));
    }

    /// Installs the real release archive (KR_GECTOR_ARCHIVE=path) into a temp models root, then
    /// tampers with the installed model and checks it is refused and quarantined.
    #[test]
    #[ignore]
    fn grammar_archive_installs_and_tampering_is_refused() {
        let archive = std::env::var("KR_GECTOR_ARCHIVE").expect("set KR_GECTOR_ARCHIVE");
        let root = std::env::temp_dir().join(format!("kr-gector-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let copy = root.join("a.tar.bz2");
        fs::copy(&archive, &copy).unwrap();
        let dir = install_verified_archive(&copy, &root, "gector-bert-en-int8").unwrap();
        assert!(prepare_installed_model(&root, "gector-bert-en-int8").is_ok());
        let mut f = fs::OpenOptions::new().append(true).open(dir.join("vocab.txt")).unwrap();
        f.write_all(b"x").unwrap();
        drop(f);
        assert!(prepare_installed_model(&root, "gector-bert-en-int8").is_err());
        assert!(!dir.exists(), "tampered model must be quarantined");
        let _ = fs::remove_dir_all(&root);
    }

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
    fn in_process_install_guard_protects_active_staging_from_sweep() {
        let root = std::env::temp_dir().join(format!("kr-inproc-sweep-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let staging = root.join(".staging").join("sherpa-onnx-moonshine-tiny-en-int8");
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("encode.int8.onnx"), b"active-download").unwrap();
        {
            let _guard = InProcessInstallGuard::acquire();
            sweep_incomplete(&root).unwrap();
            assert!(
                staging.join("encode.int8.onnx").exists(),
                "Active in-process download/staging must not be deleted by sweep_incomplete!"
            );
        }
        sweep_incomplete(&root).unwrap();
        assert!(!root.join(".staging").exists());
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
    fn every_catalog_model_has_pins_for_the_files_its_loader_asks_for() {
        for m in crate::catalog::ASR_CATALOG {
            let pins = pins_for(m.id).unwrap_or_else(|| panic!("missing pins for {}", m.id));
            assert!(pins_for(m.folder_name).is_some(), "{} folder name has no pins", m.id);
            assert!(pins.iter().any(|p| p.relative == "tokens.txt" || p.relative.ends_with("-tokens.txt")), "{}", m.id);
            if m.architecture == crate::catalog::AsrArchitecture::NemoTransducer {
                for f in ["encoder.int8.onnx", "decoder.int8.onnx", "joiner.int8.onnx"] {
                    assert!(pins.iter().any(|p| p.relative == f), "{} lacks {f}", m.id);
                }
            }
        }
    }

    #[test]
    fn extracted_archives_match_pins() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cache/pin-archives/extracted");
        let cases = [
            ("parakeet-tdt-0.6b-v3-int8", "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8"),
            ("parakeet-tdt-110m-en-int8", "sherpa-onnx-nemo-parakeet_tdt_transducer_110m-en-36000-int8"),
            ("parakeet-tdt-0.6b-ultra-int8", "sherpa-onnx-nemo-parakeet-tdt-0.6b-ultra-int8"),
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
