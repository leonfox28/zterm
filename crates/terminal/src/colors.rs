//! The Session's sole palette/appearance owner. Never forwards child OSC.
use std::fmt::Write as _;
use zterm_core::Revision;
use zterm_core::terminal::*;

#[derive(Default)]
pub(crate) struct ZtermColorState {
    pub(crate) base: TerminalColorProfile,
    overrides: Vec<Option<TerminalColorValue>>,
    pub(crate) snapshot: TerminalColorSnapshot,
    pub(crate) subscribed: bool,
    pub(crate) reverse: bool,
    saved: [Option<TerminalColorSnapshot>; 10],
    inherited_sources: Vec<Option<u16>>,
    depth: usize,
    stored: usize,
}
impl ZtermColorState {
    pub(crate) fn reset(&mut self) {
        self.overrides.clear();
        self.inherited_sources.clear();
        self.subscribed = false;
        self.reverse = false;
        self.saved = Default::default();
        self.depth = 0;
        self.stored = 0;
    }
    fn source(&self, slot: usize) -> usize {
        if !self.reverse {
            return slot;
        }
        match slot {
            0 => 7,
            7 => 0,
            8 => 15,
            15 => 8,
            256 => 257,
            257 => 256,
            _ => slot,
        }
    }
    fn value(&self, slot: usize) -> TerminalColorValue {
        let source = self.source(slot);
        self.overrides
            .get(source)
            .copied()
            .flatten()
            .unwrap_or(self.base.values[source])
    }
    fn set(&mut self, slot: usize, value: Option<TerminalColorValue>) {
        if self.overrides.is_empty() {
            self.overrides.resize(TERMINAL_COLOR_SLOTS, None);
        }
        let source = self.source(slot);
        self.overrides[source] = value;
        if let Some(inherited) = self.inherited_sources.get_mut(source) {
            *inherited = None;
        }
    }
    fn effective(&self) -> TerminalColorProfile {
        TerminalColorProfile {
            values: Box::new(std::array::from_fn(|slot| self.value(slot))),
            appearance: self.base.appearance,
        }
    }
    fn effective_snapshot(&self) -> TerminalColorSnapshot {
        TerminalColorSnapshot {
            profile: self.effective(),
            reverse: self.reverse,
            custom_cursor: (COLOR_CURSOR..=COLOR_CURSOR_TEXT)
                .any(|slot| self.overrides.get(slot).is_some_and(Option::is_some)),
            changed_at: self.snapshot.changed_at,
            inherited_sources: Box::new(std::array::from_fn(|slot| {
                let source = self.source(slot);
                self.inherited_sources
                    .get(source)
                    .copied()
                    .flatten()
                    .unwrap_or_else(|| u16::try_from(source).expect("bounded color source"))
            })),
        }
    }
    pub(crate) fn commit(&mut self, revision: Revision) {
        let next = self.effective_snapshot();
        if next != self.snapshot {
            self.snapshot = TerminalColorSnapshot {
                changed_at: revision,
                ..next
            };
        }
    }
    pub(crate) fn appearance_reply(&self) -> Vec<u8> {
        match self.base.appearance {
            TerminalAppearance::Dark => b"\x1b[?997;1n".to_vec(),
            TerminalAppearance::Light => b"\x1b[?997;2n".to_vec(),
            TerminalAppearance::Unknown => Vec::new(),
        }
    }
    pub(crate) fn stack(&mut self, store: bool, index: usize) {
        let slot = if index == 0 {
            if store {
                self.depth
            } else {
                let Some(v) = self.depth.checked_sub(1) else {
                    return;
                };
                v
            }
        } else {
            index - 1
        };
        if slot >= 10 {
            return;
        }
        if store {
            self.saved[slot] = Some(self.effective_snapshot());
            self.stored = self.stored.max(slot + 1);
            if index == 0 {
                self.depth += 1;
            }
        } else if let Some(saved) = self.saved[slot].clone() {
            self.inherited_sources.resize(TERMINAL_COLOR_SLOTS, None);
            for (slot, value) in saved.profile.values.into_iter().enumerate() {
                self.set(slot, Some(value));
                if value == TerminalColorValue::Unknown {
                    let source = self.source(slot);
                    self.inherited_sources[source] = Some(saved.inherited_sources[slot]);
                }
            }
            if index == 0 {
                self.depth -= 1;
            }
        }
    }
    pub(crate) fn stack_report(&self) -> Vec<u8> {
        format!("\x1b[?{};{}#Q", self.depth, self.stored).into_bytes()
    }
    pub(crate) fn osc(&mut self, command: u16, payload: &str, end: &str) -> Option<Vec<u8>> {
        let mut reply = String::new();
        match command {
            4 => {
                let mut fields = payload.split(';');
                while let (Some(index), Some(value)) = (fields.next(), fields.next()) {
                    if let Ok(slot @ 0..=255) = index.parse::<usize>() {
                        if value == "?" {
                            if let Some(rgb) = encoded(self.value(slot)) {
                                let _ = write!(reply, "\x1b]4;{slot};{rgb}{end}");
                            }
                        } else if let Some(rgb) = parse_color(value) {
                            self.set(slot, Some(rgb));
                        }
                    }
                }
            }
            10..=19 => {
                for (offset, value) in payload.split(';').enumerate() {
                    let Some(code) = usize::from(command).checked_add(offset) else {
                        break;
                    };
                    let Some(slot) = dynamic_slot(code) else {
                        continue;
                    };
                    if value == "?" {
                        if let Some(rgb) = encoded(self.value(slot)) {
                            let _ = write!(reply, "\x1b]{code};{rgb}{end}");
                        }
                    } else if let Some(rgb) = parse_color(value) {
                        self.set(slot, Some(rgb));
                    }
                }
            }
            104 => {
                if payload.is_empty() {
                    for slot in 0..256 {
                        self.set(slot, None);
                    }
                } else {
                    for field in payload.split(';') {
                        if let Ok(slot @ 0..=255) = field.parse::<usize>() {
                            self.set(slot, None);
                        }
                    }
                }
            }
            110..=119 => {
                if let Some(slot) = dynamic_slot(usize::from(command - 100)) {
                    self.set(slot, None);
                }
            }
            21 => {
                let mut answers = Vec::new();
                for item in payload.split(';') {
                    let (key, value) = item
                        .split_once('=')
                        .map_or((item, None), |(k, v)| (k, Some(v)));
                    // Unsupported key names are reflected only if they are bounded identifiers.
                    if key.is_empty()
                        || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    {
                        continue;
                    }
                    let slot = key_slot(key);
                    if value == Some("?") {
                        let value = slot.map_or_else(
                            || "?".to_owned(),
                            |slot| encoded(self.value(slot)).unwrap_or_default(),
                        );
                        answers.push(format!("{key}={value}"));
                    } else if let Some(slot) = slot {
                        match value {
                            None => self.set(slot, None),
                            Some("") if slot >= COLOR_CURSOR => {
                                self.set(slot, Some(TerminalColorValue::Dynamic))
                            }
                            Some(value) => {
                                if let Some(rgb) = parse_color(value) {
                                    self.set(slot, Some(rgb));
                                }
                            }
                        }
                    }
                }
                if !answers.is_empty() {
                    let _ = write!(reply, "\x1b]21;{}{end}", answers.join(";"));
                }
            }
            30001 | 30101 if payload.is_empty() => self.stack(command == 30001, 0),
            _ => return None,
        }
        Some(reply.into_bytes())
    }
}
fn dynamic_slot(code: usize) -> Option<usize> {
    match code {
        10 => Some(256),
        11 => Some(257),
        12 => Some(258),
        17 => Some(261),
        19 => Some(260),
        _ => None,
    }
}
fn key_slot(key: &str) -> Option<usize> {
    match key {
        "foreground" => Some(256),
        "background" => Some(257),
        "cursor" => Some(258),
        "cursor_text" => Some(259),
        "selection_foreground" => Some(260),
        "selection_background" => Some(261),
        _ => key.parse::<usize>().ok().filter(|slot| *slot < 256),
    }
}
fn encoded(value: TerminalColorValue) -> Option<String> {
    let TerminalColorValue::Rgb(r, g, b) = value else {
        return None;
    };
    Some(format!(
        "rgb:{:04x}/{:04x}/{:04x}",
        u16::from(r) * 257,
        u16::from(g) * 257,
        u16::from(b) * 257
    ))
}
fn parse_color(value: &str) -> Option<TerminalColorValue> {
    let value = match value.split_once('@') {
        Some((value, alpha)) if alpha.parse::<f64>().ok()? == 1.0 => value,
        Some(_) => return None,
        None => value,
    };
    parse_terminal_rgb(value).or_else(|| {
        let key = value.to_ascii_lowercase();
        crate::color_names::lookup(&key).map(|(r, g, b)| TerminalColorValue::Rgb(r, g, b))
    })
}

pub(crate) fn sgr(style: TerminalStyle) -> String {
    let mut s = String::from("0");
    for (active, code) in [
        (style.bold, 1),
        (style.dim, 2),
        (style.italic, 3),
        (style.inverse, 7),
    ] {
        if active {
            let _ = write!(s, ";{code}");
        }
    }
    let underline = match style.underline {
        TerminalUnderline::None => 0,
        TerminalUnderline::Single => 1,
        TerminalUnderline::Double => 2,
        TerminalUnderline::Curly => 3,
        TerminalUnderline::Dotted => 4,
        TerminalUnderline::Dashed => 5,
    };
    if underline != 0 {
        let _ = write!(s, ";4:{underline}");
    }
    for (color, code) in [
        (style.foreground, 38),
        (style.background, 48),
        (style.underline_color, 58),
    ] {
        match color {
            TerminalColor::Default => {}
            TerminalColor::Indexed(n) => {
                let _ = write!(s, ";{code};5;{n}");
            }
            TerminalColor::Rgb(r, g, b) => {
                let _ = write!(s, ";{code};2;{r};{g};{b}");
            }
        }
    }
    s.push('m');
    s
}
pub(crate) fn capabilities(payload: &str) -> Vec<u8> {
    let mut result = String::new();
    for encoded_name in payload.split(';') {
        let Some(name) = decode_hex(encoded_name) else {
            result.push_str("\x1bP0+r\x1b\\");
            break;
        };
        let value = match name.as_str() {
            "Co" | "colors" => "256",
            "RGB" => "8",
            "TN" | "name" => "xterm-256color",
            "Tc" | "Su" => "",
            "Smulx" => "\x1b[4:%p1%dm",
            "Setulc" => "\x1b[58:2::%p1%{65536}%/%d:%p1%{256}%/%{255}%&%d:%p1%{255}%&%dm",
            _ => {
                result.push_str("\x1bP0+r\x1b\\");
                break;
            }
        };
        let _ = write!(result, "\x1bP1+r{}", hex(&name));
        if !value.is_empty() {
            let _ = write!(result, "={}", hex(value));
        }
        result.push_str("\x1b\\");
    }
    result.into_bytes()
}
fn hex(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02X}")).collect()
}
fn decode_hex(s: &str) -> Option<String> {
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return None;
    }
    String::from_utf8(
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
            .collect::<Option<Vec<_>>>()?,
    )
    .ok()
}
