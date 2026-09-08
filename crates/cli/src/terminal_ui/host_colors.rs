use super::*;
use std::sync::atomic::AtomicBool;
use zterm_core::terminal::{
    COLOR_BACKGROUND, COLOR_CURSOR, COLOR_CURSOR_TEXT, COLOR_FOREGROUND,
    COLOR_SELECTION_BACKGROUND, COLOR_SELECTION_FOREGROUND, TerminalAppearance,
    TerminalColorProfile, TerminalColorValue, parse_terminal_rgb,
};

const PROBE_TIMEOUT: Duration = Duration::from_millis(250);
const PROBE_BUDGET: usize = 64 * 1024;

/// Physical replies never enter keyboard, prefix, selection, or child input paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum HostReply {
    Color(usize, TerminalColorValue),
    Appearance(TerminalAppearance),
    AppearanceMode(u16),
    Fence,
    Ignored,
}

pub(super) enum HostColorCommand {
    Probe,
    EnableAppearance,
}

struct Round {
    deadline: Instant,
    accepting: bool,
    seen_appearance: bool,
    bytes: usize,
}

pub(super) struct HostColors {
    pub(super) profile: TerminalColorProfile,
    round: Option<Round>,
    refresh: bool,
    enable: bool,
    mode_observed: bool,
    notifications: bool,
    dirty: bool,
    pub(super) owned_subscription: Arc<AtomicBool>,
}
impl Default for HostColors {
    fn default() -> Self {
        Self {
            profile: TerminalColorProfile::default(),
            round: None,
            refresh: true,
            enable: false,
            mode_observed: false,
            notifications: false,
            dirty: false,
            owned_subscription: Arc::new(AtomicBool::new(false)),
        }
    }
}
impl HostColors {
    pub(super) fn deadline(&self) -> Option<Instant> {
        self.round
            .as_ref()
            .filter(|r| r.accepting)
            .map(|r| r.deadline)
    }
    pub(super) fn initial_complete(&self) -> bool {
        !self.refresh && self.deadline().is_none()
    }
    pub(super) fn expire(&mut self, now: Instant) {
        if let Some(round) = &mut self.round
            && round.accepting
            && now >= round.deadline
        {
            round.accepting = false;
            self.dirty = true;
        }
    }
    pub(super) fn request_refresh(&mut self) {
        self.refresh = true;
    }
    pub(super) fn next_command(&mut self, now: Instant) -> Option<HostColorCommand> {
        self.expire(now);
        if self.enable {
            self.enable = false;
            // Restoration owns even a partially written enable sequence.
            self.owned_subscription.store(true, Ordering::Release);
            self.notifications = true;
            return Some(HostColorCommand::EnableAppearance);
        }
        if self.refresh && self.round.is_none() {
            self.refresh = false;
            self.round = Some(Round {
                deadline: now + PROBE_TIMEOUT,
                accepting: true,
                seen_appearance: false,
                bytes: 0,
            });
            return Some(HostColorCommand::Probe);
        }
        None
    }
    pub(super) fn take_update(&mut self) -> Option<TerminalColorProfile> {
        // Publish one completed observation, even when a previous fence queued
        // a new round before the active session consumed the previous update.
        if self.deadline().is_some() {
            return None;
        }
        std::mem::take(&mut self.dirty).then(|| self.profile.clone())
    }
    pub(super) fn observe(&mut self, reply: HostReply, bytes: usize, now: Instant) {
        self.expire(now);
        if reply == HostReply::Fence {
            if self.round.take().is_some() {
                self.dirty = true;
            }
            return;
        }
        if let Some(round) = &mut self.round {
            round.bytes = round.bytes.saturating_add(bytes);
            if round.bytes > PROBE_BUDGET {
                round.accepting = false;
                self.dirty = true;
            }
            if !round.accepting {
                if matches!(reply, HostReply::Appearance(_)) {
                    self.refresh = true;
                }
                return;
            }
        } else {
            if let HostReply::Appearance(appearance) = reply
                && self.notifications
            {
                self.profile.appearance = appearance;
                self.refresh = true;
                self.dirty = true;
            }
            return;
        }
        match reply {
            HostReply::Color(slot, value) => self.profile.values[slot] = value,
            HostReply::Appearance(value) => {
                let round = self.round.as_mut().expect("active observation round");
                if round.seen_appearance {
                    self.refresh = true;
                }
                round.seen_appearance = true;
                self.profile.appearance = value;
            }
            HostReply::AppearanceMode(mode) if !self.mode_observed => {
                self.mode_observed = true;
                self.notifications = matches!(mode, 1 | 3);
                self.enable = mode == 2;
            }
            _ => {}
        }
    }
    pub(super) fn flush_commands(
        &mut self,
        presenter: &mut DesktopPresenter,
        output: &mut impl Write,
    ) -> Result<(), CliError> {
        while let Some(command) = self.next_command(Instant::now()) {
            presenter.write_color_command(output, command)?;
        }
        Ok(())
    }
}

fn role(key: &str) -> Option<usize> {
    match key {
        "foreground" => Some(COLOR_FOREGROUND),
        "background" => Some(COLOR_BACKGROUND),
        "cursor" => Some(COLOR_CURSOR),
        "cursor_text" => Some(COLOR_CURSOR_TEXT),
        "selection_foreground" => Some(COLOR_SELECTION_FOREGROUND),
        "selection_background" => Some(COLOR_SELECTION_BACKGROUND),
        _ => key.parse::<u8>().ok().map(usize::from),
    }
}

pub(super) fn osc_replies(body: &[u8]) -> Vec<HostReply> {
    let Ok(body) = std::str::from_utf8(body) else {
        return vec![HostReply::Ignored];
    };
    let mut fields = body.split(';');
    let Some(command) = fields.next().and_then(|s| s.parse::<u16>().ok()) else {
        return vec![HostReply::Ignored];
    };
    let mut replies = Vec::new();
    if command == 4 {
        while let (Some(index), Some(value)) = (fields.next(), fields.next()) {
            if let (Ok(index), Some(value)) = (index.parse::<u8>(), parse_terminal_rgb(value)) {
                replies.push(HostReply::Color(usize::from(index), value));
            }
        }
    } else if command == 21 {
        for field in fields {
            if let Some((key, value)) = field.split_once('=')
                && let Some(slot) = role(key)
            {
                let value = if value.is_empty() && slot >= COLOR_CURSOR {
                    Some(TerminalColorValue::Dynamic)
                } else {
                    parse_terminal_rgb(value)
                };
                if let Some(value) = value {
                    replies.push(HostReply::Color(slot, value));
                }
            }
        }
    } else {
        for (offset, value) in fields.enumerate() {
            let Some(command) = u16::try_from(offset)
                .ok()
                .and_then(|offset| command.checked_add(offset))
            else {
                break;
            };
            let slot = match command {
                10 => COLOR_FOREGROUND,
                11 => COLOR_BACKGROUND,
                12 => COLOR_CURSOR,
                17 => COLOR_SELECTION_BACKGROUND,
                19 => COLOR_SELECTION_FOREGROUND,
                _ => continue,
            };
            if let Some(value) = parse_terminal_rgb(value) {
                replies.push(HostReply::Color(slot, value));
            }
        }
    }
    if replies.is_empty() {
        replies.push(HostReply::Ignored);
    }
    replies
}
pub(super) fn is_color_csi(raw: &[u8]) -> bool {
    raw.strip_prefix(b"\x1b[")
        .or_else(|| raw.strip_prefix(&[0x9b]))
        .is_some_and(|body| body.starts_with(b"?997;") || body.starts_with(b"?2031;"))
}
pub(super) fn csi_reply(raw: &[u8]) -> Option<HostReply> {
    let body = raw
        .strip_prefix(b"\x1b[")
        .or_else(|| raw.strip_prefix(&[0x9b]))?;
    if body == b"0n" {
        return Some(HostReply::Fence);
    }
    if let Some(mode) = body
        .strip_prefix(b"?2031;")
        .and_then(|s| s.strip_suffix(b"$y"))
    {
        return Some(parse_decimal(mode).map_or(HostReply::Ignored, HostReply::AppearanceMode));
    }
    if let Some(value) = body
        .strip_prefix(b"?997;")
        .and_then(|s| s.strip_suffix(b"n"))
    {
        return Some(match value {
            b"1" => HostReply::Appearance(TerminalAppearance::Dark),
            b"2" => HostReply::Appearance(TerminalAppearance::Light),
            _ => HostReply::Ignored,
        });
    }
    is_color_csi(raw).then_some(HostReply::Ignored)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn physical_replies_are_chunk_invariant_and_leave_prefix_keys_untouched() {
        let input = b"\x1d\x1b]11;rgb:ffff/eeee/dddd\x07\x1b]21;cursor=;cursor_text=#123456\x1b\\\x1b[?997;2n\x1b[?2031;2$y\x1b[0n.";
        for width in [1, 2, 3, 7, 1024] {
            let mut codec = HostInputCodec::new();
            let mut prefix = CommandMode::new();
            let mut replies = Vec::new();
            let mut commands = Vec::new();
            for chunk in input.chunks(width) {
                for event in codec.feed(chunk).expect("color fixture operation succeeds") {
                    if let HostInputEvent::TerminalReply { reply, .. } = &event {
                        replies.push(reply.clone());
                    }
                    for action in prefix
                        .route(event, Instant::now(), false)
                        .expect("color fixture operation succeeds")
                    {
                        match action {
                            PrefixAction::Command(command) => commands.push(command),
                            _ => panic!("reply leaked as key"),
                        }
                    }
                }
            }
            assert_eq!(commands, vec![LocalCommand::Detach]);
            assert_eq!(
                replies,
                vec![
                    HostReply::Color(COLOR_BACKGROUND, TerminalColorValue::Rgb(255, 238, 221)),
                    HostReply::Color(COLOR_CURSOR, TerminalColorValue::Dynamic),
                    HostReply::Color(COLOR_CURSOR_TEXT, TerminalColorValue::Rgb(18, 52, 86)),
                    HostReply::Appearance(TerminalAppearance::Light),
                    HostReply::AppearanceMode(2),
                    HostReply::Fence
                ]
            );
        }
    }
    #[test]
    fn paste_owns_apparent_replies_and_utf8_continuations_are_not_c1() {
        let paste = b"\x1b[200~\x1b]11;rgb:f/f/f\x07\x1b[?997;1n\x1b[201~";
        let mut codec = HostInputCodec::new();
        let mut events = Vec::new();
        for byte in paste {
            events.extend(
                codec
                    .feed(&[*byte])
                    .expect("color fixture operation succeeds"),
            );
        }
        assert_eq!(events, vec![HostInputEvent::Paste(paste.to_vec())]);
        let mut plain = Vec::new();
        for byte in "ÜÝě".as_bytes() {
            for event in codec
                .feed(&[*byte])
                .expect("color fixture operation succeeds")
            {
                let HostInputEvent::Bytes(bytes) = event else {
                    panic!("UTF-8 became control");
                };
                plain.extend(bytes);
            }
        }
        assert_eq!(plain, "ÜÝě".as_bytes());
        assert!(matches!(
            codec
                .feed(b"\x9d11;rgb:f/f/f\x9c\x9b0n")
                .expect("color fixture operation succeeds")
                .as_slice(),
            [
                HostInputEvent::TerminalReply {
                    reply: HostReply::Color(..),
                    ..
                },
                HostInputEvent::TerminalReply {
                    reply: HostReply::Fence,
                    ..
                }
            ]
        ));
    }
    #[test]
    fn split_reply_survives_epoch_fences_while_stale_keys_do_not() {
        let mut codec = HostInputCodec::new();
        assert!(
            codec
                .feed_for_epoch(b"old\x1b]11;rgb:ff", 1, 2)
                .expect("color fixture operation succeeds")
                .is_empty()
        );
        let events = codec
            .feed_for_epoch(b"/00/aa\x1b\\fresh", 2, 2)
            .expect("color fixture operation succeeds");
        assert!(matches!(
            events.as_slice(),
            [HostInputEvent::TerminalReply {
                reply: HostReply::Color(COLOR_BACKGROUND, TerminalColorValue::Rgb(255, 0, 170)),
                ..
            }]
        ));
        assert_eq!(
            codec
                .feed_for_epoch(b"next", 2, 2)
                .expect("color fixture operation succeeds"),
            vec![HostInputEvent::Bytes(b"next".to_vec())]
        );
    }
    #[test]
    fn oversized_reply_drains_through_terminator_without_exposing_its_payload() {
        let mut codec = HostInputCodec::new();
        let mut bytes = b"\x1b]11;".to_vec();
        bytes.extend(vec![b'x'; 70_000]);
        for chunk in bytes.chunks(1000) {
            assert!(
                codec
                    .feed(chunk)
                    .expect("color fixture operation succeeds")
                    .iter()
                    .all(|event| matches!(event, HostInputEvent::TerminalReply { .. }))
            );
            assert!(codec.pending.len() <= 1024);
        }
        let events = codec
            .feed(b"tail\x1b\\K")
            .expect("color fixture operation succeeds");
        assert!(matches!(events.last(),Some(HostInputEvent::Bytes(bytes)) if bytes == b"K"));
    }
    #[test]
    fn missing_fence_closes_acceptance_and_coalesces_refresh_without_relabeling_late_values() {
        let now = Instant::now();
        let mut colors = HostColors::default();
        assert!(matches!(
            colors.next_command(now),
            Some(HostColorCommand::Probe)
        ));
        colors.observe(
            HostReply::Color(1, TerminalColorValue::Rgb(1, 2, 3)),
            30,
            now,
        );
        colors.expire(now + PROBE_TIMEOUT);
        assert_eq!(
            colors
                .take_update()
                .expect("color fixture operation succeeds")
                .values[1],
            TerminalColorValue::Rgb(1, 2, 3)
        );
        for _ in 0..20 {
            colors.request_refresh();
        }
        colors.observe(
            HostReply::Color(1, TerminalColorValue::Rgb(9, 9, 9)),
            30,
            now + PROBE_TIMEOUT,
        );
        assert!(colors.next_command(now + PROBE_TIMEOUT).is_none());
        assert_eq!(colors.profile.values[1], TerminalColorValue::Rgb(1, 2, 3));
        colors.observe(HostReply::Fence, 4, now + PROBE_TIMEOUT);
        assert!(matches!(
            colors.next_command(now + PROBE_TIMEOUT),
            Some(HostColorCommand::Probe)
        ));
        assert!(colors.next_command(now + PROBE_TIMEOUT).is_none());
        colors.observe(
            HostReply::Color(2, TerminalColorValue::Rgb(4, 5, 6)),
            30,
            now + PROBE_TIMEOUT,
        );
        assert!(
            colors.take_update().is_none(),
            "an accepting round cannot publish partial observations"
        );
        colors.observe(HostReply::Fence, 4, now + PROBE_TIMEOUT);
        let profile = colors
            .take_update()
            .expect("color fixture operation succeeds");
        assert_eq!(profile.values[1], TerminalColorValue::Rgb(1, 2, 3));
        assert_eq!(profile.values[2], TerminalColorValue::Rgb(4, 5, 6));
    }
    #[test]
    fn oversized_controls_keep_framing_across_paste_markers_and_utf8_st_bytes() {
        let mut codec = HostInputCodec::new();
        let mut osc = b"\x1b]11;".to_vec();
        osc.extend(vec![b'x'; 1100]);
        osc.push(0xc3);
        assert!(
            codec
                .feed(&osc)
                .expect("oversized OSC starts drain")
                .iter()
                .all(|event| matches!(event, HostInputEvent::TerminalReply { .. }))
        );
        // The 0x9c continuation finishes Ü; neither it nor a paste marker ends OSC.
        assert!(
            codec
                .feed(b"\x9c\x1b[200~payload")
                .expect("OSC drain retains framing")
                .is_empty()
        );
        let events = codec.feed(b"\x07K").expect("BEL finishes drain");
        assert!(
            matches!(events.as_slice(), [HostInputEvent::TerminalReply { reply: HostReply::Ignored, .. }, HostInputEvent::Bytes(bytes)] if bytes == b"K")
        );
        let mut csi = b"\x1b[?997;".to_vec();
        csi.extend(vec![b'1'; HOST_SEQUENCE_BOUND]);
        assert!(
            codec
                .feed(&csi)
                .expect("oversized CSI starts drain")
                .iter()
                .all(|event| matches!(event, HostInputEvent::TerminalReply { .. }))
        );
        assert!(codec.pending.is_empty());
        let events = codec.feed(b"111nJ").expect("CSI final finishes drain");
        assert!(
            matches!(events.as_slice(), [HostInputEvent::TerminalReply { reply: HostReply::Ignored, .. }, HostInputEvent::Bytes(bytes)] if bytes == b"J")
        );
    }
    #[test]
    fn palette_probe_bounds_each_terminal_response_and_keeps_one_round() {
        #[derive(Default)]
        struct ProbeWriter { bytes: Vec<u8>, flushes: usize }
        impl Write for ProbeWriter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> { self.flushes += 1; Ok(()) }
        }
        let mut output = ProbeWriter::default();
        DesktopPresenter::default().write_color_command(&mut output, HostColorCommand::Probe).expect("physical probe");
        let text = String::from_utf8(output.bytes).expect("query text");
        let mut slots = Vec::new();
        for command in text.split("\x1b\\").filter_map(|command| command.strip_prefix("\x1b]4;")) {
            let (index, query) = command.split_once(';').expect("palette query");
            assert_eq!(query, "?", "a compound OSC response can overflow the host's fixed reply allocator");
            slots.push(index.parse::<u16>().expect("palette index"));
        }
        assert_eq!(slots, (0..256).collect::<Vec<_>>());
        assert_eq!(output.flushes, 1, "no per-index wait/round");
        assert_eq!(text.matches("\x1b[5n").count(), 1);
        assert!(text.ends_with("\x1b[5n"));
    }

    #[test]
    fn only_known_reset_subscription_is_owned_and_probe_queries_every_slot() {
        for mode in 0..=4 {
            let now = Instant::now();
            let mut colors = HostColors::default();
            colors.next_command(now);
            colors.observe(HostReply::AppearanceMode(mode), 14, now);
            let command = colors.next_command(now);
            assert_eq!(
                matches!(command, Some(HostColorCommand::EnableAppearance)),
                mode == 2
            );
            assert_eq!(colors.owned_subscription.load(Ordering::Acquire), mode == 2);
        }
        let mut bytes = Vec::new();
        DesktopPresenter::default()
            .write_color_command(&mut bytes, HostColorCommand::Probe)
            .expect("color fixture operation succeeds");
        let text = String::from_utf8(bytes).expect("color fixture operation succeeds");
        for index in 0..256 {
            assert!(text.contains(&format!(";{index};?")));
        }
        assert!(text.ends_with("\x1b[5n"));
        assert!(!text.contains("2031h"));
        assert!(!text.contains("rgb:"));
    }
}
