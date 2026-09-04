use zterm_core::terminal::{TerminalKeyboardFlags, TerminalModes};

const MOD_SHIFT: u8 = 1 << 0;
const MOD_ALT: u8 = 1 << 1;
const MOD_CONTROL: u8 = 1 << 2;
const MOD_SUPER: u8 = 1 << 3;
const MOD_HYPER: u8 = 1 << 4;
const MOD_META: u8 = 1 << 5;
const MOD_CAPS_LOCK: u8 = 1 << 6;
const MOD_NUM_LOCK: u8 = 1 << 7;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum KeyEventKind {
    Press,
    Repeat,
    Release,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FunctionalKey {
    Escape,
    Enter,
    Tab,
    Backspace,
    Insert,
    Delete,
    Left,
    Right,
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    Begin,
    Menu,
    Function(u8),
    Keypad(u8),
    KeypadDecimal,
    KeypadDivide,
    KeypadMultiply,
    KeypadSubtract,
    KeypadAdd,
    KeypadEnter,
    KeypadEqual,
    KeypadSeparator,
    KeypadLeft,
    KeypadRight,
    KeypadUp,
    KeypadDown,
    KeypadPageUp,
    KeypadPageDown,
    KeypadHome,
    KeypadEnd,
    KeypadInsert,
    KeypadDelete,
    Other(u32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum KeyCode {
    Unicode(u32),
    Functional(FunctionalKey),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct EnhancedKey {
    code: KeyCode,
    shifted: Option<char>,
    base_layout: Option<char>,
    modifiers: u8,
    pub(super) kind: KeyEventKind,
    text: Vec<char>,
    pub(super) raw: Vec<u8>,
}

impl EnhancedKey {
    pub(super) fn parse(raw: Vec<u8>) -> Option<Self> {
        if !raw.starts_with(b"\x1b[") || raw.len() < 3 {
            return None;
        }
        let final_byte = *raw.last()?;
        let parameters = raw.get(2..raw.len() - 1)?.to_vec();
        match final_byte {
            b'u' => Self::parse_csi_u(raw, &parameters),
            b'~' => Self::parse_tilde(raw, &parameters),
            b'A' | b'B' | b'C' | b'D' | b'E' | b'F' | b'H' | b'P' | b'Q' | b'R' | b'S' => {
                Self::parse_letter(raw, &parameters, final_byte)
            }
            _ => None,
        }
    }

    fn parse_csi_u(raw: Vec<u8>, parameters: &[u8]) -> Option<Self> {
        let mut fields = parameters.split(|byte| *byte == b';');
        let key_field = fields.next()?;
        let modifiers_field = fields.next();
        let text_field = fields.next();
        if fields.next().is_some() {
            return None;
        }

        let mut keys = key_field.split(|byte| *byte == b':');
        let primary = parse_u32(keys.next()?)?;
        let shifted_field = keys.next();
        let base_field = keys.next();
        if keys.next().is_some() {
            return None;
        }
        let shifted = parse_optional_char(shifted_field)?;
        let base_layout = parse_optional_char(base_field)?;
        if shifted_field.is_some_and(<[u8]>::is_empty) && base_field.is_none() {
            return None;
        }
        let (modifiers, kind) = parse_modifiers(modifiers_field)?;
        let text = parse_text(text_field)?;
        let code = key_code_from_number(primary);
        Some(Self {
            code,
            shifted,
            base_layout,
            modifiers,
            kind,
            text,
            raw,
        })
    }

    fn parse_tilde(raw: Vec<u8>, parameters: &[u8]) -> Option<Self> {
        let (key, modifiers, kind) = parse_function_parameters(parameters)?;
        let code = match key {
            2 => FunctionalKey::Insert,
            3 => FunctionalKey::Delete,
            5 => FunctionalKey::PageUp,
            6 => FunctionalKey::PageDown,
            7 => FunctionalKey::Home,
            8 => FunctionalKey::End,
            11 => FunctionalKey::Function(1),
            12 => FunctionalKey::Function(2),
            13 => FunctionalKey::Function(3),
            14 => FunctionalKey::Function(4),
            15 => FunctionalKey::Function(5),
            17 => FunctionalKey::Function(6),
            18 => FunctionalKey::Function(7),
            19 => FunctionalKey::Function(8),
            20 => FunctionalKey::Function(9),
            21 => FunctionalKey::Function(10),
            23 => FunctionalKey::Function(11),
            24 => FunctionalKey::Function(12),
            29 => FunctionalKey::Menu,
            _ => return None,
        };
        Some(Self::functional(raw, code, modifiers, kind))
    }

    fn parse_letter(raw: Vec<u8>, parameters: &[u8], final_byte: u8) -> Option<Self> {
        let (key, modifiers, kind) = if parameters.is_empty() {
            (1, 0, KeyEventKind::Press)
        } else {
            parse_function_parameters(parameters)?
        };
        if key != 1 {
            return None;
        }
        let code = match final_byte {
            b'A' => FunctionalKey::Up,
            b'B' => FunctionalKey::Down,
            b'C' => FunctionalKey::Right,
            b'D' => FunctionalKey::Left,
            b'E' => FunctionalKey::Begin,
            b'F' => FunctionalKey::End,
            b'H' => FunctionalKey::Home,
            b'P' => FunctionalKey::Function(1),
            b'Q' => FunctionalKey::Function(2),
            b'R' => FunctionalKey::Function(3),
            b'S' => FunctionalKey::Function(4),
            _ => return None,
        };
        Some(Self::functional(raw, code, modifiers, kind))
    }

    fn functional(
        raw: Vec<u8>,
        code: FunctionalKey,
        modifiers: u8,
        kind: KeyEventKind,
    ) -> Self {
        Self {
            code: KeyCode::Functional(code),
            shifted: None,
            base_layout: None,
            modifiers,
            kind,
            text: Vec::new(),
            raw,
        }
    }

    pub(super) fn is_copy_shortcut(&self) -> bool {
        let is_c = match self.code {
            KeyCode::Unicode(code) => matches!(code, 99 | 67),
            KeyCode::Functional(_) => false,
        } || self.shifted.is_some_and(|key| matches!(key, 'c' | 'C'))
            || self
                .base_layout
                .is_some_and(|key| matches!(key, 'c' | 'C'));
        let required = self.modifiers & (MOD_CONTROL | MOD_SUPER) != 0;
        let forbidden = self.modifiers & (MOD_ALT | MOD_HYPER | MOD_META) != 0;
        is_c && required && !forbidden
    }

    fn lease_key(&self) -> CopyLeaseKey {
        CopyLeaseKey {
            code: self.code,
            shifted: self.shifted,
            base_layout: self.base_layout,
            modifiers: self.modifiers & !(MOD_CAPS_LOCK | MOD_NUM_LOCK),
        }
    }

    pub(super) fn legacy_bytes(&self, modes: TerminalModes) -> Vec<u8> {
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

        let control_shift_space = primary == ' '
            && modifiers & MOD_CONTROL != 0
            && modifiers & MOD_SHIFT != 0;
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
                if modifiers & MOD_CONTROL != 0 { 0x08 } else { 0x7f },
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
            FunctionalKey::KeypadPageDown => {
                self.legacy_functional(FunctionalKey::PageDown, modes)
            }
            FunctionalKey::KeypadHome => self.legacy_functional(FunctionalKey::Home, modes),
            FunctionalKey::KeypadEnd => self.legacy_functional(FunctionalKey::End, modes),
            FunctionalKey::KeypadInsert => self.legacy_functional(FunctionalKey::Insert, modes),
            FunctionalKey::KeypadDelete => self.legacy_functional(FunctionalKey::Delete, modes),
            FunctionalKey::Function(_) => self.without_event_type(),
            FunctionalKey::Other(_) => self.raw.clone(),
        }
    }

    fn legacy_keypad_text(&self, character: char, modifiers: u8) -> Vec<u8> {
        let mut key = self.clone();
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CopyLeaseKey {
    code: KeyCode,
    shifted: Option<char>,
    base_layout: Option<char>,
    modifiers: u8,
}

#[derive(Default)]
pub(super) struct CopyKeyLease {
    active: Option<CopyLeaseKey>,
}

impl CopyKeyLease {
    pub(super) fn consume(&mut self, key: &EnhancedKey) -> bool {
        let Some(active) = self.active else {
            return false;
        };
        if active != key.lease_key() {
            return false;
        }
        match key.kind {
            KeyEventKind::Press => false,
            KeyEventKind::Repeat => true,
            KeyEventKind::Release => {
                self.active = None;
                true
            }
        }
    }

    pub(super) fn begin(&mut self, key: &EnhancedKey) {
        self.active = Some(key.lease_key());
    }
}

fn parse_function_parameters(parameters: &[u8]) -> Option<(u32, u8, KeyEventKind)> {
    let mut fields = parameters.split(|byte| *byte == b';');
    let key = parse_u32(fields.next()?)?;
    let modifiers = fields.next();
    if fields.next().is_some() {
        return None;
    }
    let (modifiers, kind) = parse_modifiers(modifiers)?;
    Some((key, modifiers, kind))
}

fn parse_modifiers(field: Option<&[u8]>) -> Option<(u8, KeyEventKind)> {
    let Some(field) = field else {
        return Some((0, KeyEventKind::Press));
    };
    let mut values = field.split(|byte| *byte == b':');
    let encoded = match values.next()? {
        [] => 1,
        value => parse_u16(value)?,
    };
    if !(1..=256).contains(&encoded) {
        return None;
    }
    let kind = match values.next() {
        None => KeyEventKind::Press,
        Some(b"1") => KeyEventKind::Press,
        Some(b"2") => KeyEventKind::Repeat,
        Some(b"3") => KeyEventKind::Release,
        Some(_) => return None,
    };
    if values.next().is_some() {
        return None;
    }
    Some((u8::try_from(encoded - 1).ok()?, kind))
}

fn parse_text(field: Option<&[u8]>) -> Option<Vec<char>> {
    let Some(field) = field else {
        return Some(Vec::new());
    };
    if field.is_empty() {
        return Some(Vec::new());
    }
    field
        .split(|byte| *byte == b':')
        .map(|value| char::from_u32(parse_u32(value)?))
        .collect()
}

fn parse_optional_char(field: Option<&[u8]>) -> Option<Option<char>> {
    match field {
        None | Some([]) => Some(None),
        Some(value) => char::from_u32(parse_u32(value)?).map(Some),
    }
}

fn parse_u16(bytes: &[u8]) -> Option<u16> {
    u16::try_from(parse_u32(bytes)?).ok()
}

fn parse_u32(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    bytes.iter().try_fold(0_u32, |value, byte| {
        value
            .checked_mul(10)?
            .checked_add(u32::from(*byte - b'0'))
    })
}

fn key_code_from_number(code: u32) -> KeyCode {
    let functional = match code {
        27 => FunctionalKey::Escape,
        13 => FunctionalKey::Enter,
        9 => FunctionalKey::Tab,
        127 => FunctionalKey::Backspace,
        57_348 => FunctionalKey::Insert,
        57_349 => FunctionalKey::Delete,
        57_350 => FunctionalKey::Left,
        57_351 => FunctionalKey::Right,
        57_352 => FunctionalKey::Up,
        57_353 => FunctionalKey::Down,
        57_354 => FunctionalKey::PageUp,
        57_355 => FunctionalKey::PageDown,
        57_356 => FunctionalKey::Home,
        57_357 => FunctionalKey::End,
        57_358..=57_362 => FunctionalKey::Other(code),
        57_363 => FunctionalKey::Menu,
        57_364..=57_398 => FunctionalKey::Function((code - 57_363) as u8),
        57_399..=57_408 => FunctionalKey::Keypad((code - 57_399) as u8),
        57_409 => FunctionalKey::KeypadDecimal,
        57_410 => FunctionalKey::KeypadDivide,
        57_411 => FunctionalKey::KeypadMultiply,
        57_412 => FunctionalKey::KeypadSubtract,
        57_413 => FunctionalKey::KeypadAdd,
        57_414 => FunctionalKey::KeypadEnter,
        57_415 => FunctionalKey::KeypadEqual,
        57_416 => FunctionalKey::KeypadSeparator,
        57_417 => FunctionalKey::KeypadLeft,
        57_418 => FunctionalKey::KeypadRight,
        57_419 => FunctionalKey::KeypadUp,
        57_420 => FunctionalKey::KeypadDown,
        57_421 => FunctionalKey::KeypadPageUp,
        57_422 => FunctionalKey::KeypadPageDown,
        57_423 => FunctionalKey::KeypadHome,
        57_424 => FunctionalKey::KeypadEnd,
        57_425 => FunctionalKey::KeypadInsert,
        57_426 => FunctionalKey::KeypadDelete,
        57_427 => FunctionalKey::Begin,
        57_428..=63_743 => FunctionalKey::Other(code),
        _ => return KeyCode::Unicode(code),
    };
    KeyCode::Functional(functional)
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

fn legacy_control(character: char) -> Option<u8> {
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
    format!("\x1b[1;{}{final_byte}", u16::from(modifiers) + 1).into_bytes()
}

fn tilde_key(parameter: u16, modifiers: u8) -> Vec<u8> {
    if modifiers == 0 {
        format!("\x1b[{parameter}~").into_bytes()
    } else {
        format!("\x1b[{parameter};{}~", u16::from(modifiers) + 1).into_bytes()
    }
}

fn keypad_operator(
    key: &EnhancedKey,
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

pub(super) fn desired_outer_keyboard_flags(
    child: TerminalKeyboardFlags,
    copy_ready: bool,
) -> TerminalKeyboardFlags {
    if child.is_empty() && copy_ready {
        TerminalKeyboardFlags::DISAMBIGUATE_ESCAPE_CODES
            .union(TerminalKeyboardFlags::REPORT_EVENT_TYPES)
            .union(TerminalKeyboardFlags::REPORT_ALTERNATE_KEYS)
    } else {
        child
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_csi_u_and_rejects_invalid_fields() {
        let key = EnhancedKey::parse(b"\x1b[99:67:99;14:2;67u".to_vec()).expect("key");
        assert_eq!(key.code, KeyCode::Unicode(99));
        assert_eq!(key.shifted, Some('C'));
        assert_eq!(key.base_layout, Some('c'));
        assert_eq!(key.modifiers, MOD_SHIFT | MOD_CONTROL | MOD_SUPER);
        assert_eq!(key.kind, KeyEventKind::Repeat);
        assert_eq!(key.text, vec!['C']);

        for invalid in [
            b"\x1b[u".as_slice(),
            b"\x1b[99;0u",
            b"\x1b[99;257u",
            b"\x1b[99;5:4u",
            b"\x1b[99:;5u",
            b"\x1b[99;5;55296u",
            b"\x1b[99;5;67;68u",
        ] {
            assert!(EnhancedKey::parse(invalid.to_vec()).is_none(), "{invalid:?}");
        }
    }

    #[test]
    fn copy_shortcut_and_lease_cover_press_repeat_release() {
        let press = EnhancedKey::parse(b"\x1b[99;5:1u".to_vec()).expect("press");
        let repeat = EnhancedKey::parse(b"\x1b[99;5:2u".to_vec()).expect("repeat");
        let release = EnhancedKey::parse(b"\x1b[99;5:3u".to_vec()).expect("release");
        assert!(press.is_copy_shortcut());
        assert!(
            EnhancedKey::parse(b"\x1b[99:67;6:1u".to_vec())
                .expect("Ctrl+Shift+C")
                .is_copy_shortcut()
        );
        assert!(
            EnhancedKey::parse(b"\x1b[99;9:1u".to_vec())
                .expect("Super+C")
                .is_copy_shortcut()
        );
        assert!(
            !EnhancedKey::parse(b"\x1b[99;7:1u".to_vec())
                .expect("Alt+Ctrl+C")
                .is_copy_shortcut()
        );

        let mut lease = CopyKeyLease::default();
        lease.begin(&press);
        assert!(!lease.consume(&press));
        assert!(lease.consume(&repeat));
        assert!(lease.consume(&release));
        assert!(!lease.consume(&repeat));
    }

    #[test]
    fn legacy_downgrade_covers_text_function_and_keypad_modes() {
        let modes = TerminalModes::default();
        let ctrl = EnhancedKey::parse(b"\x1b[120;5:1u".to_vec()).expect("ctrl x");
        assert_eq!(ctrl.legacy_bytes(modes), vec![0x18]);
        let ctrl_shift = EnhancedKey::parse(b"\x1b[120:88;6:1u".to_vec()).expect("ctrl shift x");
        assert_eq!(ctrl_shift.legacy_bytes(modes), b"\x1b[120;6u");

        let up = EnhancedKey::parse(b"\x1b[1;1:1A".to_vec()).expect("up");
        assert_eq!(up.legacy_bytes(modes), b"\x1b[A");
        assert_eq!(
            up.legacy_bytes(TerminalModes {
                application_cursor: true,
                ..TerminalModes::default()
            }),
            b"\x1bOA"
        );

        let f3 = EnhancedKey::parse(b"\x1b[1;1:1R".to_vec()).expect("F3");
        assert_eq!(f3.legacy_bytes(modes), b"\x1bOR");

        let keypad = EnhancedKey::parse(b"\x1b[57400;1:1u".to_vec()).expect("kp1");
        assert_eq!(keypad.legacy_bytes(modes), b"1");
        assert_eq!(
            keypad.legacy_bytes(TerminalModes {
                application_keypad: true,
                ..TerminalModes::default()
            }),
            b"\x1bOq"
        );

        let raw_unknown = b"\x1b[57358;1:2u";
        let unknown = EnhancedKey::parse(raw_unknown.to_vec()).expect("unknown functional key");
        assert_eq!(unknown.legacy_bytes(modes), raw_unknown);
    }

    #[test]
    fn desired_outer_mode_is_protocol_complete_for_a_local_selection() {
        assert_eq!(
            desired_outer_keyboard_flags(TerminalKeyboardFlags::default(), true).bits(),
            7
        );
        let child = TerminalKeyboardFlags::from_bits(9).expect("known keyboard flags");
        assert_eq!(desired_outer_keyboard_flags(child, true).bits(), 9);
    }
}
