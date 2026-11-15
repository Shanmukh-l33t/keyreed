use anyhow::{bail, Context, Result};
use bzip2::read::BzDecoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use tar::Archive;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsrArchitecture {
    NemoTransducer,
    Moonshine,
    MoonshineV2,
    SenseVoice,
    Whisper,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsrModelSpec {
    pub id: &'static str,
    pub display_name: &'static str,
    pub architecture: AsrArchitecture,
    pub folder_name: &'static str,
    pub download_url: &'static str,
    /// Optional pinned SHA-256 hex digest of the archive (empty = verify HTTPS + safe extraction).
    pub expected_sha256: &'static str,
    pub approx_download_mb: u32,
    pub approx_ram_mb: u32,
    pub languages: &'static str,
    pub license: &'static str,
    pub supports_hotwords: bool,
}

pub const ASR_CATALOG: &[AsrModelSpec] = &[
    AsrModelSpec {
        id: "parakeet-tdt-0.6b-v2-int8",
        display_name: "NVIDIA Parakeet TDT 0.6B v2 (INT8, Default English)",
        architecture: AsrArchitecture::NemoTransducer,
        folder_name: "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8",
        download_url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8.tar.bz2",
        expected_sha256: "6fc4426b54aa549a0019b8a75b8dea59164e71790a37ac56a045ad57f915f714",
        approx_download_mb: 460,
        approx_ram_mb: 690,
        languages: "en (Strong US/UK/Indian English + native punctuation)",
        license: "CC-BY-4.0",
        supports_hotwords: true,
    },
    AsrModelSpec {
        id: "parakeet-tdt-0.6b-v3-int8",
        display_name: "NVIDIA Parakeet TDT 0.6B v3 (INT8, 25 Languages)",
        architecture: AsrArchitecture::NemoTransducer,
        folder_name: "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8",
        download_url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8.tar.bz2",
        expected_sha256: "1495e91b59d7fc53ddca91f3431758ec99f8fa253fdb0e1e851be0f72b14b0f9",
        approx_download_mb: 465,
        approx_ram_mb: 710,
        languages: "25 European languages (auto-detect + punctuation)",
        license: "CC-BY-4.0",
        supports_hotwords: true,
    },
    AsrModelSpec {
        id: "moonshine-base-en-int8",
        display_name: "Moonshine Base (INT8, Ultra-Light Fallback)",
        architecture: AsrArchitecture::Moonshine,
        folder_name: "sherpa-onnx-moonshine-base-en-int8",
        download_url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-moonshine-base-en-int8.tar.bz2",
        expected_sha256: "21870cecaa2e44e4e2bf63e02d1072bed183ccd10284871353bd9d24dad14e5e",
        approx_download_mb: 239,
        approx_ram_mb: 165,
        languages: "en",
        license: "MIT",
        supports_hotwords: false,
    },
    AsrModelSpec {
        id: "moonshine-tiny-en-int8",
        display_name: "Moonshine Tiny (INT8, Legacy / Low-RAM PC)",
        architecture: AsrArchitecture::Moonshine,
        folder_name: "sherpa-onnx-moonshine-tiny-en-int8",
        download_url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-moonshine-tiny-en-int8.tar.bz2",
        expected_sha256: "d5fe6ec4334fef36255b2a4010412cad4c007e33103fec62fb5d17cad88086f2",
        approx_download_mb: 103,
        approx_ram_mb: 95,
        languages: "en",
        license: "MIT",
        supports_hotwords: false,
    },
    AsrModelSpec {
        id: "sense-voice-small-int8",
        display_name: "SenseVoice Small (INT8, Multilingual + Fast ITN)",
        architecture: AsrArchitecture::SenseVoice,
        folder_name: "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17",
        download_url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17.tar.bz2",
        expected_sha256: "f6b2a72ebcb1ac7a764d4cfccd886e6bcb2a95c4657c2199d0ba95ed4b9ea71a",
        approx_download_mb: 999,
        approx_ram_mb: 290,
        languages: "en, zh, ja, ko, yue",
        license: "FunASR Model License (Commercial Permissive)",
        supports_hotwords: false,
    },
    AsrModelSpec {
        id: "whisper-small.en-int8",
        display_name: "Whisper Small.en (INT8 ONNX)",
        architecture: AsrArchitecture::Whisper,
        folder_name: "sherpa-onnx-whisper-small.en",
        download_url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-whisper-small.en.tar.bz2",
        expected_sha256: "0cdba2b8aaab69e04847f3427cc9709574112e67913a1a84b7fec3a8729faa9a",
        approx_download_mb: 606,
        approx_ram_mb: 440,
        languages: "en",
        license: "MIT",
        supports_hotwords: false,
    },
];

pub fn find_model_spec(id: &str) -> Option<&'static AsrModelSpec> {
    ASR_CATALOG.iter().find(|m| m.id == id || m.folder_name == id)
}

/// Validates a relative archive path against zip-slip, parent traversal, and symlink attacks
/// (mitigates ONNX/archive path traversal classes such as CVE-2024-27318 / CVE-2026-27489).
pub fn validate_safe_relative_path(entry_path: &Path) -> Result<()> {
    if entry_path.is_absolute() {
        bail!("archive entry uses forbidden absolute path: {}", entry_path.display());
    }
    for comp in entry_path.components() {
        match comp {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("archive entry contains path traversal: {}", entry_path.display());
            }
        }
    }
    Ok(())
}

/// Safely extracts a `.tar.bz2` model bundle while rejecting symlinks, hardlinks, device nodes,
/// and `..` path traversal.
pub fn extract_tar_bz2_safely(archive_path: &Path, dest_root: &Path) -> Result<()> {
    fs::create_dir_all(dest_root)?;
    let file = File::open(archive_path)?;
    let bz = BzDecoder::new(file);
    let mut archive = Archive::new(bz);

    for entry_res in archive.entries()? {
        let mut entry = entry_res?;
        let etype = entry.header().entry_type();
        if etype.is_symlink() || etype.is_hard_link() || etype.is_block_special() || etype.is_character_special() || etype.is_fifo() {
            bail!("refusing unsafe tar entry type {:?}", etype);
        }
        let rel = entry.path()?.into_owned();
        validate_safe_relative_path(&rel)?;
        let out_path = dest_root.join(&rel);
        if etype.is_dir() {
            fs::create_dir_all(&out_path)?;
        } else if etype.is_file() {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            entry.unpack(&out_path)?;
        }
    }
    Ok(())
}

/// Downloads a file over HTTPS with mandatory SHA-256 verification.
pub fn download_verified(url: &str, dest_file: &Path, expected_sha256: &str) -> Result<String> {
    if !url.starts_with("https://") {
        bail!("model downloads require HTTPS: {}", url);
    }
    // Fail closed: an unpinned archive is a supply-chain hole (threat model T9). A malicious
    // model file parsed by onnxruntime/llama.cpp can exploit the ONNX/GGUF CVE family, so we
    // refuse to download anything without a 64-hex SHA-256 to check against.
    let exp = expected_sha256.trim();
    if exp.is_empty() || exp.len() != 64 || !exp.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!(
            "refusing unpinned model download (missing/invalid expected_sha256) for {}",
            url
        );
    }
    if let Some(parent) = dest_file.parent() {
        fs::create_dir_all(parent)?;
    }
    let resp = ureq::get(url)
        .call()
        .with_context(|| format!("HTTPS download failed for {}", url))?;
    let mut reader = resp.into_reader();
    let tmp = dest_file.with_extension("part");
    let mut out = File::create(&tmp)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        out.write_all(&buf[..n])?;
    }
    out.flush()?;
    drop(out);

    let digest = hex::encode(hasher.finalize());
    if !digest.eq_ignore_ascii_case(exp) {
        let _ = fs::remove_file(&tmp);
        bail!(
            "SHA-256 mismatch for {}: expected {}, got {}",
            url,
            exp,
            digest
        );
    }
    fs::rename(&tmp, dest_file)?;
    Ok(digest)
}

pub fn ensure_model_present(models_root: &Path, spec: &AsrModelSpec) -> Result<PathBuf> {
    crate::integrity::sweep_incomplete(models_root)?;
    let target = models_root.join(spec.folder_name);
    if target.exists() {
        return crate::integrity::prepare_installed_model(models_root, spec.id);
    }
    let incoming = models_root.join(".incoming");
    fs::create_dir_all(&incoming)?;
    let archive_tmp = incoming.join(format!("{}.tar.bz2", spec.folder_name));
    download_verified(spec.download_url, &archive_tmp, spec.expected_sha256)?;
    let install_res = crate::integrity::install_verified_archive(&archive_tmp, models_root, spec.id);
    let _ = fs::remove_file(&archive_tmp);
    install_res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_traversal_paths() {
        assert!(validate_safe_relative_path(Path::new("../evil.onnx")).is_err());
        assert!(validate_safe_relative_path(Path::new("model/../../etc/passwd")).is_err());
        assert!(validate_safe_relative_path(Path::new("/abs/path.onnx")).is_err());
        assert!(validate_safe_relative_path(Path::new("sherpa-model/encoder.int8.onnx")).is_ok());
    }

    #[test]
    fn every_catalog_entry_is_https_and_sha256_pinned() {
        assert!(!ASR_CATALOG.is_empty());
        for m in ASR_CATALOG {
            assert!(
                m.download_url.starts_with("https://"),
                "{} URL is not HTTPS: {}",
                m.id,
                m.download_url
            );
            assert_eq!(
                m.expected_sha256.len(),
                64,
                "{} is missing a 64-hex SHA-256 pin (threat model T9)",
                m.id
            );
            assert!(
                m.expected_sha256.bytes().all(|b| b.is_ascii_hexdigit()),
                "{} has non-hex SHA-256: {}",
                m.id,
                m.expected_sha256
            );
            assert!(m.approx_download_mb > 0);
        }
    }

    #[test]
    fn download_verified_fails_closed_before_network_on_bad_pin() {
        // Must reject BEFORE dialing, so a dummy unreachable URL proves no socket is touched.
        let dst = Path::new("/tmp/keyreed_should_never_exist.bin");
        assert!(download_verified("http://example.com/m.tar.bz2", dst, &"a".repeat(64)).is_err());
        assert!(download_verified("https://127.0.0.1:1/m.tar.bz2", dst, "").is_err());
        assert!(download_verified("https://127.0.0.1:1/m.tar.bz2", dst, "deadbeef").is_err());
        assert!(download_verified("https://127.0.0.1:1/m.tar.bz2", dst, &"z".repeat(64)).is_err());
    }
}
