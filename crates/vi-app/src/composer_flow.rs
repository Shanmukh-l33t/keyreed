//! Composer on the coordinator thread (plan P3.4): the selection it opened with, the request on the
//! text-job worker, the stream into the bar, and delivery into the target app or the clipboard.
//! The request state is a small pure helper (tested below); only delivery touches Win32. Text is never logged.

#![cfg(windows)]

use crate::textjobs::{JobRequest, SharedEngine};
use crossbeam_channel::Sender;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use vi_core::textjob::{CloudTarget, Instruction, JobFailure, TextJob, TextJobEvent, TextJobKind};
use vi_core::habits;
use vi_core::style::{Scope, StyleStore};
use vi_core::{AppConfig, ForegroundContext};
use vi_rewrite::compose::{self, ComposeJob};
use vi_win32::clipboard::write_clipboard_unicode;
use vi_win32::composer::ComposerMsg;
use vi_win32::selection::{self, Captured, Replaced};
use vi_win32::typing::deliver_direct_or_clipboard_fallback;
use vi_win32::ui::win_gui::WinGuiHandle;
use vi_win32::ui::{DeliverHow, QuickFixStage};
use vi_win32::DirectTypeOutcome;

/// What the Composer has asked for, without Win32: the last request and the last result.
#[derive(Debug, Default)]
struct Session {
    /// The last request: its instruction and the text it ran on (Regenerate re-runs both).
    last: Option<(Instruction, String)>,
    /// The last finished result: a follow-up runs on it.
    result: Option<String>,
}

impl Session {
    /// A new request runs on the kept selection, else on the draft typed into the bar, else on nothing.
    fn submit_source(&self, selection: Option<&str>, use_selection: bool, draft: Option<String>) -> String {
        selection.filter(|_| use_selection).map(str::to_string).or(draft).unwrap_or_default()
    }

    /// A follow-up runs on the latest result; there is nothing to follow up before the first answer.
    fn follow_up(&self, instruction: Instruction) -> Option<(Instruction, String)> {
        let result = self.result.as_deref().filter(|r| !r.trim().is_empty())?;
        Some((instruction, result.to_string()))
    }

    /// Regenerate runs the last request again, from the text it originally ran on.
    fn regenerate(&self) -> Option<(Instruction, String)> {
        self.last.clone()
    }

    fn started(&mut self, instruction: Instruction, source: String) {
        self.last = Some((instruction, source));
    }

    /// A cancelled or empty answer is not kept, so a follow-up never builds on half a result.
    fn finished(&mut self, text: String) {
        if !text.trim().is_empty() {
            self.result = Some(text);
        }
    }
}

#[derive(Default)]
pub struct ComposerFlow {
    session: Session,
    /// The selection the Composer opened with (memory only; dropped when it closes).
    captured: Option<Captured>,
    /// The window it opened in; delivery re-verifies it.
    ctx: ForegroundContext,
    /// The cloud endpoint the route rule chose, or `None` for the local model.
    cloud: Option<CloudTarget>,
    /// The running job and its cancel flag.
    job: Option<u64>,
    /// True while the bar is on screen: its microphone has somewhere to put dictated words.
    open: bool,
    cancel: Option<Arc<AtomicBool>>,
    next_id: u64,
}

impl ComposerFlow {
    /// The Composer opened: hold the selection, the window and the route it will run on.
    pub fn open(&mut self, captured: Option<Captured>, ctx: ForegroundContext, cloud: Option<CloudTarget>) {
        *self = Self { captured, ctx, cloud, open: true, next_id: self.next_id, ..Self::default() };
    }

    /// The Composer closed: stop a running request and forget the selection.
    pub fn close(&mut self) {
        self.cancel();
        *self = Self { next_id: self.next_id, ..Self::default() };
    }

    /// Whether the bar is open (a closed Composer has nowhere to take a dictated transcript).
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// A preset or typed instruction: the selection when the user kept it, the draft typed into the bar, or nothing.
    pub fn submit(
        &mut self,
        cfg: &AppConfig,
        instruction: Instruction,
        use_selection: bool,
        draft: Option<String>,
        jobs: &Sender<JobRequest>,
        engine: &SharedEngine,
        style: &StyleStore,
    ) {
        let source = self.session.submit_source(self.captured.as_ref().map(|c| c.text.as_str()), use_selection, draft);
        self.start(cfg, instruction, source, jobs, engine, style);
    }

    /// A preset pressed on the result: it runs on the latest result.
    pub fn follow_up(&mut self, cfg: &AppConfig, instruction: Instruction, jobs: &Sender<JobRequest>, engine: &SharedEngine, style: &StyleStore) {
        if let Some((instruction, source)) = self.session.follow_up(instruction) {
            self.start(cfg, instruction, source, jobs, engine, style);
        }
    }

    /// Run the last request again, from its original source.
    pub fn regenerate(&mut self, cfg: &AppConfig, jobs: &Sender<JobRequest>, engine: &SharedEngine, style: &StyleStore) {
        if let Some((instruction, source)) = self.session.regenerate() {
            self.start(cfg, instruction, source, jobs, engine, style);
        }
    }

    /// Stop the running request; the bar keeps the prompt.
    pub fn cancel(&mut self) {
        if let Some(flag) = &self.cancel {
            flag.store(true, Ordering::Relaxed);
        }
    }

    fn start(&mut self, cfg: &AppConfig, instruction: Instruction, source: String, jobs: &Sender<JobRequest>, engine: &SharedEngine, style: &StyleStore) {
        if self.job.is_some() {
            return;
        }
        self.next_id += 1;
        let id = self.next_id;
        let cancel: Arc<AtomicBool> = Default::default();
        let job = TextJob {
            id,
            kind: TextJobKind::Compose,
            instruction: instruction.clone(),
            source: source.clone(),
            rich: self.captured.as_ref().is_some_and(|c| c.structured),
            target: self.ctx.clone(),
            max_output_tokens: compose::token_limit(&ComposeJob::new(instruction.clone(), &source)),
            terms: if cfg.dictionary.enabled { cfg.dictionary.vocabulary.clone() } else { Vec::new() },
            cancel: cancel.clone(),
            cloud: self.cloud.clone(),
            style_block: habits::style_block(style, cfg.dialect, Scope::for_category(self.ctx.category)),
        };
        if jobs.send((job, engine.clone())).is_ok() {
            self.session.started(instruction, source);
            self.job = Some(id);
            self.cancel = Some(cancel);
        }
    }

    /// A text-job event; anything that is not the running request is ignored. The bar shows its own
    /// streaming state, so nothing is sent on submit.
    pub fn on_job_event(&mut self, ev: TextJobEvent, gui: &WinGuiHandle) {
        if let Some(msg) = self.handle(ev) {
            gui.composer(msg);
        }
    }

    fn handle(&mut self, ev: TextJobEvent) -> Option<ComposerMsg> {
        match ev {
            TextJobEvent::Token { id, text } if self.job == Some(id) => Some(ComposerMsg::Token(text)),
            TextJobEvent::Done { id, text, cancelled } if self.job == Some(id) => {
                self.finish();
                // A cancelled answer was cleared in the bar already and is not kept.
                if cancelled {
                    return None;
                }
                self.session.finished(text.clone());
                Some(ComposerMsg::Done(text))
            }
            TextJobEvent::Failed { id, reason } if self.job == Some(id) => {
                self.finish();
                Some(ComposerMsg::Failed {
                    message: failure_sentence(reason).to_string(),
                    settings: reason == JobFailure::NoWritingModel,
                })
            }
            _ => None,
        }
    }

    fn finish(&mut self) {
        self.job = None;
        self.cancel = None;
    }

    /// Put the result into the target window or on the clipboard, and say so on the pill.
    pub fn deliver(&mut self, how: DeliverHow, text: &str, cfg: &AppConfig, gui: &WinGuiHandle) {
        let notice = match how {
            DeliverHow::Replace => match &self.captured {
                Some(captured) => match selection::replace(captured, text, &self.ctx) {
                    Ok(Replaced::Pasted) => "Replaced",
                    Ok(_) => "Copied · press Ctrl+V",
                    Err(_) => copy_to_clipboard(text, cfg),
                },
                None => copy_to_clipboard(text, cfg),
            },
            DeliverHow::Insert => {
                // The gate must be released before the clipboard fallback: it is not reentrant.
                let typed = {
                    let _gate = vi_win32::input_gate();
                    deliver_direct_or_clipboard_fallback(
                        text,
                        &self.ctx,
                        cfg.privacy.block_password_fields,
                        false,
                        cfg.privacy.exclude_from_clipboard_history,
                    )
                };
                match typed {
                    Ok(DirectTypeOutcome::TypedDirectly { .. }) => "Inserted",
                    Ok(DirectTypeOutcome::FallbackToClipboard { .. }) => "Copied · press Ctrl+V",
                    Err(_) => copy_to_clipboard(text, cfg),
                }
            }
            DeliverHow::Copy => copy_to_clipboard(text, cfg),
        };
        gui.quick_fix(QuickFixStage::Notice(notice.to_string()));
    }
}

/// The clipboard fallback for every delivery path: the text still reaches the user when the target refuses.
fn copy_to_clipboard(text: &str, cfg: &AppConfig) -> &'static str {
    let _gate = vi_win32::input_gate();
    match write_clipboard_unicode(text, cfg.privacy.exclude_from_clipboard_history) {
        Ok(()) => "Copied · press Ctrl+V",
        Err(_) => "The clipboard is busy. Nothing was copied.",
    }
}

fn failure_sentence(reason: JobFailure) -> &'static str {
    match reason {
        JobFailure::NoWritingModel => "No writing model is installed and cloud text jobs are off.",
        JobFailure::NoGrammarModel => "The grammar model isn't installed.",
        JobFailure::Unavailable => "This kind of request isn't available in this build.",
        JobFailure::SelectionTooLong => "That text is too long for the model. Select less of it.",
        JobFailure::ModelFailed => "The model didn't answer. Try again.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vi_rewrite::RewriteEngine;

    fn flow() -> ComposerFlow {
        let mut f = ComposerFlow::default();
        f.open(None, ForegroundContext::default(), None);
        f
    }

    #[test]
    fn the_bar_is_open_only_between_open_and_close() {
        let mut f = ComposerFlow::default();
        assert!(!f.is_open(), "a closed Composer takes no dictation");
        f.open(None, ForegroundContext::default(), None);
        assert!(f.is_open());
        f.close();
        assert!(!f.is_open());
    }

    #[test]
    fn a_request_runs_on_the_selection_when_kept_else_on_the_typed_draft() {
        let s = Session::default();
        assert_eq!(s.submit_source(Some("selected"), true, None), "selected");
        assert_eq!(s.submit_source(Some("selected"), true, Some("draft".into())), "selected", "the selection wins");
        assert_eq!(s.submit_source(Some("selected"), false, Some("draft".into())), "draft", "selection removed: the draft");
        assert_eq!(s.submit_source(Some("selected"), false, None), "");
        assert_eq!(s.submit_source(None, true, None), "");
    }

    #[test]
    fn follow_ups_run_on_the_result_and_regenerate_on_the_original_source() {
        let mut s = Session::default();
        assert!(s.follow_up(Instruction::Shorter).is_none(), "nothing to follow up before an answer");
        s.started(Instruction::Improve, "the selection".into());
        s.finished("a better selection".into());
        assert_eq!(s.follow_up(Instruction::Shorter), Some((Instruction::Shorter, "a better selection".into())));
        assert_eq!(s.regenerate(), Some((Instruction::Improve, "the selection".into())));
        // A cancelled or empty answer leaves the latest result alone.
        s.finished("   ".into());
        assert_eq!(s.follow_up(Instruction::Shorter).unwrap().1, "a better selection");
    }

    #[test]
    fn submit_hands_the_worker_a_compose_job() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let engine: SharedEngine = Arc::new(parking_lot::Mutex::new(RewriteEngine::new(None)));
        let mut f = flow();
        f.submit(&AppConfig::default(), Instruction::Improve, true, None, &tx, &engine, &StyleStore::default());
        let (job, _) = rx.try_recv().expect("a job");
        assert_eq!((job.kind, job.instruction), (TextJobKind::Compose, Instruction::Improve));
        assert_eq!(job.source, "", "no selection was captured");
        assert_eq!(job.max_output_tokens, compose::token_limit(&ComposeJob::new(Instruction::Improve, "")));
        assert!(job.cloud.is_none(), "the local route carries no cloud target");
        // A second submit while one runs is dropped.
        f.submit(&AppConfig::default(), Instruction::Shorter, true, None, &tx, &engine, &StyleStore::default());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn a_learned_habit_reaches_the_composer_request() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let engine: SharedEngine = Arc::new(parking_lot::Mutex::new(RewriteEngine::new(None)));
        let mut style = StyleStore::default();
        assert!(style.set_locked("dialect", Scope::Everywhere, "british", 1));
        let mut f = flow();
        f.submit(&AppConfig::default(), Instruction::Fix, true, None, &tx, &engine, &style);
        let (job, _) = rx.try_recv().expect("a job");
        assert!(job.style_block.as_deref().is_some_and(|b| b.contains("British")), "the spelling habit is in the prompt");
    }

    #[test]
    fn only_this_flows_events_reach_the_bar() {
        let mut f = flow();
        assert!(f.handle(TextJobEvent::Token { id: 99, text: "x".into() }).is_none());
        assert!(f.handle(TextJobEvent::Done { id: 99, text: "x".into(), cancelled: false }).is_none());
        assert!(f.handle(TextJobEvent::Failed { id: 99, reason: JobFailure::ModelFailed }).is_none());
    }

    #[test]
    fn a_failure_names_the_missing_writing_model_and_offers_settings() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let engine: SharedEngine = Arc::new(parking_lot::Mutex::new(RewriteEngine::new(None)));
        let mut f = flow();
        f.submit(&AppConfig::default(), Instruction::Fix, true, None, &tx, &engine, &StyleStore::default());
        let (job, _) = rx.try_recv().expect("a job");
        match f.handle(TextJobEvent::Failed { id: job.id, reason: JobFailure::NoWritingModel }) {
            Some(ComposerMsg::Failed { message, settings }) => {
                assert!(message.contains("No writing model"));
                assert!(settings);
            }
            _ => panic!("a Failed message"),
        }
        assert!(f.job.is_none(), "the running job is cleared");
    }
}