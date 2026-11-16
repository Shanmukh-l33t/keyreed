//! Exact C ABI struct and function bindings for `sherpa-onnx` v2.0.1 (`c-api.h`).
//! Loaded dynamically via `libloading` (`sherpa-onnx-c-api.dll` on Windows, `libsherpa-onnx-c-api.so` on Linux).

#![allow(non_camel_case_types, dead_code)]

use libloading::{Library, Symbol};
use std::ffi::{c_char, c_void};
use std::path::Path;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxFeatureConfig {
    pub sample_rate: i32,
    pub feature_dim: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineTransducerModelConfig {
    pub encoder: *const c_char,
    pub decoder: *const c_char,
    pub joiner: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineParaformerModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineNemoEncDecCtcModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineWhisperModelConfig {
    pub encoder: *const c_char,
    pub decoder: *const c_char,
    pub language: *const c_char,
    pub task: *const c_char,
    pub tail_paddings: i32,
    pub enable_token_timestamps: i32,
    pub enable_segment_timestamps: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineCanaryModelConfig {
    pub encoder: *const c_char,
    pub decoder: *const c_char,
    pub src_lang: *const c_char,
    pub tgt_lang: *const c_char,
    pub use_pnc: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineCohereTranscribeModelConfig {
    pub encoder: *const c_char,
    pub decoder: *const c_char,
    pub language: *const c_char,
    pub use_punct: i32,
    pub use_itn: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineFireRedAsrModelConfig {
    pub encoder: *const c_char,
    pub decoder: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineFireRedAsrCtcModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineMoonshineModelConfig {
    pub preprocessor: *const c_char,
    pub encoder: *const c_char,
    pub uncached_decoder: *const c_char,
    pub cached_decoder: *const c_char,
    pub merged_decoder: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineTdnnModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineLMConfig {
    pub model: *const c_char,
    pub scale: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineSenseVoiceModelConfig {
    pub model: *const c_char,
    pub language: *const c_char,
    pub use_itn: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineDolphinModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineZipformerCtcModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineWenetCtcModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineOmnilingualAsrCtcModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineFunASRNanoModelConfig {
    pub encoder_adaptor: *const c_char,
    pub llm: *const c_char,
    pub embedding: *const c_char,
    pub tokenizer: *const c_char,
    pub system_prompt: *const c_char,
    pub user_prompt: *const c_char,
    pub max_new_tokens: i32,
    pub temperature: f32,
    pub top_p: f32,
    pub seed: i32,
    pub language: *const c_char,
    pub itn: i32,
    pub hotwords: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineQwen3ASRModelConfig {
    pub conv_frontend: *const c_char,
    pub encoder: *const c_char,
    pub decoder: *const c_char,
    pub tokenizer: *const c_char,
    pub max_total_len: i32,
    pub max_new_tokens: i32,
    pub temperature: f32,
    pub top_p: f32,
    pub seed: i32,
    pub hotwords: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineMedAsrCtcModelConfig {
    pub model: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineModelConfig {
    pub transducer: SherpaOnnxOfflineTransducerModelConfig,
    pub paraformer: SherpaOnnxOfflineParaformerModelConfig,
    pub nemo_ctc: SherpaOnnxOfflineNemoEncDecCtcModelConfig,
    pub whisper: SherpaOnnxOfflineWhisperModelConfig,
    pub tdnn: SherpaOnnxOfflineTdnnModelConfig,
    pub tokens: *const c_char,
    pub num_threads: i32,
    pub debug: i32,
    pub provider: *const c_char,
    pub model_type: *const c_char,
    pub modeling_unit: *const c_char,
    pub bpe_vocab: *const c_char,
    pub telespeech_ctc: *const c_char,
    pub sense_voice: SherpaOnnxOfflineSenseVoiceModelConfig,
    pub moonshine: SherpaOnnxOfflineMoonshineModelConfig,
    pub fire_red_asr: SherpaOnnxOfflineFireRedAsrModelConfig,
    pub dolphin: SherpaOnnxOfflineDolphinModelConfig,
    pub zipformer_ctc: SherpaOnnxOfflineZipformerCtcModelConfig,
    pub canary: SherpaOnnxOfflineCanaryModelConfig,
    pub wenet_ctc: SherpaOnnxOfflineWenetCtcModelConfig,
    pub omnilingual: SherpaOnnxOfflineOmnilingualAsrCtcModelConfig,
    pub medasr: SherpaOnnxOfflineMedAsrCtcModelConfig,
    pub funasr_nano: SherpaOnnxOfflineFunASRNanoModelConfig,
    pub fire_red_asr_ctc: SherpaOnnxOfflineFireRedAsrCtcModelConfig,
    pub qwen3_asr: SherpaOnnxOfflineQwen3ASRModelConfig,
    pub cohere_transcribe: SherpaOnnxOfflineCohereTranscribeModelConfig,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxHomophoneReplacerConfig {
    pub dict_dir: *const c_char,
    pub lexicon: *const c_char,
    pub rule_fsts: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineCtcFstDecoderConfig {
    pub graph: *const c_char,
    pub max_active: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineRecognizerConfig {
    pub feat_config: SherpaOnnxFeatureConfig,
    pub model_config: SherpaOnnxOfflineModelConfig,
    pub lm_config: SherpaOnnxOfflineLMConfig,
    pub decoding_method: *const c_char,
    pub max_active_paths: i32,
    pub hotwords_file: *const c_char,
    pub hotwords_score: f32,
    pub rule_fsts: *const c_char,
    pub rule_fars: *const c_char,
    pub blank_penalty: f32,
    pub hr: SherpaOnnxHomophoneReplacerConfig,
    pub ctc_fst_decoder_config: SherpaOnnxOfflineCtcFstDecoderConfig,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxSileroVadModelConfig {
    pub model: *const c_char,
    pub threshold: f32,
    pub min_silence_duration: f32,
    pub min_speech_duration: f32,
    pub window_size: i32,
    pub max_speech_duration: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxTenVadModelConfig {
    pub model: *const c_char,
    pub threshold: f32,
    pub min_silence_duration: f32,
    pub min_speech_duration: f32,
    pub window_size: i32,
    pub max_speech_duration: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxVadModelConfig {
    pub silero_vad: SherpaOnnxSileroVadModelConfig,
    pub sample_rate: i32,
    pub num_threads: i32,
    pub provider: *const c_char,
    pub debug: i32,
    pub ten_vad: SherpaOnnxTenVadModelConfig,
}

#[repr(C)]
pub struct SherpaOnnxSpeechSegment {
    pub start: i64,
    pub samples: *mut f32,
    pub n: i32,
}

pub type SherpaOnnxOfflineRecognizer = c_void;
pub type SherpaOnnxOfflineStream = c_void;
pub type SherpaOnnxVoiceActivityDetector = c_void;

pub struct SherpaApi {
    _onnx_lib: Option<Library>,
    _lib: Library,
    pub get_version: unsafe extern "C" fn() -> *const c_char,
    pub create_offline_recognizer:
        unsafe extern "C" fn(*const SherpaOnnxOfflineRecognizerConfig) -> *const SherpaOnnxOfflineRecognizer,
    pub destroy_offline_recognizer: unsafe extern "C" fn(*const SherpaOnnxOfflineRecognizer),
    pub create_offline_stream:
        unsafe extern "C" fn(*const SherpaOnnxOfflineRecognizer) -> *const SherpaOnnxOfflineStream,
    pub create_offline_stream_with_hotwords:
        unsafe extern "C" fn(*const SherpaOnnxOfflineRecognizer, *const c_char) -> *const SherpaOnnxOfflineStream,
    pub destroy_offline_stream: unsafe extern "C" fn(*const SherpaOnnxOfflineStream),
    pub accept_waveform_offline:
        unsafe extern "C" fn(*const SherpaOnnxOfflineStream, i32, *const f32, i32),
    pub decode_offline_stream:
        unsafe extern "C" fn(*const SherpaOnnxOfflineRecognizer, *const SherpaOnnxOfflineStream),
    pub get_offline_stream_result_as_json:
        unsafe extern "C" fn(*const SherpaOnnxOfflineStream) -> *const c_char,
    pub destroy_offline_stream_result_json: unsafe extern "C" fn(*const c_char),
    pub create_vad:
        unsafe extern "C" fn(*const SherpaOnnxVadModelConfig, f32) -> *const SherpaOnnxVoiceActivityDetector,
    pub destroy_vad: unsafe extern "C" fn(*const SherpaOnnxVoiceActivityDetector),
    pub vad_accept_waveform:
        unsafe extern "C" fn(*const SherpaOnnxVoiceActivityDetector, *const f32, i32),
    pub vad_empty: unsafe extern "C" fn(*const SherpaOnnxVoiceActivityDetector) -> i32,
    pub vad_front:
        unsafe extern "C" fn(*const SherpaOnnxVoiceActivityDetector) -> *const SherpaOnnxSpeechSegment,
    pub vad_pop: unsafe extern "C" fn(*const SherpaOnnxVoiceActivityDetector),
    pub vad_flush: unsafe extern "C" fn(*const SherpaOnnxVoiceActivityDetector),
    pub vad_reset: unsafe extern "C" fn(*const SherpaOnnxVoiceActivityDetector),
    pub vad_detected: unsafe extern "C" fn(*const SherpaOnnxVoiceActivityDetector) -> i32,
    pub destroy_speech_segment: unsafe extern "C" fn(*const SherpaOnnxSpeechSegment),
}

unsafe impl Send for SherpaApi {}
unsafe impl Sync for SherpaApi {}

impl SherpaApi {
    pub fn load_from_dir(lib_dir: &Path) -> anyhow::Result<Self> {
        #[cfg(target_os = "windows")]
        let (ort_name, sherpa_name) = ("onnxruntime.dll", "sherpa-onnx-c-api.dll");
        #[cfg(not(target_os = "windows"))]
        let (ort_name, sherpa_name) = ("libonnxruntime.so", "libsherpa-onnx-c-api.so");

        let ort_path = lib_dir.join(ort_name);
        let sherpa_path = lib_dir.join(sherpa_name);

        unsafe {
            #[cfg(target_os = "windows")]
            {
                if let Ok(canon) = std::fs::canonicalize(lib_dir) {
                    let s = canon.to_string_lossy();
                    let trimmed = s.strip_prefix(r"\\?\").unwrap_or(&s);
                    let wide: Vec<u16> = trimmed.encode_utf16().chain(std::iter::once(0)).collect();
                    extern "system" {
                        fn AddDllDirectory(NewDirectory: *const u16) -> *mut c_void;
                    }
                    let _ = AddDllDirectory(wide.as_ptr());
                }
            }
            let onnx_lib = if ort_path.exists() {
                Some(Library::new(&ort_path)?)
            } else {
                None
            };
            let lib = if sherpa_path.exists() {
                Library::new(&sherpa_path)?
            } else {
                Library::new(sherpa_name)?
            };

            macro_rules! sym {
                ($name:literal) => {{
                    let s: Symbol<_> = lib.get($name)?;
                    *s
                }};
            }

            Ok(Self {
                get_version: sym!(b"SherpaOnnxGetVersionStr\0"),
                create_offline_recognizer: sym!(b"SherpaOnnxCreateOfflineRecognizer\0"),
                destroy_offline_recognizer: sym!(b"SherpaOnnxDestroyOfflineRecognizer\0"),
                create_offline_stream: sym!(b"SherpaOnnxCreateOfflineStream\0"),
                create_offline_stream_with_hotwords: sym!(
                    b"SherpaOnnxCreateOfflineStreamWithHotwords\0"
                ),
                destroy_offline_stream: sym!(b"SherpaOnnxDestroyOfflineStream\0"),
                accept_waveform_offline: sym!(b"SherpaOnnxAcceptWaveformOffline\0"),
                decode_offline_stream: sym!(b"SherpaOnnxDecodeOfflineStream\0"),
                get_offline_stream_result_as_json: sym!(b"SherpaOnnxGetOfflineStreamResultAsJson\0"),
                destroy_offline_stream_result_json: sym!(
                    b"SherpaOnnxDestroyOfflineStreamResultJson\0"
                ),
                create_vad: sym!(b"SherpaOnnxCreateVoiceActivityDetector\0"),
                destroy_vad: sym!(b"SherpaOnnxDestroyVoiceActivityDetector\0"),
                vad_accept_waveform: sym!(b"SherpaOnnxVoiceActivityDetectorAcceptWaveform\0"),
                vad_empty: sym!(b"SherpaOnnxVoiceActivityDetectorEmpty\0"),
                vad_front: sym!(b"SherpaOnnxVoiceActivityDetectorFront\0"),
                vad_pop: sym!(b"SherpaOnnxVoiceActivityDetectorPop\0"),
                vad_flush: sym!(b"SherpaOnnxVoiceActivityDetectorFlush\0"),
                vad_reset: sym!(b"SherpaOnnxVoiceActivityDetectorReset\0"),
                vad_detected: sym!(b"SherpaOnnxVoiceActivityDetectorDetected\0"),
                destroy_speech_segment: sym!(b"SherpaOnnxDestroySpeechSegment\0"),
                _onnx_lib: onnx_lib,
                _lib: lib,
            })
        }
    }
}
