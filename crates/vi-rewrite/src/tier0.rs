use once_cell::sync::Lazy;
use regex::Regex;
use vi_core::{AppCategory, PersonalDictionary, WritingMode};

#[derive(Debug, Clone)]
pub struct Tier0Output {
    pub text: String,
    pub requires_confirmation: bool,
    pub warning_note: Option<String>,
    pub skipped_as_hallucination: bool,
}

static RE_MULTI_SPACE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[ \t]{2,}").unwrap());
/// A stray "Mm"/"Mhm" hallucinated from trailing noise, only right after a finished sentence
/// (never units like "5 mm").
static RE_TRAILING_MM: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)([.!?])\s+(?:m{2,}|mhm+|hm+)[-–—]*[.!?]?\s*$").unwrap());
/// The same sound at the very start of a dictation, followed by a comma/dash.
static RE_LEADING_MM: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^\s*(?:m{2,}|mhm+)[-–—,.]+\s*").unwrap());

/// One whole token that is a hesitation sound. "hmm" and "ah" are left alone because they can be
/// meaningful, and "err" is a real word.
static RE_FILLER_WORD: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^(?:um+|uh+|uhm+|erm+|er)$").unwrap());
static RE_STUTTER: Lazy<Regex> = Lazy::new(|| {
    // Rust regex doesn't have backreferences; handled token-by-token below.
    Regex::new(r"\s+").unwrap()
});
static RE_SELF_REPAIR: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\b(?:on |at |to |for )?(?:\d+(?:[.,]\d+)?\s+)?[a-z0-9:._-]{1,24}[,?.!\s]+(?:uh+\s+|um+\s+)?(?:sorry[,]?\s+no\s+wait|no[,]?\s+wait|sorry[,]?\s+i\s+mean|scratch\s+that[,]?|no\s+sorry[,]?|wait\s+no[,]?)[,\s]+",
    )
    .unwrap()
});
static RE_NOT_ABLE_REASON: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(?:i\s+)?am\s+not\s+able\s+to\s+find\s+what\s+is\s+the\s+reason\s+for\s+the\s+([a-z0-9_ -]+?)\s+going\s+down\b").unwrap()
});
static RE_NOT_ABLE_GENERIC: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(?:am|is|was|are|were)\s+not\s+able\s+to\s+find\s+what\s+is\s+the\s+reason\s+for\b").unwrap()
});
static RE_INDIRECT_WHAT_IS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(find|identify|understand|know|see|tell|explain)\s+(what|why|where|how|who)\s+is\s+(the\s+[a-z0-9_]+)\s+(going|failing|crashing|happening|causing|breaking|timing)\b").unwrap()
});
static RE_MYSELF_INTRO: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^myself\s+([a-z][a-z]+)\s+from\b").unwrap()
});
static RE_HAVING_DOUBT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(?:i\s+am\s+|am\s+)?having\s+(?:one|a)\s+doubt\s+(?:regarding|about)\b").unwrap()
});
static RE_REVERT_EARLIEST: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\bkindly\s+revert(?:\s+back)?\s+at\s+the\s+earliest\b").unwrap()
});
static RE_REVERT_BACK: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\brevert\s+back\b").unwrap()
});
static RE_DANGEROUS_CMD: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)(?:\brm\s+-[a-z]*r[a-z]*f|\brm\s+-[a-z]*f[a-z]*r|\bsudo\s+rm\b|\bdel\s+/[sqf]|\b(?:rd|rmdir)\s+/s|\bformat\s+[a-z]:|\bmkfs\b|\bdd\s+if=|\bremove-item\b[^\n]*-recurse|\bdrop\s+(?:table|database)\b|\btruncate\s+table\b|\bkubectl\s+delete\b|\bterraform\s+destroy\b|\bgit\s+push\s+(?:--force|-f)\b|\bgit\s+reset\s+--hard\b|\bchmod\s+-r\s+777\b|\bshutdown\b|\breboot\b)",
    )
    .unwrap()
});

fn compile_pairs(pairs: &[(&'static str, &'static str)]) -> Vec<(Regex, &'static str)> {
    pairs.iter().map(|&(p, r)| (Regex::new(p).unwrap(), r)).collect()
}

static RE_TERM_RULES: Lazy<Vec<(Regex, &'static str)>> = Lazy::new(|| {
    compile_pairs(&[
        (r"(?i)\b(?:kubractl|kubecontrol|kube\s+ctl|kube\s+control)\b", "kubectl"),
        (r"(?i)\bdouble\s+dash\s+([a-z0-9_-]+)", "--$1"),
        (r"(?i)\bdash\s+dash\s+([a-z0-9_-]+)", "--$1"),
        (r"(?i)\bdash\s+and\b", "-n"),
        (r"(?i)\s+-and\s+", " -n "),
        (r"(?i)\bdash\s+([a-z0-9]{1,4})\b", "-$1"),
        (r"(?i)\bhyphen\s+([a-z0-9]{1,4})\b", "-$1"),
        (r"(?i)\s+pipe\s+", " | "),
        (r"(?i)\s+redirect\s+to\s+", " > "),
        (r"(?i)\s+append\s+to\s+", " >> "),
        (r"(?i)\b([a-z])\s+colon\s+back\s*slash\s*([a-z0-9_.-]+)", "$1:\\$2"),
        (r"(?i)([a-z]:\\[a-z0-9_.\-\\]+)\s+back\s*slash\s*([a-z0-9_.-]+)", "$1\\$2"),
        (r"(?i)\b([a-z0-9_]+)\s+underscore\s+([a-z0-9_]+)\b", "${1}_$2"),
        (r"(?i)\b(\d{1,3})\s+dot\s+(\d{1,3})\s+dot\s+(\d{1,3})\s+dot\s+(\d{1,3})\b", "$1.$2.$3.$4"),
        (r"(?i)\bv\s*(\d+)\s+dot\s+(\d+)\s+dot\s+(\d+)\b", "v$1.$2.$3"),
        (r"(?i)\b([a-z0-9_-]+)\s+dot\s+([a-z0-9_-]+)\b", "$1.$2"),
    ])
});

static RE_NEW_PARAGRAPH: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\s*\b(?:new|next)\s+paragraph\b[.,]?\s*").unwrap());
static RE_NEW_LINE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\s*\b(?:new|next)\s+line\b[.,]?\s*").unwrap());
static RE_UNDERSCORE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\b([a-z0-9_]+)\s+underscore\s+([a-z0-9_]+)\b").unwrap());
static RE_WIN_DRIVE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\b([a-z])\s+colon\s+back\s*slash\s*([a-z0-9_.-]+)").unwrap());
static RE_WIN_SEG: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)([a-z]:\\[a-z0-9_.\-\\]+)\s+back\s*slash\s*([a-z0-9_.-]+)").unwrap());
static RE_PORT_HTTP_RULES: Lazy<Vec<(Regex, &'static str)>> = Lazy::new(|| {
    compile_pairs(&[
        (r"(?i)\bport\s+eighty[\s-]+eighty\b", "port 8080"),
        (r"(?i)\bport\s+eighty\b", "port 80"),
        (r"(?i)\bport\s+four\s+four\s+three\b", "port 443"),
        (r"(?i)\bport\s+eight\s+four\s+four\s+three\b", "port 8443"),
        (r"(?i)\bport\s+three\s+thousand\b", "port 3000"),
        (r"(?i)\bport\s+five\s+four\s+three\s+two\b", "port 5432"),
        (r"(?i)\bhttp\s+four\s+(?:oh|zero)\s+one\b", "HTTP 401"),
        (r"(?i)\bhttp\s+four\s+(?:oh|zero)\s+three\b", "HTTP 403"),
        (r"(?i)\bhttp\s+four\s+(?:oh|zero)\s+four\b", "HTTP 404"),
        (r"(?i)\bhttp\s+five\s+hundred\b", "HTTP 500"),
        (r"(?i)\bhttp\s+two\s+hundred\b", "HTTP 200"),
        (r"(?i)\b(\d{1,3})\s+dot\s+(\d{1,3})\s+dot\s+(\d{1,3})\s+dot\s+(\d{1,3})\b", "$1.$2.$3.$4"),
        (r"(?i)\bv\s*(\d+)\s+dot\s+(\d+)\s+dot\s+(\d+)\b", "v$1.$2.$3"),
        (r"(?i)\bv\s*(\d+)\s+dot\s+(\d+)\b", "v$1.$2"),
        (r"(?i)\b(version\s+)(\d+)\s+dot\s+(\d+)\s+dot\s+(\d+)\b", "${1}$2.$3.$4"),
        (r"(?i)\b(version\s+)(\d+)\s+dot\s+(\d+)\b", "${1}$2.$3"),
        (r"(?i)\b([a-z0-9_-]+)\s+dot\s+(rs|py|js|ts|tsx|jsx|go|c|cpp|h|json|yaml|yml|toml|md|txt|exe|dll|ps1|sh|sql|com|org|net|io|ai|dev|in|cc|local)\b", "$1.$2"),
        (r"(?i)\b([a-z0-9_-]+)\s+dot\s+([a-z0-9_-]+\.[a-z]{2,})\b", "$1.$2"),
        (r"(?i)\b([a-z0-9._+-]+)\s+at\s+([a-z0-9.-]+\.[a-z]{2,})\b", "$1@$2"),
        (r"(?i)\b(\d{1,2}(?::\d{2})?)\s*am\b", "$1 AM"),
        (r"(?i)\b(\d{1,2}(?::\d{2})?)\s*pm\b", "$1 PM"),
        (r"([a-zA-Z])([$₹€£]\d)", "$1 $2"),
        (r"(?i)\bhttp(\d{3})\b", "HTTP $1"),
        (r"(?i)\b(?:authmiddleware|off_middleware)\.rs\b", "auth_middleware.rs"),
        (r"(?i)\bcubanets\b", "Kubernetes"),
    ])
});
static RE_BULLET: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(?:^|\s+)(?:bullet\s+point|bullet\s+(?:one|two|three|four|five|\d+))[:.,]?\s+").unwrap()
});
pub(crate) static RE_PRONOUN_I: Lazy<Regex> = Lazy::new(|| Regex::new(r"\bi\b").unwrap());
pub(crate) static RE_PRONOUN_IM: Lazy<Regex> = Lazy::new(|| Regex::new(r"\bi'(m|ve|ll|d)\b").unwrap());
static RE_TECH_CASING: Lazy<Vec<(Regex, &'static str)>> = Lazy::new(|| {
    TECH_CASING
        .iter()
        .map(|&(lower, canonical)| (Regex::new(&format!(r"(?i)\b{}\b", regex::escape(lower))).unwrap(), canonical))
        .collect()
});
static RE_LEAD_BASICALLY: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^(?:so[,]?\s+)?basically[,]?\s+").unwrap());
static RE_FORMAL_RULES: Lazy<Vec<(Regex, &'static str)>> = Lazy::new(|| {
    compile_pairs(&[
        (r"(?i)^hey\b[,]?\s*", "Hello, "),
        (r"(?i)\bi'm\b", "I am"),
        (r"(?i)\bcan't\b", "cannot"),
        (r"(?i)\bwon't\b", "will not"),
        (r"(?i)\bcouldn't\b", "could not"),
        (r"(?i)\bshouldn't\b", "should not"),
        (r"(?i)\bwouldn't\b", "would not"),
        (r"(?i)\bdidn't\b", "did not"),
        (r"(?i)\bdoesn't\b", "does not"),
        (r"(?i)\bisn't\b", "is not"),
        (r"(?i)\baren't\b", "are not"),
        (r"(?i)\bgonna\b", "going to"),
        (r"(?i)\bwanna\b", "want to"),
        (r"(?i)\bkinda\b", "somewhat"),
    ])
});
static RE_CASUAL_RULES: Lazy<Vec<(Regex, &'static str)>> = Lazy::new(|| {
    compile_pairs(&[
        (r"(?i)\bi\s+am\b", "I'm"),
        (r"(?i)\bi\s+will\b", "I'll"),
        (r"(?i)\bwe\s+are\b", "we're"),
        (r"(?i)\bdo\s+not\b", "don't"),
        (r"(?i)\bdoes\s+not\b", "doesn't"),
        (r"(?i)\bdid\s+not\b", "didn't"),
        (r"(?i)\bcannot\b", "can't"),
        (r"(?i)\bcan\s+not\b", "can't"),
        (r"(?i)\bwill\s+not\b", "won't"),
    ])
});
/// Well-known Indian-English / non-native constructions that have one safe deterministic fix.
pub(crate) static RE_ESL_RULES: Lazy<Vec<(Regex, &'static str)>> = Lazy::new(|| {
    compile_pairs(&[
        (r"(?i)^myself\s+([a-z]+),?\s+", "I am $1, and "),
        (r"(?i)\bkindly\s+do\s+the\s+needful(?:\s+and)?\b", "please"),
        (r"(?i)\bat\s+the\s+earliest\b", "as soon as possible"),
        (r"(?i)\binformations\b", "information"),
        (r"(?i)\b(discuss(?:ing|ed|es)?)\s+about\b", "$1"),
        (r"(?i)\bexplain\s+me\b", "explain to me"),
        (r"(?i)\b(he|she|it)\s+don't\b", "$1 doesn't"),
        (r"(?i)\b(we|you|they)\s+was\b", "$1 were"),
        (r"(?i)\bsince\s+((?:\d+|a|one|two|three|four|five|six|seven|eight|nine|ten|twelve)\s+(?:minutes?|hours?|days?|weeks?|months?|years?))\b", "for $1"),
    ])
});
static RE_SCOPE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*)\s+colon\s+colon\s+([A-Za-z_][A-Za-z0-9_]*)\b").unwrap());
static RE_COULDNT: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\bcouldn't\b").unwrap());

const KNOWN_HALLUCINATIONS: &[&str] = &[
    "thank you for watching",
    "thanks for watching",
    "subtitles by amara.org",
    "please subscribe",
    "like and subscribe",
    "you",
];

/// Built-in canonical casing for universal developer & IT vocabulary.
const TECH_CASING: &[(&str, &str)] = &[
    ("postgresql", "PostgreSQL"),
    ("postgres", "Postgres"),
    ("kubernetes", "Kubernetes"),
    ("kubectl", "kubectl"),
    ("nginx", "nginx"),
    ("oauth2", "OAuth2"),
    ("oauth", "OAuth"),
    ("jwt", "JWT"),
    ("http", "HTTP"),
    ("https", "HTTPS"),
    ("json", "JSON"),
    ("yaml", "YAML"),
    ("sql", "SQL"),
    ("api", "API"),
    ("apis", "APIs"),
    ("url", "URL"),
    ("urls", "URLs"),
    ("github", "GitHub"),
    ("gitlab", "GitLab"),
    ("typescript", "TypeScript"),
    ("javascript", "JavaScript"),
    ("fastapi", "FastAPI"),
    ("powershell", "PowerShell"),
    ("ctrl", "Ctrl"),
    ("ist", "IST"),
    ("utc", "UTC"),
    ("monday", "Monday"),
    ("tuesday", "Tuesday"),
    ("wednesday", "Wednesday"),
    ("thursday", "Thursday"),
    ("friday", "Friday"),
    ("saturday", "Saturday"),
    ("sunday", "Sunday"),
    ("january", "January"),
    ("february", "February"),
    ("april", "April"),
    ("june", "June"),
    ("july", "July"),
    ("september", "September"),
    ("october", "October"),
    ("november", "November"),
    ("december", "December"),
    ("sendinput", "SendInput"),
    ("keyeventf_unicode", "KEYEVENTF_UNICODE"),
    ("cryptprotectdata", "CryptProtectData"),
    ("setdefaultdlldirectories", "SetDefaultDllDirectories"),
    ("iaudioclient", "IAudioClient"),
    ("wasapi", "WASAPI"),
    ("onnx", "ONNX"),
    ("gguf", "GGUF"),
    ("uipi", "UIPI"),
    ("dpapi", "DPAPI"),
    ("hwnd", "HWND"),
    ("pkce", "PKCE"),
    ("mib", "MiB"),
    ("gib", "GiB"),
    ("redis", "Redis"),
    ("docker", "Docker"),
    ("terraform", "Terraform"),
    ("graphql", "GraphQL"),
    ("grpc", "gRPC"),
    ("websocket", "WebSocket"),
    ("sha256", "SHA-256"),
    ("utf8", "UTF-8"),
    ("utf16", "UTF-16"),
    ("api_key", "API_KEY"),
    ("qwen3.5-0.8b-q4_k_m", "Qwen3.5-0.8B-Q4_K_M"),
    ("keyreed", "KeyReed"),
];

pub fn is_likely_silence_hallucination(raw: &str, speech_ratio: f32) -> bool {
    let norm = raw
        .trim()
        .trim_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace())
        .to_lowercase();
    if norm.is_empty() {
        return true;
    }
    if speech_ratio < 0.08 && KNOWN_HALLUCINATIONS.contains(&norm.as_str()) {
        return true;
    }
    // Weak English models fill silence with a repeated letter ("uuuuuuuuuu") or a stray character. A single
    // letter on its own is only speech when it is a word ("I", "a").
    let letters: Vec<char> = norm.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.len() < 2 && !matches!(norm.as_str(), "i" | "a") {
        return true;
    }
    if letters.len() >= 3 && letters.windows(2).all(|w| w[0] == w[1]) {
        return true;
    }
    false
}

/// Trims the ellipsis runs weak models emit for trailing silence ("Hello ........" -> "Hello."). A single
/// full stop, a real ellipsis inside the sentence and a normal word are left untouched.
fn strip_trailing_ellipsis(s: &str) -> String {
    let trimmed = s.trim_end();
    let without = trimmed.trim_end_matches(['.', '\u{2026}', ' ']);
    let dots = trimmed.len() - without.len();
    if dots < 3 {
        return s.to_string();
    }
    let mut out = without.to_string();
    if out.chars().any(|c| c.is_alphanumeric()) {
        out.push('.');
    }
    out
}

/// Drops standalone filler tokens. A sentence-ending mark that followed the filler moves onto the
/// previous word ("so, um." -> "so."), commas around it disappear with it, and when the filler
/// started a sentence the next word takes the capital (skipped for terminal commands).
fn remove_fillers(input: &str, hand_off_capital: bool) -> String {
    let is_end = |c: char| matches!(c, '.' | '!' | '?');
    let mut out: Vec<String> = Vec::new();
    let mut capitalize_next = false;
    for tok in input.split_whitespace() {
        let core = tok.trim_end_matches(|c: char| matches!(c, ',' | '.' | '!' | '?' | ';' | ':'));
        if !RE_FILLER_WORD.is_match(core) {
            let mut t = tok.to_string();
            if capitalize_next {
                if let Some(first) = t.chars().next().filter(|c| c.is_lowercase()) {
                    t = format!("{}{}", first.to_uppercase(), &t[first.len_utf8()..]);
                }
                capitalize_next = false;
            }
            out.push(t);
            continue;
        }
        let starts_sentence = out.last().map_or(true, |p| p.ends_with(is_end));
        if let Some(end) = tok[core.len()..].chars().find(|&c| is_end(c)) {
            if let Some(prev) = out.last_mut().filter(|p| !p.ends_with(is_end)) {
                *prev = format!("{}{}", prev.trim_end_matches(|c: char| matches!(c, ',' | ';' | ':')), end);
            }
            capitalize_next = hand_off_capital;
        } else if starts_sentence {
            capitalize_next = hand_off_capital;
        }
    }
    out.join(" ")
}

/// Words that are safe to treat as an accidental stutter when exactly doubled ("the the", "I I").
/// Repeats of any other word ("had had", "that that", "no no", "bye bye") are real language.
const STUTTER_WORDS: &[&str] = &[
    "the", "a", "an", "to", "of", "in", "and", "i", "is", "it", "we", "you", "my", "so", "but", "for",
    "on", "at", "or", "if",
];

fn dedup_adjacent_stutters(input: &str) -> String {
    let tokens: Vec<&str> = RE_STUTTER.split(input.trim()).filter(|s| !s.is_empty()).collect();
    if tokens.is_empty() {
        return String::new();
    }
    let plain = |t: &str| t.chars().all(|c| c.is_alphabetic());
    let mut out: Vec<&str> = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        let tok = tokens[i];
        out.push(tok);
        // Exactly two in a row, both free of punctuation; a third copy means it is not a stutter.
        if plain(tok)
            && STUTTER_WORDS.iter().any(|w| tok.eq_ignore_ascii_case(w))
            && tokens.get(i + 1).is_some_and(|n| plain(n) && n.eq_ignore_ascii_case(tok))
            && !tokens.get(i + 2).is_some_and(|n| n.eq_ignore_ascii_case(tok))
        {
            i += 1;
        }
        i += 1;
    }
    out.join(" ")
}

fn compile_terminal_command(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    // Remove leading conversational filler if user accidentally said "um" before command
    s = remove_fillers(&s, false);

    for (re, rep) in RE_TERM_RULES.iter() {
        s = re.replace_all(&s, *rep).to_string();
        s = re.replace_all(&s, *rep).to_string();
    }

    // Deterministic spoken `slash` path assembler
    const UNIX_ROOTS: &[&str] = &[
        "etc", "var", "usr", "opt", "tmp", "home", "root", "proc", "sys", "mnt", "srv", "dev",
    ];
    let raw_toks: Vec<&str> = s.split_whitespace().collect();
    let mut assembled: Vec<String> = Vec::with_capacity(raw_toks.len());
    let mut idx = 0usize;
    while idx < raw_toks.len() {
        if raw_toks[idx].eq_ignore_ascii_case("slash") {
            if let Some(&next_seg) = raw_toks.get(idx + 1) {
                let start_new_abs = match assembled.last() {
                    None => true,
                    Some(prev) if prev.starts_with('-') || matches!(prev.as_str(), "|" | ">" | ">>") => true,
                    Some(prev) if !prev.contains('/') && UNIX_ROOTS.iter().any(|r| next_seg.eq_ignore_ascii_case(r)) => true,
                    _ => false,
                };
                if start_new_abs {
                    assembled.push(format!("/{}", next_seg));
                } else if let Some(prev) = assembled.last_mut() {
                    prev.push('/');
                    prev.push_str(next_seg);
                }
                idx += 2;
                continue;
            }
        }
        assembled.push(raw_toks[idx].to_string());
        idx += 1;
    }
    s = assembled.join(" ");
    // Strip trailing sentence period added by punctuation-aware ASR models (like Parakeet)
    s = s.trim().trim_end_matches('.').trim().to_string();
    // Shell commands generally start lowercase when ASR capitalized the first word ("Kubectl" -> "kubectl", "Git" -> "git")
    let mut chars = s.chars();
    if let Some(first) = chars.next() {
        let rest: String = chars.collect();
        let first_word_end = rest.find(' ').unwrap_or(rest.len());
        let first_word = format!("{}{}", first, &rest[..first_word_end]);
        const COMMON_BINS: &[&str] = &[
            "git", "kubectl", "docker", "cargo", "npm", "pnpm", "yarn", "python", "python3",
            "pip", "uv", "ssh", "scp", "curl", "wget", "ls", "cd", "cat", "grep", "rg", "find",
            "chmod", "chown", "systemctl", "journalctl", "terraform", "helm", "az", "aws", "gcloud",
            " winget", "ipconfig", "netstat", "Get-", "Set-",
        ];
        if COMMON_BINS.iter().any(|b| first_word.eq_ignore_ascii_case(b))
            && !first_word.starts_with("Get-")
            && !first_word.starts_with("Set-")
        {
            s = format!("{}{}", first_word.to_lowercase(), &rest[first_word_end..]);
        }
    }
    RE_MULTI_SPACE.replace_all(&s, " ").trim().to_string()
}

fn apply_spoken_formatting(input: &str, mode: WritingMode, category: AppCategory) -> String {
    let mut s = input.to_string();
    s = RE_NEW_PARAGRAPH.replace_all(&s, "\n\n").to_string();
    s = RE_NEW_LINE.replace_all(&s, "\n").to_string();

    if mode != WritingMode::TerminalCode {
        if mode != WritingMode::Exact {
            // Convert spoken identifier underscores: `API underscore KEY` -> `API_KEY`, `auth underscore middleware` -> `auth_middleware`
            s = RE_UNDERSCORE.replace_all(&s, "${1}_$2").to_string();
            s = RE_UNDERSCORE.replace_all(&s, "${1}_$2").to_string();

            // Convert spoken Windows drive paths: `C colon backslash Dev backslash keyreed` -> `C:\Dev\keyreed`
            s = RE_WIN_DRIVE.replace_all(&s, |caps: &regex::Captures| {
                format!("{}:\\{}", caps[1].to_uppercase(), &caps[2])
            }).to_string();
            s = RE_WIN_SEG.replace_all(&s, "$1\\$2").to_string();
            s = RE_WIN_SEG.replace_all(&s, "$1\\$2").to_string();
        }

        // Convert common spoken port, HTTP status, IPv4, semver, filename/domain dots, email, and currency spacing
        for (re, rep) in RE_PORT_HTTP_RULES.iter() {
            s = re.replace_all(&s, *rep).to_string();
        }
    }

    // Format spoken bullet points ("bullet one ...", "bullet point ...") in Notes / Document / Email
    if matches!(
        category,
        AppCategory::Notes | AppCategory::Document | AppCategory::Email
    ) || s.to_lowercase().contains("bullet point ")
    {
        if RE_BULLET.is_match(&s) {
            s = RE_BULLET.replace_all(&s, "\n- ").trim_start_matches('\n').to_string();
        }
    }
    s
}

fn capitalize_sentences_and_pronoun_i(input: &str) -> String {
    let s = RE_PRONOUN_I.replace_all(input, "I").to_string();
    let s = RE_PRONOUN_IM.replace_all(&s, "I'$1").to_string();

    let chars: Vec<char> = s.chars().collect();
    let mut result = String::with_capacity(s.len() + 2);
    let mut cap_next = true;
    for (idx, &ch) in chars.iter().enumerate() {
        if cap_next && ch.is_alphabetic() {
            // Check if the token starting at `idx` is a code identifier, version tag (`v0.2.0`),
            // filename (`auth_middleware.rs`), path, or email that must preserve its leading lowercase!
            let token_end = chars[idx..]
                .iter()
                .position(|c| c.is_whitespace())
                .map(|p| idx + p)
                .unwrap_or(chars.len());
            let token: String = chars[idx..token_end].iter().collect();
            let trimmed_tok = token.trim_end_matches(|c: char| matches!(c, '.' | ',' | '!' | '?' | ':' | ';'));
            let is_ver_tag = trimmed_tok.starts_with('v')
                && trimmed_tok.len() >= 4
                && trimmed_tok.chars().nth(1).map(|c| c.is_ascii_digit()).unwrap_or(false)
                && trimmed_tok.contains('.');
            let is_code_or_file = trimmed_tok.contains('_')
                || trimmed_tok.contains('\\')
                || trimmed_tok.contains('/')
                || trimmed_tok.contains('@')
                || trimmed_tok.contains('.');
            if is_ver_tag || is_code_or_file {
                result.push(ch);
            } else {
                for up in ch.to_uppercase() {
                    result.push(up);
                }
            }
            cap_next = false;
        } else {
            result.push(ch);
            if ch == '\n' {
                cap_next = true;
            } else if matches!(ch, '.' | '!' | '?') {
                // Only treat `.` as sentence boundary if followed by whitespace or end-of-string,
                // never inside filenames (`auth_middleware.rs`) or domains (`github.com`)!
                let next_is_space = chars
                    .get(idx + 1)
                    .map(|c| c.is_whitespace())
                    .unwrap_or(true);
                if next_is_space {
                    cap_next = true;
                }
            }
        }
    }
    result
}

fn is_question_sentence(s: &str) -> bool {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.ends_with('?') {
        return true;
    }
    let mut words = trimmed
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphabetic()).to_lowercase());
    let first_word = words.next().unwrap_or_default();
    let second_word = words.next().unwrap_or_default();
    if matches!(first_word.as_str(), "do" | "does" | "did")
        && matches!(second_word.as_str(), "not" | "nt" | "don't")
    {
        return false;
    }
    if matches!(first_word.as_str(), "when" | "where" | "while") {
        return matches!(
            second_word.as_str(),
            "is" | "are" | "was" | "were" | "do" | "does" | "did" | "can" | "could"
                | "will" | "would" | "should" | "shall" | "has" | "have" | "had"
        );
    }
    matches!(
        first_word.as_str(),
        "what" | "why" | "how" | "who" | "which" | "whom" | "whose"
            | "can" | "could" | "would" | "should" | "will" | "shall" | "is" | "are"
            | "am" | "was" | "were" | "do" | "does" | "did" | "have" | "has"
    )
}

fn ensure_terminal_punctuation(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let last = trimmed.chars().last().unwrap();
    if matches!(last, '.' | '?' | '!' | ':' | ';') {
        return trimmed.to_string();
    }
    if is_question_sentence(trimmed) {
        format!("{}?", trimmed)
    } else {
        format!("{}.", trimmed)
    }
}

fn apply_dictionary_and_casing(input: &str, dict: &PersonalDictionary, apply_tech: bool) -> String {
    let mut s = input.to_string();
    if dict.enabled {
        for rule in &dict.replacements {
            let pat = if rule.case_sensitive {
                format!(r"\b{}\b", regex::escape(&rule.spoken))
            } else {
                format!(r"(?i)\b{}\b", regex::escape(&rule.spoken))
            };
            if let Ok(re) = Regex::new(&pat) {
                s = re.replace_all(&s, rule.written.as_str()).to_string();
            }
        }
        for term in &dict.vocabulary {
            let pat = format!(r"(?i)\b{}\b", regex::escape(term));
            if let Ok(re) = Regex::new(&pat) {
                s = re.replace_all(&s, term.as_str()).to_string();
            }
        }
    }
    if apply_tech {
        for (re, canonical) in RE_TECH_CASING.iter() {
            if re.is_match(&s) {
                let prev = s.clone();
                s = re
                    .replace_all(&prev, |caps: &regex::Captures| {
                        let Some(m) = caps.get(0) else {
                            return String::new();
                        };
                        let prev_char = prev[..m.start()].chars().next_back();
                        let next_slice = &prev[m.end()..];
                        let next_is_dot_ident = next_slice.starts_with('.')
                            && next_slice[1..]
                                .chars()
                                .next()
                                .map(|c| c.is_ascii_alphanumeric())
                                .unwrap_or(false);
                        if matches!(prev_char, Some('.' | '@' | '/' | '\\' | '_' | '-'))
                            || next_is_dot_ident
                        {
                            m.as_str().to_string()
                        } else {
                            (*canonical).to_string()
                        }
                    })
                    .to_string();
            }
        }
    }
    s
}

/// Runs deterministic sub-millisecond cleanup (Tier 0).
pub fn run_tier0(
    raw_transcript: &str,
    speech_ratio: f32,
    mode: WritingMode,
    category: AppCategory,
    dict: &PersonalDictionary,
) -> Tier0Output {
    if is_likely_silence_hallucination(raw_transcript, speech_ratio) {
        return Tier0Output {
            text: String::new(),
            requires_confirmation: false,
            warning_note: None,
            skipped_as_hallucination: true,
        };
    }

    if mode == WritingMode::TerminalCode {
        let cmd = compile_terminal_command(raw_transcript);
        let cmd = apply_dictionary_and_casing(&cmd, dict, false);
        let dangerous = RE_DANGEROUS_CMD.is_match(&cmd);
        let warning = if dangerous {
            Some(
                "Destructive or privileged command detected in voice input. Copied to clipboard only; review carefully before pasting."
                    .to_string(),
            )
        } else {
            None
        };
        return Tier0Output {
            text: cmd,
            requires_confirmation: dangerous,
            warning_note: warning,
            skipped_as_hallucination: false,
        };
    }

    let mut s = strip_trailing_ellipsis(raw_transcript.trim());
    // Strip unmatched leading quote artifacts occasionally emitted by BPE tokenizers (e.g. Moonshine `"'I checked...`)
    if (s.starts_with("\"'") || s.starts_with("'\"") || s.starts_with('"') || s.starts_with('\''))
        && !s.ends_with('"')
        && !s.ends_with('\'')
    {
        s = s.trim_start_matches(|c: char| c == '"' || c == '\'').trim_start().to_string();
    }

    if mode != WritingMode::Exact {
        s = RE_TRAILING_MM.replace(&s, "$1").to_string();
        s = RE_LEADING_MM.replace(&s, "").to_string();
        s = remove_fillers(&s, true);
        s = dedup_adjacent_stutters(&s);
        // Resolve spoken self-repairs ("push to Tuesday, sorry no wait, Wednesday" -> "push to Wednesday")
        s = RE_SELF_REPAIR
            .replace_all(&s, |caps: &regex::Captures| {
                let m = caps.get(0).map(|m| m.as_str()).unwrap_or("");
                let lower = m.to_lowercase();
                for prep in ["on ", "at ", "to ", "for "] {
                    if lower.starts_with(prep) {
                        return prep.to_string();
                    }
                }
                String::new()
            })
            .to_string();
        s = dedup_adjacent_stutters(&s);
    }

    if matches!(mode, WritingMode::Smart | WritingMode::Professional | WritingMode::Technical | WritingMode::Casual) {
        // These phrase rewrites change the user's wording, so only the polish mode applies them.
        if mode == WritingMode::Professional {
            s = RE_NOT_ABLE_REASON
                .replace_all(&s, "I could not identify the reason the $1 is going down")
                .to_string();
            s = RE_NOT_ABLE_GENERIC
                .replace_all(&s, "could not identify the reason for")
                .to_string();
        }
        s = RE_INDIRECT_WHAT_IS
            .replace_all(&s, "$1 $2 $3 is $4")
            .to_string();
        if mode == WritingMode::Professional {
            s = RE_MYSELF_INTRO.replace_all(&s, "This is $1 from").to_string();
            s = RE_REVERT_EARLIEST
                .replace_all(&s, "please get back to me as soon as possible")
                .to_string();
            s = RE_HAVING_DOUBT
                .replace_all(&s, "I have a question about")
                .to_string();
            s = RE_REVERT_BACK.replace_all(&s, "reply").to_string();
        }
        for (re, rep) in RE_ESL_RULES.iter() {
            s = re.replace_all(&s, *rep).to_string();
        }
        s = RE_LEAD_BASICALLY.replace_all(&s, "").to_string();
    }

    // Mode-specific style shaping so all 6 writing modes produce meaningful differences
    match mode {
        WritingMode::Professional => {
            for (re, rep) in RE_FORMAL_RULES.iter() {
                s = re.replace_all(&s, *rep).to_string();
            }
        }
        WritingMode::Casual => {
            for (re, rep) in RE_CASUAL_RULES.iter() {
                s = re.replace_all(&s, *rep).to_string();
            }
        }
        WritingMode::Technical => {
            // Preserve technical precision; convert spoken "colon colon" between identifiers into `::`
            s = RE_SCOPE.replace_all(&s, "$1::$2").to_string();
            s = RE_COULDNT.replace_all(&s, "could not").to_string();
        }
        _ => {}
    }

    s = apply_spoken_formatting(&s, mode, category);
    s = RE_MULTI_SPACE.replace_all(&s, " ").trim().to_string();
    s = capitalize_sentences_and_pronoun_i(&s);
    s = apply_dictionary_and_casing(&s, dict, mode != WritingMode::Exact);

    if mode != WritingMode::Casual || s.split_whitespace().count() > 12 || is_question_sentence(&s) {
        s = ensure_terminal_punctuation(&s);
    }

    Tier0Output {
        text: s,
        requires_confirmation: false,
        warning_note: None,
        skipped_as_hallucination: false,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_repeated_letter_or_a_stray_character_is_not_speech() {
        assert!(is_likely_silence_hallucination("uuuuuuuuuuuuuuuuuu", 0.9), "a repeated letter is noise");
        assert!(is_likely_silence_hallucination("...", 0.9));
        assert!(!is_likely_silence_hallucination("I", 0.9), "a single letter can be a word");
        assert!(!is_likely_silence_hallucination("a", 0.9));
        assert!(!is_likely_silence_hallucination("Hello there", 0.9));
    }

    #[test]
    fn trailing_ellipsis_from_silence_is_trimmed() {
        assert_eq!(strip_trailing_ellipsis("Hello ..............."), "Hello.");
        assert_eq!(strip_trailing_ellipsis("Hello..."), "Hello.");
        assert_eq!(strip_trailing_ellipsis("Hello."), "Hello.", "one full stop is real text");
        assert_eq!(strip_trailing_ellipsis("and so on"), "and so on");
        assert_eq!(strip_trailing_ellipsis("..."), "");
    }

    use super::*;

    #[test]
    fn strips_stray_trailing_mm_but_keeps_millimetres() {
        let dict = PersonalDictionary::default();
        let out = run_tier0("the website is going down on port 8080. Mm-.", 1.0, WritingMode::Smart, AppCategory::General, &dict);
        assert!(out.text.ends_with("8080."), "{}", out.text);
        let units = run_tier0("use a 5 mm bolt and a 10 mm washer.", 1.0, WritingMode::Smart, AppCategory::General, &dict);
        assert!(units.text.contains("5 mm") && units.text.contains("10 mm"), "{}", units.text);
        let exact = run_tier0("port 8080. Mm-.", 1.0, WritingMode::Exact, AppCategory::General, &dict);
        assert!(exact.text.to_lowercase().contains("mm"), "Exact keeps everything: {}", exact.text);
    }

    /// Settings and onboarding display `WritingMode::example()` pairs. They must be real Tier-0 output.
    #[test]
    fn writing_mode_ui_examples_match_real_tier0_output() {
        let dict = PersonalDictionary::default();
        let mut failures = Vec::new();
        for mode in WritingMode::ALL {
            let (spoken, written) = mode.example();
            let out = run_tier0(spoken, 1.0, mode, AppCategory::General, &dict);
            if out.text != written {
                failures.push(format!("{mode:?}: expected {written:?}, got {:?}", out.text));
            }
        }
        assert!(failures.is_empty(), "UI examples drifted from Tier-0:\n{}", failures.join("\n"));
    }

    #[test]
    fn polishes_master_prompt_example_conservatively() {
        let dict = PersonalDictionary::default();
        let out = run_tier0(
            "I checked the server but I am not able to find what is the reason for the website going down",
            0.85,
            WritingMode::Smart,
            AppCategory::General,
            &dict,
        );
        assert_eq!(
            out.text,
            "I checked the server but I am not able to find what is the reason for the website going down."
        );
        let pro = run_tier0(
            "I checked the server but I am not able to find what is the reason for the website going down",
            0.85,
            WritingMode::Professional,
            AppCategory::General,
            &dict,
        );
        assert_eq!(pro.text, "I checked the server but I could not identify the reason the website is going down.");
    }

    #[test]
    fn resolves_spoken_self_correction() {
        let dict = PersonalDictionary::default();
        let out = run_tier0(
            "um send the report on tuesday uh sorry no wait wednesday at 4 pm",
            0.9,
            WritingMode::Smart,
            AppCategory::Email,
            &dict,
        );
        assert!(out.text.contains("on Wednesday"), "got: {}", out.text);
        assert!(!out.text.to_lowercase().contains("tuesday"), "got: {}", out.text);
    }

    #[test]
    fn terminal_mode_compiles_flags_and_flags_dangerous_commands() {
        let dict = PersonalDictionary::default();
        let safe = run_tier0(
            "Kubectl get pods dash n prod dash o wide.",
            0.9,
            WritingMode::TerminalCode,
            AppCategory::Terminal,
            &dict,
        );
        assert_eq!(safe.text, "kubectl get pods -n prod -o wide");
        assert!(!safe.requires_confirmation);

        let risky = run_tier0(
            "sudo rm dash rf slash var slash tmp slash cache",
            0.9,
            WritingMode::TerminalCode,
            AppCategory::Terminal,
            &dict,
        );
        assert_eq!(risky.text, "sudo rm -rf /var/tmp/cache");
        assert!(risky.requires_confirmation);
    }

    #[test]
    fn six_writing_modes_produce_meaningful_differences_and_preserve_identifiers() {
        let dict = PersonalDictionary::default();
        let sample = "um i am not able to find what is the reason for the website going down on 192 dot 168 dot 1 dot 10";
        let smart = run_tier0(sample, 0.9, WritingMode::Smart, AppCategory::General, &dict).text;
        let exact = run_tier0(sample, 0.9, WritingMode::Exact, AppCategory::General, &dict).text;
        let pro = run_tier0(sample, 0.9, WritingMode::Professional, AppCategory::Email, &dict).text;
        let tech = run_tier0(sample, 0.9, WritingMode::Technical, AppCategory::CodeEditor, &dict).text;

        assert!(exact.to_lowercase().contains("um "), "Exact must preserve wording: {exact}");
        assert!(!smart.to_lowercase().contains("um "), "Smart must strip fillers: {smart}");
        assert!(smart.contains("not able to find"), "Smart keeps the user's wording: {smart}");
        assert!(pro.contains("could not identify"), "Professional uses formal full words: {pro}");
        assert!(tech.contains("not able to find"), "Technical keeps the user's wording: {tech}");
        assert!(tech.contains("192.168.1.10"), "IP address must be preserved: {tech}");

        let chat_in = "um i will check auth_middleware dot rs now";
        let casual = run_tier0(chat_in, 0.9, WritingMode::Casual, AppCategory::Chat, &dict).text;
        assert_eq!(casual, "I'll check auth_middleware.rs now");
    }

    #[test]
    fn preserves_all_seven_master_directive_benchmark_tokens() {
        let dict = PersonalDictionary::default();
        let raw = "v 0 dot 2 dot 0 sets api underscore key in auth underscore middleware dot rs using Qwen3.5-0.8B-Q4_K_M on 10 dot 0 dot 0 dot 1 for ₹1,250 under C colon backslash Dev backslash keyreed";
        let tech = run_tier0(raw, 0.95, WritingMode::Technical, AppCategory::CodeEditor, &dict).text;
        assert!(tech.starts_with("v0.2.0 "), "v0.2.0 must not be capitalized to V0.2.0: {tech}");
        assert!(tech.contains("API_KEY"), "must preserve API_KEY: {tech}");
        assert!(tech.contains("auth_middleware.rs"), "must preserve auth_middleware.rs: {tech}");
        assert!(tech.contains("Qwen3.5-0.8B-Q4_K_M"), "must preserve Qwen3.5-0.8B-Q4_K_M: {tech}");
        assert!(tech.contains("10.0.0.1"), "must format 10.0.0.1: {tech}");
        assert!(tech.contains("₹1,250"), "must preserve ₹1,250: {tech}");
        assert!(tech.contains(r"C:\Dev\KeyReed") || tech.contains(r"C:\Dev\keyreed"), "must format Windows path: {tech}");
    }

    fn smart(raw: &str) -> String {
        run_tier0(raw, 1.0, WritingMode::Smart, AppCategory::General, &PersonalDictionary::default()).text
    }

    #[test]
    fn filler_removal_keeps_sentence_end_and_hands_off_capital() {
        assert_eq!(smart("I think so, um. Then we ship"), "I think so. Then we ship.");
        assert_eq!(smart("Okay. Um, the build is green"), "Okay. The build is green.");
        assert_eq!(smart("so um, the build is green"), "So the build is green.");
        assert_eq!(smart("Hmm, ah, that is odd"), "Hmm, ah, that is odd.");
    }

    #[test]
    fn stutter_dedupe_only_collapses_doubled_function_words() {
        assert_eq!(smart("he had had enough"), "He had had enough.");
        assert_eq!(smart("I said that that is fine"), "I said that that is fine.");
        assert_eq!(smart("no no it works"), "No no it works.");
        assert_eq!(smart("the the build is stuck"), "The build is stuck.");
        assert_eq!(smart("i i think so"), "I think so.");
        assert_eq!(smart("the, the build"), "The, the build.");
        let exact = run_tier0("the the build", 1.0, WritingMode::Exact, AppCategory::General, &PersonalDictionary::default());
        assert!(exact.text.to_lowercase().contains("the the"), "{}", exact.text);
    }

    #[test]
    fn phrase_rewrites_only_in_professional_mode() {
        let raw = "please revert back, I am having a doubt regarding the plan";
        assert!(smart(raw).contains("revert back"));
        let pro = run_tier0(raw, 1.0, WritingMode::Professional, AppCategory::General, &PersonalDictionary::default()).text;
        assert!(pro.contains("reply") && pro.contains("I have a question about"), "{pro}");
    }

    #[test]
    fn ambiguous_words_are_not_recased() {
        assert_eq!(smart("we march forward in august"), "We march forward in august.");
        assert!(smart("see you on monday").contains("Monday"));
    }
}
