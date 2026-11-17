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
    /// Personal-dictionary terms Quick Fix never changes.
    pub terms: Vec<String>,
    /// `habits::style_block` for the Composer's system prompt; `None` when nothing was learned.
    pub style_block: Option<String>,
    pub cancel: Arc<AtomicBool>,
    /// The cloud endpoint to run on, set only when the route rule chose it (never under Maximum Privacy).
    pub cloud: Option<CloudTarget>,
}

/// A cloud endpoint for a text job. Built by `route::Route::target` only, which keeps the offline profile off it.
#[derive(Clone)]
pub struct CloudTarget {
    pub endpoint: String,
    pub model: String,
    pub api_key: String,
}

/// The API key is redacted: a job is occasionally formatted into a log line, and the text never is.
impl std::fmt::Debug for CloudTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudTarget")
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("api_key", &"<redacted>")
            .finish()
    }
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
    /// The grammar model for Quick Fix is not installed or did not load.
    NoGrammarModel,
}

/// One Quick Fix change: a byte range of the job's `source` and its replacement. `label` is "Spelling",
/// "Grammar" or "Rule" (what the Review panel shows); `silent` means the edit-class table allows applying it
/// without review in this app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixEdit {
    pub start: usize,
    pub end: usize,
    pub replacement: String,
    pub label: &'static str,
    pub silent: bool,
}

/// Worker to UI events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextJobEvent {
    Stage { id: u64, stage: &'static str },
    Token { id: u64, text: String },
    Done { id: u64, text: String, cancelled: bool },
    /// A Quick Fix finished: the corrected text and every change in it (empty when nothing needed fixing).
    QuickFixDone { id: u64, text: String, edits: Vec<FixEdit> },
    Failed { id: u64, reason: JobFailure },
}
