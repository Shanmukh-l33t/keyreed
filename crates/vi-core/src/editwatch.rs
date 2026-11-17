//! Edit-watcher logic (plan P4.2, the pure part): which targets may be watched, when to read, where the inserted text
//! is in the field, what the user changed, and what is worth keeping. No Windows calls, no clock, no I/O: a state
//! machine driven by explicit events and millisecond timestamps, and functions over plain strings, so every rule is
//! unit-tested. The thread that reads the field (P4.1) and the wiring (lane A) sit on top of this.
//!
//! Privacy: field text is only ever a function argument. Nothing here stores, logs or returns it; what leaves is
//! word pairs (vocabulary) and (habit, value) counters.
//!
//! Positions are counted in `char`s of the field string as passed in; the caller converts UI Automation's UTF-16
//! offsets.

use crate::analysers::analyse;
use crate::dictionary::PersonalDictionary;
use crate::style::{LearnState, Scope, Source, StyleStore, VOCAB_PROMOTE_COUNT};
use crate::types::AppCategory;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

// ---------------------------------------------------------------- target gate

/// Fields longer than this are not watched.
pub const MAX_FIELD_CHARS: usize = 8_000;

const TERMINALS: &[&str] = &[
    "windowsterminal.exe", "wt.exe", "cmd.exe", "powershell.exe", "pwsh.exe", "conhost.exe", "alacritty.exe",
    "wezterm-gui.exe", "kitty.exe", "mintty.exe", "putty.exe", "mobaxterm.exe",
];
const TERMINAL_CLASSES: &[&str] = &["consolewindowclass", "cascadia_hosting_window_class", "putty", "mintty"];
const PASSWORD_MANAGERS: &[&str] =
    &["keepass.exe", "keepassxc.exe", "1password.exe", "bitwarden.exe", "dashlane.exe", "nordpass.exe"];
/// Browsers and Chromium-based apps (Electron and WebView hosts). Window classes catch the rest.
const BROWSER_PROCESSES: &[&str] = &[
    "chrome.exe", "msedge.exe", "firefox.exe", "brave.exe", "vivaldi.exe", "arc.exe", "opera.exe", "slack.exe", "teams.exe",
    "ms-teams.exe", "olk.exe", "code.exe", "cursor.exe", "windsurf.exe", "discord.exe", "whatsapp.exe", "signal.exe",
    "notion.exe", "obsidian.exe", "mattermost.exe", "element.exe", "msedgewebview2.exe",
];
const BROWSER_CLASS_PREFIXES: &[&str] = &["chrome_", "mozillawindowclass", "mozillauiwindowclass"];

/// What is being typed into, as reported when KeyReed finished inserting.
#[derive(Debug, Clone, Default)]
pub struct Target {
    pub process_name: String,
    pub window_class: String,
    pub category: AppCategory,
    pub is_password_field: bool,
    pub is_private_browsing: bool,
    pub is_elevated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    Native,
    /// A browser or Chromium-based app; only watched when the browser switch is on.
    Browser,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deny {
    LearningOff,
    PasswordField,
    PasswordManager,
    PrivateBrowsing,
    Elevated,
    Terminal,
    Excluded,
    Browser,
    /// Reading this application was too slow twice earlier in the session.
    SlowThisSession,
}

/// Applications that proved too slow to read during this session.
#[derive(Debug, Default)]
pub struct SessionBans(HashSet<String>);

impl SessionBans {
    pub fn ban(&mut self, process_name: &str) {
        self.0.insert(process_name.trim().to_ascii_lowercase());
    }

    pub fn is_banned(&self, process_name: &str) -> bool {
        self.0.contains(&process_name.trim().to_ascii_lowercase())
    }
}

pub fn is_browser_like(process_name: &str, window_class: &str) -> bool {
    let (p, c) = (process_name.trim().to_ascii_lowercase(), window_class.trim().to_ascii_lowercase());
    BROWSER_PROCESSES.contains(&p.as_str()) || BROWSER_CLASS_PREFIXES.iter().any(|x| c.starts_with(x))
}

/// Decides, before anything is read, whether a field may be watched. The learning switch comes first, so with
/// learning off no other check (and no read) happens. `light_profile` keeps browsers off whatever the switch says.
pub fn gate(target: &Target, store: &StyleStore, light_profile: bool, bans: &SessionBans) -> Result<TargetKind, Deny> {
    if store.learn_from_edits != LearnState::On {
        return Err(Deny::LearningOff);
    }
    let proc = target.process_name.trim().to_ascii_lowercase();
    let class = target.window_class.trim().to_ascii_lowercase();
    if target.is_password_field || target.category == AppCategory::PasswordProtected {
        return Err(Deny::PasswordField);
    }
    if PASSWORD_MANAGERS.contains(&proc.as_str()) {
        return Err(Deny::PasswordManager);
    }
    if target.is_private_browsing {
        return Err(Deny::PrivateBrowsing);
    }
    if target.is_elevated {
        return Err(Deny::Elevated);
    }
    if target.category == AppCategory::Terminal || TERMINALS.contains(&proc.as_str()) || TERMINAL_CLASSES.contains(&class.as_str()) {
        return Err(Deny::Terminal);
    }
    if store.is_excluded(&proc) {
        return Err(Deny::Excluded);
    }
    if bans.is_banned(&proc) {
        return Err(Deny::SlowThisSession);
    }
    if is_browser_like(&proc, &class) {
        return if store.learn_in_browsers && !light_profile { Ok(TargetKind::Browser) } else { Err(Deny::Browser) };
    }
    Ok(TargetKind::Native)
}

pub fn field_length_ok(chars: usize) -> bool {
    chars <= MAX_FIELD_CHARS
}

// ------------------------------------------------------------ schedule state machine

pub const SETTLE_MS: u64 = 700;
pub const POLL_MS: u64 = 2_000;
pub const MAX_WATCH_MS: u64 = 60_000;
pub const SLOW_READ_MS: u64 = 30;
const MAX_SLOW_READS: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The field sends text-changed notifications: read once, `SETTLE_MS` after the last one.
    Notifications,
    /// No notifications (or no subscription): read every `POLL_MS` while the field keeps focus.
    Polling,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    MaxDuration,
    FocusLost,
    /// A new insertion or another KeyReed action started.
    NextAction,
    /// A read failed: the field is gone. The last successful read stands.
    FieldGone,
    /// Two reads took longer than `SLOW_READ_MS`: stop, and the caller bans the application for the session.
    TooSlow,
    FieldTooLong,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Nothing to do before `until` (milliseconds on the caller's clock), unless an event arrives.
    Wait { until: u64 },
    /// Read the field now and report with `read_done`.
    Read,
    /// The watch is over. `unread_change`: a change notification arrived that no read has covered.
    Stop { reason: EndReason, unread_change: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadResult {
    Ok { chars: usize },
    /// The field could not be read (closed, replaced, access denied).
    Gone,
}

#[derive(Debug)]
pub struct Watch {
    start: u64,
    mode: Mode,
    mult: u64,
    slow_reads: u32,
    last_change: Option<u64>,
    dirty: bool,
    next_poll: u64,
    read_started: Option<u64>,
    ended: Option<EndReason>,
}

impl Watch {
    pub fn new(now: u64, mode: Mode) -> Self {
        Self {
            start: now,
            mode,
            mult: 1,
            slow_reads: 0,
            last_change: None,
            dirty: false,
            next_poll: now + POLL_MS,
            read_started: None,
            ended: None,
        }
    }

    /// A text-changed notification arrived. Restarts the settle time.
    pub fn text_changed(&mut self, now: u64) {
        self.last_change = Some(now);
        self.dirty = true;
    }

    /// What to do at `now`. Call after every event and whenever a wait runs out.
    pub fn due(&mut self, now: u64) -> Step {
        if let Some(reason) = self.ended {
            return self.stop(reason);
        }
        if now.saturating_sub(self.start) >= MAX_WATCH_MS {
            return self.end(EndReason::MaxDuration);
        }
        let deadline = self.start + MAX_WATCH_MS;
        let ready_at = match self.mode {
            Mode::Notifications => match (self.dirty, self.last_change) {
                (true, Some(t)) => t + SETTLE_MS * self.mult,
                _ => return Step::Wait { until: deadline },
            },
            Mode::Polling => self.next_poll,
        };
        if now >= ready_at {
            self.read_started = Some(now);
            Step::Read
        } else {
            Step::Wait { until: ready_at.min(deadline) }
        }
    }

    /// Reports a finished read: how long it took and what it found. Applies the throttling rule: a read over
    /// `SLOW_READ_MS` doubles the interval, a second one ends the watch.
    pub fn read_done(&mut self, now: u64, took_ms: u64, result: ReadResult) {
        if self.ended.is_some() {
            return;
        }
        // A change that arrived while the read was running is not covered by it.
        if self.last_change.zip(self.read_started).map_or(true, |(c, s)| c <= s) {
            self.dirty = false;
        }
        self.read_started = None;
        match result {
            ReadResult::Gone => self.ended = Some(EndReason::FieldGone),
            ReadResult::Ok { chars } if !field_length_ok(chars) => self.ended = Some(EndReason::FieldTooLong),
            ReadResult::Ok { .. } => {}
        }
        if took_ms > SLOW_READ_MS {
            self.slow_reads += 1;
            self.mult *= 2;
            if self.slow_reads >= MAX_SLOW_READS && self.ended.is_none() {
                self.ended = Some(EndReason::TooSlow);
            }
        }
        self.next_poll = now + POLL_MS * self.mult;
    }

    /// The field lost focus.
    pub fn focus_lost(&mut self) -> Step {
        self.end(EndReason::FocusLost)
    }

    /// A new insertion or another KeyReed action starts.
    pub fn next_action(&mut self) -> Step {
        self.end(EndReason::NextAction)
    }

    pub fn ended(&self) -> Option<EndReason> {
        self.ended
    }

    fn end(&mut self, reason: EndReason) -> Step {
        let reason = *self.ended.get_or_insert(reason);
        self.stop(reason)
    }

    fn stop(&self, reason: EndReason) -> Step {
        Step::Stop { reason, unread_change: self.dirty }
    }
}

// ------------------------------------------------------------------------ locate

/// What KeyReed inserted and where it went, recorded when the insertion finished.
#[derive(Debug, Clone)]
pub struct Insertion {
    pub inserted: String,
    /// Char index where the inserted text starts, from the caret after insertion minus its length.
    pub start: Option<usize>,
    /// Chars that followed the inserted text in the field.
    pub suffix_len: Option<usize>,
}

/// At least this share of the inserted words must still be in the located region.
const MIN_REGION_OVERLAP_PERCENT: usize = 50;

fn word_counts(s: &str) -> HashMap<String, usize> {
    let mut m = HashMap::new();
    for t in tokens(s).iter().filter(|t| t.is_word) {
        *m.entry(t.text.to_lowercase()).or_insert(0) += 1;
    }
    m
}

fn byte_of(s: &str, char_idx: usize) -> usize {
    s.char_indices().nth(char_idx).map_or(s.len(), |(b, _)| b)
}

/// Finds the part of `field` that is KeyReed's text, as the user left it. First by position (where the text
/// started and how much follows it, so edits inside it are found), then by a unique exact match of the inserted
/// text. `None` when neither works: the text was deleted or rewritten, and nothing is learned.
pub fn locate<'a>(field: &'a str, ins: &Insertion) -> Option<&'a str> {
    if ins.inserted.trim().is_empty() {
        return None;
    }
    if let (Some(start), Some(suffix)) = (ins.start, ins.suffix_len) {
        let n = field.chars().count();
        if let Some(end) = n.checked_sub(suffix).filter(|&e| e >= start) {
            let region = &field[byte_of(field, start)..byte_of(field, end)];
            let inserted_chars = ins.inserted.chars().count();
            let small_enough = region.chars().count() <= inserted_chars * 4 + 400;
            let (want, have) = (word_counts(&ins.inserted), word_counts(region));
            let total: usize = want.values().sum();
            let kept: usize = want.iter().map(|(w, n)| (*n).min(have.get(w).copied().unwrap_or(0))).sum();
            if small_enough && total > 0 && kept * 100 >= total * MIN_REGION_OVERLAP_PERCENT {
                return Some(region);
            }
        }
    }
    let mut hits = field.match_indices(ins.inserted.as_str());
    match (hits.next(), hits.next()) {
        (Some((i, m)), None) => Some(&field[i..i + m.len()]),
        _ => None,
    }
}

// -------------------------------------------------------------------- tokens, diff

const PEEL: &str = ".,;:!?()[]{}\"'\u{201c}\u{201d}\u{2018}\u{2019}\u{2026}";

#[derive(Debug, Clone, PartialEq, Eq)]
struct Tok<'a> {
    text: &'a str,
    /// Has a letter or digit (a word, a name, a URL, `v2.0`); punctuation marks are not words.
    is_word: bool,
    /// First token of the text, of a line, or after `.`, `!` or `?`.
    sentence_start: bool,
}

/// Splits on whitespace and peels the punctuation around each chunk into its own token; anything glued together
/// inside (`don't`, `a/b`, `example.com`) stays one token.
fn tokens(s: &str) -> Vec<Tok<'_>> {
    let mut out: Vec<Tok> = Vec::new();
    let mut at_start = true;
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        let ws = rest.find(|c: char| !c.is_whitespace()).unwrap_or(rest.len());
        if rest[..ws].contains('\n') {
            at_start = true;
        }
        i += ws;
        if i >= s.len() {
            break;
        }
        let end = s[i..].find(char::is_whitespace).map_or(s.len(), |n| i + n);
        let chunk = &s[i..end];
        let cs = chunk.find(|c: char| !PEEL.contains(c)).unwrap_or(chunk.len());
        let ce = chunk.rfind(|c: char| !PEEL.contains(c)).map_or(cs, |p| p + chunk[p..].chars().next().map_or(1, char::len_utf8));
        // Byte ranges in `s`: each leading mark, the core, each trailing mark.
        let mut spans: Vec<(usize, usize)> = chunk[..cs].char_indices().map(|(o, c)| (i + o, i + o + c.len_utf8())).collect();
        if ce > cs {
            spans.push((i + cs, i + ce));
        }
        spans.extend(chunk[ce..].char_indices().map(|(o, c)| (i + ce + o, i + ce + o + c.len_utf8())));
        for (a, b) in spans {
            let text = &s[a..b];
            let is_word = text.chars().any(char::is_alphanumeric);
            out.push(Tok { text, is_word, sentence_start: at_start });
            if is_word {
                at_start = false;
            } else if matches!(text, "." | "!" | "?") {
                at_start = true;
            }
        }
        i = end;
    }
    out
}

/// A changed stretch: token ranges in the old and the new text.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Hunk {
    old: std::ops::Range<usize>,
    new: std::ops::Range<usize>,
}

const DIFF_CELL_LIMIT: usize = 4_000_000;

fn diff_hunks(a: &[Tok], b: &[Tok]) -> Option<Vec<Hunk>> {
    let pre = a.iter().zip(b).take_while(|(x, y)| x.text == y.text).count();
    let suf = a[pre..].iter().rev().zip(b[pre..].iter().rev()).take_while(|(x, y)| x.text == y.text).count();
    let (am, bm) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);
    if am.len().saturating_mul(bm.len()) > DIFF_CELL_LIMIT {
        return None;
    }
    let (n, m) = (am.len(), bm.len());
    let mut t = vec![0u32; (n + 1) * (m + 1)];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            t[i * (m + 1) + j] = if am[i].text == bm[j].text {
                t[(i + 1) * (m + 1) + j + 1] + 1
            } else {
                t[(i + 1) * (m + 1) + j].max(t[i * (m + 1) + j + 1])
            };
        }
    }
    let mut hunks: Vec<Hunk> = Vec::new();
    let (mut i, mut j) = (0, 0);
    let mut open: Option<(usize, usize)> = None;
    let close = |open: &mut Option<(usize, usize)>, i: usize, j: usize, hunks: &mut Vec<Hunk>| {
        if let Some((oi, oj)) = open.take() {
            hunks.push(Hunk { old: pre + oi..pre + i, new: pre + oj..pre + j });
        }
    };
    while i < n || j < m {
        if i < n && j < m && am[i].text == bm[j].text {
            close(&mut open, i, j, &mut hunks);
            i += 1;
            j += 1;
            continue;
        }
        open.get_or_insert((i, j));
        if j < m && (i == n || t[i * (m + 1) + j + 1] >= t[(i + 1) * (m + 1) + j]) {
            j += 1;
        } else {
            i += 1;
        }
    }
    close(&mut open, i, j, &mut hunks);
    Some(hunks)
}

// ----------------------------------------------------------------------- classify

/// Share of the original words that may change before the edit is treated as a rewrite and ignored.
const MAX_CHANGED_PERCENT: usize = 40;
const MAX_VOCAB_PER_EDIT: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ignore {
    LearningOff,
    NothingChanged,
    NoWords,
    /// More than 40 percent of the words changed, or the text is too large to compare.
    Rewritten,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Classification {
    /// (what KeyReed wrote, what the user made it): candidates for the dictionary.
    pub vocabulary: Vec<(String, String)>,
    /// (habit key, value) evidence, one sighting each, for `StyleStore::observe`.
    pub habits: Vec<(&'static str, String)>,
}

const COMMON_WORDS: &str = "a about above across act add after again against age ago agree air all allow almost alone along already also \
although always am among an and angry another answer any anyone anything anyway appear are area around as ask at away back bad \
bag base be bear beat because become bed been before begin behind being believe best better between big bit black blue board \
body book born both box boy bring brother build busy but buy by bye call came can cannot car care carry case catch cause \
change check child children city class clean clear close cold come company complete could country course cover cut dark \
day dear deal decide deep did die do does doing done door down draw dream drive drop during each early earth easy eat \
effect else end enough even evening ever every example except eye face fact fall family far fast father feel few field \
fill find fine finish fire first five floor follow food foot for force form found four free friend from front full game \
gave get girl give go going gone good got great green ground group grow had half hand happen happy hard has have he head \
hear heard heart held help her here high him his hold hole home hope hot hour house how however hundred i idea if in \
inside into is it its job just keep kept kind knew know land large last late later laugh learn least leave left less let \
letter life light like line list little live long look lost lot love low made main make man many may maybe me mean meet \
men might mind miss money month more morning most mother move much must my name near need never new next nice night no \
none nor not note nothing now number of off office often old on once one only open or order other our out over own page \
paper part pass past pay people person place plan plant play please point power present press pretty problem public pull \
put question quick quite rain reach read ready real really reason red remember rest right river road room round run \
said same saw say school sea second see seem seen sell send sense sent set seven several shall she ship short should show \
side since sit six sleep small so some someone something sometimes soon sorry sound speak stand start state stay step \
still stop story street strong study such sun sure table take talk tell ten than thank thanks that the their them then \
there these they thing think third this those though thought three through time to today together told too took top \
toward town tree true try turn two under until up upon us use usually very wait walk want warm was watch water way we \
wear weather week well went were what when where whether which while white who whole whom whose why wide will wind \
window wish with without woman women word work world would write wrong year yes yet you young your accept affect \
buy right write rite wright weather whether witch which their there theyre your youre its its to too two then than \
sea see here hear new knew know no one won for four fore pair pear peace piece plain plane sail sale tail tale \
wait weight way weigh whole hole write wood would";

fn common_words() -> &'static HashSet<&'static str> {
    static S: OnceLock<HashSet<&'static str>> = OnceLock::new();
    S.get_or_init(|| COMMON_WORDS.split_whitespace().collect())
}

pub fn is_common_word(w: &str) -> bool {
    common_words().contains(w.to_lowercase().as_str())
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            cur.push((prev[j] + usize::from(ca != cb)).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// A rough sound key: the first letter, then consonants only with similar ones merged and repeats removed.
fn sound_key(w: &str) -> String {
    let mut out = String::new();
    for (i, c) in w.to_lowercase().chars().filter(|c| c.is_alphabetic()).enumerate() {
        let c = match c {
            'c' | 'q' => 'k',
            'z' => 's',
            'x' => 'k',
            other => other,
        };
        if i > 0 && "aeiouyhw".contains(c) {
            continue;
        }
        if !out.ends_with(c) {
            out.push(c);
        }
    }
    out
}

fn similar_looking(a: &str, b: &str) -> bool {
    let (la, lb): (Vec<char>, Vec<char>) = (a.to_lowercase().chars().collect(), b.to_lowercase().chars().collect());
    let d = levenshtein(&la, &lb);
    d >= 1 && (d <= la.len().max(lb.len()) / 2 || (la.len().min(lb.len()) >= 4 && sound_key(a) == sound_key(b)))
}

fn vocabulary_word(w: &str) -> bool {
    let n = w.chars().count();
    (3..=32).contains(&n)
        && w.chars().next().is_some_and(char::is_alphabetic)
        && w.chars().all(|c| c.is_alphanumeric() || c == '-')
}

fn strip_non_alnum(w: &str) -> String {
    w.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase()
}

const GREETINGS: &[&str] = &["good morning", "good afternoon", "good evening", "greetings", "hello", "hi", "hey", "dear"];
const SIGN_OFFS: &[&str] = &[
    "best regards", "kind regards", "warm regards", "with regards", "regards", "thanks and regards", "thank you", "thanks",
    "many thanks", "cheers", "sincerely", "best wishes", "best", "yours sincerely", "yours truly",
];

fn greeting_of(s: &str) -> Option<&'static str> {
    let l = s.trim_start().to_lowercase();
    GREETINGS.iter().copied().find(|g| l.strip_prefix(g).is_some_and(|r| !r.starts_with(|c: char| c.is_alphanumeric())))
}

fn sign_off_of(s: &str) -> Option<&'static str> {
    s.lines()
        .rev()
        .filter(|l| !l.trim().is_empty())
        .take(2)
        .find_map(|l| {
            let l = l.trim().trim_end_matches(|c: char| ",.!;:".contains(c)).to_lowercase();
            SIGN_OFFS.iter().copied().find(|s| *s == l)
        })
}

fn single<'a>(obs: &'a [crate::analysers::Observation], key: &str) -> Option<&'a str> {
    let mut it = obs.iter().filter(|o| o.key == key);
    match (it.next(), it.next()) {
        (Some(o), None) => Some(o.value),
        _ => None,
    }
}

/// Compares KeyReed's text with what the user left and sorts the differences. Pure: no store, no clock.
pub fn classify(inserted: &str, region: &str) -> Result<Classification, Ignore> {
    let (a, b) = (tokens(inserted), tokens(region));
    let old_words = a.iter().filter(|t| t.is_word).count();
    if old_words == 0 {
        return Err(Ignore::NoWords);
    }
    if a.iter().map(|t| t.text).eq(b.iter().map(|t| t.text)) {
        return Err(Ignore::NothingChanged);
    }
    let hunks = diff_hunks(&a, &b).ok_or(Ignore::Rewritten)?;
    let words = |ts: &[Tok], r: &std::ops::Range<usize>| ts[r.clone()].iter().filter(|t| t.is_word).count();
    // A replacement counts as its shorter side, but never less than half the longer one, so `do not` -> `don't`
    // is one changed word and a one-word-for-a-sentence swap is not free.
    let cost = |h: &Hunk| {
        let (o, n) = (words(&a, &h.old), words(&b, &h.new));
        if o.min(n) == 0 { o.max(n) } else { o.min(n).max(o.max(n) / 2) }
    };
    let changed: usize = hunks.iter().map(cost).sum();
    if changed * 100 > old_words * MAX_CHANGED_PERCENT {
        return Err(Ignore::Rewritten);
    }

    let mut c = Classification::default();
    for h in &hunks {
        let (ow, nw): (Vec<&Tok>, Vec<&Tok>) =
            (a[h.old.clone()].iter().filter(|t| t.is_word).collect(), b[h.new.clone()].iter().filter(|t| t.is_word).collect());
        let (old_punct, new_punct) = (h.old.len() - ow.len(), h.new.len() - nw.len());
        if ow.len() != 1 || nw.len() != 1 || old_punct + new_punct != 0 {
            continue;
        }
        let (old, new) = (ow[0], nw[0]);
        let (o, n) = (old.text, new.text);
        let case_only = o != n && o.to_lowercase() == n.to_lowercase();
        let first_upper = |s: &str| s.chars().next().is_some_and(char::is_uppercase);
        if case_only {
            if (old.sentence_start || is_common_word(o)) && first_upper(o) != first_upper(n) {
                let v = if first_upper(n) { "yes" } else { "no" };
                c.habits.push(("sentence_capital", v.to_string()));
            } else if !is_common_word(o) && !old.sentence_start && vocabulary_word(o) && vocabulary_word(n) {
                c.vocabulary.push((o.to_string(), n.to_string()));
            }
            continue;
        }
        if !vocabulary_word(o) || !vocabulary_word(n) || (is_common_word(o) && is_common_word(n)) {
            continue;
        }
        // Spelling out a contraction or dropping its apostrophe is a typo or a style change, not a name.
        if strip_non_alnum(o) == strip_non_alnum(n) || !similar_looking(o, n) {
            continue;
        }
        c.vocabulary.push((o.to_string(), n.to_string()));
    }
    c.vocabulary.truncate(MAX_VOCAB_PER_EDIT);

    // Habits that the edit flipped: the same analysers the drafts use, run on both versions.
    let (oa, na) = (analyse(inserted), analyse(region));
    for key in ["dialect", "contractions", "serial_comma", "dash_style", "final_full_stop", "emoji"] {
        if let (Some(ov), Some(nv)) = (single(&oa, key), single(&na, key)) {
            if ov != nv {
                c.habits.push((key_static(key), nv.to_string()));
            }
        }
    }
    if let (Some(og), Some(ng)) = (greeting_of(inserted), greeting_of(region)) {
        if og != ng {
            c.habits.push(("greeting", ng.to_string()));
        }
    }
    if let (Some(os), Some(ns)) = (sign_off_of(inserted), sign_off_of(region)) {
        if os != ns {
            c.habits.push(("sign_off", ns.to_string()));
        }
    }
    Ok(c)
}

fn key_static(key: &str) -> &'static str {
    ["dialect", "contractions", "serial_comma", "dash_style", "final_full_stop", "emoji"]
        .into_iter()
        .find(|k| *k == key)
        .unwrap_or("dialect")
}

// ------------------------------------------------------- the rule on the store and the dictionary

/// Everything needed to take back one promotion: only what this promotion added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoToken {
    pub from: String,
    pub to: String,
    added_term: bool,
    /// The term that was there before, differing only in case, to put back.
    previous_term: Option<String>,
    added_rule: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VocabOutcome {
    /// Seen once (or not yet twice): waits in the review list.
    Review { from: String, to: String, count: u32 },
    /// Seen twice: now in the dictionary. Announce it and offer Undo.
    Promoted(UndoToken),
    /// The dictionary already has it.
    AlreadyKnown,
    /// Learning is off or the words cannot be kept.
    Dropped,
}

fn seen_total(store: &StyleStore, from: &str, to: &str) -> u32 {
    store.vocabulary_candidates.iter().filter(|c| c.from == from && c.to == to).map(|c| c.count).sum()
}

/// A correction seen twice goes to the dictionary (with an undo token); seen once it goes to the review list. The
/// sightings are counted across contexts. The new word always becomes a dictionary term (speech-recognition hint
/// and glossary); the old word becomes a replacement rule only when it is not an everyday word and the dictionary
/// has no rule for it yet, so a learned rule can never rewrite ordinary text or override one the user made.
pub fn learn_vocabulary(
    store: &mut StyleStore,
    dict: &mut PersonalDictionary,
    from: &str,
    to: &str,
    context: &str,
    now: u64,
) -> VocabOutcome {
    let (from, to) = (from.trim(), to.trim());
    let rule_known = dict.replacements.iter().any(|r| r.spoken.eq_ignore_ascii_case(from));
    if dict.vocabulary.iter().any(|v| v == to) && (rule_known || is_common_word(from)) {
        return VocabOutcome::AlreadyKnown;
    }
    if store.record_candidate(from, to, context, now) == 0 {
        return VocabOutcome::Dropped;
    }
    let count = seen_total(store, from, to);
    if count < VOCAB_PROMOTE_COUNT {
        return VocabOutcome::Review { from: from.to_string(), to: to.to_string(), count };
    }
    let existing = dict.vocabulary.iter().find(|v| v.eq_ignore_ascii_case(to)).cloned();
    let added_term = existing.is_none();
    let previous_term = existing.filter(|e| e != to);
    if previous_term.is_some() {
        dict.remove_term(to);
    }
    if added_term || previous_term.is_some() {
        dict.add_term(to);
    }
    let added_rule = !is_common_word(from) && !rule_known;
    if added_rule {
        dict.upsert_replacement(from, to, false);
    }
    store.remove_candidate(from, to);
    VocabOutcome::Promoted(UndoToken {
        from: from.to_string(),
        to: to.to_string(),
        added_term: added_term || previous_term.is_some(),
        previous_term,
        added_rule,
    })
}

/// Takes back one promotion, removing only what it added.
pub fn undo_learned(dict: &mut PersonalDictionary, token: &UndoToken) {
    if token.added_term {
        dict.remove_term(&token.to);
        if let Some(prev) = &token.previous_term {
            dict.add_term(prev.clone());
        }
    }
    if token.added_rule {
        dict.replacements.retain(|r| !(r.spoken.eq_ignore_ascii_case(&token.from) && r.written == token.to));
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditReport {
    pub ignored: Option<Ignore>,
    pub vocabulary: Vec<VocabOutcome>,
    pub habits_recorded: usize,
}

/// The whole step once a watch has a final read: locate has produced `region`; classify it and record the
/// evidence. The texts are dropped when this returns.
pub fn learn_from_edit(
    store: &mut StyleStore,
    dict: &mut PersonalDictionary,
    inserted: &str,
    region: &str,
    context: &str,
    scope: Scope,
    now: u64,
) -> EditReport {
    let mut report = EditReport::default();
    if store.learn_from_edits != LearnState::On {
        report.ignored = Some(Ignore::LearningOff);
        return report;
    }
    match classify(inserted, region) {
        Err(why) => report.ignored = Some(why),
        Ok(c) => {
            for (from, to) in &c.vocabulary {
                report.vocabulary.push(learn_vocabulary(store, dict, from, to, context, now));
            }
            report.habits_recorded = c.habits.iter().filter(|(k, v)| store.observe(Source::Edits, k, scope, v, 1, now)).count();
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(proc_: &str, class: &str) -> Target {
        Target { process_name: proc_.into(), window_class: class.into(), ..Target::default() }
    }

    fn open() -> (StyleStore, SessionBans) {
        (StyleStore::default(), SessionBans::default())
    }

    // ---- gate

    #[test]
    fn gate_allows_native_apps_and_applies_every_deny_rule() {
        let (mut store, bans) = open();
        assert_eq!(gate(&target("notepad.exe", "Notepad"), &store, false, &bans), Ok(TargetKind::Native));
        assert_eq!(gate(&target("WINWORD.EXE", "OpusApp"), &store, false, &bans), Ok(TargetKind::Native));

        let mut t = target("notepad.exe", "Edit");
        t.is_password_field = true;
        assert_eq!(gate(&t, &store, false, &bans), Err(Deny::PasswordField));
        let mut t = target("notepad.exe", "Edit");
        t.category = AppCategory::PasswordProtected;
        assert_eq!(gate(&t, &store, false, &bans), Err(Deny::PasswordField));
        assert_eq!(gate(&target("KeePassXC.exe", "Qt5QWindowIcon"), &store, false, &bans), Err(Deny::PasswordManager));
        let mut t = target("msedge.exe", "Chrome_WidgetWin_1");
        t.is_private_browsing = true;
        assert_eq!(gate(&t, &store, false, &bans), Err(Deny::PrivateBrowsing));
        let mut t = target("notepad.exe", "Edit");
        t.is_elevated = true;
        assert_eq!(gate(&t, &store, false, &bans), Err(Deny::Elevated));
        assert_eq!(gate(&target("WindowsTerminal.exe", "CASCADIA_HOSTING_WINDOW_CLASS"), &store, false, &bans), Err(Deny::Terminal));
        assert_eq!(gate(&target("whatever.exe", "ConsoleWindowClass"), &store, false, &bans), Err(Deny::Terminal));
        let mut t = target("whatever.exe", "Edit");
        t.category = AppCategory::Terminal;
        assert_eq!(gate(&t, &store, false, &bans), Err(Deny::Terminal));

        // The user's exclusion list, case-insensitive.
        assert!(store.exclude_app("Notepad.exe"));
        assert_eq!(gate(&target("notepad.exe", "Notepad"), &store, false, &bans), Err(Deny::Excluded));
    }

    #[test]
    fn gate_checks_the_learning_switch_first() {
        let (mut store, bans) = open();
        for state in [LearnState::Off, LearnState::Paused] {
            store.learn_from_edits = state;
            // Even a password field reports the switch: nothing else is looked at.
            let mut t = target("notepad.exe", "Edit");
            t.is_password_field = true;
            assert_eq!(gate(&t, &store, false, &bans), Err(Deny::LearningOff));
            assert_eq!(gate(&target("notepad.exe", "Notepad"), &store, false, &bans), Err(Deny::LearningOff));
        }
    }

    #[test]
    fn browsers_are_off_by_default_and_stay_off_on_the_light_profile() {
        let (mut store, bans) = open();
        for (p, c) in [("msedge.exe", "Chrome_WidgetWin_1"), ("slack.exe", "Chrome_WidgetWin_1"), ("firefox.exe", "MozillaWindowClass"), ("unknown.exe", "Chrome_WidgetWin_0"), ("olk.exe", "x")] {
            assert_eq!(gate(&target(p, c), &store, false, &bans), Err(Deny::Browser), "{p}");
        }
        store.learn_in_browsers = true;
        assert_eq!(gate(&target("msedge.exe", "Chrome_WidgetWin_1"), &store, false, &bans), Ok(TargetKind::Browser));
        assert_eq!(gate(&target("msedge.exe", "Chrome_WidgetWin_1"), &store, true, &bans), Err(Deny::Browser));
        // Private browsing and exclusions still win over the browser switch.
        let mut t = target("msedge.exe", "Chrome_WidgetWin_1");
        t.is_private_browsing = true;
        assert_eq!(gate(&t, &store, false, &bans), Err(Deny::PrivateBrowsing));
    }

    #[test]
    fn slow_applications_are_banned_for_the_session() {
        let (store, mut bans) = open();
        bans.ban("Notepad.exe");
        assert_eq!(gate(&target("notepad.exe", "Notepad"), &store, false, &bans), Err(Deny::SlowThisSession));
        assert_eq!(gate(&target("wordpad.exe", "WordPadClass"), &store, false, &bans), Ok(TargetKind::Native));
    }

    #[test]
    fn field_length_cap_is_8000() {
        assert!(field_length_ok(8_000));
        assert!(!field_length_ok(8_001));
    }

    // ---- schedule

    #[test]
    fn notifications_read_once_700ms_after_the_last_change() {
        let mut w = Watch::new(1_000, Mode::Notifications);
        // Nothing changed: nothing to read, wait for a notification up to the 60 s limit.
        assert_eq!(w.due(1_500), Step::Wait { until: 61_000 });
        w.text_changed(2_000);
        assert_eq!(w.due(2_300), Step::Wait { until: 2_700 });
        // Another change restarts the settle time.
        w.text_changed(2_500);
        assert_eq!(w.due(2_700), Step::Wait { until: 3_200 });
        assert_eq!(w.due(3_199), Step::Wait { until: 3_200 });
        assert_eq!(w.due(3_200), Step::Read);
        w.read_done(3_203, 3, ReadResult::Ok { chars: 40 });
        // One read per burst.
        assert_eq!(w.due(3_300), Step::Wait { until: 61_000 });
    }

    #[test]
    fn a_change_during_a_read_is_not_lost() {
        let mut w = Watch::new(0, Mode::Notifications);
        w.text_changed(100);
        assert_eq!(w.due(800), Step::Read);
        w.text_changed(805); // arrives while reading
        w.read_done(810, 10, ReadResult::Ok { chars: 10 });
        assert_eq!(w.due(1_000), Step::Wait { until: 1_505 });
        assert_eq!(w.due(1_505), Step::Read);
    }

    #[test]
    fn polling_reads_every_two_seconds() {
        let mut w = Watch::new(10_000, Mode::Polling);
        assert_eq!(w.due(10_000), Step::Wait { until: 12_000 });
        assert_eq!(w.due(12_000), Step::Read);
        w.read_done(12_001, 1, ReadResult::Ok { chars: 5 });
        assert_eq!(w.due(13_000), Step::Wait { until: 14_001 });
        assert_eq!(w.due(14_001), Step::Read);
    }

    #[test]
    fn the_watch_ends_after_sixty_seconds() {
        let mut w = Watch::new(0, Mode::Polling);
        assert_eq!(w.due(59_999), Step::Read);
        w.read_done(59_999, 1, ReadResult::Ok { chars: 5 });
        assert_eq!(w.due(60_000), Step::Stop { reason: EndReason::MaxDuration, unread_change: false });
        // The wait never runs past the limit.
        let mut w = Watch::new(0, Mode::Polling);
        w.read_done(58_000, 1, ReadResult::Ok { chars: 5 });
        assert_eq!(w.due(58_001), Step::Wait { until: 60_000 });
    }

    #[test]
    fn focus_loss_and_the_next_action_end_the_watch_and_report_unread_changes() {
        let mut w = Watch::new(0, Mode::Notifications);
        w.text_changed(100);
        assert_eq!(w.focus_lost(), Step::Stop { reason: EndReason::FocusLost, unread_change: true });
        // The first reason stands.
        assert_eq!(w.next_action(), Step::Stop { reason: EndReason::FocusLost, unread_change: true });
        assert_eq!(w.due(5_000), Step::Stop { reason: EndReason::FocusLost, unread_change: true });
        let mut w = Watch::new(0, Mode::Polling);
        assert_eq!(w.next_action(), Step::Stop { reason: EndReason::NextAction, unread_change: false });
    }

    #[test]
    fn a_slow_read_doubles_the_interval_and_two_end_watching() {
        let mut w = Watch::new(0, Mode::Polling);
        assert_eq!(w.due(2_000), Step::Read);
        w.read_done(2_040, 40, ReadResult::Ok { chars: 5 }); // slow: interval 4 s
        assert_eq!(w.due(2_041), Step::Wait { until: 6_040 });
        assert_eq!(w.due(6_040), Step::Read);
        w.read_done(6_100, 60, ReadResult::Ok { chars: 5 }); // second slow read
        assert_eq!(w.due(6_101), Step::Stop { reason: EndReason::TooSlow, unread_change: false });

        // 30 ms is not slow; the notification settle doubles as well.
        let mut w = Watch::new(0, Mode::Notifications);
        w.text_changed(0);
        assert_eq!(w.due(700), Step::Read);
        w.read_done(730, 30, ReadResult::Ok { chars: 5 });
        assert_eq!(w.ended(), None);
        w.text_changed(1_000);
        assert_eq!(w.due(1_700), Step::Read);
        w.read_done(1_731, 31, ReadResult::Ok { chars: 5 });
        w.text_changed(2_000);
        assert_eq!(w.due(2_700), Step::Wait { until: 3_400 });
    }

    #[test]
    fn a_gone_or_oversized_field_ends_the_watch() {
        let mut w = Watch::new(0, Mode::Polling);
        assert_eq!(w.due(2_000), Step::Read);
        w.read_done(2_001, 1, ReadResult::Gone);
        assert_eq!(w.due(2_002), Step::Stop { reason: EndReason::FieldGone, unread_change: false });
        let mut w = Watch::new(0, Mode::Polling);
        assert_eq!(w.due(2_000), Step::Read);
        w.read_done(2_001, 1, ReadResult::Ok { chars: 8_001 });
        assert_eq!(w.due(2_002), Step::Stop { reason: EndReason::FieldTooLong, unread_change: false });
    }

    // ---- locate

    fn ins(text: &str, start: Option<usize>, suffix: Option<usize>) -> Insertion {
        Insertion { inserted: text.into(), start, suffix_len: suffix }
    }

    #[test]
    fn locate_uses_the_caret_position_first() {
        // "Hello " was already there; the inserted text starts at 6; 4 chars follow it.
        let field = "Hello We met cooper netties today. Bye!";
        let i = ins("We met kubernetes today.", Some(6), Some(5));
        // Edited inside: the region still starts at 6 and ends before the suffix " Bye!".
        assert_eq!(locate(field, &i), Some("We met cooper netties today."));
        // Unchanged text is found the same way.
        let field = "Hello We met kubernetes today. Bye!";
        assert_eq!(locate(field, &i), Some("We met kubernetes today."));
    }

    #[test]
    fn locate_counts_chars_not_bytes() {
        let field = "Café: ok we met kubernetas today é";
        let i = ins("ok we met kubernetes today", Some(6), Some(2));
        assert_eq!(locate(field, &i), Some("ok we met kubernetas today"));
    }

    #[test]
    fn locate_falls_back_to_a_unique_match_and_refuses_ambiguity() {
        let i = ins("See you at six.", None, None);
        assert_eq!(locate("Hi. See you at six. Bye", &i), Some("See you at six."));
        assert_eq!(locate("See you at six. See you at six.", &i), None);
        assert_eq!(locate("Something else entirely", &i), None);
        // A position that does not fit (the text before it changed) falls back to the match.
        let i = ins("See you at six.", Some(500), Some(2));
        assert_eq!(locate("Hi. See you at six. K", &i), Some("See you at six."));
    }

    #[test]
    fn locate_gives_up_when_the_text_was_rewritten_or_deleted() {
        let i = ins("Please send the quarterly report to the finance team by Friday.", Some(0), Some(0));
        assert_eq!(locate("Ok.", &i), None);
        assert_eq!(locate("Totally different words appear in this field now instead.", &i), None);
        assert_eq!(locate("", &i), None);
        assert_eq!(locate("anything", &ins("   ", Some(0), Some(0))), None);
        // Far more text than was inserted: the position points at something else (not a region 50 times too big).
        let i = ins("short note here", Some(0), Some(0));
        assert_eq!(locate(&format!("short note here {}", "x ".repeat(500)), &i), Some("short note here"), "unique match instead");
        assert_eq!(locate(&format!("short note there {}", "x ".repeat(500)), &i), None);
    }

    // ---- diff and classify

    fn vocab(old: &str, new: &str) -> Vec<(String, String)> {
        classify(old, new).unwrap().vocabulary
    }

    #[test]
    fn tokens_keep_glued_words_whole_and_mark_sentence_starts() {
        let t = tokens("Don't go. See example.com/a, then \"stop\"!");
        let words: Vec<&str> = t.iter().map(|t| t.text).collect();
        assert_eq!(words, ["Don't", "go", ".", "See", "example.com/a", ",", "then", "\"", "stop", "\"", "!"]);
        assert!(t[0].sentence_start && t[3].sentence_start && !t[1].sentence_start && !t[6].sentence_start);
        let t = tokens("one\ntwo");
        assert!(t[1].sentence_start);
    }

    #[test]
    fn an_uncommon_word_replaced_by_a_similar_one_is_a_vocabulary_candidate() {
        assert_eq!(vocab("We deploy on kubernets every week.", "We deploy on Kubernetes every week."), [("kubernets".into(), "Kubernetes".into())]);
        assert_eq!(vocab("Ask Shanmuk about it.", "Ask Shanmukh about it."), [("Shanmuk".into(), "Shanmukh".into())]);
        // A sound-alike with different letters.
        assert_eq!(vocab("Open the kubernetties dashboard now.", "Open the Kubernetes dashboard now."), [("kubernetties".into(), "Kubernetes".into())]);
        // A casing correction of an uncommon word in the middle of a sentence.
        assert_eq!(vocab("We deploy on kubernetes every week.", "We deploy on Kubernetes every week."), [("kubernetes".into(), "Kubernetes".into())]);
    }

    #[test]
    fn swaps_between_common_words_and_unlike_words_are_ignored() {
        assert!(vocab("I think their going home now.", "I think there going home now.").is_empty());
        assert!(vocab("Write the weather report today.", "Write the whether report today.").is_empty());
        // Not similar: a different word, not a correction of the same one.
        assert!(vocab("We use postgres for storage here.", "We use sqlite for storage here.").is_empty());
        // Apostrophes: a typo fix, not a name.
        assert!(vocab("I dont know about this one.", "I don't know about this one.").is_empty());
        // Numbers and very short words are not vocabulary.
        assert!(vocab("It costs 120 rupees in total.", "It costs 125 rupees in total.").is_empty());
        assert!(vocab("We saw teh cat sit down.", "We saw the cat sit down.").is_empty(), "common 'the'");
    }

    #[test]
    fn more_than_forty_percent_changed_is_ignored() {
        let old = "Please send the report to the team by Friday morning.";
        assert_eq!(classify(old, "Kindly forward that document over to everybody on Friday afternoon."), Err(Ignore::Rewritten));
        // 4 of 10 words is allowed, 5 of 10 is not.
        let ten = "one two three four five six seven eight nine ten";
        assert!(classify(ten, "one 2 3 four five six seven eight nine ten").is_ok());
        assert!(classify(ten, "1 2 3 4 five six seven eight nine ten").is_ok());
        assert_eq!(classify(ten, "1 2 3 4 5 six seven eight nine ten"), Err(Ignore::Rewritten));
    }

    #[test]
    fn nothing_changed_and_no_words_are_ignored() {
        assert_eq!(classify("Same text.", "Same text."), Err(Ignore::NothingChanged));
        assert_eq!(classify("Same  text.", "Same text."), Err(Ignore::NothingChanged), "whitespace only");
        assert_eq!(classify("", "Something"), Err(Ignore::NoWords));
        assert_eq!(classify("...", "ok"), Err(Ignore::NoWords));
    }

    fn habits(old: &str, new: &str) -> Vec<(&'static str, String)> {
        classify(old, new).unwrap().habits
    }

    #[test]
    fn style_changes_become_counters() {
        // Contractions, both ways.
        assert_eq!(habits("I do not think it is ready.", "I don't think it's ready."), [("contractions", "uses".to_string())]);
        assert_eq!(habits("I don't think it's ready.", "I do not think it is ready."), [("contractions", "avoids".to_string())]);
        // Spelling dialect.
        assert_eq!(habits("My favorite color is blue.", "My favourite colour is blue."), [("dialect", "british".to_string())]);
        // Serial comma and the final full stop of a short message.
        assert_eq!(habits("We bought apples, oranges and pears.", "We bought apples, oranges, and pears."), [("serial_comma", "yes".to_string())]);
        assert_eq!(habits("See you at six.", "See you at six"), [("final_full_stop", "no".to_string())]);
        // Dashes.
        assert_eq!(habits("It works - mostly.", "It works \u{2014} mostly."), [("dash_style", "em_dash".to_string())]);
        // Capitalisation at the start of a sentence, both ways.
        assert_eq!(habits("ok. see you at six.", "ok. See you at six."), [("sentence_capital", "yes".to_string())]);
        assert_eq!(habits("Thanks for the update.", "thanks for the update."), [("sentence_capital", "no".to_string())]);
        assert_eq!(habits("Then i went home today.", "Then I went home today."), [("sentence_capital", "yes".to_string())]);
        // Greetings and sign-offs from a closed list.
        assert_eq!(habits("Hi Ravi, the file is attached. Thanks", "Hello Ravi, the file is attached. Thanks"), [("greeting", "hello".to_string())]);
        assert_eq!(habits("Hello Ravi, the file is attached.\nThanks,\nSam", "Hello Ravi, the file is attached.\nBest regards,\nSam"), [("sign_off", "best regards".to_string())]);
        // A typo fix says nothing about style.
        assert!(habits("I recieve the mail today.", "I receive the mail today.").is_empty());
    }

    // ---- the vocabulary rule

    fn learning() -> (StyleStore, PersonalDictionary) {
        (StyleStore::default(), PersonalDictionary::default())
    }

    #[test]
    fn once_goes_to_the_review_list_twice_goes_to_the_dictionary() {
        let (mut store, mut dict) = learning();
        let first = learn_vocabulary(&mut store, &mut dict, "kubernets", "Kubernetes", "chat", 10);
        assert_eq!(first, VocabOutcome::Review { from: "kubernets".into(), to: "Kubernetes".into(), count: 1 });
        assert!(dict.vocabulary.is_empty() && dict.replacements.is_empty());
        assert_eq!(store.vocabulary_candidates.len(), 1);

        // The second sighting may be in another kind of application.
        let second = learn_vocabulary(&mut store, &mut dict, "kubernets", "Kubernetes", "email", 20);
        let VocabOutcome::Promoted(token) = second else { panic!("{second:?}") };
        assert_eq!(dict.vocabulary, ["Kubernetes"]);
        assert_eq!((dict.replacements[0].spoken.as_str(), dict.replacements[0].written.as_str()), ("kubernets", "Kubernetes"));
        assert!(store.vocabulary_candidates.is_empty(), "promoted candidates leave the review list");

        // Undo removes exactly what was added.
        undo_learned(&mut dict, &token);
        assert!(dict.vocabulary.is_empty() && dict.replacements.is_empty());
        // A third sighting starts again from the review list.
        assert!(matches!(learn_vocabulary(&mut store, &mut dict, "kubernets", "Kubernetes", "chat", 30), VocabOutcome::Review { count: 1, .. }));
    }

    #[test]
    fn already_known_words_are_not_counted_again() {
        let (mut store, mut dict) = learning();
        dict.add_term("Kubernetes");
        dict.upsert_replacement("kubernets", "Kubernetes", false);
        assert_eq!(learn_vocabulary(&mut store, &mut dict, "kubernets", "Kubernetes", "chat", 1), VocabOutcome::AlreadyKnown);
        assert!(store.vocabulary_candidates.is_empty());
    }

    #[test]
    fn undo_never_removes_what_the_user_had() {
        let (mut store, mut dict) = learning();
        // The user already has the term (same case) but no rule: promotion adds only the rule.
        dict.add_term("Kubernetes");
        learn_vocabulary(&mut store, &mut dict, "kubernets", "Kubernetes", "chat", 1);
        let VocabOutcome::Promoted(t) = learn_vocabulary(&mut store, &mut dict, "kubernets", "Kubernetes", "chat", 2) else { panic!() };
        assert_eq!(dict.replacements.len(), 1);
        undo_learned(&mut dict, &t);
        assert_eq!(dict.vocabulary, ["Kubernetes"], "the user's own term stays");
        assert!(dict.replacements.is_empty());

        // The user's own rule for the same spoken word is never overridden.
        let (mut store, mut dict) = learning();
        dict.upsert_replacement("kubernets", "K8s", false);
        learn_vocabulary(&mut store, &mut dict, "kubernets", "Kubernetes", "chat", 1);
        let VocabOutcome::Promoted(t) = learn_vocabulary(&mut store, &mut dict, "kubernets", "Kubernetes", "chat", 2) else { panic!() };
        assert_eq!(dict.replacements[0].written, "K8s");
        undo_learned(&mut dict, &t);
        assert_eq!(dict.replacements[0].written, "K8s");
    }

    #[test]
    fn a_casing_correction_replaces_the_term_and_undo_restores_it() {
        let (mut store, mut dict) = learning();
        dict.add_term("kubernetes");
        learn_vocabulary(&mut store, &mut dict, "kubernetes", "Kubernetes", "chat", 1);
        let VocabOutcome::Promoted(t) = learn_vocabulary(&mut store, &mut dict, "kubernetes", "Kubernetes", "chat", 2) else { panic!() };
        assert_eq!(dict.vocabulary, ["Kubernetes"]);
        undo_learned(&mut dict, &t);
        assert_eq!(dict.vocabulary, ["kubernetes"]);
    }

    #[test]
    fn no_rule_is_learned_for_an_everyday_word() {
        let (mut store, mut dict) = learning();
        // "wind" is an everyday word: a rule from it would rewrite ordinary dictation.
        learn_vocabulary(&mut store, &mut dict, "wind", "Wynd", "chat", 1);
        let VocabOutcome::Promoted(_) = learn_vocabulary(&mut store, &mut dict, "wind", "Wynd", "chat", 2) else { panic!() };
        assert_eq!(dict.vocabulary, ["Wynd"]);
        assert!(dict.replacements.is_empty());
    }

    #[test]
    fn learning_switches_gate_everything() {
        let (mut store, mut dict) = learning();
        store.learn_from_edits = LearnState::Paused;
        let r = learn_from_edit(&mut store, &mut dict, "We use kubernets daily here.", "We use Kubernetes daily here.", "chat", Scope::Chat, 1);
        assert_eq!(r.ignored, Some(Ignore::LearningOff));
        assert!(store.vocabulary_candidates.is_empty() && store.items.is_empty());
        assert_eq!(learn_vocabulary(&mut store, &mut dict, "kubernets", "Kubernetes", "chat", 1), VocabOutcome::Dropped);
    }

    #[test]
    fn a_whole_edit_lands_in_the_store_and_the_dictionary_without_keeping_text() {
        let (mut store, mut dict) = learning();
        let old = "I don't know, we deploy on kubernets every week.";
        let new = "I do not know, we deploy on Kubernetes every week.";
        let r = learn_from_edit(&mut store, &mut dict, old, new, "chat", Scope::Chat, 5);
        assert_eq!(r.ignored, None);
        assert!(matches!(r.vocabulary[..], [VocabOutcome::Review { count: 1, .. }]));
        assert_eq!(r.habits_recorded, 1);
        assert_eq!(store.items.iter().filter(|i| i.key == "contractions" && i.value == "avoids").count(), 2, "chat and everywhere");
        let r = learn_from_edit(&mut store, &mut dict, old, new, "email", Scope::Email, 6);
        assert!(matches!(r.vocabulary[..], [VocabOutcome::Promoted(_)]));
        assert_eq!(dict.vocabulary, ["Kubernetes"]);
        // Only words and counters were kept: no stored string holds a sentence of the user's text.
        let json = serde_json::to_string(&store).unwrap() + &serde_json::to_string(&dict).unwrap();
        for sentence in ["we deploy on", "every week", "I don't know"] {
            assert!(!json.contains(sentence), "{sentence}");
        }
    }

    #[test]
    fn rewrites_teach_nothing() {
        let (mut store, mut dict) = learning();
        let r = learn_from_edit(&mut store, &mut dict, "Please send the report to the team by Friday morning.", "Kindly forward that document to everybody on Friday afternoon, thanks.", "email", Scope::Email, 1);
        assert_eq!(r.ignored, Some(Ignore::Rewritten));
        assert!(store.items.is_empty() && store.vocabulary_candidates.is_empty() && dict.vocabulary.is_empty());
    }
}
