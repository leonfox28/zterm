//! Mode-aware typed key encoding shared by native input and the desktop adapter.
use zterm_core::terminal::{TerminalKeyboardFlags, TerminalModes};

/// Keyboard protocol modifier bit.
pub const MOD_SHIFT: u8 = 1 << 0;
/// Keyboard protocol modifier bit.
pub const MOD_ALT: u8 = 1 << 1;
/// Keyboard protocol modifier bit.
pub const MOD_CONTROL: u8 = 1 << 2;
/// Keyboard protocol modifier bit.
pub const MOD_SUPER: u8 = 1 << 3;
/// Keyboard protocol modifier bit.
pub const MOD_HYPER: u8 = 1 << 4;
/// Keyboard protocol modifier bit.
pub const MOD_META: u8 = 1 << 5;
/// Keyboard protocol modifier bit.
pub const MOD_CAPS_LOCK: u8 = 1 << 6;
/// Keyboard protocol modifier bit.
pub const MOD_NUM_LOCK: u8 = 1 << 7;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Typed terminal key identity, independent of a platform event parser.
pub enum KeyEventKind {
    /// Corresponding terminal key or key event.
    Press,
    /// Corresponding terminal key or key event.
    Repeat,
    /// Corresponding terminal key or key event.
    Release,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Typed terminal key identity, independent of a platform event parser.
pub enum FunctionalKey {
    /// Corresponding terminal key or key event.
    Escape,
    /// Corresponding terminal key or key event.
    Enter,
    /// Corresponding terminal key or key event.
    Tab,
    /// Corresponding terminal key or key event.
    Backspace,
    /// Corresponding terminal key or key event.
    Insert,
    /// Corresponding terminal key or key event.
    Delete,
    /// Corresponding terminal key or key event.
    Left,
    /// Corresponding terminal key or key event.
    Right,
    /// Corresponding terminal key or key event.
    Up,
    /// Corresponding terminal key or key event.
    Down,
    /// Corresponding terminal key or key event.
    PageUp,
    /// Corresponding terminal key or key event.
    PageDown,
    /// Corresponding terminal key or key event.
    Home,
    /// Corresponding terminal key or key event.
    End,
    /// Corresponding terminal key or key event.
    Begin,
    /// Corresponding terminal key or key event.
    Menu,
    /// Corresponding terminal key or key event.
    Function(u8),
    /// Corresponding terminal key or key event.
    Keypad(u8),
    /// Corresponding terminal key or key event.
    KeypadDecimal,
    /// Corresponding terminal key or key event.
    KeypadDivide,
    /// Corresponding terminal key or key event.
    KeypadMultiply,
    /// Corresponding terminal key or key event.
    KeypadSubtract,
    /// Corresponding terminal key or key event.
    KeypadAdd,
    /// Corresponding terminal key or key event.
    KeypadEnter,
    /// Corresponding terminal key or key event.
    KeypadEqual,
    /// Corresponding terminal key or key event.
    KeypadSeparator,
    /// Corresponding terminal key or key event.
    KeypadLeft,
    /// Corresponding terminal key or key event.
    KeypadRight,
    /// Corresponding terminal key or key event.
    KeypadUp,
    /// Corresponding terminal key or key event.
    KeypadDown,
    /// Corresponding terminal key or key event.
    KeypadPageUp,
    /// Corresponding terminal key or key event.
    KeypadPageDown,
    /// Corresponding terminal key or key event.
    KeypadHome,
    /// Corresponding terminal key or key event.
    KeypadEnd,
    /// Corresponding terminal key or key event.
    KeypadInsert,
    /// Corresponding terminal key or key event.
    KeypadDelete,
    /// Corresponding terminal key or key event.
    Other(u32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Typed terminal key identity, independent of a platform event parser.
pub enum KeyCode {
    /// Corresponding terminal key or key event.
    Unicode(u32),
    /// Corresponding terminal key or key event.
    Functional(FunctionalKey),
}

/// A borrowed key report. Native platforms provide typed values directly;
/// only desktop adapters supply an opaque fallback from their own parser.
#[derive(Clone, Copy, Debug)]
pub struct KeyInput<'a> {
    /// Physical or Unicode identity.
    pub code: KeyCode,
    /// Shifted printable identity, if the platform supplies one.
    pub shifted: Option<char>,
    /// Protocol modifier bits.
    pub modifiers: u8,
    /// Press, repeat or release.
    pub kind: KeyEventKind,
    /// Text associated with this key.
    pub text: &'a [char],
    /// Desktop-only fallback for an unrecognized report.
    pub raw: &'a [u8],
}
impl KeyInput<'_> {
    /// Encodes a native key using the child's current progressive enhancements.
    /// Desktop raw protocol forwarding remains in the desktop parser.
    pub fn encode(&self, modes: TerminalModes) -> Vec<u8> {
        let flags = modes.keyboard_flags;
        let all = flags.contains(TerminalKeyboardFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES);
        let events = flags.contains(TerminalKeyboardFlags::REPORT_EVENT_TYPES);
        let disambiguate = all || flags.contains(TerminalKeyboardFlags::DISAMBIGUATE_ESCAPE_CODES);
        let modifiers = self.modifiers & !(MOD_CAPS_LOCK | MOD_NUM_LOCK);
        let text_key = matches!(self.code, KeyCode::Unicode(_)) && modifiers & !(MOD_SHIFT) == 0;
        let recovery_key = matches!(
            self.code,
            KeyCode::Functional(
                FunctionalKey::Enter | FunctionalKey::Tab | FunctionalKey::Backspace
            )
        );
        if flags.is_empty() || (!all && (text_key || recovery_key)) {
            return self.legacy_bytes(modes);
        }
        if self.kind == KeyEventKind::Release && !events {
            return Vec::new();
        }
        if !disambiguate && !events {
            return self.legacy_bytes(modes);
        }
        // Functional keys keep their canonical CSI-letter/tilde identity.
        let (number, suffix) = match self.code {
            KeyCode::Unicode(code) => (code, 'u'),
            KeyCode::Functional(key) => enhanced_functional(key),
        };
        let mut key_field = number.to_string();
        if suffix == 'u'
            && flags.contains(TerminalKeyboardFlags::REPORT_ALTERNATE_KEYS)
            && let Some(shifted) = self.shifted
        {
            key_field.push(':');
            key_field.push_str(&(shifted as u32).to_string());
        }
        let modifier = u16::from(if all { self.modifiers } else { modifiers }) + 1;
        let mut report = format!("\x1b[{key_field};{modifier}");
        if events && self.kind != KeyEventKind::Press {
            report.push_str(if self.kind == KeyEventKind::Repeat {
                ":2"
            } else {
                ":3"
            });
        }
        if all
            && flags.contains(TerminalKeyboardFlags::REPORT_ASSOCIATED_TEXT)
            && self.kind != KeyEventKind::Release
        {
            let text = self
                .text
                .iter()
                .filter(|c| !c.is_control())
                .map(|c| (*c as u32).to_string())
                .collect::<Vec<_>>()
                .join(":");
            if !text.is_empty() {
                report.push(';');
                report.push_str(&text);
            }
        }
        report.push(suffix);
        report.into_bytes()
    }

    /// Encodes the report using synchronized cursor/keypad modes.
    pub fn legacy_bytes(&self, modes: TerminalModes) -> Vec<u8> {
        if self.kind == KeyEventKind::Release {
            return Vec::new();
        }
        match self.code {
            KeyCode::Unicode(code) => self.legacy_unicode(code),
            KeyCode::Functional(key) => self.legacy_functional(key, modes),
        }
    }

    fn legacy_unicode(&self, code: u32) -> Vec<u8> {
        let Some(primary) = char::from_u32(code).filter(|character| *character != '\0') else {
            if self.text.is_empty() {
                return self.without_event_type();
            }
            return self.text.iter().collect::<String>().into_bytes();
        };
        let modifiers = self.modifiers & !(MOD_CAPS_LOCK | MOD_NUM_LOCK);
        if modifiers & (MOD_HYPER | MOD_META) != 0 {
            return self.without_event_type();
        }

        let mut text = if self.text.is_empty() {
            self.shifted
                .filter(|_| modifiers & MOD_SHIFT != 0)
                .unwrap_or(primary)
                .to_string()
        } else {
            self.text.iter().collect::<String>()
        };

        #[cfg(target_os = "macos")]
        if modifiers & MOD_SUPER != 0 {
            return Vec::new();
        }
        #[cfg(not(target_os = "macos"))]
        if modifiers & MOD_SUPER != 0 {
            return text.into_bytes();
        }

        let control_shift_space =
            primary == ' ' && modifiers & MOD_CONTROL != 0 && modifiers & MOD_SHIFT != 0;
        let legacy_combo = modifiers & !(MOD_SHIFT | MOD_ALT | MOD_CONTROL) == 0
            && (control_shift_space
                || !(modifiers & MOD_CONTROL != 0 && modifiers & MOD_SHIFT != 0));
        if !legacy_combo {
            return self.without_event_type();
        }

        if modifiers & MOD_CONTROL != 0 {
            let Some(control) = legacy_control(primary) else {
                return self.without_event_type();
            };
            text.clear();
            text.push(char::from(control));
        }

        let mut bytes = Vec::with_capacity(text.len().saturating_add(1));
        if modifiers & MOD_ALT != 0 {
            bytes.push(0x1b);
        }
        bytes.extend_from_slice(text.as_bytes());
        bytes
    }

    fn legacy_functional(&self, key: FunctionalKey, modes: TerminalModes) -> Vec<u8> {
        let modifiers = self.modifiers & !(MOD_CAPS_LOCK | MOD_NUM_LOCK);
        if matches!(
            key,
            FunctionalKey::Escape
                | FunctionalKey::Enter
                | FunctionalKey::Tab
                | FunctionalKey::Backspace
        ) && modifiers & (MOD_SUPER | MOD_HYPER | MOD_META) != 0
        {
            return self.without_event_type();
        }
        match key {
            FunctionalKey::Escape => c0_bytes(0x1b, modifiers, false),
            FunctionalKey::Enter => c0_bytes(b'\r', modifiers, false),
            FunctionalKey::Backspace => c0_bytes(
                if modifiers & MOD_CONTROL != 0 {
                    0x08
                } else {
                    0x7f
                },
                modifiers,
                false,
            ),
            FunctionalKey::Tab => {
                let shifted = modifiers & MOD_SHIFT != 0;
                let mut bytes = Vec::new();
                if modifiers & MOD_ALT != 0 {
                    bytes.push(0x1b);
                }
                bytes.extend_from_slice(if shifted { b"\x1b[Z" } else { b"\t" });
                bytes
            }
            FunctionalKey::Insert => tilde_key(2, modifiers),
            FunctionalKey::Delete => tilde_key(3, modifiers),
            FunctionalKey::PageUp => tilde_key(5, modifiers),
            FunctionalKey::PageDown => tilde_key(6, modifiers),
            FunctionalKey::Home => letter_key(b'H', modifiers, modes.application_cursor),
            FunctionalKey::End => letter_key(b'F', modifiers, modes.application_cursor),
            FunctionalKey::Up => letter_key(b'A', modifiers, modes.application_cursor),
            FunctionalKey::Down => letter_key(b'B', modifiers, modes.application_cursor),
            FunctionalKey::Right => letter_key(b'C', modifiers, modes.application_cursor),
            FunctionalKey::Left => letter_key(b'D', modifiers, modes.application_cursor),
            FunctionalKey::Begin => letter_key(b'E', modifiers, modes.application_cursor),
            FunctionalKey::Menu => tilde_key(29, modifiers),
            FunctionalKey::Function(number @ 1..=4) => {
                letter_key(b'P' + number - 1, modifiers, true)
            }
            FunctionalKey::Function(number @ 5..=12) => {
                let parameter = [15, 17, 18, 19, 20, 21, 23, 24][usize::from(number - 5)];
                tilde_key(parameter, modifiers)
            }
            FunctionalKey::Keypad(digit) => {
                if modes.application_keypad && modifiers == 0 {
                    vec![0x1b, b'O', b'p' + digit]
                } else {
                    self.legacy_keypad_text(char::from(b'0' + digit), modifiers)
                }
            }
            FunctionalKey::KeypadDecimal => {
                keypad_operator(self, b'n', '.', modifiers, modes.application_keypad)
            }
            FunctionalKey::KeypadDivide => {
                keypad_operator(self, b'o', '/', modifiers, modes.application_keypad)
            }
            FunctionalKey::KeypadMultiply => {
                keypad_operator(self, b'j', '*', modifiers, modes.application_keypad)
            }
            FunctionalKey::KeypadSubtract => {
                keypad_operator(self, b'm', '-', modifiers, modes.application_keypad)
            }
            FunctionalKey::KeypadAdd => {
                keypad_operator(self, b'k', '+', modifiers, modes.application_keypad)
            }
            FunctionalKey::KeypadEnter => {
                if modes.application_keypad && modifiers == 0 {
                    b"\x1bOM".to_vec()
                } else {
                    c0_bytes(b'\r', modifiers, false)
                }
            }
            FunctionalKey::KeypadEqual => {
                keypad_operator(self, b'X', '=', modifiers, modes.application_keypad)
            }
            FunctionalKey::KeypadSeparator => {
                keypad_operator(self, b'l', ',', modifiers, modes.application_keypad)
            }
            FunctionalKey::KeypadLeft => self.legacy_functional(FunctionalKey::Left, modes),
            FunctionalKey::KeypadRight => self.legacy_functional(FunctionalKey::Right, modes),
            FunctionalKey::KeypadUp => self.legacy_functional(FunctionalKey::Up, modes),
            FunctionalKey::KeypadDown => self.legacy_functional(FunctionalKey::Down, modes),
            FunctionalKey::KeypadPageUp => self.legacy_functional(FunctionalKey::PageUp, modes),
            FunctionalKey::KeypadPageDown => self.legacy_functional(FunctionalKey::PageDown, modes),
            FunctionalKey::KeypadHome => self.legacy_functional(FunctionalKey::Home, modes),
            FunctionalKey::KeypadEnd => self.legacy_functional(FunctionalKey::End, modes),
            FunctionalKey::KeypadInsert => self.legacy_functional(FunctionalKey::Insert, modes),
            FunctionalKey::KeypadDelete => self.legacy_functional(FunctionalKey::Delete, modes),
            FunctionalKey::Function(_) => self.without_event_type(),
            FunctionalKey::Other(_) => self.raw.to_vec(),
        }
    }

    fn legacy_keypad_text(&self, character: char, modifiers: u8) -> Vec<u8> {
        let mut key = *self;
        key.code = KeyCode::Unicode(character as u32);
        key.modifiers = modifiers;
        key.legacy_unicode(character as u32)
    }

    fn without_event_type(&self) -> Vec<u8> {
        let code = match self.code {
            KeyCode::Unicode(code) => code,
            KeyCode::Functional(key) => functional_number(key),
        };
        format!("\x1b[{code};{}u", u16::from(self.modifiers) + 1).into_bytes()
    }
}

fn functional_number(key: FunctionalKey) -> u32 {
    match key {
        FunctionalKey::Escape => 27,
        FunctionalKey::Enter => 13,
        FunctionalKey::Tab => 9,
        FunctionalKey::Backspace => 127,
        FunctionalKey::Menu => 57_363,
        FunctionalKey::Function(number) => 57_363 + u32::from(number),
        FunctionalKey::Keypad(digit) => 57_399 + u32::from(digit),
        FunctionalKey::KeypadDecimal => 57_409,
        FunctionalKey::KeypadDivide => 57_410,
        FunctionalKey::KeypadMultiply => 57_411,
        FunctionalKey::KeypadSubtract => 57_412,
        FunctionalKey::KeypadAdd => 57_413,
        FunctionalKey::KeypadEnter => 57_414,
        FunctionalKey::KeypadEqual => 57_415,
        FunctionalKey::KeypadSeparator => 57_416,
        FunctionalKey::KeypadLeft => 57_417,
        FunctionalKey::KeypadRight => 57_418,
        FunctionalKey::KeypadUp => 57_419,
        FunctionalKey::KeypadDown => 57_420,
        FunctionalKey::KeypadPageUp => 57_421,
        FunctionalKey::KeypadPageDown => 57_422,
        FunctionalKey::KeypadHome => 57_423,
        FunctionalKey::KeypadEnd => 57_424,
        FunctionalKey::KeypadInsert => 57_425,
        FunctionalKey::KeypadDelete => 57_426,
        FunctionalKey::Begin => 57_427,
        FunctionalKey::Other(code) => code,
        FunctionalKey::Insert => 57_348,
        FunctionalKey::Delete => 57_349,
        FunctionalKey::Left => 57_350,
        FunctionalKey::Right => 57_351,
        FunctionalKey::Up => 57_352,
        FunctionalKey::Down => 57_353,
        FunctionalKey::PageUp => 57_354,
        FunctionalKey::PageDown => 57_355,
        FunctionalKey::Home => 57_356,
        FunctionalKey::End => 57_357,
    }
}

/// Returns the legacy control byte for a printable key.
pub fn legacy_control(character: char) -> Option<u8> {
    let byte = u8::try_from(character as u32).ok()?;
    Some(match byte {
        b' ' | b'2' | b'@' => 0,
        b'a'..=b'z' => byte - b'a' + 1,
        b'A'..=b'Z' => byte - b'A' + 1,
        b'3' | b'[' => 27,
        b'4' | b'\\' => 28,
        b'5' | b']' => 29,
        b'6' | b'^' | b'~' => 30,
        b'7' | b'/' | b'_' => 31,
        b'8' | b'?' => 127,
        b'0' | b'1' | b'9' | b';' => byte,
        _ => return None,
    })
}

fn c0_bytes(byte: u8, modifiers: u8, force_shift_sequence: bool) -> Vec<u8> {
    let mut bytes = Vec::new();
    if modifiers & MOD_ALT != 0 {
        bytes.push(0x1b);
    }
    if force_shift_sequence && modifiers & MOD_SHIFT != 0 {
        bytes.extend_from_slice(b"\x1b[Z");
    } else {
        bytes.push(byte);
    }
    bytes
}

fn letter_key(final_byte: u8, modifiers: u8, application: bool) -> Vec<u8> {
    if modifiers == 0 {
        return vec![0x1b, if application { b'O' } else { b'[' }, final_byte];
    }
    format!(
        "\x1b[1;{}{}",
        u16::from(modifiers) + 1,
        char::from(final_byte)
    )
    .into_bytes()
}

fn tilde_key(parameter: u16, modifiers: u8) -> Vec<u8> {
    if modifiers == 0 {
        format!("\x1b[{parameter}~").into_bytes()
    } else {
        format!("\x1b[{parameter};{}~", u16::from(modifiers) + 1).into_bytes()
    }
}

fn keypad_operator(
    key: &KeyInput<'_>,
    application_final: u8,
    character: char,
    modifiers: u8,
    application: bool,
) -> Vec<u8> {
    if application && modifiers == 0 {
        vec![0x1b, b'O', application_final]
    } else {
        key.legacy_keypad_text(character, modifiers)
    }
}

fn enhanced_functional(key: FunctionalKey) -> (u32, char) {
    use FunctionalKey::*;
    match key {
        Insert => (2, '~'),
        Delete => (3, '~'),
        PageUp => (5, '~'),
        PageDown => (6, '~'),
        Left => (1, 'D'),
        Right => (1, 'C'),
        Up => (1, 'A'),
        Down => (1, 'B'),
        Home => (1, 'H'),
        End => (1, 'F'),
        Function(1) => (1, 'P'),
        Function(2) => (1, 'Q'),
        Function(3) => (13, '~'),
        Function(4) => (1, 'S'),
        Function(n @ 5..=12) => ([15, 17, 18, 19, 20, 21, 23, 24][usize::from(n - 5)], '~'),
        _ => (functional_number(key), 'u'),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(code: KeyCode, modifiers: u8) -> KeyInput<'static> {
        KeyInput {
            code,
            modifiers,
            shifted: None,
            kind: KeyEventKind::Press,
            text: &[],
            raw: &[],
        }
    }
    #[test]
    fn native_cursor_keys_follow_modes_and_emit_letter_not_decimal() {
        let up = key(KeyCode::Functional(FunctionalKey::Up), 0);
        assert_eq!(up.encode(TerminalModes::default()), b"\x1b[A");
        let modes = TerminalModes {
            application_cursor: true,
            ..Default::default()
        };
        assert_eq!(up.encode(modes), b"\x1bOA");
        assert_eq!(key(up.code, MOD_CONTROL).encode(modes), b"\x1b[1;5A");
    }
    #[test]
    fn native_control_alt_and_kitty_release_are_distinct() {
        let mut c = key(KeyCode::Unicode('c' as u32), MOD_CONTROL | MOD_ALT);
        assert_eq!(c.encode(TerminalModes::default()), b"\x1b\x03");
        let modes = TerminalModes {
            keyboard_flags: TerminalKeyboardFlags::DISAMBIGUATE_ESCAPE_CODES
                .union(TerminalKeyboardFlags::REPORT_EVENT_TYPES),
            ..Default::default()
        };
        assert_eq!(c.encode(modes), b"\x1b[99;7u");
        c.kind = KeyEventKind::Release;
        assert_eq!(c.encode(modes), b"\x1b[99;7:3u");
        assert!(c.encode(TerminalModes::default()).is_empty());
        c.code = KeyCode::Functional(FunctionalKey::Enter);
        assert!(c.encode(modes).is_empty());
    }
    #[test]
    fn ime_text_keeps_unicode_and_reports_no_invented_physical_key() {
        let chars: Vec<_> = "中文e\u{301}".chars().collect();
        let input = KeyInput {
            text: &chars,
            ..key(KeyCode::Unicode(0), 0)
        };
        assert_eq!(
            input.encode(TerminalModes::default()),
            "中文e\u{301}".as_bytes()
        );
        let modes = TerminalModes {
            keyboard_flags: TerminalKeyboardFlags::ALL,
            ..Default::default()
        };
        assert_eq!(input.encode(modes), b"\x1b[0;1;20013:25991:101:769u");
    }
}
