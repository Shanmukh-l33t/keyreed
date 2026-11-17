//! "Press your shortcut" capture and the conflict rules for KeyReed's own chords (plan F1, P2.7).
//!
//! Everything here is pure. The Settings capture field feeds it only the key events that field itself receives,
//! so nothing outside the field is ever recorded.

use crate::config::AppConfig;
use crate::types::HotkeySpec;

/// Left and right Ctrl, Alt, Shift and Win, in the bit order used by [`ShortcutCapture`].
const MOD_VKS: [u16; 8] = [0xA2, 0xA3, 0xA4, 0xA5, 0xA0, 0xA1, 0x5B, 0x5C];

fn is_modifier(vk: u16) -> bool {
    MOD_VKS.contains(&vk) || matches!(vk, 0x10..=0x12)
}

/// Plain-language name of one key.
pub fn key_name(vk: u16) -> String {
    let s = match vk {
        0x30..=0x39 | 0x41..=0x5A => return (vk as u8 as char).to_string(),
        0x70..=0x87 => return format!("F{}", vk - 0x6F),
        0x60..=0x69 => return format!("Num {}", vk - 0x60),
        0x08 => "Backspace",
        0x09 => "Tab",
        0x0D => "Enter",
        0x13 => "Pause",
        0x14 => "Caps Lock",
        0x1B => "Esc",
        0x20 => "Space",
        0x21 => "Page Up",
        0x22 => "Page Down",
        0x23 => "End",
        0x24 => "Home",
        0x25 => "Left",
        0x26 => "Up",
        0x27 => "Right",
        0x28 => "Down",
        0x2C => "Print Screen",
        0x2D => "Insert",
        0x2E => "Delete",
        0x5B => "Left Win",
        0x5C => "Right Win",
        0x5D => "Menu",
        0x6A => "Num *",
        0x6B => "Num +",
        0x6D => "Num -",
        0x6E => "Num .",
        0x6F => "Num /",
        0xA0 => "Left Shift",
        0xA1 => "Right Shift",
        0xA2 => "Left Ctrl",
        0xA3 => "Right Ctrl",
        0xA4 => "Left Alt",
        0xA5 => "Right Alt",
        0xBA => ";",
        0xBB => "=",
        0xBC => ",",
        0xBD => "-",
        0xBE => ".",
        0xBF => "/",
        0xC0 => "`",
        0xDB => "[",
        0xDC => "\\",
        0xDD => "]",
        0xDE => "'",
        _ => return format!("Key {vk:#04X}"),
    };
    s.to_string()
}

/// A named chord: modifiers in the order Ctrl, Alt, Shift, Win, then the key (`vk == 0`: modifiers only).
pub fn chord(ctrl: bool, alt: bool, shift: bool, win: bool, vk: u16) -> HotkeySpec {
    let mut parts: Vec<String> = [(ctrl, "Ctrl"), (alt, "Alt"), (shift, "Shift"), (win, "Win")]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, n)| n.to_string())
        .collect();
    if vk != 0 {
        parts.push(key_name(vk));
    }
    HotkeySpec { ctrl, alt, shift, win, vk, display_name: parts.join(" + ") }
}

/// Same chord, whatever the stored names say.
pub fn same_chord(a: &HotkeySpec, b: &HotkeySpec) -> bool {
    a.vk == b.vk && a.ctrl == b.ctrl && a.alt == b.alt && a.shift == b.shift && a.win == b.win
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureStep {
    /// Still waiting for the chord to finish.
    Pending,
    Done(HotkeySpec),
    /// Esc on its own: keep the old shortcut.
    Cancelled,
}

/// State of one capture. Feed it left/right-specific virtual keys (the caller maps VK_CONTROL and friends).
#[derive(Debug, Default, Clone)]
pub struct ShortcutCapture {
    held: u8,
    peak: u8,
}

impl ShortcutCapture {
    /// One key event seen by the capture field. A chord ends at its first non-modifier key; a modifier-only chord
    /// ends when every modifier is released, and only counts when `modifier_only_allowed` (dictation).
    pub fn key(&mut self, vk: u16, down: bool, modifier_only_allowed: bool) -> CaptureStep {
        if let Some(i) = MOD_VKS.iter().position(|m| *m == vk) {
            let bit = 1u8 << i;
            if down {
                self.held |= bit;
                self.peak |= bit;
                return CaptureStep::Pending;
            }
            self.held &= !bit;
            if self.held == 0 && self.peak != 0 {
                let peak = std::mem::take(&mut self.peak);
                if modifier_only_allowed {
                    return CaptureStep::Done(modifier_only(peak));
                }
            }
            return CaptureStep::Pending;
        }
        if !down || is_modifier(vk) {
            return CaptureStep::Pending;
        }
        if vk == 0x1B && self.held == 0 {
            return CaptureStep::Cancelled;
        }
        let (c, a, s, w) = flags(self.held);
        CaptureStep::Done(chord(c, a, s, w, vk))
    }

    /// The modifiers held right now, for the live field text ("Ctrl + Win + ..."); empty when none.
    pub fn held_label(&self) -> String {
        if self.held == 0 {
            return String::new();
        }
        let (c, a, s, w) = flags(self.held);
        format!("{} + ...", chord(c, a, s, w, 0).display_name)
    }
}

fn flags(bits: u8) -> (bool, bool, bool, bool) {
    (bits & 0b11 != 0, bits & 0b1100 != 0, bits & 0b11_0000 != 0, bits & 0b1100_0000 != 0)
}

fn modifier_only(bits: u8) -> HotkeySpec {
    if bits.count_ones() == 1 {
        // A single modifier keeps its side, as the existing "Right Ctrl" preset does.
        let vk = MOD_VKS[bits.trailing_zeros() as usize];
        return HotkeySpec { ctrl: false, alt: false, shift: false, win: false, vk, display_name: key_name(vk) };
    }
    let (c, a, s, w) = flags(bits);
    chord(c, a, s, w, 0)
}

/// Which of KeyReed's chords a shortcut is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutRole {
    Dictation,
    QuickFix,
    Composer,
}

impl ShortcutRole {
    fn name(self) -> &'static str {
        match self {
            ShortcutRole::Dictation => "push-to-talk",
            ShortcutRole::QuickFix => "Quick Fix",
            ShortcutRole::Composer => "Composer",
        }
    }
}

/// Why a captured chord cannot be used (`blocks`), or what the user should know about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutProblem {
    pub message: String,
    pub blocks: bool,
}

fn block(message: String) -> Option<ShortcutProblem> {
    Some(ShortcutProblem { message, blocks: true })
}

impl AppConfig {
    pub fn shortcut(&self, role: ShortcutRole) -> &HotkeySpec {
        match role {
            ShortcutRole::Dictation => &self.ptt_hotkey,
            ShortcutRole::QuickFix => &self.quick_fix_hotkey,
            ShortcutRole::Composer => &self.composer_hotkey,
        }
    }

    /// The conflict rules for a chord the user wants for `role`: reserved Windows and editing chords, keys used for
    /// typing, chords without a key where one is needed, KeyReed's own chords, and prefix conflicts (a modifier-only
    /// dictation chord such as Ctrl + Win starts recording before Ctrl + Win + G is complete).
    pub fn shortcut_problem(&self, role: ShortcutRole, c: &HotkeySpec) -> Option<ShortcutProblem> {
        let name = &c.display_name;
        if let Some(m) = c.check_conflict(None) {
            return block(m);
        }
        let modifier_only = c.vk == 0 || is_modifier(c.vk);
        if role != ShortcutRole::Dictation && modifier_only {
            return block(format!("Add a key to {name}, for example Ctrl + Win + G."));
        }
        let typing_key = !modifier_only && !(0x70..=0x87).contains(&c.vk) && !matches!(c.vk, 0x13 | 0x2C | 0x5D);
        if typing_key && !(c.ctrl || c.alt || c.win) {
            return block(format!("{name} is used for typing. Hold Ctrl, Alt or Win with it."));
        }
        if modifier_only && c.vk != 0 && !matches!(c.vk, 0xA3 | 0xA5) {
            return block(format!(
                "{name} on its own is part of too many other shortcuts. Try Right Ctrl, Right Alt or Ctrl + Win."
            ));
        }
        let others = [
            (ShortcutRole::Dictation.name(), &self.ptt_hotkey, Some(ShortcutRole::Dictation)),
            (ShortcutRole::QuickFix.name(), &self.quick_fix_hotkey, Some(ShortcutRole::QuickFix)),
            (ShortcutRole::Composer.name(), &self.composer_hotkey, Some(ShortcutRole::Composer)),
            ("mode-cycle", &self.mode_cycle_hotkey, None),
            ("cancel", &self.cancel_hotkey, None),
        ];
        let others: Vec<_> = others.into_iter().filter(|(_, _, r)| *r != Some(role)).collect();
        if let Some((what, _, _)) = others.iter().find(|(_, o, _)| same_chord(o, c)) {
            return block(format!("{name} is already the {what} shortcut."));
        }
        let within = |small: &HotkeySpec, big: &HotkeySpec| {
            (!small.ctrl || big.ctrl) && (!small.alt || big.alt) && (!small.shift || big.shift) && (!small.win || big.win)
        };
        let ptt = &self.ptt_hotkey;
        if role != ShortcutRole::Dictation && ptt.vk == 0 && within(ptt, c) {
            return Some(ShortcutProblem {
                message: format!(
                    "Holding {} starts dictation first; KeyReed drops that recording as soon as {name} is complete.",
                    ptt.display_name
                ),
                blocks: false,
            });
        }
        if role == ShortcutRole::Dictation && c.vk == 0 {
            if let Some((what, o, _)) = others.iter().find(|(_, o, _)| o.vk != 0 && !is_modifier(o.vk) && within(c, o)) {
                return Some(ShortcutProblem {
                    message: format!(
                        "{name} also starts the {what} shortcut ({}); dictation starts first and is dropped when it completes.",
                        o.display_name
                    ),
                    blocks: false,
                });
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTRL: u16 = 0xA2;
    const WIN: u16 = 0x5B;
    const G: u16 = 0x47;

    #[test]
    fn capture_ends_at_the_key_or_when_modifiers_are_released() {
        let mut c = ShortcutCapture::default();
        assert_eq!(c.key(CTRL, true, false), CaptureStep::Pending);
        assert_eq!(c.key(WIN, true, false), CaptureStep::Pending);
        assert_eq!(c.held_label(), "Ctrl + Win + ...");
        let CaptureStep::Done(s) = c.key(G, true, false) else { panic!("expected a chord") };
        assert_eq!((s.ctrl, s.win, s.alt, s.vk, s.display_name.as_str()), (true, true, false, G, "Ctrl + Win + G"));

        // Modifier-only: counts for dictation when every modifier is up, not before.
        let mut c = ShortcutCapture::default();
        c.key(CTRL, true, true);
        c.key(WIN, true, true);
        assert_eq!(c.key(WIN, false, true), CaptureStep::Pending);
        let CaptureStep::Done(s) = c.key(CTRL, false, true) else { panic!("expected a chord") };
        assert_eq!((s.vk, s.display_name.as_str()), (0, "Ctrl + Win"));

        // A single modifier keeps its side, like the Right Ctrl preset.
        let mut c = ShortcutCapture::default();
        c.key(0xA3, true, true);
        assert_eq!(c.key(0xA3, false, true), CaptureStep::Done(HotkeySpec::right_ctrl()));

        // Not allowed for other chords: waiting continues and the next try starts clean.
        let mut c = ShortcutCapture::default();
        c.key(CTRL, true, false);
        assert_eq!(c.key(CTRL, false, false), CaptureStep::Pending);
        let CaptureStep::Done(s) = c.key(0x77, true, false) else { panic!("expected a chord") };
        assert_eq!(s.display_name, "F8");

        // Esc alone cancels; with a modifier it is a chord; key-ups of plain keys are ignored.
        let mut c = ShortcutCapture::default();
        assert_eq!(c.key(G, false, false), CaptureStep::Pending);
        assert_eq!(c.key(0x1B, true, false), CaptureStep::Cancelled);
        c.key(CTRL, true, false);
        assert!(matches!(c.key(0x1B, true, false), CaptureStep::Done(_)));
    }

    #[test]
    fn key_names_are_plain() {
        assert_eq!(key_name(0x41), "A");
        assert_eq!(key_name(0x7B), "F12");
        assert_eq!(key_name(0x20), "Space");
        assert_eq!(key_name(0xBF), "/");
        assert_eq!(key_name(0xFF), "Key 0xFF");
        assert_eq!(chord(true, false, true, false, 0x20).display_name, HotkeySpec::ctrl_shift_space().display_name);
    }

    #[test]
    fn conflict_rules() {
        let cfg = AppConfig::default();
        let blocks = |role, s: &HotkeySpec| cfg.shortcut_problem(role, s).is_some_and(|p| p.blocks);
        let warns = |role, s: &HotkeySpec| cfg.shortcut_problem(role, s).is_some_and(|p| !p.blocks);
        use ShortcutRole::*;
        // The defaults are clean.
        for r in [Dictation, QuickFix, Composer] {
            assert_eq!(cfg.shortcut_problem(r, cfg.shortcut(r)), None, "{r:?}");
        }
        // Reserved editing chords and typing keys.
        assert!(blocks(QuickFix, &chord(true, false, false, false, 0x43)));
        assert!(blocks(Dictation, &chord(false, false, false, false, G)));
        assert!(blocks(QuickFix, &chord(false, false, true, false, G)));
        assert!(!blocks(Dictation, &chord(false, false, false, false, 0x77)), "F8 alone is fine");
        // Modifier-only only for dictation, and not a lone left modifier or Shift.
        assert!(blocks(QuickFix, &chord(true, false, false, true, 0)));
        assert!(!blocks(Dictation, &chord(true, false, false, true, 0)));
        assert!(blocks(Dictation, &modifier_only(1 << 4)), "Left Shift");
        assert!(blocks(Dictation, &modifier_only(1)), "Left Ctrl");
        assert!(!blocks(Dictation, &modifier_only(1 << 3)), "Right Alt");
        // KeyReed's own chords, by chord rather than by name.
        let mut qf_as_ptt = cfg.quick_fix_hotkey.clone();
        qf_as_ptt.display_name = "whatever".into();
        assert!(blocks(Dictation, &qf_as_ptt));
        assert!(blocks(QuickFix, &cfg.composer_hotkey));
        assert!(blocks(Composer, &cfg.mode_cycle_hotkey));
        assert!(!blocks(QuickFix, &cfg.quick_fix_hotkey), "its own chord is not a conflict");
        // Prefix conflicts warn, both ways round.
        let mut c2 = cfg.clone();
        c2.ptt_hotkey = HotkeySpec::default(); // Ctrl + Win
        assert!(c2.shortcut_problem(QuickFix, &c2.quick_fix_hotkey).is_some_and(|p| !p.blocks));
        assert!(c2.shortcut_problem(Dictation, &c2.ptt_hotkey).is_some_and(|p| !p.blocks));
        assert!(!warns(QuickFix, &chord(true, true, false, false, G)), "no Ctrl + Win prefix with the default push-to-talk");
    }
}
