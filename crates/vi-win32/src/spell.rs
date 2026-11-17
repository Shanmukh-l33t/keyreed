//! Windows' built-in spell checker (`ISpellChecker`, Windows 8 and later) for the Quick Fix spelling layer.
//!
//! Works in the process, offline, with the dictionaries Windows ships (the user's own words in the Windows dictionary
//! are respected). Created on first use. COM is entered multithreaded, so one checker may be used from the worker thread
//! that runs Quick Fix jobs without an apartment of its own.

use windows::Win32::Foundation::S_OK;
use windows::core::{w, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Globalization::{ISpellChecker, ISpellCheckerFactory, SpellCheckerFactory, CORRECTIVE_ACTION_GET_SUGGESTIONS, CORRECTIVE_ACTION_REPLACE};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};

pub struct WindowsSpeller {
    /// `None` until first use; `Some(None)` when Windows has no checker for the language.
    checker: Option<Option<ISpellChecker>>,
}

// SAFETY: the checker is created after `CoInitializeEx(COINIT_MULTITHREADED)`; the spell checker COM class is
// registered for both apartment models, so its interface pointers may be used from any thread of the MTA.
unsafe impl Send for WindowsSpeller {}

impl Default for WindowsSpeller {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowsSpeller {
    pub fn new() -> Self {
        Self { checker: None }
    }

    /// Whether Windows has an English spell checker (created on first call).
    pub fn available(&mut self) -> bool {
        self.checker().is_some()
    }

    fn checker(&mut self) -> Option<&ISpellChecker> {
        self.checker.get_or_insert_with(create_checker).as_ref()
    }

    /// Misspelled words in `text` as `(start, end, first suggestion)`, byte offsets. Words with no suggestion are skipped.
    pub fn misspellings(&mut self, text: &str) -> Vec<(usize, usize, String)> {
        let Some(checker) = self.checker() else { return Vec::new() };
        // SAFETY: plain COM calls on interfaces this struct owns; every returned string is freed once.
        unsafe { check(checker, text).unwrap_or_default() }
    }
}

fn create_checker() -> Option<ISpellChecker> {
    // SAFETY: COM initialisation and class creation; failures mean "no spell checker".
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let factory: ISpellCheckerFactory = CoCreateInstance(&SpellCheckerFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        if !factory.IsSupported(w!("en-US")).ok()?.as_bool() {
            return None;
        }
        factory.CreateSpellChecker(w!("en-US")).ok()
    }
}

unsafe fn check(checker: &ISpellChecker, text: &str) -> windows::core::Result<Vec<(usize, usize, String)>> {
    let utf16: Vec<u16> = text.encode_utf16().collect();
    // Byte offset of every UTF-16 index (a surrogate pair's second unit maps to the same character).
    let mut byte_at = Vec::with_capacity(utf16.len() + 1);
    for (b, c) in text.char_indices() {
        byte_at.extend(std::iter::repeat(b).take(c.len_utf16()));
    }
    byte_at.push(text.len());
    let errors = checker.Check(&HSTRING::from(text))?;
    let mut out = Vec::new();
    loop {
        let mut next = None;
        if errors.Next(&mut next) != S_OK {
            break;
        }
        let Some(error) = next else { break };
        let (start, len) = (error.StartIndex()? as usize, error.Length()? as usize);
        if start + len > utf16.len() {
            continue;
        }
        let word = String::from_utf16_lossy(&utf16[start..start + len]);
        let action = error.CorrectiveAction()?;
        let suggestion = if action == CORRECTIVE_ACTION_REPLACE {
            take_string(error.Replacement()?)
        } else if action == CORRECTIVE_ACTION_GET_SUGGESTIONS {
            first_suggestion(checker, &word)
        } else {
            None // CORRECTIVE_ACTION_DELETE (a doubled word) and anything else is left to the other layers
        };
        if let Some(s) = suggestion.filter(|s| !s.is_empty() && *s != word) {
            out.push((byte_at[start], byte_at[start + len], s));
        }
    }
    Ok(out)
}

unsafe fn first_suggestion(checker: &ISpellChecker, word: &str) -> Option<String> {
    let list = checker.Suggest(&HSTRING::from(word)).ok()?;
    let mut item = [PWSTR::null()];
    let mut fetched = 0u32;
    if list.Next(&mut item, Some(&mut fetched)).is_ok() && fetched == 1 {
        take_string(item[0])
    } else {
        None
    }
}

/// Reads a COM-allocated string and frees it.
unsafe fn take_string(p: PWSTR) -> Option<String> {
    if p.is_null() {
        return None;
    }
    let s = PCWSTR(p.0).to_string().ok();
    CoTaskMemFree(Some(p.0 as *const _));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_misspellings_with_byte_offsets() {
        let mut s = WindowsSpeller::new();
        if !s.available() {
            eprintln!("no Windows spell checker for en-US: test skipped");
            return;
        }
        let text = "I recieve the file caf\u{e9} and teh report.";
        let found = s.misspellings(text);
        let words: Vec<(&str, &str)> = found.iter().map(|(a, b, fix)| (&text[*a..*b], fix.as_str())).collect();
        assert!(words.contains(&("recieve", "receive")), "{words:?}");
        assert!(words.contains(&("teh", "the")), "{words:?}");
        assert!(s.misspellings("This sentence is fine.").is_empty());
    }
}
