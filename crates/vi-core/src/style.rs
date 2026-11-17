//! "My style" store (plan P4.3, contracts in E4): learned writing habits with their evidence, vocabulary candidates,
//! suggestion counters and the learning switches, kept in `style.json` beside `history.json`.
//!
//! Only habits, counters and word pairs are stored, never the text they came from. A learned habit applies only once
//! it has [`MIN_SENTENCES`] observations and [`MIN_AGREE_PERCENT`] percent of them agree (starting values from the
//! plan, to be tuned); a habit the user set by hand ("locked") always applies and is never overridden.

use crate::types::AppCategory;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

pub const STYLE_VERSION: u32 = 1;
/// Observations needed before a learned habit applies.
pub const MIN_SENTENCES: u32 = 30;
/// Share of those observations that must agree, in percent.
pub const MIN_AGREE_PERCENT: u32 = 80;
/// A vocabulary correction seen this many times is added to the dictionary.
pub const VOCAB_PROMOTE_COUNT: u32 = 2;

const MAX_ITEMS: usize = 256;
const MAX_CANDIDATES: usize = 200;
const MAX_CHIPS: usize = 100;
const MAX_EXCLUDED_APPS: usize = 100;
const MAX_TEXT_CHARS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LearnState {
    #[default]
    On,
    Off,
    Paused,
}

/// Where a habit applies: everywhere, or only in one kind of application.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    #[default]
    Everywhere,
    Email,
    Chat,
    Documents,
}

impl Scope {
    /// Maps the foreground application category to a style scope.
    pub fn for_category(category: AppCategory) -> Self {
        match category {
            AppCategory::Email => Scope::Email,
            AppCategory::Chat => Scope::Chat,
            AppCategory::Document | AppCategory::Notes => Scope::Documents,
            _ => Scope::Everywhere,
        }
    }
}

/// What a piece of evidence came from; decides which learning switch applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The user's edits to text KeyReed typed.
    Edits,
    /// Text the user handed to Fix or Improve.
    Drafts,
    /// Text the user pasted into Settings on purpose: always allowed.
    Samples,
}

/// One habit value for a (key, scope), with its evidence. `seen` counts every observation of that (key, scope) and
/// `agree` those that had this value, so `agree / seen` is this value's share.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct StyleItem {
    pub key: String,
    pub scope: Scope,
    pub value: String,
    pub seen: u32,
    pub agree: u32,
    pub locked: bool,
    pub updated: u64,
}

impl StyleItem {
    fn passes_threshold(&self) -> bool {
        self.seen >= MIN_SENTENCES && u64::from(self.agree) * 100 >= u64::from(MIN_AGREE_PERCENT) * u64::from(self.seen)
    }
}

/// A word KeyReed typed that the user replaced with a similar-looking or similar-sounding one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct VocabCandidate {
    pub from: String,
    pub to: String,
    pub context: String,
    pub count: u32,
    pub first_seen: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StyleStore {
    pub version: u32,
    pub learn_from_edits: LearnState,
    pub learn_from_drafts: LearnState,
    /// Stays off until read-back in Chromium-based windows passes its cost check (plan P0.5).
    pub learn_in_browsers: bool,
    pub items: Vec<StyleItem>,
    pub vocabulary_candidates: Vec<VocabCandidate>,
    pub chip_counts: BTreeMap<String, u32>,
    /// Lower-case executable names that are never read.
    pub excluded_apps: Vec<String>,
}

impl Default for StyleStore {
    fn default() -> Self {
        Self {
            version: STYLE_VERSION,
            learn_from_edits: LearnState::On,
            learn_from_drafts: LearnState::On,
            learn_in_browsers: false,
            items: Vec::new(),
            vocabulary_candidates: Vec::new(),
            chip_counts: BTreeMap::new(),
            excluded_apps: Vec::new(),
        }
    }
}

/// What `StyleStore::load` did with the file on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadOutcome {
    /// There was no file; a fresh store was started.
    Fresh,
    Loaded,
    /// The file was unreadable or written by a newer KeyReed; it was moved here untouched and a fresh store started.
    SetAside(PathBuf),
}

fn valid_text(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty() && t.chars().count() <= MAX_TEXT_CHARS
}

impl StyleStore {
    pub fn default_path() -> PathBuf {
        crate::config::AppConfig::default_data_dir().join("style.json")
    }

    /// Loads `path`. A missing file gives a fresh store. A file that is not valid, or that was written by a newer
    /// version, is moved aside (`style.invalid.json`, `style.v<N>.json`) and a fresh store is started, so nothing
    /// the user had is lost and an old build never overwrites newer data. Errors only when the file cannot be read
    /// or moved, so the caller does not overwrite what it could not read.
    pub fn load(path: &Path) -> Result<(Self, LoadOutcome)> {
        let raw = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok((Self::default(), LoadOutcome::Fresh)),
            Err(e) => return Err(e).with_context(|| format!("failed to read {}", path.display())),
        };
        let value = std::str::from_utf8(&raw)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .filter(|v| v.is_object());
        let Some(value) = value else {
            return Self::set_aside(path, "style.invalid.json");
        };
        match value.get("version").map_or(Some(1), |v| v.as_u64()) {
            Some(v) if (1..=u64::from(STYLE_VERSION)).contains(&v) => {}
            Some(v) if v > u64::from(STYLE_VERSION) => return Self::set_aside(path, &format!("style.v{v}.json")),
            _ => return Self::set_aside(path, "style.invalid.json"),
        }
        match serde_json::from_value::<Self>(value) {
            Ok(mut store) => {
                store.normalise();
                Ok((store, LoadOutcome::Loaded))
            }
            Err(_) => Self::set_aside(path, "style.invalid.json"),
        }
    }

    fn set_aside(path: &Path, name: &str) -> Result<(Self, LoadOutcome)> {
        let aside = path.with_file_name(name);
        fs::rename(path, &aside).with_context(|| format!("failed to set aside {}", path.display()))?;
        Ok((Self::default(), LoadOutcome::SetAside(aside)))
    }

    /// Atomic write, the pattern of `AppConfig::save_atomic`.
    pub fn save_atomic(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create style directory {}", parent.display()))?;
        }
        let tmp = path.with_extension("json.tmp");
        let body = serde_json::to_string_pretty(self)?;
        fs::write(&tmp, body)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Repairs what a hand-edited or damaged file can break: empty keys or values, more agreement than
    /// observations, and sizes beyond the caps.
    fn normalise(&mut self) {
        self.version = STYLE_VERSION;
        self.items.retain(|i| valid_text(&i.key) && valid_text(&i.value));
        for i in &mut self.items {
            i.agree = i.agree.min(i.seen);
        }
        self.items.truncate(MAX_ITEMS);
        self.vocabulary_candidates
            .retain(|c| valid_text(&c.from) && valid_text(&c.to) && c.count > 0);
        self.vocabulary_candidates.truncate(MAX_CANDIDATES);
        while self.chip_counts.len() > MAX_CHIPS {
            self.chip_counts.pop_last();
        }
        self.excluded_apps.truncate(MAX_EXCLUDED_APPS);
    }

    fn may_learn(&self, source: Source) -> bool {
        match source {
            Source::Edits => self.learn_from_edits == LearnState::On,
            Source::Drafts => self.learn_from_drafts == LearnState::On,
            Source::Samples => true,
        }
    }

    /// Records `n` sentences that showed `value` for habit `key` in `context`; the evidence counts for that scope and
    /// for Everywhere. Returns false when the matching learning switch is not on or the input is not usable.
    pub fn observe(&mut self, source: Source, key: &str, context: Scope, value: &str, n: u32, now: u64) -> bool {
        if n == 0 || !self.may_learn(source) || !valid_text(key) || !valid_text(value) {
            return false;
        }
        let (key, value) = (key.trim(), value.trim());
        self.observe_scope(key, context, value, n, now);
        if context != Scope::Everywhere {
            self.observe_scope(key, Scope::Everywhere, value, n, now);
        }
        true
    }

    fn observe_scope(&mut self, key: &str, scope: Scope, value: &str, n: u32, now: u64) {
        let mut before = 0;
        for i in self.items.iter_mut().filter(|i| i.key == key && i.scope == scope) {
            before = before.max(i.seen);
            i.seen = i.seen.saturating_add(n);
            if i.value == value {
                i.agree = i.agree.saturating_add(n);
                i.updated = now;
            }
        }
        if !self.items.iter().any(|i| i.key == key && i.scope == scope && i.value == value) && self.items.len() < MAX_ITEMS {
            self.items.push(StyleItem {
                key: key.to_string(),
                scope,
                value: value.to_string(),
                seen: before.saturating_add(n),
                agree: n,
                locked: false,
                updated: now,
            });
        }
    }

    /// The value of habit `key` to apply in `context`, if any. Precedence: a hand-set value in this scope, a hand-set
    /// value everywhere, a learned value in this scope, a learned value everywhere.
    pub fn applied(&self, key: &str, context: Scope) -> Option<&str> {
        let scopes: &[Scope] = if context == Scope::Everywhere { &[Scope::Everywhere] } else { &[context, Scope::Everywhere] };
        for locked in [true, false] {
            for scope in scopes {
                let hit = self.items.iter().find(|i| {
                    i.key == key && i.scope == *scope && if locked { i.locked } else { !i.locked && i.passes_threshold() }
                });
                if let Some(item) = hit {
                    return Some(&item.value);
                }
            }
        }
        None
    }

    /// Sets a habit by hand (Keep or Edit on the Style Card): this value is locked, any other value of the same
    /// (key, scope) is unlocked and keeps its evidence. Returns false for an empty or over-long key or value.
    pub fn set_locked(&mut self, key: &str, scope: Scope, value: &str, now: u64) -> bool {
        if !valid_text(key) || !valid_text(value) {
            return false;
        }
        let (key, value) = (key.trim(), value.trim());
        let mut found = false;
        for i in self.items.iter_mut().filter(|i| i.key == key && i.scope == scope) {
            i.locked = i.value == value;
            found |= i.locked;
            if i.locked {
                i.updated = now;
            }
        }
        if !found {
            self.items.push(StyleItem {
                key: key.to_string(),
                scope,
                value: value.to_string(),
                seen: 0,
                agree: 0,
                locked: true,
                updated: now,
            });
        }
        true
    }

    /// Removes one value of a habit (Remove on the Style Card). Returns whether it existed.
    pub fn remove(&mut self, key: &str, scope: Scope, value: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|i| !(i.key == key && i.scope == scope && i.value == value));
        self.items.len() != before
    }

    /// Clears everything learned; the switches and the excluded-app list stay.
    pub fn reset_learned(&mut self) {
        self.items.clear();
        self.vocabulary_candidates.clear();
        self.chip_counts.clear();
    }

    /// Counts one sighting of the correction `from` to `to` in `context` (an application category name) and returns
    /// its new count; 0 when edit learning is not on or a word is unusable. Add the pair to the dictionary and call
    /// `remove_candidate` once the count reaches [`VOCAB_PROMOTE_COUNT`]. At most 200 candidates are kept: the
    /// oldest goes first.
    pub fn record_candidate(&mut self, from: &str, to: &str, context: &str, now: u64) -> u32 {
        if !self.may_learn(Source::Edits) || !valid_text(from) || !valid_text(to) || context.chars().count() > MAX_TEXT_CHARS {
            return 0;
        }
        let (from, to) = (from.trim(), to.trim());
        if let Some(c) = self
            .vocabulary_candidates
            .iter_mut()
            .find(|c| c.from == from && c.to == to && c.context == context)
        {
            c.count = c.count.saturating_add(1);
            return c.count;
        }
        if self.vocabulary_candidates.len() >= MAX_CANDIDATES {
            if let Some(oldest) = (0..self.vocabulary_candidates.len()).min_by_key(|&i| self.vocabulary_candidates[i].first_seen) {
                self.vocabulary_candidates.remove(oldest);
            }
        }
        self.vocabulary_candidates.push(VocabCandidate {
            from: from.to_string(),
            to: to.to_string(),
            context: context.to_string(),
            count: 1,
            first_seen: now,
        });
        1
    }

    /// Drops every candidate for the pair (after promotion, or when the user dismisses it).
    pub fn remove_candidate(&mut self, from: &str, to: &str) -> bool {
        let before = self.vocabulary_candidates.len();
        self.vocabulary_candidates.retain(|c| !(c.from == from && c.to == to));
        self.vocabulary_candidates.len() != before
    }

    /// Counts one accepted or rejected suggestion under a caller-chosen key such as `fix:accepted`.
    pub fn bump_chip(&mut self, key: &str) {
        if !valid_text(key) || (!self.chip_counts.contains_key(key) && self.chip_counts.len() >= MAX_CHIPS) {
            return;
        }
        let n = self.chip_counts.entry(key.trim().to_string()).or_insert(0);
        *n = n.saturating_add(1);
    }

    pub fn is_excluded(&self, process_name: &str) -> bool {
        self.excluded_apps.iter().any(|a| a.eq_ignore_ascii_case(process_name.trim()))
    }

    pub fn exclude_app(&mut self, process_name: &str) -> bool {
        let name = process_name.trim().to_ascii_lowercase();
        if !valid_text(&name) || self.is_excluded(&name) || self.excluded_apps.len() >= MAX_EXCLUDED_APPS {
            return false;
        }
        self.excluded_apps.push(name);
        true
    }

    pub fn include_app(&mut self, process_name: &str) -> bool {
        let before = self.excluded_apps.len();
        self.excluded_apps.retain(|a| !a.eq_ignore_ascii_case(process_name.trim()));
        self.excluded_apps.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with(key: &str, scope: Scope, value: &str, seen: u32, agree: u32) -> StyleStore {
        let mut s = StyleStore::default();
        s.items.push(StyleItem { key: key.into(), scope, value: value.into(), seen, agree, ..Default::default() });
        s
    }

    #[test]
    fn learned_habit_needs_enough_sentences_and_agreement() {
        assert_eq!(store_with("dialect", Scope::Everywhere, "british", 29, 29).applied("dialect", Scope::Everywhere), None);
        assert_eq!(store_with("dialect", Scope::Everywhere, "british", 30, 30).applied("dialect", Scope::Everywhere), Some("british"));
        // 24 of 30 is exactly 80 percent; 23 of 30 is not enough
        assert_eq!(store_with("dialect", Scope::Everywhere, "british", 30, 24).applied("dialect", Scope::Everywhere), Some("british"));
        assert_eq!(store_with("dialect", Scope::Everywhere, "british", 30, 23).applied("dialect", Scope::Everywhere), None);
    }

    #[test]
    fn observe_keeps_a_share_per_value_and_only_the_majority_applies() {
        let mut s = StyleStore::default();
        for _ in 0..25 {
            assert!(s.observe(Source::Samples, "serial_comma", Scope::Everywhere, "yes", 1, 10));
        }
        for _ in 0..5 {
            s.observe(Source::Samples, "serial_comma", Scope::Everywhere, "no", 1, 11);
        }
        assert_eq!(s.items.len(), 2);
        let yes = s.items.iter().find(|i| i.value == "yes").unwrap();
        let no = s.items.iter().find(|i| i.value == "no").unwrap();
        assert_eq!((yes.seen, yes.agree, no.seen, no.agree), (30, 25, 30, 5));
        assert_eq!(s.applied("serial_comma", Scope::Everywhere), Some("yes"));
        // a later run of "no" lowers yes below the bar
        s.observe(Source::Samples, "serial_comma", Scope::Everywhere, "no", 10, 12);
        assert_eq!(s.applied("serial_comma", Scope::Everywhere), None);
        // a value first seen later starts with the total seen so far
        s.observe(Source::Samples, "serial_comma", Scope::Everywhere, "mixed", 2, 13);
        let mixed = s.items.iter().find(|i| i.value == "mixed").unwrap();
        assert_eq!((mixed.seen, mixed.agree), (42, 2));
        assert!(!s.observe(Source::Samples, "serial_comma", Scope::Everywhere, "yes", 0, 14), "n = 0 records nothing");
        assert!(!s.observe(Source::Samples, " ", Scope::Everywhere, "yes", 1, 14));
        assert!(!s.observe(Source::Samples, "k", Scope::Everywhere, &"x".repeat(65), 1, 14));
    }

    #[test]
    fn a_hand_set_habit_always_applies_and_beats_learned_ones() {
        let mut s = store_with("dialect", Scope::Everywhere, "american", 100, 100);
        assert_eq!(s.applied("dialect", Scope::Everywhere), Some("american"));
        assert!(s.set_locked("dialect", Scope::Everywhere, "british", 5));
        assert_eq!(s.applied("dialect", Scope::Everywhere), Some("british"));
        // the learned value keeps its evidence but no longer applies, even as evidence grows
        s.observe(Source::Samples, "dialect", Scope::Everywhere, "american", 500, 6);
        assert_eq!(s.applied("dialect", Scope::Everywhere), Some("british"));
        // editing the locked value moves the lock
        assert!(s.set_locked("dialect", Scope::Everywhere, "american", 7));
        assert_eq!(s.applied("dialect", Scope::Everywhere), Some("american"));
        assert_eq!(s.items.iter().filter(|i| i.locked).count(), 1);
        assert!(!s.set_locked("dialect", Scope::Everywhere, "  ", 8));
        assert!(s.remove("dialect", Scope::Everywhere, "american"));
        assert!(!s.remove("dialect", Scope::Everywhere, "american"));
    }

    #[test]
    fn scope_precedence_is_locked_before_learned_and_specific_before_general() {
        let mut s = StyleStore::default();
        s.observe(Source::Samples, "contractions", Scope::Email, "avoid", 40, 1);
        // evidence in Email also counts everywhere
        assert_eq!(s.applied("contractions", Scope::Email), Some("avoid"));
        assert_eq!(s.applied("contractions", Scope::Everywhere), Some("avoid"));
        assert_eq!(s.applied("contractions", Scope::Chat), Some("avoid"));
        // chat has its own learned habit, which wins in chat only
        s.observe(Source::Samples, "contractions", Scope::Chat, "use", 40, 2);
        assert_eq!(s.applied("contractions", Scope::Chat), Some("use"));
        assert_eq!(s.applied("contractions", Scope::Email), Some("avoid"));
        // a hand-set value everywhere beats the learned chat habit
        s.set_locked("contractions", Scope::Everywhere, "use", 3);
        s.set_locked("contractions", Scope::Everywhere, "avoid", 4);
        assert_eq!(s.applied("contractions", Scope::Chat), Some("avoid"));
        // a hand-set value in a scope beats a hand-set value everywhere
        s.set_locked("contractions", Scope::Chat, "use", 5);
        assert_eq!(s.applied("contractions", Scope::Chat), Some("use"));
        assert_eq!(s.applied("other", Scope::Chat), None);
    }

    #[test]
    fn scope_follows_the_application_category() {
        assert_eq!(Scope::for_category(AppCategory::Email), Scope::Email);
        assert_eq!(Scope::for_category(AppCategory::Chat), Scope::Chat);
        assert_eq!(Scope::for_category(AppCategory::Document), Scope::Documents);
        assert_eq!(Scope::for_category(AppCategory::Notes), Scope::Documents);
        assert_eq!(Scope::for_category(AppCategory::CodeEditor), Scope::Everywhere);
        assert_eq!(Scope::for_category(AppCategory::General), Scope::Everywhere);
    }

    #[test]
    fn learning_switches_gate_what_is_recorded() {
        let mut s = StyleStore::default();
        s.learn_from_drafts = LearnState::Paused;
        s.learn_from_edits = LearnState::Off;
        assert!(!s.observe(Source::Drafts, "k", Scope::Chat, "v", 1, 1));
        assert!(!s.observe(Source::Edits, "k", Scope::Chat, "v", 1, 1));
        assert_eq!(s.record_candidate("kubernetes", "Kubernetes", "chat", 1), 0);
        assert!(s.items.is_empty() && s.vocabulary_candidates.is_empty());
        assert!(s.observe(Source::Samples, "k", Scope::Chat, "v", 1, 1), "pasted samples are an explicit act");
        s.learn_from_drafts = LearnState::On;
        assert!(s.observe(Source::Drafts, "k", Scope::Chat, "v", 1, 1));
        s.learn_from_edits = LearnState::On;
        assert_eq!(s.record_candidate("kubernetes", "Kubernetes", "chat", 1), 1);
    }

    #[test]
    fn vocabulary_candidates_count_and_stay_bounded() {
        let mut s = StyleStore::default();
        assert_eq!(s.record_candidate("kuber netes", "Kubernetes", "chat", 10), 1);
        assert_eq!(s.record_candidate("kuber netes", "Kubernetes", "chat", 20), 2);
        assert!(2 >= VOCAB_PROMOTE_COUNT);
        assert_eq!(s.record_candidate("kuber netes", "Kubernetes", "email", 30), 1, "another context counts apart");
        assert_eq!(s.record_candidate("", "x", "chat", 1), 0);
        assert_eq!(s.record_candidate("x", &"y".repeat(65), "chat", 1), 0);
        assert!(s.remove_candidate("kuber netes", "Kubernetes"));
        assert!(!s.remove_candidate("kuber netes", "Kubernetes"));
        for n in 0..(MAX_CANDIDATES as u64 + 5) {
            s.record_candidate(&format!("w{n}"), "v", "chat", 100 + n);
        }
        assert_eq!(s.vocabulary_candidates.len(), MAX_CANDIDATES);
        assert!(!s.vocabulary_candidates.iter().any(|c| c.from == "w0"), "the oldest candidate is dropped first");
        assert!(s.vocabulary_candidates.iter().any(|c| c.from == format!("w{}", MAX_CANDIDATES + 4)));
    }

    #[test]
    fn items_chips_and_excluded_apps_are_bounded_and_validated() {
        let mut s = StyleStore::default();
        for n in 0..(MAX_ITEMS + 10) {
            s.observe(Source::Samples, &format!("k{n}"), Scope::Everywhere, "v", 1, 1);
        }
        assert_eq!(s.items.len(), MAX_ITEMS);
        for n in 0..(MAX_CHIPS + 10) {
            s.bump_chip(&format!("c{n}"));
        }
        s.bump_chip("c0");
        assert_eq!(s.chip_counts.len(), MAX_CHIPS);
        assert_eq!(s.chip_counts["c0"], 2, "an existing counter still counts when the map is full");
        s.bump_chip("");
        assert!(!s.chip_counts.contains_key(""));
        assert!(s.exclude_app(" KeePass.exe "));
        assert!(!s.exclude_app("keepass.EXE"), "already excluded");
        assert!(s.is_excluded("KEEPASS.exe"));
        assert!(!s.is_excluded("notepad.exe"));
        assert!(s.include_app("keepass.exe"));
        assert!(!s.include_app("keepass.exe"));
        assert!(!s.exclude_app("  "));
    }

    #[test]
    fn reset_clears_what_was_learned_and_keeps_the_settings() {
        let mut s = StyleStore::default();
        s.learn_from_edits = LearnState::Paused;
        s.exclude_app("keepass.exe");
        s.observe(Source::Samples, "k", Scope::Chat, "v", 3, 1);
        s.learn_from_drafts = LearnState::On;
        s.record_candidate("a", "b", "chat", 1); // refused: edits are paused
        s.bump_chip("fix:accepted");
        s.reset_learned();
        assert!(s.items.is_empty() && s.vocabulary_candidates.is_empty() && s.chip_counts.is_empty());
        assert_eq!(s.learn_from_edits, LearnState::Paused);
        assert_eq!(s.excluded_apps, vec!["keepass.exe".to_string()]);
    }

    #[test]
    fn saves_atomically_and_loads_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("style.json");
        assert_eq!(StyleStore::load(&path).unwrap().1, LoadOutcome::Fresh);
        let mut s = StyleStore::default();
        s.observe(Source::Samples, "dialect", Scope::Email, "british", 31, 99);
        s.set_locked("dash_style", Scope::Everywhere, "spaced en dash", 100);
        s.record_candidate("kuber netes", "Kubernetes", "chat", 101);
        s.bump_chip("fix:accepted");
        s.exclude_app("keepass.exe");
        s.learn_from_edits = LearnState::Paused;
        s.save_atomic(&path).unwrap();
        assert!(!path.with_extension("json.tmp").exists());
        let (loaded, outcome) = StyleStore::load(&path).unwrap();
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert_eq!(loaded, s);
        let raw: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(raw["version"], 1);
        assert_eq!(raw["learn_from_edits"], "paused");
        assert_eq!(raw["items"][0]["scope"], "email");
    }

    #[test]
    fn a_corrupt_file_is_set_aside_and_a_fresh_store_started() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("style.json");
        for (i, body) in [&b"{ not json"[..], b"[1, 2]", b"\xff\xfe\x00garbage", br#"{"version": 0}"#, br#"{"version": 1, "items": 7}"#]
            .into_iter()
            .enumerate()
        {
            fs::write(&path, body).unwrap();
            let (s, outcome) = StyleStore::load(&path).unwrap();
            assert_eq!(s, StyleStore::default(), "case {i}");
            let aside = dir.path().join("style.invalid.json");
            assert_eq!(outcome, LoadOutcome::SetAside(aside.clone()), "case {i}");
            assert_eq!(fs::read(&aside).unwrap(), body, "the original is kept byte for byte");
            assert!(!path.exists());
        }
    }

    #[test]
    fn a_newer_file_is_set_aside_not_overwritten_or_misread() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("style.json");
        let body = br#"{"version": 2, "items": "a different shape"}"#;
        fs::write(&path, body).unwrap();
        let (s, outcome) = StyleStore::load(&path).unwrap();
        assert_eq!(s, StyleStore::default());
        let aside = dir.path().join("style.v2.json");
        assert_eq!(outcome, LoadOutcome::SetAside(aside.clone()));
        assert_eq!(fs::read(aside).unwrap(), body);
    }

    #[test]
    fn missing_fields_default_unknown_fields_are_ignored_and_damage_is_repaired() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("style.json");
        fs::write(&path, r#"{"learn_in_browsers": true, "future": 1,
            "items": [{"key": "dialect", "value": "british", "seen": 5, "agree": 9},
                      {"key": "", "value": "x"}, {"key": "k", "value": "  "}],
            "vocabulary_candidates": [{"from": "a", "to": "b", "count": 0}, {"from": "c", "to": "d", "count": 2}]}"#)
            .unwrap();
        let (s, outcome) = StyleStore::load(&path).unwrap();
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert_eq!(s.version, STYLE_VERSION);
        assert!(s.learn_in_browsers);
        assert_eq!((s.learn_from_edits, s.learn_from_drafts), (LearnState::On, LearnState::On));
        assert_eq!(s.items.len(), 1);
        assert_eq!((s.items[0].seen, s.items[0].agree, s.items[0].scope), (5, 5, Scope::Everywhere));
        assert_eq!(s.vocabulary_candidates.len(), 1);
        assert_eq!(s.vocabulary_candidates[0].from, "c");
    }
}
