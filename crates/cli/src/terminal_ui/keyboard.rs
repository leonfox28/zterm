use zterm_core::terminal::{TerminalKeyboardFlags, TerminalModes};

use zterm_client::keyboard::{FunctionalKey, KeyCode, KeyInput, legacy_control,
    MOD_SHIFT, MOD_ALT, MOD_CONTROL, MOD_SUPER, MOD_HYPER, MOD_META, MOD_CAPS_LOCK, MOD_NUM_LOCK};
pub(super) use zterm_client::keyboard::KeyEventKind;

/// A binding is a key and exact modifiers, independent of its wire encoding.
#[derive(Clone, Copy)]
pub(super) struct Shortcut {
    character: char,
    modifiers: u8,
}

impl Shortcut {
    pub(super) const fn plain(character: char) -> Self {
        Self {
            character,
            modifiers: 0,
        }
    }

    pub(super) const fn control(character: char) -> Self {
        Self {
            character,
            modifiers: MOD_CONTROL,
        }
    }

    pub(super) fn matches_legacy(self, byte: u8) -> bool {
        match self.modifiers {
            0 => self.character == char::from(byte),
            MOD_CONTROL => legacy_control(self.character) == Some(byte),
            _ => false,
        }
    }
}

/// Reported key identity for owning releases even if Ctrl was released first.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) struct KeyIdentity(KeyCode);

impl KeyIdentity {
    pub(super) fn from_legacy(byte: u8) -> Option<Self> {
        if matches!(byte, b'\t' | b'\r' | 0x7f) {
            return Some(Self(key_code_from_number(u32::from(byte))));
        }
        let code = match byte {
            0x1d => u32::from(']'),
            1..=26 => u32::from(b'a' + byte - 1),
            b' '..=b'~' => u32::from(byte.to_ascii_lowercase()),
            _ => return None,
        };
        Some(Self(KeyCode::Unicode(code)))
    }
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
    pub(super) fn matches_shortcut(&self, shortcut: Shortcut) -> bool {
        let modifiers = self.modifiers & !(MOD_CAPS_LOCK | MOD_NUM_LOCK);
        modifiers == shortcut.modifiers
            && (self.code == KeyCode::Unicode(u32::from(shortcut.character))
                || self.base_layout == Some(shortcut.character)
                || modifiers & MOD_SHIFT != 0 && self.shifted == Some(shortcut.character))
    }

    pub(super) fn identity(&self) -> KeyIdentity {
        KeyIdentity(
            self.base_layout
                .map_or(self.code, |key| KeyCode::Unicode(u32::from(key))),
        )
    }

    pub(super) fn is_modifier(&self) -> bool {
        matches!(
            self.code,
            KeyCode::Functional(FunctionalKey::Other(57441..=57452))
        )
    }
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

    fn functional(raw: Vec<u8>, code: FunctionalKey, modifiers: u8, kind: KeyEventKind) -> Self {
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
            || self.base_layout.is_some_and(|key| matches!(key, 'c' | 'C'));
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
        KeyInput { code: self.code, shifted: self.shifted, modifiers: self.modifiers,
            kind: self.kind, text: &self.text, raw: &self.raw }.legacy_bytes(modes)
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
    pub(super) fn owns(&self, key: &EnhancedKey, selection_finalized: bool) -> bool {
        (self.active == Some(key.lease_key()) && key.kind != KeyEventKind::Press)
            || (selection_finalized && key.kind == KeyEventKind::Press && key.is_copy_shortcut())
    }
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
        value.checked_mul(10)?.checked_add(u32::from(*byte - b'0'))
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
            assert!(
                EnhancedKey::parse(invalid.to_vec()).is_none(),
                "{invalid:?}"
            );
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
