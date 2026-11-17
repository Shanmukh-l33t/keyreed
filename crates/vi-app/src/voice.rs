//! Where a capture's transcript goes (plan P3.7). Ordinary dictation delivers it to the target app; the
//! Composer's microphone and the voice-command chord take it instead. Nothing here touches Win32, so the
//! routing is tested without a desktop.

use vi_core::textjob::Instruction;

/// The path a recording was started for. The coordinator holds it for the length of the capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CaptureTarget {
    /// Ordinary dictation: the transcript is delivered as it is today.
    #[default]
    Deliver,
    /// The Composer's microphone button: the transcript goes into the bar.
    Composer,
    /// The voice-command chord: the transcript is the instruction for the selected text.
    VoiceCommand,
}

/// The longest spoken instruction KeyReed sends: an instruction, not a document.
pub const MAX_INSTRUCTION_CHARS: usize = 500;

/// A voice-command chord released sooner than this was a tap: the Composer opens for typing instead of
/// recording. At or after it, the chord was held and the spoken words are the instruction.
pub const TAP_MS: u64 = 350;

/// What the voice-command chord did between its press and its release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChordRelease {
    /// A tap on a captured selection: open the Composer on it, exactly as the Composer chord does.
    OpenComposer,
    /// Held: transcribe the recording and run it as the instruction.
    Speak,
}

/// The release decision. Only a selection that was actually captured may open the Composer: with nothing
/// selected the press already refused, and a tap then opens nothing.
pub fn release(armed: bool, held_ms: u64) -> Option<ChordRelease> {
    if !armed {
        return None;
    }
    Some(if held_ms < TAP_MS { ChordRelease::OpenComposer } else { ChordRelease::Speak })
}

/// Whether a capture for `target` may start: a closed Composer has nowhere to put the words, and a voice
/// command needs the selection it will rewrite.
pub fn may_record(target: CaptureTarget, composer_open: bool, has_selection: bool) -> bool {
    match target {
        CaptureTarget::Deliver => true,
        CaptureTarget::Composer => composer_open,
        CaptureTarget::VoiceCommand => has_selection,
    }
}

/// What to do with a finished transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transcript<'a> {
    /// Deliver it as ordinary dictation.
    Dictation,
    /// Show it in the Composer's input; nothing is delivered.
    Composer(&'a str),
    /// Run it as the instruction of a voice command; nothing is delivered.
    VoiceCommand(Instruction),
    /// Put it nowhere: the Composer closed while recording, or nothing usable was said.
    Drop,
}

/// The routing decision: only ordinary dictation delivers.
pub fn route<'a>(target: CaptureTarget, composer_open: bool, transcript: &'a str) -> Transcript<'a> {
    let words = transcript.trim();
    match target {
        CaptureTarget::Deliver => Transcript::Dictation,
        CaptureTarget::Composer if composer_open && !words.is_empty() => Transcript::Composer(words),
        CaptureTarget::Composer => Transcript::Drop,
        CaptureTarget::VoiceCommand => match instruction(words) {
            Some(i) => Transcript::VoiceCommand(i),
            None => Transcript::Drop,
        },
    }
}

/// The spoken words as a custom instruction: trimmed, capped, and `None` when nothing was said.
pub fn instruction(spoken: &str) -> Option<Instruction> {
    let words = spoken.trim();
    if words.is_empty() {
        return None;
    }
    Some(Instruction::Custom(words.chars().take(MAX_INSTRUCTION_CHARS).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_ordinary_dictation_delivers() {
        assert_eq!(route(CaptureTarget::Deliver, true, "hello"), Transcript::Dictation);
        assert_eq!(route(CaptureTarget::Composer, true, " hello "), Transcript::Composer("hello"));
        assert_eq!(
            route(CaptureTarget::VoiceCommand, true, " make it formal "),
            Transcript::VoiceCommand(Instruction::Custom("make it formal".into()))
        );
    }

    #[test]
    fn a_closed_composer_or_nothing_said_puts_the_transcript_nowhere() {
        assert_eq!(route(CaptureTarget::Composer, false, "hello"), Transcript::Drop);
        assert_eq!(route(CaptureTarget::Composer, true, "   "), Transcript::Drop);
        assert_eq!(route(CaptureTarget::VoiceCommand, true, "\r\n"), Transcript::Drop, "a whisper is not an instruction");
    }

    #[test]
    fn a_capture_starts_only_where_it_has_somewhere_to_go() {
        assert!(may_record(CaptureTarget::Deliver, false, false));
        assert!(!may_record(CaptureTarget::Composer, false, true), "the bar closed: no recording");
        assert!(may_record(CaptureTarget::Composer, true, true));
        assert!(!may_record(CaptureTarget::VoiceCommand, true, false), "nothing selected: no recording");
        assert!(may_record(CaptureTarget::VoiceCommand, true, true));
    }

    #[test]
    fn a_short_tap_opens_the_composer_and_a_hold_speaks() {
        assert_eq!(release(true, 0), Some(ChordRelease::OpenComposer));
        assert_eq!(release(true, TAP_MS - 1), Some(ChordRelease::OpenComposer));
        // The threshold belongs to the hold, so the shortest intentional hold is never taken for a tap.
        assert_eq!(release(true, TAP_MS), Some(ChordRelease::Speak));
        assert_eq!(release(true, 4_000), Some(ChordRelease::Speak));
        // Nothing was captured (no selection, or a running dictation): the release does nothing at all.
        assert_eq!(release(false, 0), None);
        assert_eq!(release(false, 4_000), None);
    }

    #[test]
    fn a_spoken_instruction_is_trimmed_and_capped() {
        assert_eq!(instruction("  tighten this  "), Some(Instruction::Custom("tighten this".into())));
        assert_eq!(instruction("   "), None);
        let Some(Instruction::Custom(long)) = instruction(&"a".repeat(MAX_INSTRUCTION_CHARS + 40)) else {
            panic!("expected a custom instruction")
        };
        assert_eq!(long.chars().count(), MAX_INSTRUCTION_CHARS);
        // The cap counts characters, not bytes: a multi-byte instruction is cut cleanly.
        let Some(Instruction::Custom(wide)) = instruction(&"é".repeat(MAX_INSTRUCTION_CHARS + 1)) else {
            panic!("expected a custom instruction")
        };
        assert_eq!(wide.chars().count(), MAX_INSTRUCTION_CHARS);
    }
}