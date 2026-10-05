//! Keybind parsing (ARCHITECTURE.md §8.1 `keybinds`, UI_SPEC.md §15).
//!
//! The keybinds map stores human strings (`"Ctrl+Shift+P"`). The shell parses
//! them once with [`Keybind::parse`] and routes egui shortcuts through
//! [`Keybind::matches`] (or the free-function form
//! [`crate::keybind_matches`]).

use std::fmt;

use egui::{Key, KeyboardShortcut, Modifiers};

/// A parsed keybind: modifier set plus one key. Modifier-less bindings
/// (`"Tab"` for accept-ghost-text, per UI_SPEC.md §15) are valid.
///
/// `Ctrl` here is egui's "command" sense: the Control key on Windows/Linux
/// (egui sets both `ctrl` and `command` there) and the ⌘ key on macOS. The
/// physical Mac ⌃ is not addressable — Windows-first (PLAN.md §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Keybind {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: Key,
}

impl Keybind {
    /// Parse a keybind string such as `"Ctrl+Shift+P"`.
    ///
    /// Case-insensitive (including `"ctrl"`), whitespace around `+` is
    /// tolerated, and the modifiers may arrive in any order. Accepts the
    /// alias spellings `control`/`cmd`/`command`/`win`/`super`/`meta` for
    /// Ctrl and `option`/`opt` for Alt.
    ///
    /// Returns `None` for garbage: empty strings, unknown tokens, two keys,
    /// or a binding with no key at all.
    pub fn parse(s: &str) -> Option<Keybind> {
        let mut keybind = Keybind {
            ctrl: false,
            alt: false,
            shift: false,
            key: Key::A, // placeholder; replaced below
        };
        let mut key: Option<Key> = None;

        for token in s.split('+') {
            let token = token.trim().to_ascii_lowercase();
            if token.is_empty() {
                return None;
            }
            match token.as_str() {
                "ctrl" | "control" | "cmd" | "command" | "win" | "super" | "meta" => {
                    keybind.ctrl = true;
                }
                "shift" => keybind.shift = true,
                "alt" | "opt" | "option" => keybind.alt = true,
                _ => {
                    if key.is_some() {
                        return None; // e.g. "Ctrl+Shift+P+L"
                    }
                    key = Some(parse_key(&token)?);
                }
            }
        }

        keybind.key = key?;
        Some(keybind)
    }

    /// Build a [`Keybind`] from an egui shortcut, as delivered by the
    /// platform integration. `command`/`ctrl` collapse into [`Keybind::ctrl`].
    pub fn from_shortcut(shortcut: &KeyboardShortcut) -> Keybind {
        Keybind {
            ctrl: shortcut.modifiers.ctrl || shortcut.modifiers.command,
            alt: shortcut.modifiers.alt,
            shift: shortcut.modifiers.shift,
            key: shortcut.logical_key,
        }
    }

    /// Does this keybind match the given egui shortcut? The routing helper
    /// the shell calls per key event. `mac_cmd` is ignored (Windows-first);
    /// `ctrl` and `command` are treated as the same modifier.
    pub fn matches(&self, shortcut: &KeyboardShortcut) -> bool {
        let m: &Modifiers = &shortcut.modifiers;
        let ctrl = m.ctrl || m.command;
        ctrl == self.ctrl
            && m.shift == self.shift
            && m.alt == self.alt
            && shortcut.logical_key == self.key
    }

    /// The egui [`Modifiers`] for this keybind (Ctrl mapped to `command`,
    /// which is `ctrl` on Windows/Linux and ⌘ on macOS).
    pub fn modifiers(&self) -> Modifiers {
        Modifiers {
            alt: self.alt,
            ctrl: self.ctrl,
            shift: self.shift,
            mac_cmd: false,
            command: self.ctrl,
        }
    }

    /// The canonical display form, e.g. `"Ctrl+Shift+P"` (the same shape
    /// [`Keybind::parse`] accepts — see the round-trip tests). Provided by
    /// `Display`/`ToString`; the canonical string is built by
    /// [`Keybind::canonical`].
    pub fn canonical(&self) -> String {
        let mut s = String::new();
        if self.ctrl {
            s.push_str("Ctrl+");
        }
        if self.alt {
            s.push_str("Alt+");
        }
        if self.shift {
            s.push_str("Shift+");
        }
        s.push_str(key_label(self.key));
        s
    }
}

impl fmt::Display for Keybind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.canonical())
    }
}

/// The free-function form of [`Keybind::matches`]:
/// `matches(&Keybind, &egui::KeyboardShortcut)`.
pub fn keybind_matches(keybind: &Keybind, shortcut: &KeyboardShortcut) -> bool {
    keybind.matches(shortcut)
}

/// Parse a single (lowercased) key token. Keep in sync with [`key_label`].
fn parse_key(token: &str) -> Option<Key> {
    let key = match token {
        "a" => Key::A,
        "b" => Key::B,
        "c" => Key::C,
        "d" => Key::D,
        "e" => Key::E,
        "f" => Key::F,
        "g" => Key::G,
        "h" => Key::H,
        "i" => Key::I,
        "j" => Key::J,
        "k" => Key::K,
        "l" => Key::L,
        "m" => Key::M,
        "n" => Key::N,
        "o" => Key::O,
        "p" => Key::P,
        "q" => Key::Q,
        "r" => Key::R,
        "s" => Key::S,
        "t" => Key::T,
        "u" => Key::U,
        "v" => Key::V,
        "w" => Key::W,
        "x" => Key::X,
        "y" => Key::Y,
        "z" => Key::Z,
        "0" => Key::Num0,
        "1" => Key::Num1,
        "2" => Key::Num2,
        "3" => Key::Num3,
        "4" => Key::Num4,
        "5" => Key::Num5,
        "6" => Key::Num6,
        "7" => Key::Num7,
        "8" => Key::Num8,
        "9" => Key::Num9,
        "f1" => Key::F1,
        "f2" => Key::F2,
        "f3" => Key::F3,
        "f4" => Key::F4,
        "f5" => Key::F5,
        "f6" => Key::F6,
        "f7" => Key::F7,
        "f8" => Key::F8,
        "f9" => Key::F9,
        "f10" => Key::F10,
        "f11" => Key::F11,
        "f12" => Key::F12,
        "enter" | "return" => Key::Enter,
        "esc" | "escape" => Key::Escape,
        "tab" => Key::Tab,
        "space" => Key::Space,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "insert" => Key::Insert,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "up" | "arrowup" => Key::ArrowUp,
        "down" | "arrowdown" => Key::ArrowDown,
        "left" | "arrowleft" => Key::ArrowLeft,
        "right" | "arrowright" => Key::ArrowRight,
        "," | "comma" => Key::Comma,
        "." | "period" => Key::Period,
        "/" | "slash" => Key::Slash,
        "-" | "minus" => Key::Minus,
        "\\" | "backslash" => Key::Backslash,
        _ => return None,
    };
    Some(key)
}

/// The canonical label for a key. Keep in sync with [`parse_key`]; unknown
/// keys fall back to egui's own name (display-only — they may not re-parse).
fn key_label(key: Key) -> &'static str {
    match key {
        Key::A => "A",
        Key::B => "B",
        Key::C => "C",
        Key::D => "D",
        Key::E => "E",
        Key::F => "F",
        Key::G => "G",
        Key::H => "H",
        Key::I => "I",
        Key::J => "J",
        Key::K => "K",
        Key::L => "L",
        Key::M => "M",
        Key::N => "N",
        Key::O => "O",
        Key::P => "P",
        Key::Q => "Q",
        Key::R => "R",
        Key::S => "S",
        Key::T => "T",
        Key::U => "U",
        Key::V => "V",
        Key::W => "W",
        Key::X => "X",
        Key::Y => "Y",
        Key::Z => "Z",
        Key::Num0 => "0",
        Key::Num1 => "1",
        Key::Num2 => "2",
        Key::Num3 => "3",
        Key::Num4 => "4",
        Key::Num5 => "5",
        Key::Num6 => "6",
        Key::Num7 => "7",
        Key::Num8 => "8",
        Key::Num9 => "9",
        Key::F1 => "F1",
        Key::F2 => "F2",
        Key::F3 => "F3",
        Key::F4 => "F4",
        Key::F5 => "F5",
        Key::F6 => "F6",
        Key::F7 => "F7",
        Key::F8 => "F8",
        Key::F9 => "F9",
        Key::F10 => "F10",
        Key::F11 => "F11",
        Key::F12 => "F12",
        Key::Enter => "Enter",
        Key::Escape => "Esc",
        Key::Tab => "Tab",
        Key::Space => "Space",
        Key::Backspace => "Backspace",
        Key::Delete => "Delete",
        Key::Insert => "Insert",
        Key::Home => "Home",
        Key::End => "End",
        Key::PageUp => "PageUp",
        Key::PageDown => "PageDown",
        Key::ArrowUp => "Up",
        Key::ArrowDown => "Down",
        Key::ArrowLeft => "Left",
        Key::ArrowRight => "Right",
        Key::Comma => "Comma",
        Key::Period => "Period",
        Key::Slash => "Slash",
        Key::Minus => "Minus",
        Key::Backslash => "Backslash",
        other => other.name(),
    }
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn shortcut(ctrl: bool, shift: bool, alt: bool, key: Key) -> KeyboardShortcut {
        KeyboardShortcut::new(
            Modifiers {
                alt,
                ctrl,
                shift,
                mac_cmd: false,
                command: ctrl, // egui on Windows sets command = ctrl
            },
            key,
        )
    }

    #[test]
    fn parses_all_default_keybinds() {
        let defaults = default_keybinds();
        for (name, binding) in [
            ("command_palette", &defaults.command_palette),
            ("fuzzy_find", &defaults.fuzzy_find),
            ("toggle_chat", &defaults.toggle_chat),
            ("save", &defaults.save),
        ] {
            let kb = Keybind::parse(binding).unwrap_or_else(|| panic!("{name} must parse"));
            assert_eq!(
                kb.to_string(),
                *binding,
                "{name} round-trips through Display"
            );
        }
    }

    fn default_keybinds() -> crate::KeybindsSettings {
        crate::KeybindsSettings::default()
    }

    #[test]
    fn parse_is_case_insensitive_and_trim_tolerant() {
        // "ctrl" in any case, spaces around '+'.
        let kb = Keybind::parse(" ctrl + shift + p ").expect("must parse");
        assert_eq!(
            kb,
            Keybind {
                ctrl: true,
                alt: false,
                shift: true,
                key: Key::P
            }
        );
        assert_eq!(kb.to_string(), "Ctrl+Shift+P");

        assert_eq!(Keybind::parse("CTRL+P"), Keybind::parse("Ctrl+P"));
        assert_eq!(Keybind::parse("Command+P"), Keybind::parse("Ctrl+P"));
        assert_eq!(Keybind::parse("Option+S"), Keybind::parse("Alt+S"));
    }

    #[test]
    fn parse_accepts_modifierless_keys_and_aliases() {
        let kb = Keybind::parse("Tab").expect("modifier-less binding");
        assert_eq!(kb.key, Key::Tab);
        assert!(!kb.ctrl && !kb.alt && !kb.shift);

        let kb = Keybind::parse("Ctrl+,").expect("comma key");
        assert_eq!(kb.key, Key::Comma);
        assert_eq!(kb.to_string(), "Ctrl+Comma");

        let kb = Keybind::parse("ctrl+f5").expect("function key");
        assert_eq!(kb.key, Key::F5);

        // Alt+Ctrl order does not matter.
        assert_eq!(Keybind::parse("Alt+Ctrl+X"), Keybind::parse("Ctrl+Alt+X"));
    }

    #[test]
    fn parse_garbage_returns_none() {
        for garbage in [
            "",               // empty
            "   ",            // whitespace
            "hello world",    // unknown key, and a space (not a '+')
            "Ctrl+",          // no key
            "Ctrl",           // modifiers only
            "Shift",          // modifiers only
            "abc",            // unknown multi-letter token
            "Ctrl+??",        // unknown punctuation
            "Ctrl+Shift+P+L", // two keys
            "Ctrl++P",        // empty token between '+'
        ] {
            assert!(
                Keybind::parse(garbage).is_none(),
                "expected {garbage:?} to be rejected"
            );
        }
    }

    #[test]
    fn matches_round_trips_for_all_defaults() {
        let defaults = crate::KeybindsSettings::default();
        let cases = [
            (&defaults.command_palette, Key::P, true, true, false),
            (&defaults.fuzzy_find, Key::P, true, false, false),
            (&defaults.toggle_chat, Key::L, true, false, false),
            (&defaults.save, Key::S, true, false, false),
        ];
        for (binding, key, ctrl, shift, alt) in cases {
            let kb = Keybind::parse(binding).expect("default parses");
            assert!(
                kb.matches(&shortcut(ctrl, shift, alt, key)),
                "{binding} must match its own shortcut"
            );
        }
    }

    #[test]
    fn matches_rejects_wrong_key_and_modifiers() {
        let save = Keybind::parse(&crate::KeybindsSettings::default().save).expect("parses");
        // Wrong key.
        assert!(!save.matches(&shortcut(true, false, false, Key::X)));
        // Missing modifier.
        assert!(!save.matches(&shortcut(false, false, false, Key::S)));
        // Extra modifier.
        assert!(!save.matches(&shortcut(true, false, true, Key::S)));
        assert!(!save.matches(&shortcut(true, true, false, Key::S)));

        // command == ctrl on Windows, so a command-only shortcut matches Ctrl.
        let command_only = KeyboardShortcut::new(
            Modifiers {
                ctrl: false,
                command: true,
                ..Default::default()
            },
            Key::S,
        );
        assert!(save.matches(&command_only));

        // mac_cmd alone does NOT count as Ctrl (Windows-first: it is never
        // set on Windows, and on Mac it is the physical ⌃).
        let mac_cmd_only = KeyboardShortcut::new(
            Modifiers {
                mac_cmd: true,
                ..Default::default()
            },
            Key::S,
        );
        assert!(!save.matches(&mac_cmd_only));
    }

    #[test]
    fn free_function_matches_helper_agrees() {
        let palette = Keybind::parse("Ctrl+Shift+P").expect("parses");
        let yes = shortcut(true, true, false, Key::P);
        let no = shortcut(true, false, false, Key::P);
        assert!(keybind_matches(&palette, &yes));
        assert!(!keybind_matches(&palette, &no));
        assert_eq!(keybind_matches(&palette, &yes), palette.matches(&yes));
    }

    #[test]
    fn from_shortcut_round_trips() {
        let kb = Keybind::parse("Ctrl+Shift+P").expect("parses");
        let rebuilt = Keybind::from_shortcut(&KeyboardShortcut::new(kb.modifiers(), Key::P));
        assert_eq!(rebuilt, kb);
        assert_eq!(rebuilt.to_string(), "Ctrl+Shift+P");
    }

    #[test]
    fn display_round_trips_through_parse() {
        for s in [
            "Ctrl+Shift+P",
            "Ctrl+P",
            "Ctrl+L",
            "Ctrl+S",
            "Ctrl+Alt+Delete",
            "F12",
            "Ctrl+Comma",
        ] {
            let kb = Keybind::parse(s).unwrap_or_else(|| panic!("{s} must parse"));
            let redisplayed = kb.to_string();
            assert_eq!(
                Keybind::parse(&redisplayed),
                Some(kb),
                "{redisplayed} must re-parse to the same keybind"
            );
        }
    }
}
