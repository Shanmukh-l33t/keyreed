//! Text jobs: Quick Fix, deeper fix, Composer and voice-command work that runs on the text-job
//! worker thread instead of the coordinator, so dictation is never blocked by it.

use crate::types::ForegroundContext;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextJobKind {
    QuickFix,
    DeepFix,
    Compose,
    VoiceCommand,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Instruction {
    Fix,
    Improve,
    Shorter,
    Tone,
    Summarise,
    Custom(String),
}

#[derive(Debug, Clone)]
pub struct TextJob {
    pub id: u64,
    pub kind: TextJobKind,
    pub instruction: Instruction,
    /// The selected text, or empty when there is none. Never logged.
    pub source: String,
    /// The copy carried formats besides plain text (links, images, tables, ...).
    pub rich: bool,
    /// The window the job was triggered in.
    pub target: ForegroundContext,
    pub max_output_tokens: u32,
    pub cancel: Arc<AtomicBool>,
}

/// Why a text job failed. A closed list, so the UI can say each in one plain sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobFailure {
    /// No writing model is installed and the cloud is not allowed for text jobs.
    NoWritingModel,
    /// This job kind is not available in this build or configuration.
    Unavailable,
    /// The model or endpoint failed to answer.
    ModelFailed,
}

/// Worker to UI events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextJobEvent {
    Stage { id: u64, stage: &'static str },
    Token { id: u64, text: String },
    Done { id: u64, text: String, cancelled: bool },
    Failed { id: u64, reason: JobFailure },
}
