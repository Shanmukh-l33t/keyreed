//! Edit watcher wiring (plan P4.2): every text insertion KeyReed makes is recorded here, and a dedicated thread
//! decides whether the field may be read (`vi_core::editwatch::gate`), watches it with change notifications or
//! polling, and turns what the user corrected into dictionary words and style evidence. The rules and the state
//! machine are tested in `vi_core`; this file is the Windows end: the channel, the clock and the UI Automation calls.
//!
//! Privacy: the field text stays in this module's worker. Only a word pair, its undo token and a copy of the
//! dictionary cross the channel, and nothing is logged.
//!
//! Threads: one worker, created at the first request, named `keyreed-learning`. With no watch it blocks on its
//! channel and holds no timer, so idle cost is unchanged; with learning off nothing is even subscribed.

#![cfg(windows)]

use crossbeam_channel::{unbounded, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};
use vi_core::dictionary::PersonalDictionary;
use vi_core::editwatch::{
    self, EndReason, Insertion, Mode, ReadResult, SessionBans, Step, Target, VocabOutcome, Watch, MAX_FIELD_CHARS,
};
use vi_core::style::{LearnState, Scope, StyleStore};
use vi_core::{AppConfig, ForegroundContext, OutputMode};
use vi_win32::uia;

/// Written by `capture_foreground_context` when a window's title must not be read (a password manager or a private
/// browser window): both are refused by the gate anyway.
const REDACTED_TITLE: &str = "[Redacted Private Window]";

/// What the coordinator hears back from the worker.
#[derive(Debug, Clone)]
pub enum LearningEvent {
    /// One correction seen twice became a dictionary word. `dictionary` is the worker's copy after the edit, so the
    /// coordinator can save exactly what was learned.
    Learned { word: String, token: editwatch::UndoToken, dictionary: PersonalDictionary },
}

enum Msg {
    /// KeyReed finished typing into a field.
    Insert(Record),
    /// A text-changed notification from the UI Automation thread.
    Changed,
}

struct Record {
    inserted: String,
    start: Option<usize>,
    suffix_len: Option<usize>,
    target: Target,
    hwnd: isize,
    learn: bool,
    dictionary: PersonalDictionary,
}

/// The learning worker, as the coordinator sees it.
pub struct Learning {
    tx: Sender<Msg>,
    events: Receiver<LearningEvent>,
}

impl Learning {
    /// Starts the worker. It does no work until the first insertion.
    pub fn spawn() -> Self {
        let (tx, msgs) = unbounded();
        let (event_tx, events) = unbounded();
        let notify = tx.clone();
        if let Err(e) = std::thread::Builder::new()
            .name("keyreed-learning".into())
            .spawn(move || worker(msgs, event_tx, notify))
        {
            log::warn!("the learning worker was not started: {e}");
        }
        Self { tx, events }
    }

    /// Results for the coordinator's loop.
    pub fn events(&self) -> &Receiver<LearningEvent> {
        &self.events
    }

    /// Records that KeyReed finished typing `inserted` into the field behind `ctx`. `start` and `suffix_len` are the
    /// char position of the text and the chars after it, when the caller knows them; the worker finds them otherwise.
    /// Call it only after text was actually typed: a clipboard fallback leaves nothing to watch.
    pub fn record_insertion(
        &self,
        inserted: &str,
        start: Option<usize>,
        suffix_len: Option<usize>,
        ctx: &ForegroundContext,
        learn: bool,
        dictionary: &PersonalDictionary,
    ) {
        // Nothing to watch without text or a window; KeyReed's own windows (the practice box) are not learned from.
        if inserted.trim().is_empty() || ctx.hwnd == 0 || ctx.process_id == std::process::id() {
            return;
        }
        let _ = self.tx.send(Msg::Insert(Record {
            inserted: inserted.to_string(),
            start,
            suffix_len,
            target: target_from(ctx),
            hwnd: ctx.hwnd as isize,
            learn,
            dictionary: dictionary.clone(),
        }));
    }
}

/// What was typed into, in the form the gate takes.
pub fn target_from(ctx: &ForegroundContext) -> Target {
    Target {
        process_name: ctx.process_name.clone(),
        window_class: ctx.window_class.clone(),
        category: ctx.category,
        is_password_field: ctx.is_password_field,
        is_private_browsing: vi_win32::context::is_private_browsing_title(&ctx.window_title_hint)
            || ctx.window_title_hint == REDACTED_TITLE,
        is_elevated: ctx.is_elevated,
    }
}

/// Only text that was typed leaves something to watch; a clipboard fallback typed nothing.
pub fn delivered_by_typing(via: OutputMode) -> bool {
    via == OutputMode::DirectTyping
}

/// The store's switch follows the app setting. The store's own Pause (set on the Settings page) is left alone.
pub fn apply_switch(store: &mut StyleStore, learn: bool) {
    if !learn {
        store.learn_from_edits = LearnState::Off;
    } else if store.learn_from_edits == LearnState::Off {
        store.learn_from_edits = LearnState::On;
    }
}

/// The pill's Undo: takes back one promotion, removing only what it added. The caller saves the config.
pub fn undo_learning(cfg: &mut AppConfig, token: &editwatch::UndoToken) {
    editwatch::undo_learned(&mut cfg.dictionary, token);
}

// ---------------------------------------------------------------------------------------------------------------
// The worker
// ---------------------------------------------------------------------------------------------------------------

/// One field being watched.
struct Active {
    watch: Watch,
    /// When to look again, on the worker's clock.
    due: u64,
    subscription: Option<uia::WatchId>,
    hwnd: isize,
    target: Target,
    insertion: Insertion,
    scope: Scope,
    /// The application category, as the vocabulary candidates record it.
    context: String,
    /// The last region a read found, so a repeated polling read is not counted as a new correction.
    last_region: Option<String>,
    dict: PersonalDictionary,
}

fn worker(msgs: Receiver<Msg>, events: Sender<LearningEvent>, notify: Sender<Msg>) {
    let epoch = Instant::now();
    let now = || epoch.elapsed().as_millis() as u64;
    let mut store = StyleStore::load(&StyleStore::default_path()).map(|(s, _)| s).unwrap_or_default();
    let mut bans = SessionBans::default();
    let mut active: Option<Active> = None;

    loop {
        // Sleep until the next read is due, or until a message arrives. With no watch this blocks for good.
        let message = match active.as_ref() {
            Some(a) => match msgs.recv_timeout(Duration::from_millis(a.due.saturating_sub(now()).max(1))) {
                Ok(m) => Some(m),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            },
            None => match msgs.recv() {
                Ok(m) => Some(m),
                Err(_) => return,
            },
        };
        match message {
            Some(Msg::Insert(record)) => {
                if let Some(mut a) = active.take() {
                    end(&mut a, EndReason::NextAction, &mut bans);
                }
                active = start(record, &mut store, &mut bans, &notify, &events, now());
            }
            Some(Msg::Changed) => {
                if let Some(a) = &mut active {
                    a.watch.text_changed(now());
                }
            }
            None => {}
        }
        // Then do what the state machine says, until it wants to wait again.
        while let Some(a) = active.as_mut() {
            match a.watch.due(now()) {
                Step::Read => read_field(a, &mut store, &events, now()),
                Step::Wait { until } => {
                    a.due = until;
                    break;
                }
                Step::Stop { reason, .. } => {
                    end(a, reason, &mut bans);
                    active = None;
                }
            }
        }
    }
}

/// The gate, the subscription, and one read taken before the user can edit: that read fixes where the text sits.
fn start(
    record: Record,
    store: &mut StyleStore,
    bans: &mut SessionBans,
    notify: &Sender<Msg>,
    events: &Sender<LearningEvent>,
    now: u64,
) -> Option<Active> {
    apply_switch(store, record.learn);
    // shortcut: no light-profile flag exists yet, so browsers follow the switch alone; pass the profile in when H1's
    // light profile lands.
    editwatch::gate(&record.target, store, false, bans).ok()?;
    // Notifications when the field supports them, polling otherwise; either way nothing is read until the watch asks.
    let sender = notify.clone();
    let subscription = uia::service()
        .watch(uia::Target::Window(record.hwnd), move || {
            let _ = sender.send(Msg::Changed);
        })
        .ok();
    let scope = Scope::for_category(record.target.category);
    let mut active = Active {
        watch: Watch::new(now, if subscription.is_some() { Mode::Notifications } else { Mode::Polling }),
        due: now,
        subscription,
        hwnd: record.hwnd,
        insertion: Insertion { inserted: record.inserted, start: record.start, suffix_len: record.suffix_len },
        // The candidate context is the application category, in the form the store records it.
        context: format!("{:?}", record.target.category).to_lowercase(),
        target: record.target,
        scope,
        last_region: None,
        dict: record.dictionary,
    };
    read_field(&mut active, store, events, now);
    Some(active)
}

fn end(a: &mut Active, reason: EndReason, bans: &mut SessionBans) {
    if let Some(id) = a.subscription.take() {
        uia::service().unwatch(id);
    }
    if reason == EndReason::TooSlow {
        bans.ban(&a.target.process_name);
    }
}

/// Reads the field; the whole of what is learned leaves as word pairs.
fn read_field(a: &mut Active, store: &mut StyleStore, events: &Sender<LearningEvent>, now: u64) {
    let started = Instant::now();
    let read = uia::service().read_text(uia::Target::Window(a.hwnd), MAX_FIELD_CHARS + 1);
    let took_ms = started.elapsed().as_millis() as u64;
    let (result, text) = match read {
        // A refused or expired read is a field that is gone; the watch ends silently either way.
        Err(_) => (ReadResult::Gone, None),
        Ok(t) if t.truncated => (ReadResult::Ok { chars: MAX_FIELD_CHARS + 1 }, None),
        Ok(t) => (ReadResult::Ok { chars: t.text.chars().count() }, Some(t.text)),
    };
    if let Some(text) = &text {
        if let Some(region) = editwatch::locate(text, &a.insertion) {
            let chars = text.chars().count();
            if a.insertion.start.is_none() || a.insertion.suffix_len.is_none() {
                let start = text[..region.as_ptr() as usize - text.as_ptr() as usize].chars().count();
                a.insertion.start = Some(start);
                a.insertion.suffix_len = Some(chars - start - region.chars().count());
            }
            // Polling reads the same text again every couple of seconds; only a change is evidence.
            if a.last_region.as_deref() != Some(region) {
                let report = editwatch::learn_from_edit(store, &mut a.dict, &a.insertion.inserted, region, &a.context, a.scope, now);
                if report.ignored.is_none() && (report.habits_recorded > 0 || !report.vocabulary.is_empty()) {
                    let _ = store.save_atomic(&StyleStore::default_path());
                }
                for outcome in &report.vocabulary {
                    if let VocabOutcome::Promoted(token) = outcome {
                        let _ = events.send(LearningEvent::Learned { word: token.to.clone(), token: token.clone(), dictionary: a.dict.clone() });
                    }
                }
                a.last_region = Some(region.to_string());
            }
        }
    }
    a.watch.read_done(now, took_ms, result);
}

#[cfg(test)]
mod tests {
    use super::*;
    use vi_core::editwatch::{gate, Deny};
    use vi_core::style::LearnState;
    use vi_core::AppCategory;

    fn ctx(process: &str, class: &str) -> ForegroundContext {
        ForegroundContext { hwnd: 42, process_id: 4242, process_name: process.into(), window_class: class.into(), ..ForegroundContext::default() }
    }

    #[test]
    fn the_target_carries_what_the_gate_looks_at() {
        let mut c = ctx("WINWORD.EXE", "OpusApp");
        c.category = AppCategory::Document;
        c.is_password_field = true;
        c.is_elevated = true;
        let t = target_from(&c);
        assert_eq!((t.process_name.as_str(), t.window_class.as_str(), t.category), ("WINWORD.EXE", "OpusApp", AppCategory::Document));
        assert!(t.is_password_field && t.is_elevated && !t.is_private_browsing);

        // A private window keeps its title from us, so the redacted hint is what says so.
        let mut c = ctx("msedge.exe", "Chrome_WidgetWin_1");
        c.window_title_hint = REDACTED_TITLE.into();
        assert!(target_from(&c).is_private_browsing);
        let mut c = ctx("msedge.exe", "Chrome_WidgetWin_1");
        c.window_title_hint = "InPrivate - Bing".into();
        assert!(target_from(&c).is_private_browsing);
    }

    #[test]
    fn only_a_typed_delivery_is_recorded() {
        assert!(delivered_by_typing(OutputMode::DirectTyping));
        for other in [OutputMode::Clipboard, OutputMode::AskBeforeInserting] {
            assert!(!delivered_by_typing(other), "{other:?}");
        }
    }

    #[test]
    fn the_app_switch_closes_the_gate_before_anything_else_is_looked_at() {
        let mut store = StyleStore::default();
        let bans = SessionBans::default();
        let t = target_from(&ctx("notepad.exe", "Notepad"));
        assert!(gate(&t, &store, false, &bans).is_ok());
        apply_switch(&mut store, false);
        assert_eq!(gate(&t, &store, false, &bans), Err(Deny::LearningOff));
        apply_switch(&mut store, true);
        assert!(gate(&t, &store, false, &bans).is_ok());
        // The store's own Pause is not undone by an app setting that says "on".
        store.learn_from_edits = LearnState::Paused;
        apply_switch(&mut store, true);
        assert_eq!(gate(&t, &store, false, &bans), Err(Deny::LearningOff));
        apply_switch(&mut store, false);
        assert_eq!(store.learn_from_edits, LearnState::Off);
    }

    #[test]
    fn undo_takes_the_learned_word_back_out_of_the_config() {
        let mut cfg = AppConfig::default();
        let mut store = StyleStore::default();
        editwatch::learn_vocabulary(&mut store, &mut cfg.dictionary, "kubernets", "Kubernetes", "chat", 1);
        let VocabOutcome::Promoted(token) = editwatch::learn_vocabulary(&mut store, &mut cfg.dictionary, "kubernets", "Kubernetes", "chat", 2) else {
            panic!("the second sighting is a promotion")
        };
        assert_eq!(cfg.dictionary.vocabulary, ["Kubernetes"]);
        undo_learning(&mut cfg, &token);
        assert!(cfg.dictionary.vocabulary.is_empty() && cfg.dictionary.replacements.is_empty());
    }
}