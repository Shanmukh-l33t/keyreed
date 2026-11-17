//! Quick Fix on the coordinator thread (plan F1): the chord, the selection service, the engine's result, the Review
//! panel, replacement and Undo. The engine itself runs on the text-job worker.

#![cfg(windows)]

use crate::textjobs::{JobRequest, SharedEngine};
use anyhow::{Context, Result};
use crossbeam_channel::Sender;
use vi_core::textjob::{FixEdit, JobFailure, TextJob, TextJobEvent, TextJobKind};
use vi_core::{AppConfig, ForegroundContext};
use vi_rewrite::{gector::Gector, quickfix::QuickFixEngine};
use vi_win32::composer::{ReviewEdit, ReviewRequest};
use vi_win32::selection::{self, Captured, Replaced};
use vi_win32::ui::win_gui::WinGuiHandle;
use vi_win32::ui::{QuickFixStage, ReviewChoice};

const MODEL_FOLDER: &str = "gector-bert-en-int8";

/// Loads the grammar model and the Windows spell checker; runs on the worker thread at the first Quick Fix.
pub fn load_engine(cfg: &AppConfig) -> Result<QuickFixEngine> {
    let paths = crate::RuntimePaths::discover(cfg);
    let root = paths.find_installed_model_root(MODEL_FOLDER).context("the grammar model is not installed")?;
    let gector = Gector::load(&paths.sherpa_lib_dir.join("onnxruntime.dll"), &root.join(MODEL_FOLDER), 0)?;
    Ok(QuickFixEngine::new(Box::new(gector)).with_speller(Box::new(crate::WindowsSpelling(vi_win32::spell::WindowsSpeller::new()))))
}

struct Pending {
    id: u64,
    ctx: ForegroundContext,
    captured: Captured,
}

#[derive(Default)]
pub struct QuickFixFlow {
    next_id: u64,
    pending: Option<Pending>,
    /// The window of the last replacement and its change count, for Undo.
    undo: Option<(u64, u32)>,
}

impl QuickFixFlow {
    /// The Quick Fix chord: capture the selection and hand it to the worker.
    pub fn start(&mut self, cfg: &AppConfig, gui: &WinGuiHandle, jobs: &Sender<JobRequest>, engine: &SharedEngine) {
        if self.pending.is_some() {
            return;
        }
        let ctx = vi_win32::context::capture_foreground_context(cfg.privacy.redact_sensitive_window_titles);
        if ctx.process_id == std::process::id() {
            return;
        }
        gui.quick_fix(QuickFixStage::Working);
        let captured = match selection::capture(&ctx) {
            // The editor copied a whole line because nothing was selected: replacing it would duplicate the line.
            Ok(c) if c.copied_line => return refuse(gui, selection::Refusal::NothingSelected.sentence()),
            Ok(c) => c,
            Err(r) => return refuse(gui, r.sentence()),
        };
        self.next_id += 1;
        let id = self.next_id;
        let job = TextJob {
            id,
            kind: TextJobKind::QuickFix,
            instruction: vi_core::textjob::Instruction::Fix,
            source: captured.text.clone(),
            rich: captured.structured,
            target: ctx.clone(),
            max_output_tokens: 0,
            terms: if cfg.dictionary.enabled { cfg.dictionary.vocabulary.clone() } else { Vec::new() },
            cancel: Default::default(),
        };
        if jobs.send((job, engine.clone())).is_err() {
            return refuse(gui, "Quick Fix isn't available right now.");
        }
        self.pending = Some(Pending { id, ctx, captured });
    }

    /// A text-job event; anything that is not this flow's job is ignored.
    pub fn on_job_event(&mut self, ev: TextJobEvent, cfg: &AppConfig, gui: &WinGuiHandle) {
        match ev {
            TextJobEvent::QuickFixDone { id, text, edits } if self.is_pending(id) => {
                let p = self.pending.take().expect("checked");
                if edits.is_empty() {
                    gui.quick_fix(QuickFixStage::LooksGood);
                } else if !cfg.quick_fix_always_review && !p.captured.structured && edits.iter().all(|e| e.silent) {
                    self.apply(gui, &p, &text, edits.len() as u32);
                } else {
                    gui.quick_fix(QuickFixStage::Notice(format!("{} to review", plural(edits.len()))));
                    let request = review_request(&p.captured.text, &edits, p.captured.structured);
                    gui.show_review(request, !cfg.quick_fix_intro_seen);
                    self.pending = Some(p);
                }
            }
            TextJobEvent::Failed { id, reason } if self.is_pending(id) => {
                self.pending = None;
                refuse(gui, match reason {
                    JobFailure::NoGrammarModel => "The grammar model isn't installed.",
                    _ => "Quick Fix couldn't finish. Nothing was changed.",
                });
            }
            _ => {}
        }
    }

    fn is_pending(&self, id: u64) -> bool {
        self.pending.as_ref().is_some_and(|p| p.id == id)
    }

    /// The Review panel closed. `text` is the original with the accepted changes applied.
    pub fn on_review_done(&mut self, choice: ReviewChoice, text: &str, accepted: u32, gui: &WinGuiHandle) {
        let Some(p) = self.pending.take() else { return };
        match choice {
            ReviewChoice::Replace if text != p.captured.text => self.apply(gui, &p, text, accepted),
            ReviewChoice::Copy => match vi_win32::clipboard::write_clipboard_unicode(text, true) {
                Ok(()) => gui.quick_fix(QuickFixStage::Notice("Copied".into())),
                Err(_) => refuse(gui, "The clipboard is busy. Nothing was changed."),
            },
            _ => {}
        }
    }

    fn apply(&mut self, gui: &WinGuiHandle, p: &Pending, text: &str, changes: u32) {
        match selection::replace(&p.captured, text, &p.ctx) {
            Ok(Replaced::Pasted) => {
                self.undo = Some((p.ctx.hwnd, changes));
                gui.quick_fix(QuickFixStage::Fixed { changes, can_deepen: false });
            }
            Ok(_) => gui.quick_fix(QuickFixStage::Notice("Copied · press Ctrl+V".into())),
            Err(r) => refuse(gui, r.sentence()),
        }
    }

    pub fn undo(&mut self, gui: &WinGuiHandle) {
        let Some((hwnd, changes)) = self.undo.take() else { return };
        match selection::undo(hwnd) {
            Ok(()) => gui.quick_fix(QuickFixStage::Undone { changes }),
            Err(r) => refuse(gui, r.sentence()),
        }
    }
}

fn refuse(gui: &WinGuiHandle, sentence: &str) {
    gui.quick_fix(QuickFixStage::Refused(sentence.to_string()));
}

fn plural(n: usize) -> String {
    format!("{n} change{}", if n == 1 { "" } else { "s" })
}

fn review_request(original: &str, edits: &[FixEdit], structured: bool) -> ReviewRequest {
    ReviewRequest {
        original: original.to_string(),
        edits: edits
            .iter()
            .map(|e| ReviewEdit { start: e.start, end: e.end, replacement: e.replacement.clone(), label: e.label })
            .collect(),
        note: structured.then(|| "The selection has formatting (links, images or tables) that replacing would lose.".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_request_carries_edits_and_the_formatting_note() {
        let e = FixEdit { start: 2, end: 5, replacement: "have".into(), label: "Grammar", silent: false };
        let r = review_request("I has a cat.", std::slice::from_ref(&e), false);
        assert_eq!((r.edits[0].start, r.edits[0].end, r.edits[0].label), (2, 5, "Grammar"));
        assert!(r.note.is_none());
        assert!(review_request("x", &[e], true).note.is_some());
        assert_eq!(plural(1), "1 change");
        assert_eq!(plural(3), "3 changes");
    }
}
