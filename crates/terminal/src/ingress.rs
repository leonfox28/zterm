use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use zterm_core::terminal::{
    MAX_SIDE_EVENTS_PER_UPDATE, MAX_TITLE_BYTES, RejectedEffect, TerminalClipboardWrite,
    TerminalHostEffect, TerminalSideEvent, TerminalSize, UnsupportedSequenceKind,
};

use crate::engine::AlacrittyEngine;
use crate::{
    MAX_CONTROL_SEQUENCE_BYTES, MAX_CONTROL_STRING_BYTES, MAX_OSC52_BASE64_BYTES,
    MAX_REPLY_BYTES_PER_UPDATE,
};

const PRIMARY_DEVICE_ATTRIBUTES_REPLY: &[u8] = b"\x1b[?1;2c";
const DEVICE_STATUS_OK_REPLY: &[u8] = b"\x1b[0n";

#[derive(Clone, Copy)]
enum StringKind {
    Osc,
    Dcs,
    Apc,
    Pm,
    Sos,
}

struct Sequence {
    bytes: Vec<u8>,
    input_bytes: usize,
    overflowed: bool,
}

impl Sequence {
    fn new(prefix: &[u8]) -> Self {
        Self {
            bytes: prefix.to_vec(),
            input_bytes: prefix.len(),
            overflowed: false,
        }
    }

    fn push(&mut self, byte: u8) {
        if self.note_input_byte() {
            self.bytes.push(byte);
        }
    }

    fn note_input_byte(&mut self) -> bool {
        let retain = self.input_bytes < MAX_CONTROL_SEQUENCE_BYTES && !self.overflowed;
        self.input_bytes = self.input_bytes.saturating_add(1);
        if !retain {
            self.overflowed = true;
        }
        retain
    }
}

struct ControlString {
    kind: StringKind,
    bytes: Vec<u8>,
    overflowed: bool,
    saw_escape: bool,
    clipboard: bool,
}

impl ControlString {
    fn new(kind: StringKind) -> Self {
        Self {
            kind,
            bytes: Vec::new(),
            overflowed: false,
            saw_escape: false,
            clipboard: false,
        }
    }

    fn push(&mut self, byte: u8) {
        let maximum = if self.clipboard {
            MAX_OSC52_BASE64_BYTES.saturating_add(b"52;c;".len())
        } else {
            MAX_CONTROL_STRING_BYTES
        };
        if self.bytes.len() < maximum {
            self.bytes.push(byte);
            if matches!(self.kind, StringKind::Osc) && self.bytes == b"52;" {
                self.clipboard = true;
            }
        } else {
            self.overflowed = true;
        }
    }
}

enum PolicyState {
    Ground,
    Escape(Sequence),
    Csi(Sequence),
    String(ControlString),
}

pub(crate) struct UpdateCollector {
    replies: Vec<u8>,
    events: Vec<TerminalSideEvent>,
    dropped_events: u64,
    host_effect: Option<TerminalHostEffect>,
}

impl UpdateCollector {
    pub(crate) fn new() -> Self {
        Self {
            replies: Vec::new(),
            events: Vec::new(),
            dropped_events: 0,
            host_effect: None,
        }
    }

    pub(crate) fn push_event(&mut self, event: TerminalSideEvent) {
        if self.events.len() < MAX_SIDE_EVENTS_PER_UPDATE {
            self.events.push(event);
        } else {
            self.dropped_events = self.dropped_events.saturating_add(1);
        }
    }

    fn push_reply(&mut self, reply: &[u8]) -> Result<(), IngressError> {
        if self.replies.len().saturating_add(reply.len()) > MAX_REPLY_BYTES_PER_UPDATE {
            return Err(IngressError::ReplyOverflow);
        }
        self.replies.extend_from_slice(reply);
        Ok(())
    }

    fn set_host_effect(&mut self, effect: TerminalHostEffect) {
        self.host_effect = Some(effect);
    }

    pub(crate) fn finish(
        mut self,
    ) -> (Vec<u8>, Vec<TerminalSideEvent>, Option<TerminalHostEffect>) {
        if self.dropped_events > 0 {
            if self.events.len() == MAX_SIDE_EVENTS_PER_UPDATE {
                self.events.pop();
                self.dropped_events = self.dropped_events.saturating_add(1);
            }
            self.events.push(TerminalSideEvent::EventsDropped {
                count: self.dropped_events,
            });
        }
        (self.replies, self.events, self.host_effect)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IngressError {
    ReplyOverflow,
}

pub(crate) struct TerminalIngressPolicy {
    state: PolicyState,
    utf8: Vec<u8>,
}

impl Default for TerminalIngressPolicy {
    fn default() -> Self {
        Self {
            state: PolicyState::Ground,
            utf8: Vec::with_capacity(4),
        }
    }
}

impl TerminalIngressPolicy {
    pub(crate) fn process(
        &mut self,
        bytes: &[u8],
        engine: &mut AlacrittyEngine,
        output: &mut UpdateCollector,
    ) -> Result<(), IngressError> {
        for byte in bytes {
            self.process_byte(*byte, engine, output)?;
            for event in engine.take_events() {
                output.push_event(event);
            }
        }
        Ok(())
    }

    fn process_byte(
        &mut self,
        byte: u8,
        engine: &mut AlacrittyEngine,
        output: &mut UpdateCollector,
    ) -> Result<(), IngressError> {
        let state = std::mem::replace(&mut self.state, PolicyState::Ground);
        self.state = match state {
            PolicyState::Ground => self.process_ground(byte, engine, output),
            PolicyState::Escape(mut sequence) => {
                if matches!(byte, 0x18 | 0x1a) {
                    PolicyState::Ground
                } else if byte == 0x1b {
                    // ESC is an ECMA-48 "anywhere" transition. Discard the
                    // incomplete sequence and restart framing so a nested
                    // CSI/OSC introducer cannot be passed through to the
                    // upstream parser as part of the old sequence.
                    PolicyState::Escape(Sequence::new(b"\x1b"))
                } else if let Some(state) = c1_introducer(byte) {
                    state
                } else if is_embedded_control(byte) {
                    if sequence.note_input_byte() {
                        dispatch_embedded_control(byte, engine, output);
                    }
                    PolicyState::Escape(sequence)
                } else if byte >= 0x80 {
                    if sequence.note_input_byte() {
                        output.push_event(TerminalSideEvent::UnsupportedSequence(
                            UnsupportedSequenceKind::Control,
                        ));
                    }
                    PolicyState::Escape(sequence)
                } else if byte == b'[' && sequence.bytes.len() == 1 && !sequence.overflowed {
                    sequence.push(byte);
                    PolicyState::Csi(sequence)
                } else if sequence.bytes.len() == 1 && !sequence.overflowed {
                    match byte {
                        b']' => PolicyState::String(ControlString::new(StringKind::Osc)),
                        b'P' => PolicyState::String(ControlString::new(StringKind::Dcs)),
                        b'_' => PolicyState::String(ControlString::new(StringKind::Apc)),
                        b'^' => PolicyState::String(ControlString::new(StringKind::Pm)),
                        b'X' => PolicyState::String(ControlString::new(StringKind::Sos)),
                        _ => {
                            sequence.push(byte);
                            if is_escape_final(byte) {
                                self.dispatch_escape(sequence, engine, output);
                                PolicyState::Ground
                            } else {
                                PolicyState::Escape(sequence)
                            }
                        }
                    }
                } else {
                    sequence.push(byte);
                    if is_escape_final(byte) {
                        self.dispatch_escape(sequence, engine, output);
                        PolicyState::Ground
                    } else {
                        PolicyState::Escape(sequence)
                    }
                }
            }
            PolicyState::Csi(mut sequence) => {
                if matches!(byte, 0x18 | 0x1a) {
                    PolicyState::Ground
                } else if byte == 0x1b {
                    PolicyState::Escape(Sequence::new(b"\x1b"))
                } else if let Some(state) = c1_introducer(byte) {
                    state
                } else if is_embedded_control(byte) {
                    if sequence.note_input_byte() {
                        dispatch_embedded_control(byte, engine, output);
                    }
                    PolicyState::Csi(sequence)
                } else if byte >= 0x80 {
                    if sequence.note_input_byte() {
                        output.push_event(TerminalSideEvent::UnsupportedSequence(
                            UnsupportedSequenceKind::Control,
                        ));
                    }
                    PolicyState::Csi(sequence)
                } else {
                    sequence.push(byte);
                    if is_csi_final(byte) {
                        self.dispatch_csi(sequence, engine, output)?;
                        PolicyState::Ground
                    } else {
                        PolicyState::Csi(sequence)
                    }
                }
            }
            PolicyState::String(mut string) => {
                if matches!(byte, 0x18 | 0x1a) {
                    PolicyState::Ground
                } else if byte == 0x9c || (matches!(string.kind, StringKind::Osc) && byte == 0x07) {
                    self.dispatch_string(string, output);
                    PolicyState::Ground
                } else if string.saw_escape && byte == b'\\' {
                    string.saw_escape = false;
                    self.dispatch_string(string, output);
                    PolicyState::Ground
                } else {
                    if string.saw_escape {
                        string.push(0x1b);
                        string.saw_escape = false;
                    }
                    if byte == 0x1b {
                        string.saw_escape = true;
                    } else {
                        string.push(byte);
                    }
                    PolicyState::String(string)
                }
            }
        };
        Ok(())
    }

    fn process_ground(
        &mut self,
        byte: u8,
        engine: &mut AlacrittyEngine,
        output: &mut UpdateCollector,
    ) -> PolicyState {
        if byte == 0x1b {
            self.flush_partial_utf8(engine);
            return PolicyState::Escape(Sequence::new(b"\x1b"));
        }
        if self.utf8.is_empty()
            && let Some(state) = c1_introducer(byte)
        {
            return state;
        }
        if byte < 0x20 || byte == 0x7f {
            self.flush_partial_utf8(engine);
            if byte == 0x07 {
                output.push_event(TerminalSideEvent::AudibleBell);
            } else {
                engine.feed_raw(&[byte]);
            }
            return PolicyState::Ground;
        }
        if byte < 0x80 && self.utf8.is_empty() {
            engine.feed_raw(&[byte]);
            return PolicyState::Ground;
        }

        self.utf8.push(byte);
        self.drain_utf8(engine, output);
        PolicyState::Ground
    }

    fn drain_utf8(&mut self, engine: &mut AlacrittyEngine, output: &mut UpdateCollector) {
        loop {
            match std::str::from_utf8(&self.utf8) {
                Ok(text) if !text.is_empty() => {
                    let Some(character) = text.chars().next() else {
                        break;
                    };
                    let bytes = self.utf8.clone();
                    self.utf8.clear();
                    if engine.accept_character(character) {
                        engine.feed_raw(&bytes);
                    } else {
                        output.push_event(TerminalSideEvent::UnsupportedSequence(
                            UnsupportedSequenceKind::Character,
                        ));
                    }
                    break;
                }
                Ok(_) => break,
                Err(error) if error.error_len().is_none() && self.utf8.len() < 4 => break,
                Err(error) => {
                    let invalid = error.error_len().unwrap_or(1).min(self.utf8.len());
                    let remainder = self.utf8.split_off(invalid);
                    engine.feed_raw(&self.utf8);
                    self.utf8 = remainder;
                    if self.utf8.is_empty() {
                        break;
                    }
                }
            }
        }
    }

    fn flush_partial_utf8(&mut self, engine: &mut AlacrittyEngine) {
        if !self.utf8.is_empty() {
            engine.feed_raw(&self.utf8);
            self.utf8.clear();
        }
    }

    fn dispatch_escape(
        &self,
        sequence: Sequence,
        engine: &mut AlacrittyEngine,
        output: &mut UpdateCollector,
    ) {
        if sequence.overflowed {
            output.push_event(TerminalSideEvent::UnsupportedSequence(
                UnsupportedSequenceKind::Escape,
            ));
        } else if sequence.bytes == b"\x1bg" {
            output.push_event(TerminalSideEvent::VisualBell);
        } else if sequence.bytes == b"\x1bc" {
            engine.feed_reset(&sequence.bytes);
        } else {
            engine.feed_raw(&sequence.bytes);
        }
    }

    fn dispatch_csi(
        &self,
        sequence: Sequence,
        engine: &mut AlacrittyEngine,
        output: &mut UpdateCollector,
    ) -> Result<(), IngressError> {
        if sequence.overflowed || sequence.bytes.len() < 3 {
            output.push_event(TerminalSideEvent::UnsupportedSequence(
                UnsupportedSequenceKind::Csi,
            ));
            return Ok(());
        }
        let final_byte = *sequence.bytes.last().unwrap_or(&0);
        let body = &sequence.bytes[2..sequence.bytes.len() - 1];
        let (marker, parameters) = parse_parameters(body);

        if final_byte == b'c' && marker.is_none() && matches!(parameters.as_deref(), Some([] | [0]))
        {
            return output.push_reply(PRIMARY_DEVICE_ATTRIBUTES_REPLY);
        }
        if final_byte == b'n' {
            match (marker, parameters.as_deref()) {
                (None, Some([5])) => return output.push_reply(DEVICE_STATUS_OK_REPLY),
                (None, Some([6])) => return output.push_reply(&engine.cursor_report(false)),
                (Some(b'?'), Some([6])) => return output.push_reply(&engine.cursor_report(true)),
                _ => {
                    output.push_event(TerminalSideEvent::UnsupportedSequence(
                        UnsupportedSequenceKind::Csi,
                    ));
                    return Ok(());
                }
            }
        }
        if final_byte == b't' {
            if marker.is_none()
                && let Some([8, rows, columns]) = parameters.as_deref()
                && *rows > 0
                && *columns > 0
            {
                output.push_event(TerminalSideEvent::ResizeRequested(TerminalSize::new(
                    *rows, *columns,
                )));
            } else {
                output.push_event(TerminalSideEvent::UnsupportedSequence(
                    UnsupportedSequenceKind::Csi,
                ));
            }
            return Ok(());
        }
        if final_byte == b'u' {
            return dispatch_keyboard_mode(sequence, marker, parameters, engine, output);
        }
        if matches!(final_byte, b'h' | b'l') && marker == Some(b'?') {
            let Some(parameters) = parameters else {
                output.push_event(TerminalSideEvent::UnsupportedSequence(
                    UnsupportedSequenceKind::Csi,
                ));
                return Ok(());
            };
            let enabled = final_byte == b'h';
            let mut forwarded = Vec::new();
            for parameter in parameters {
                match parameter {
                    9 => engine.set_legacy_x10_mouse(enabled),
                    2026 => output.push_event(TerminalSideEvent::UnsupportedSequence(
                        UnsupportedSequenceKind::Csi,
                    )),
                    value => forwarded.push(value),
                }
            }
            if !forwarded.is_empty() {
                let changes_screen = forwarded
                    .iter()
                    .any(|value| matches!(value, 47 | 1047 | 1049));
                let body = forwarded
                    .iter()
                    .map(u16::to_string)
                    .collect::<Vec<_>>()
                    .join(";");
                let sequence = format!("\x1b[?{body}{}", char::from(final_byte));
                if changes_screen {
                    engine.feed_screen_transition(sequence.as_bytes());
                } else {
                    engine.feed_raw(sequence.as_bytes());
                }
            }
            return Ok(());
        }
        let sgr_extra = final_byte == b'm' && sgr_contains_underline_color(body);
        let unsupported = matches!(final_byte, b'b' | b'c') || body.contains(&b'$') || sgr_extra;
        if unsupported {
            output.push_event(TerminalSideEvent::UnsupportedSequence(
                UnsupportedSequenceKind::Csi,
            ));
        } else {
            engine.feed_raw(&sequence.bytes);
        }
        Ok(())
    }

    fn dispatch_string(&self, string: ControlString, output: &mut UpdateCollector) {
        if string.overflowed {
            if string.clipboard {
                output.push_event(TerminalSideEvent::EffectRejected(
                    RejectedEffect::ClipboardWrite,
                ));
            } else {
                let kind = if matches!(string.kind, StringKind::Osc) {
                    UnsupportedSequenceKind::Osc
                } else {
                    UnsupportedSequenceKind::Control
                };
                output.push_event(TerminalSideEvent::UnsupportedSequence(kind));
            }
            return;
        }
        if !matches!(string.kind, StringKind::Osc) {
            output.push_event(TerminalSideEvent::UnsupportedSequence(
                UnsupportedSequenceKind::Control,
            ));
            return;
        }
        let mut fields = string.bytes.splitn(2, |byte| *byte == b';');
        let command = fields.next().unwrap_or_default();
        let payload = fields.next().unwrap_or_default();
        match command {
            b"0" | b"2" => {
                let (title, truncated) = bounded_text(payload);
                output.push_event(TerminalSideEvent::TitleChanged { title, truncated });
            }
            b"1" => {
                let (icon_name, truncated) = bounded_text(payload);
                output.push_event(TerminalSideEvent::IconNameChanged {
                    icon_name,
                    truncated,
                });
            }
            b"52" => {
                dispatch_clipboard(payload, output);
            }
            _ => output.push_event(TerminalSideEvent::UnsupportedSequence(
                UnsupportedSequenceKind::Osc,
            )),
        }
    }
}

fn dispatch_keyboard_mode(
    sequence: Sequence,
    marker: Option<u8>,
    parameters: Option<Vec<u16>>,
    engine: &mut AlacrittyEngine,
    output: &mut UpdateCollector,
) -> Result<(), IngressError> {
    let admitted = match (marker, parameters.as_deref()) {
        (Some(b'?'), Some([])) => {
            let reply = format!("\x1b[?{}u", engine.keyboard_mode_bits());
            return output.push_reply(reply.as_bytes());
        }
        (Some(b'='), Some([] | [0..=0x1f]))
        | (Some(b'='), Some([0..=0x1f, 0..=3]))
        | (Some(b'>'), Some([] | [0..=0x1f]))
        | (Some(b'<'), Some([]))
        | (Some(b'<'), Some([_])) => true,
        _ => false,
    };
    if !admitted {
        output.push_event(TerminalSideEvent::UnsupportedSequence(
            UnsupportedSequenceKind::Csi,
        ));
        return Ok(());
    }

    engine.feed_raw(&sequence.bytes);
    Ok(())
}

fn dispatch_clipboard(payload: &[u8], output: &mut UpdateCollector) {
    let mut fields = payload.splitn(2, |byte| *byte == b';');
    let selector = fields.next().unwrap_or_default();
    let data = fields.next().unwrap_or_default();
    if data == b"?" {
        output.push_event(TerminalSideEvent::EffectRejected(
            RejectedEffect::ClipboardRead,
        ));
        return;
    }
    let value = (selector == b"c" && !data.is_empty())
        .then(|| BASE64_STANDARD.decode(data).ok())
        .flatten()
        .and_then(|decoded| String::from_utf8(decoded).ok())
        .and_then(|text| TerminalClipboardWrite::new(text).ok());
    if let Some(value) = value {
        output.set_host_effect(TerminalHostEffect::ClipboardWrite(value));
    } else {
        output.push_event(TerminalSideEvent::EffectRejected(
            RejectedEffect::ClipboardWrite,
        ));
    }
}

fn c1_introducer(byte: u8) -> Option<PolicyState> {
    match byte {
        0x9b => Some(PolicyState::Csi(Sequence::new(b"\x1b["))),
        0x9d => Some(PolicyState::String(ControlString::new(StringKind::Osc))),
        0x90 => Some(PolicyState::String(ControlString::new(StringKind::Dcs))),
        0x9f => Some(PolicyState::String(ControlString::new(StringKind::Apc))),
        0x9e => Some(PolicyState::String(ControlString::new(StringKind::Pm))),
        0x98 => Some(PolicyState::String(ControlString::new(StringKind::Sos))),
        _ => None,
    }
}

fn is_embedded_control(byte: u8) -> bool {
    matches!(byte, 0x00..=0x17 | 0x19 | 0x1c..=0x1f | 0x7f)
}

fn dispatch_embedded_control(byte: u8, engine: &mut AlacrittyEngine, output: &mut UpdateCollector) {
    match byte {
        0x07 => output.push_event(TerminalSideEvent::AudibleBell),
        0x7f => {}
        _ => engine.feed_raw(&[byte]),
    }
}

fn is_escape_final(byte: u8) -> bool {
    (0x30..=0x7e).contains(&byte)
}

fn is_csi_final(byte: u8) -> bool {
    (0x40..=0x7e).contains(&byte)
}

fn parse_parameters(body: &[u8]) -> (Option<u8>, Option<Vec<u16>>) {
    let (marker, parameters) = body
        .split_first()
        .filter(|(first, _)| matches!(**first, b'?' | b'>' | b'<' | b'='))
        .map_or((None, body), |(first, rest)| (Some(*first), rest));
    if parameters.is_empty() {
        return (marker, Some(Vec::new()));
    }
    let mut parsed = Vec::new();
    for value in parameters.split(|byte| *byte == b';') {
        if value.is_empty() {
            parsed.push(0);
            continue;
        }
        let Ok(text) = std::str::from_utf8(value) else {
            return (marker, None);
        };
        let Ok(number) = text.parse::<u16>() else {
            return (marker, None);
        };
        parsed.push(number);
    }
    (marker, Some(parsed))
}

fn sgr_contains_underline_color(body: &[u8]) -> bool {
    let parameters = body.split(|byte| *byte == b';').collect::<Vec<_>>();
    let mut index = 0;
    while index < parameters.len() {
        let parameter = parameters[index];
        let value = decimal_parameter(
            parameter
                .split(|byte| *byte == b':')
                .next()
                .unwrap_or_default(),
        );
        if matches!(value, Some(58 | 59)) {
            return true;
        }

        // Semicolon-form foreground/background colors consume their mode and
        // color components as SGR parameters. Do not mistake an RGB component
        // or palette index of 58/59 for a top-level underline-color attribute.
        if !parameter.contains(&b':') && matches!(value, Some(38 | 48)) {
            let color_mode = parameters.get(index + 1).and_then(|parameter| {
                decimal_parameter(
                    parameter
                        .split(|byte| *byte == b':')
                        .next()
                        .unwrap_or_default(),
                )
            });
            index = index.saturating_add(match color_mode {
                Some(2) => 5,
                Some(5) => 3,
                _ => 2,
            });
        } else {
            index += 1;
        }
    }
    false
}

fn decimal_parameter(bytes: &[u8]) -> Option<u16> {
    if bytes.is_empty() {
        return Some(0);
    }
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

fn bounded_text(bytes: &[u8]) -> (String, bool) {
    let truncated = bytes.len() > MAX_TITLE_BYTES;
    let retained = bytes.get(..MAX_TITLE_BYTES).unwrap_or(bytes);
    (String::from_utf8_lossy(retained).into_owned(), truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameter_parser_is_bounded_and_strict() {
        assert_eq!(parse_parameters(b"?6"), (Some(b'?'), Some(vec![6])));
        assert_eq!(parse_parameters(b"8;24;80"), (None, Some(vec![8, 24, 80])));
        assert_eq!(parse_parameters(b"38:2:1:2:3"), (None, None));
    }

    #[test]
    fn underline_color_detection_observes_sgr_parameter_boundaries() {
        assert!(sgr_contains_underline_color(b"058;5;1"));
        assert!(sgr_contains_underline_color(b"1;58:2::1:2:3"));
        assert!(sgr_contains_underline_color(b"38;5;58;59"));
        assert!(!sgr_contains_underline_color(b"38;2;58;59;60"));
        assert!(!sgr_contains_underline_color(b"48:2::58:59:60"));
    }
}
