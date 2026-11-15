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
static RE_FILLERS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(um+|uh+|uhm+|erm+|er+|hmm+|ah+)\b[,.]?\s*").unwrap()
});
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
    Regex::new(r"(?i)\b(?:am|was|are|were)\s+not\s+able\s+to\s+find\s+what\s+is\s+the\s+reason\s+for\b").unwrap()
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
    ("march", "March"),
    ("april", "April"),
    ("june", "June"),
    ("july", "July"),
    ("august", "August"),
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
    false
}

fn dedup_adjacent_stutters(input: &str) -> String {
    let tokens: Vec<&str> = RE_STUTTER.split(input.trim()).filter(|s| !s.is_empty()).collect();
    if tokens.is_empty() {
        return String::new();
    }
    let mut out: Vec<&str> = Vec::with_capacity(tokens.len());
    for tok in tokens {
        let clean_curr = tok.trim_matches(|c: char| c.is_ascii_punctuation());
        if let Some(prev) = out.last() {
            let clean_prev = prev.trim_matches(|c: char| c.is_ascii_punctuation());
            // Collapse accidental spoken word stutters ("the the", "i i", "in in")
            if !clean_curr.is_empty()
                && clean_curr.len() <= 5
                && clean_curr.chars().all(|c| c.is_alphabetic())
                && clean_curr.eq_ignore_ascii_case(clean_prev)
            {
                continue;
            }
        }
        out.push(tok);
    }
    out.join(" ")
}

fn compile_terminal_command(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    // Remove leading conversational filler if user accidentally said "um" before command
    s = RE_FILLERS.replace_all(&s, "").to_string();

    let replacements = [
        (r"(?i)\bdouble\s+dash\s+([a-z0-9_-]+)", "--$1"),
        (r"(?i)\bdash\s+dash\s+([a-z0-9_-]+)", "--$1"),
        (r"(?i)\bdash\s+([a-z0-9]{1,4})\b", "-$1"),
        (r"(?i)\bhyphen\s+([a-z0-9]{1,4})\b", "-$1"),
        (r"(?i)\s+pipe\s+", " | "),
        (r"(?i)\s+redirect\s+to\s+", " > "),
        (r"(?i)\s+append\s+to\s+", " >> "),
        (r"(?i)\b([a-z0-9_-]+)\s+dot\s+([a-z0-9_-]+)\b", "$1.$2"),
        (r"(?i)\b([a-z0-9_]+)\s+underscore\s+([a-z0-9_]+)\b", "$1_$2"),
    ];
    for (pat, rep) in replacements {
        let re = Regex::new(pat).unwrap();
        s = re.replace_all(&s, rep).to_string();
        s = re.replace_all(&s, rep).to_string();
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
    let re_np = Regex::new(r"(?i)\s*\b(?:new|next)\s+paragraph\b[.,]?\s*").unwrap();
    s = re_np.replace_all(&s, "\n\n").to_string();
    let re_nl = Regex::new(r"(?i)\s*\b(?:new|next)\s+line\b[.,]?\s*").unwrap();
    s = re_nl.replace_all(&s, "\n").to_string();

    if mode != WritingMode::TerminalCode {
        // Convert spoken IPv4 addresses: `192 dot 168 dot 1 dot 10` -> `192.168.1.10`
        let re_ipv4 = Regex::new(
            r"(?i)\b(\d{1,3})\s+dot\s+(\d{1,3})\s+dot\s+(\d{1,3})\s+dot\s+(\d{1,3})\b",
        )
        .unwrap();
        s = re_ipv4.replace_all(&s, "$1.$2.$3.$4").to_string();

        // Convert spoken semantic version numbers: `version 1 dot 2 dot 0` -> `version 1.2.0`
        let re_semver3 = Regex::new(
            r"(?i)\b(version\s+|v\s*)(\d+)\s+dot\s+(\d+)\s+dot\s+(\d+)\b",
        )
        .unwrap();
        s = re_semver3.replace_all(&s, "${1}$2.$3.$4").to_string();
        let re_semver2 = Regex::new(r"(?i)\b(version\s+|v\s*)(\d+)\s+dot\s+(\d+)\b").unwrap();
        s = re_semver2.replace_all(&s, "${1}$2.$3").to_string();

        // Convert unambiguous filename/domain spoken dots: `auth_middleware dot rs`, `example dot com`
        let re_file_dot = Regex::new(
            r"(?i)\b([a-z0-9_-]+)\s+dot\s+(rs|py|js|ts|tsx|jsx|go|c|cpp|h|json|yaml|yml|toml|md|txt|exe|dll|ps1|sh|sql|com|org|net|io|ai|dev|in|cc|local)\b",
        )
        .unwrap();
        s = re_file_dot.replace_all(&s, "$1.$2").to_string();
        let re_at = Regex::new(r"(?i)\b([a-z0-9._+-]+)\s+at\s+([a-z0-9.-]+\.[a-z]{2,})\b").unwrap();
        s = re_at.replace_all(&s, "$1@$2").to_string();
    }

    // Format spoken bullet points ("bullet one ...", "bullet point ...") in Notes / Document / Email
    if matches!(
        category,
        AppCategory::Notes | AppCategory::Document | AppCategory::Email
    ) || s.to_lowercase().contains("bullet point ")
    {
        let re_bullet = Regex::new(r"(?i)(?:^|\s+)(?:bullet\s+point|bullet\s+(?:one|two|three|four|five|\d+))[:.,]?\s+").unwrap();
        if re_bullet.is_match(&s) {
            s = re_bullet.replace_all(&s, "\n- ").trim_start_matches('\n').to_string();
        }
    }
    s
}

fn capitalize_sentences_and_pronoun_i(input: &str) -> String {
    let re_i = Regex::new(r"\bi\b").unwrap();
    let s = re_i.replace_all(input, "I").to_string();
    let re_im = Regex::new(r"\bi'(m|ve|ll|d)\b").unwrap();
    let s = re_im.replace_all(&s, "I'$1").to_string();

    let chars: Vec<char> = s.chars().collect();
    let mut result = String::with_capacity(s.len() + 2);
    let mut cap_next = true;
    for (idx, &ch) in chars.iter().enumerate() {
        if cap_next && ch.is_alphabetic() {
            for up in ch.to_uppercase() {
                result.push(up);
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
    let first_word = trimmed
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|c: char| !c.is_alphabetic())
        .to_lowercase();
    matches!(
        first_word.as_str(),
        "what" | "why" | "how" | "where" | "when" | "who" | "which" | "whom" | "whose"
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
        for (lower, canonical) in TECH_CASING {
            let pat = format!(r"(?i)\b{}\b", regex::escape(lower));
            if let Ok(re) = Regex::new(&pat) {
                s = re.replace_all(&s, *canonical).to_string();
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

    let mut s = raw_transcript.trim().to_string();

    if mode != WritingMode::Exact {
        s = RE_FILLERS.replace_all(&s, "").to_string();
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
    }

    if matches!(mode, WritingMode::Smart | WritingMode::Professional | WritingMode::Technical | WritingMode::Casual) {
        if mode == WritingMode::Professional {
            s = RE_NOT_ABLE_REASON
                .replace_all(&s, "I could not identify the reason the $1 is going down")
                .to_string();
            s = RE_NOT_ABLE_GENERIC
                .replace_all(&s, "could not identify the reason for")
                .to_string();
        } else if mode == WritingMode::Technical {
            s = RE_NOT_ABLE_REASON
                .replace_all(&s, "Unable to identify the cause of the $1 outage")
                .to_string();
            s = RE_NOT_ABLE_GENERIC
                .replace_all(&s, "unable to identify the cause of")
                .to_string();
        } else {
            s = RE_NOT_ABLE_REASON
                .replace_all(&s, "I couldn't identify the reason the $1 is going down")
                .to_string();
            s = RE_NOT_ABLE_GENERIC
                .replace_all(&s, "couldn't identify the reason for")
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
        } else {
            s = RE_MYSELF_INTRO.replace_all(&s, "I'm $1 from").to_string();
            s = RE_REVERT_EARLIEST
                .replace_all(&s, "please reply as soon as you can")
                .to_string();
        }
        s = RE_HAVING_DOUBT
            .replace_all(&s, "I have a question about")
            .to_string();
        s = RE_REVERT_BACK.replace_all(&s, "reply").to_string();
        let re_lead_basically = Regex::new(r"(?i)^(?:so[,]?\s+)?basically[,]?\s+").unwrap();
        s = re_lead_basically.replace_all(&s, "").to_string();
    }

    // Mode-specific style shaping so all 6 writing modes produce meaningful differences
    match mode {
        WritingMode::Professional => {
            let formal_rules = [
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
            ];
            for (pat, rep) in formal_rules {
                let re = Regex::new(pat).unwrap();
                s = re.replace_all(&s, rep).to_string();
            }
        }
        WritingMode::Casual => {
            let casual_rules = [
                (r"(?i)\bi\s+am\b", "I'm"),
                (r"(?i)\bi\s+will\b", "I'll"),
                (r"(?i)\bwe\s+are\b", "we're"),
                (r"(?i)\bdo\s+not\b", "don't"),
                (r"(?i)\bdoes\s+not\b", "doesn't"),
                (r"(?i)\bdid\s+not\b", "didn't"),
                (r"(?i)\bcannot\b", "can't"),
                (r"(?i)\bcan\s+not\b", "can't"),
                (r"(?i)\bwill\s+not\b", "won't"),
            ];
            for (pat, rep) in casual_rules {
                let re = Regex::new(pat).unwrap();
                s = re.replace_all(&s, rep).to_string();
            }
        }
        WritingMode::Technical => {
            // Preserve technical precision; convert spoken "colon colon" between identifiers into `::`
            let re_scope = Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*)\s+colon\s+colon\s+([A-Za-z_][A-Za-z0-9_]*)\b").unwrap();
            s = re_scope.replace_all(&s, "$1::$2").to_string();
            let re_cant = Regex::new(r"(?i)\bcouldn't\b").unwrap();
            s = re_cant.replace_all(&s, "could not").to_string();
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
    use super::*;

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
            "I checked the server but I couldn't identify the reason the website is going down."
        );
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
        assert!(smart.contains("couldn't identify"), "Smart uses natural contraction: {smart}");
        assert!(pro.contains("could not identify"), "Professional uses formal full words: {pro}");
        assert!(tech.contains("Unable to identify"), "Technical uses technical phrasing: {tech}");
        assert!(tech.contains("192.168.1.10"), "IP address must be preserved: {tech}");

        let chat_in = "um i will check auth_middleware dot rs now";
        let casual = run_tier0(chat_in, 0.9, WritingMode::Casual, AppCategory::Chat, &dict).text;
        assert_eq!(casual, "I'll check auth_middleware.rs now");
    }
}
