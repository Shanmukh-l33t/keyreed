//! Quick Fix on the coordinator thread (plan F1): the chord, the selection service, the engine's result, the Review
//! panel, replacement and Undo. The engine itself runs on the text-job worker.

#![cfg(windows)]

use crate::learning::Learning;
use crate::textjobs::{JobRequest, SharedEngine};
use anyhow::{Context, Result};
use crossbeam_channel::Sender;
use vi_core::dictionary::PersonalDictionary;
use vi_core::habits::{self, Dialect};
use vi_core::style::{Scope, StyleStore};
use vi_core::textjob::{FixEdit, Instruction, JobFailure, TextJob, TextJobEvent, TextJobKind};
use vi_core::{AppConfig, ForegroundContext};
use vi_rewrite::compose::{self, ComposeJob};
use vi_rewrite::{gector::Gector, quickfix::QuickFixEngine};
use vi_win32::composer::{ReviewEdit, ReviewRequest};
use vi_win32::selection::{self, Captured, Replaced};
use vi_win32::ui::win_gui::WinGuiHandle;
use vi_win32::ui::{QuickFixStage, ReviewChoice};

const MODEL_FOLDER: &str = "gector-bert-en-int8";

/// Voice-command jobs take their ids from a separate range: every flow sees every text-job event, so a
/// Composer job's `Done` must never match a voice command's id.
const VOICE_ID_BASE: u64 = 1 << 32;

/// A voice command with nothing selected has nothing to rewrite.
const NOTHING_SELECTED: &str = "Select the text to rewrite first.";

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
    /// The learning switch and the dictionary as they were when the fix started.
    learn: bool,
    dictionary: PersonalDictionary,
}

#[derive(Default)]
pub struct QuickFixFlow {
    next_id: u64,
    pending: Option<Pending>,
    /// A voice command's selection, held between the chord's press and its release.
    voice: Option<(ForegroundContext, Captured)>,
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
            // Quick Fix runs no language model, so it takes no style block.
            style_block: None,
            cancel: Default::default(),
            cloud: None,
        };
        if jobs.send((job, engine.clone())).is_err() {
            return refuse(gui, "Quick Fix isn't available right now.");
        }
        self.pending = Some(Pending { id, ctx, captured, learn: cfg.learn_from_edits, dictionary: cfg.dictionary.clone() });
    }

    /// The voice-command chord's press: capture the selection exactly as Quick Fix does and hold it until the
    /// chord is released. False when there is nothing to rewrite; the pill then says why.
    pub fn arm_voice(&mut self, cfg: &AppConfig, gui: &WinGuiHandle) -> bool {
        if self.voice.is_some() {
            return false;
        }
        if self.pending.is_some() {
            // A second job would replace the pending one, and the Review panel on screen would then be
            // showing a change that no longer has a selection behind it.
            refuse(gui, "Finish the change in the Review panel first.");
            return false;
        }
        let ctx = vi_win32::context::capture_foreground_context(cfg.privacy.redact_sensitive_window_titles);
        if ctx.process_id == std::process::id() {
            return false;
        }
        match selection::capture(&ctx) {
            // The editor copied a whole line because nothing was selected: rewriting it would replace the line.
            Ok(c) if c.copied_line => refuse(gui, NOTHING_SELECTED),
            Ok(c) => {
                self.voice = Some((ctx, c));
                return true;
            }
            Err(selection::Refusal::NothingSelected) => refuse(gui, NOTHING_SELECTED),
            Err(r) => refuse(gui, r.sentence()),
        }
        false
    }

    /// The chord's release: run the spoken words on the held selection. The answer arrives in the Review panel
    /// as one change over the whole selection, so Replace, Copy and Undo work as they do for Quick Fix.
    pub fn run_voice(&mut self, instruction: Instruction, cfg: &AppConfig, gui: &WinGuiHandle, jobs: &Sender<JobRequest>, engine: &SharedEngine) {
        let Some((ctx, captured)) = self.voice.take() else { return };
        self.next_id += 1;
        let id = VOICE_ID_BASE + self.next_id;
        let route = crate::route_for(cfg);
        let job = TextJob {
            id,
            kind: TextJobKind::VoiceCommand,
            instruction: instruction.clone(),
            source: captured.text.clone(),
            rich: captured.structured,
            target: ctx.clone(),
            max_output_tokens: compose::token_limit(&ComposeJob::new(instruction, &captured.text)),
            terms: if cfg.dictionary.enabled { cfg.dictionary.vocabulary.clone() } else { Vec::new() },
            // A spoken instruction is the user's own words; the learned habits are not added to it.
            style_block: None,
            cancel: Default::default(),
            cloud: crate::job_cloud(cfg, &route),
        };
        gui.quick_fix(QuickFixStage::Working);
        if jobs.send((job, engine.clone())).is_err() {
            return refuse(gui, "KeyReed couldn't start the voice command.");
        }
        self.pending = Some(Pending { id, ctx, captured, learn: cfg.learn_from_edits, dictionary: cfg.dictionary.clone() });
    }

    /// The chord was cancelled, the microphone failed or nothing was said: the held selection is handed back so a
    /// tap can open the Composer on it. Every other caller just forgets the selection and ignores it.
    pub fn cancel_voice(&mut self) -> Option<(ForegroundContext, Captured)> {
        self.voice.take()
    }

    /// A text-job event; anything that is not this flow's job is ignored.
    pub fn on_job_event(&mut self, ev: TextJobEvent, cfg: &AppConfig, style: &StyleStore, gui: &WinGuiHandle, learning: &Learning) {
        match ev {
            TextJobEvent::QuickFixDone { id, text, edits } if self.is_pending(id) => {
                let p = self.pending.take().expect("checked");
                // A change that would undo a habit the user holds (a locked "British spelling", the spelling
                // setting) is dropped here: it is never reviewed, never applied and never counted as accepted.
                let offered = edits.len();
                let edits = keep_edits(&p.captured.text, edits, style, cfg.dialect, Scope::for_category(p.ctx.category));
                // The engine's text still holds the dropped changes, so rebuild it from the kept edits.
                let text = if edits.len() == offered { text } else { apply_edits(&p.captured.text, &edits) };
                if edits.is_empty() {
                    gui.quick_fix(QuickFixStage::LooksGood);
                } else if !cfg.quick_fix_always_review && !p.captured.structured && edits.iter().all(|e| e.silent) {
                    self.apply(gui, &p, &text, edits.len() as u32, learning);
                } else {
                    gui.quick_fix(QuickFixStage::Notice(format!("{} to review", plural(edits.len()))));
                    let request = review_request(&p.captured.text, &edits, p.captured.structured);
                    gui.show_review(request, !cfg.quick_fix_intro_seen);
                    self.pending = Some(p);
                }
            }
            TextJobEvent::Done { id, text, cancelled } if self.is_pending(id) => {
                let p = self.pending.take().expect("checked");
                if cancelled || text.trim().is_empty() {
                    return refuse(gui, "The voice command came back empty. Nothing was changed.");
                }
                // One change over the whole selection, so Replace, Copy and Undo work as they do for Quick Fix.
                let edit = FixEdit { start: 0, end: p.captured.text.len(), replacement: text, label: "Voice", silent: false };
                gui.quick_fix(QuickFixStage::Notice("1 change to review".into()));
                gui.show_review(review_request(&p.captured.text, std::slice::from_ref(&edit), p.captured.structured), false);
                self.pending = Some(p);
            }
            TextJobEvent::Failed { id, reason } if self.is_pending(id) => {
                self.pending = None;
                // The missing grammar model is the one failure the user can fix, and exactly where this sentence says.
                let settings = id < VOICE_ID_BASE && reason == JobFailure::NoGrammarModel;
                gui.quick_fix(QuickFixStage::Refused { text: failure_sentence(id, reason).to_string(), settings });
            }
            _ => {}
        }
    }

    fn is_pending(&self, id: u64) -> bool {
        self.pending.as_ref().is_some_and(|p| p.id == id)
    }

    /// The Review panel closed. `text` is the original with the accepted changes applied.
    pub fn on_review_done(&mut self, choice: ReviewChoice, text: &str, accepted: u32, gui: &WinGuiHandle, learning: &Learning) {
        let Some(p) = self.pending.take() else { return };
        match choice {
            ReviewChoice::Replace if text != p.captured.text => self.apply(gui, &p, text, accepted, learning),
            ReviewChoice::Copy => match vi_win32::clipboard::write_clipboard_unicode(text, true) {
                Ok(()) => gui.quick_fix(QuickFixStage::Notice("Copied".into())),
                Err(_) => refuse(gui, "The clipboard is busy. Nothing was changed."),
            },
            _ => {}
        }
    }

    fn apply(&mut self, gui: &WinGuiHandle, p: &Pending, text: &str, changes: u32, learning: &Learning) {
        match selection::replace(&p.captured, text, &p.ctx) {
            Ok(Replaced::Pasted) => {
                self.undo = Some((p.ctx.hwnd, changes));
                // Only a pasted replacement is worth watching; the other outcomes put nothing in the field.
                learning.record_insertion(text, None, None, &p.ctx, p.learn, &p.dictionary);
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
    gui.quick_fix(QuickFixStage::Refused { text: sentence.to_string(), settings: false });
}

/// Why a job stopped, in one sentence. Voice commands take their ids from `VOICE_ID_BASE`, so the reason is
/// always about the right kind of job.
fn failure_sentence(id: u64, reason: JobFailure) -> &'static str {
    if id >= VOICE_ID_BASE {
        return match reason {
            JobFailure::NoWritingModel => "No writing model is installed and cloud text jobs are off.",
            _ => "The voice command couldn't finish. Nothing was changed.",
        };
    }
    match reason {
        // Neither model is available: name the install route, and for a long selection the way out.
        JobFailure::NoGrammarModel => "The grammar model isn't installed; install it in Settings › Speech models.",
        JobFailure::SelectionTooLong => "That selection is too long for the writing model. Select less text, or install the grammar model.",
        _ => "Quick Fix couldn't finish. Nothing was changed.",
    }
}

fn plural(n: usize) -> String {
    format!("{n} change{}", if n == 1 { "" } else { "s" })
}

/// The edits that do not undo a habit the user holds.
fn keep_edits(source: &str, edits: Vec<FixEdit>, store: &StyleStore, dialect: Dialect, scope: Scope) -> Vec<FixEdit> {
    edits.into_iter().filter(|e| !conflicts_with_habits(source, e, store, dialect, scope)).collect()
}

fn conflicts_with_habits(source: &str, e: &FixEdit, store: &StyleStore, dialect: Dialect, scope: Scope) -> bool {
    sentence_around(source, e)
        .is_some_and(|(before, after)| habits::edit_conflicts_with_dialect(&before, &after, store, scope, dialect))
}

/// The sentence of `source` holding the edit, and that sentence with the edit applied: the habit check needs
/// the whole sentence (the serial comma, a collective noun's verb), not just the changed word.
fn sentence_around(source: &str, e: &FixEdit) -> Option<(String, String)> {
    source.get(e.start..e.end)?;
    let start = source.get(..e.start)?.rfind(|c: char| matches!(c, '.' | '!' | '?' | '\n')).map_or(0, |i| i + 1);
    let end = source.get(e.end..)?.find(|c: char| matches!(c, '.' | '!' | '?' | '\n')).map_or(source.len(), |i| e.end + i + 1);
    let mut after = String::with_capacity(end - start + e.replacement.len());
    after.push_str(&source[start..e.start]);
    after.push_str(&e.replacement);
    after.push_str(&source[e.end..end]);
    Some((source[start..end].to_string(), after))
}

/// `source` with `edits` applied; the silent path applies the kept ones only.
fn apply_edits(source: &str, edits: &[FixEdit]) -> String {
    let mut out = String::with_capacity(source.len());
    let mut at = 0;
    for e in edits {
        let Some(gap) = source.get(at..e.start).filter(|_| e.start >= at) else { return source.to_string() };
        out.push_str(gap);
        out.push_str(&e.replacement);
        at = e.end;
    }
    match source.get(at..) {
        Some(rest) => out.push_str(rest),
        None => return source.to_string(),
    }
    out
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

    #[test]
    fn a_voice_command_is_one_change_over_the_whole_selection() {
        let source = "I has a cat.";
        let edit = FixEdit { start: 0, end: source.len(), replacement: "I have a cat.".into(), label: "Voice", silent: false };
        let r = review_request(source, std::slice::from_ref(&edit), false);
        assert_eq!((r.edits.len(), r.edits[0].start, r.edits[0].end, r.edits[0].label), (1, 0, 12, "Voice"));
        assert_eq!(apply_edits(source, &[edit]), "I have a cat.");
    }

    #[test]
    fn a_voice_command_failure_is_not_described_as_a_quick_fix() {
        assert_eq!(
            failure_sentence(VOICE_ID_BASE + 1, JobFailure::NoWritingModel),
            "No writing model is installed and cloud text jobs are off."
        );
        assert!(failure_sentence(VOICE_ID_BASE + 1, JobFailure::ModelFailed).contains("voice command"));
        assert!(failure_sentence(1, JobFailure::ModelFailed).contains("Quick Fix"));
        // The refusal names the install route, and a too-long selection names its way out.
        assert!(failure_sentence(1, JobFailure::NoGrammarModel).contains("Settings › Speech models"));
        assert!(failure_sentence(1, JobFailure::SelectionTooLong).contains("Select less text"));
    }

    #[test]
    fn a_locked_habit_drops_its_own_edit_and_keeps_the_others() {
        let mut s = StyleStore::default();
        assert!(s.set_locked("dialect", Scope::Everywhere, "british", 1));
        let source = "The colour of the sky. I has a cat.";
        let spelling = FixEdit { start: 4, end: 10, replacement: "color".into(), label: "Spelling", silent: true };
        let grammar = FixEdit { start: 25, end: 28, replacement: "have".into(), label: "Grammar", silent: false };
        let kept = keep_edits(source, vec![spelling.clone(), grammar.clone()], &s, Dialect::Auto, Scope::Everywhere);
        assert_eq!(kept, vec![grammar], "the spelling the user keeps is not changed");
        assert_eq!(apply_edits(source, &kept), "The colour of the sky. I have a cat.");
        // Without the habit, or with the other spelling chosen, the edit stands.
        assert_eq!(keep_edits(source, vec![spelling.clone()], &StyleStore::default(), Dialect::Auto, Scope::Everywhere).len(), 1);
        assert_eq!(keep_edits(source, vec![spelling.clone()], &s, Dialect::American, Scope::Everywhere).len(), 1);
        // The spelling setting restrains on its own.
        assert!(keep_edits(source, vec![spelling], &StyleStore::default(), Dialect::British, Scope::Everywhere).is_empty());
    }
}
