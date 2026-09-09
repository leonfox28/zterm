use super::keyboard::{EnhancedKey, KeyEventKind, KeyIdentity, Shortcut};
use super::{
    CONTROL_PREFIX_TIMEOUT, CliError, DomainErrorKind, HostInputEvent, terminal_daemon_error,
};
use std::time::Instant;

const PREFIX: Shortcut = Shortcut::control(']');
const HELD_COMMAND_KEYS: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LocalCommand {
    Detach,
    Upload,
    CancelUpload,
}

// New zterm controls extend this table and the command executor, not decoding.
const BINDINGS: &[(Shortcut, LocalCommand)] = &[
    (Shortcut::plain('.'), LocalCommand::Detach),
    (Shortcut::plain('v'), LocalCommand::Upload),
    (Shortcut::plain('c'), LocalCommand::CancelUpload),
];

#[derive(Eq, PartialEq)]
pub(super) enum PrefixAction {
    Input(HostInputEvent),
    Command(LocalCommand),
}

impl std::fmt::Debug for PrefixAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Input(_) => f.write_str("Input([REDACTED])"),
            Self::Command(command) => f.debug_tuple("Command").field(command).finish(),
        }
    }
}

enum KeyInput<'a> {
    Legacy(u8),
    Enhanced(&'a EnhancedKey),
}

impl KeyInput<'_> {
    fn matches(&self, shortcut: Shortcut) -> bool {
        match self {
            Self::Legacy(byte) => shortcut.matches_legacy(*byte),
            Self::Enhanced(key) => key.matches_shortcut(shortcut),
        }
    }
}

enum KeyRoute {
    Forward,
    Consume,
    Command(LocalCommand),
}

/// One local command state for both legacy and enhanced input. It never changes
/// terminal keyboard modes or replays an incomplete/unknown command.
#[derive(Default)]
pub(super) struct CommandMode {
    pending_deadline: Option<Instant>,
    owned_keys: Vec<KeyIdentity>,
    // A cancelled non-ASCII legacy key may span stdin chunks. Consume its UTF-8
    // continuation bytes without retaining text or dropping the following key.
    cancelled_utf8_tail: u8,
}

impl CommandMode {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) const fn deadline(&self) -> Option<Instant> {
        self.pending_deadline
    }

    pub(super) fn cancel(&mut self) {
        self.pending_deadline = None;
    }

    pub(super) fn clear_pending(&mut self) {
        self.cancel();
        self.owned_keys.clear();
        self.cancelled_utf8_tail = 0;
    }

    /// Retires paused text while still consuming releases of locally owned keys.
    pub(super) fn finish_upload_pause(&mut self) {
        self.cancel();
        self.cancelled_utf8_tail = 0;
    }

    pub(super) fn route(
        &mut self,
        event: HostInputEvent,
        now: Instant,
        report_events: bool,
    ) -> Result<Vec<PrefixAction>, CliError> {
        self.expire(now);
        match event {
            HostInputEvent::TerminalReply { .. } => Ok(Vec::new()),
            HostInputEvent::Bytes(bytes) => self.feed(&bytes, now, report_events),
            HostInputEvent::EnhancedKey(ref key) => {
                let action = match self.key(KeyInput::Enhanced(key), now, report_events)? {
                    KeyRoute::Forward => Some(PrefixAction::Input(event)),
                    KeyRoute::Consume => None,
                    KeyRoute::Command(command) => Some(PrefixAction::Command(command)),
                };
                Ok(action.into_iter().collect())
            }
            HostInputEvent::LegacyCtrlC => {
                let action = match self.key(KeyInput::Legacy(3), now, report_events)? {
                    KeyRoute::Forward => Some(PrefixAction::Input(event)),
                    KeyRoute::Consume => None,
                    KeyRoute::Command(command) => Some(PrefixAction::Command(command)),
                };
                Ok(action.into_iter().collect())
            }
            HostInputEvent::PageUp | HostInputEvent::PageDown => {
                let raw = if matches!(event, HostInputEvent::PageUp) {
                    super::PAGE_UP
                } else {
                    super::PAGE_DOWN
                };
                let key = EnhancedKey::parse(raw.to_vec()).expect("fixed page key sequence");
                let action = match self.key(KeyInput::Enhanced(&key), now, report_events)? {
                    KeyRoute::Forward => Some(PrefixAction::Input(event)),
                    KeyRoute::Consume => None,
                    KeyRoute::Command(command) => Some(PrefixAction::Command(command)),
                };
                Ok(action.into_iter().collect())
            }
            HostInputEvent::Opaque(_) => {
                if self.pending_deadline.take().is_some() {
                    Ok(Vec::new())
                } else {
                    Ok(vec![PrefixAction::Input(event)])
                }
            }
            HostInputEvent::Paste(_) | HostInputEvent::Mouse(_) => {
                self.cancel();
                Ok(vec![PrefixAction::Input(event)])
            }
            // Already interpreted enhanced input must not be scanned again.
            HostInputEvent::ForwardedBytes(_) => Ok(vec![PrefixAction::Input(event)]),
        }
    }

    pub(super) fn feed(
        &mut self,
        bytes: &[u8],
        now: Instant,
        report_events: bool,
    ) -> Result<Vec<PrefixAction>, CliError> {
        self.expire(now);
        let mut actions = Vec::new();
        let mut ordinary = Vec::new();
        for &byte in bytes {
            if self.cancelled_utf8_tail != 0 {
                if byte & 0xc0 == 0x80 {
                    self.cancelled_utf8_tail -= 1;
                    continue;
                }
                self.cancelled_utf8_tail = 0;
            }
            match self.key(KeyInput::Legacy(byte), now, report_events)? {
                KeyRoute::Forward => ordinary.push(byte),
                KeyRoute::Consume => {
                    self.cancelled_utf8_tail = match byte {
                        0xc2..=0xdf => 1,
                        0xe0..=0xef => 2,
                        0xf0..=0xf4 => 3,
                        _ => 0,
                    };
                }
                KeyRoute::Command(command) => {
                    if !ordinary.is_empty() {
                        actions.push(PrefixAction::Input(HostInputEvent::ForwardedBytes(
                            std::mem::take(&mut ordinary),
                        )));
                    }
                    actions.push(PrefixAction::Command(command));
                }
            }
        }
        if !ordinary.is_empty() {
            actions.push(PrefixAction::Input(HostInputEvent::ForwardedBytes(
                ordinary,
            )));
        }
        Ok(actions)
    }

    fn expire(&mut self, now: Instant) {
        if self
            .pending_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.cancel();
        }
    }

    fn key(
        &mut self,
        input: KeyInput<'_>,
        now: Instant,
        report_events: bool,
    ) -> Result<KeyRoute, CliError> {
        if let KeyInput::Legacy(byte) = &input
            && let Some(identity) = KeyIdentity::from_legacy(*byte)
        {
            self.owned_keys.retain(|owned| *owned != identity);
        }
        if let KeyInput::Enhanced(key) = &input {
            let identity = key.identity();
            if let Some(index) = self.owned_keys.iter().position(|owned| *owned == identity) {
                if key.kind == KeyEventKind::Repeat {
                    return Ok(KeyRoute::Consume);
                }
                self.owned_keys.swap_remove(index);
                if key.kind == KeyEventKind::Release {
                    return Ok(KeyRoute::Consume);
                }
            }
            // Unowned modifier/lifecycle events retain their existing owner.
            // In particular, a forwarded Ctrl press still needs its release.
            if key.is_modifier() || key.kind != KeyEventKind::Press {
                return Ok(KeyRoute::Forward);
            }
        }
        if self.pending_deadline.take().is_some() {
            if input.matches(PREFIX) {
                return Ok(KeyRoute::Forward);
            }
            self.own(&input, report_events)?;
            return Ok(BINDINGS
                .iter()
                .find_map(|(binding, command)| {
                    input
                        .matches(*binding)
                        .then_some(KeyRoute::Command(*command))
                })
                .unwrap_or(KeyRoute::Consume));
        }
        if input.matches(PREFIX) {
            self.own(&input, report_events)?;
            self.pending_deadline = Some(now + CONTROL_PREFIX_TIMEOUT);
            Ok(KeyRoute::Consume)
        } else {
            Ok(KeyRoute::Forward)
        }
    }

    fn own(&mut self, key: &KeyInput<'_>, report_events: bool) -> Result<(), CliError> {
        if !report_events {
            return Ok(());
        }
        let identity = match key {
            KeyInput::Legacy(byte) => KeyIdentity::from_legacy(*byte),
            KeyInput::Enhanced(key) => Some(key.identity()),
        };
        if let Some(identity) = identity {
            if self.owned_keys.len() == HELD_COMMAND_KEYS {
                return Err(terminal_daemon_error(
                    DomainErrorKind::ResourceExhausted,
                    "too many held local command keys",
                ));
            }
            self.owned_keys.push(identity);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::{HostInputCodec, PAGE_DOWN, PAGE_UP};
    use super::*;

    #[test]
    fn upload_and_cancel_use_plain_prefix_suffix_and_keep_ctrl_v_unchanged() {
        for input in [
            b"\x1dv\x1dc".as_slice(),
            b"\x1b[93;5u\x1b[118u\x1b[118;1:2u\x1b[118;1:3u\x1b[93;5u\x1b[99u",
        ] {
            for chunk_size in [1, input.len()] {
                assert_eq!(
                    run(input, chunk_size),
                    (
                        vec![LocalCommand::Upload, LocalCommand::CancelUpload],
                        Vec::new()
                    )
                );
            }
        }
        assert_eq!(run(b"\x16", 1), (Vec::new(), vec![0x16]));
        assert_eq!(
            run(b"\x1b[118;5u", 1),
            (Vec::new(), b"\x1b[118;5u".to_vec())
        );
    }

    #[test]
    fn upload_completion_keeps_owned_releases_but_clears_pending_suffix() {
        let mut codec = HostInputCodec::new();
        let mut mode = CommandMode::new();
        for event in codec.feed(b"\x1b[93;5u\x1b[118u").expect("decode presses") {
            let _ = mode
                .route(event, Instant::now(), true)
                .expect("route presses");
        }
        mode.finish_upload_pause();
        for event in codec
            .feed(b"\x1b[118;1:2u\x1b[118;1:3u\x1b[93;1:3u")
            .expect("decode held keys")
        {
            assert!(
                mode.route(event, Instant::now(), true)
                    .expect("route held keys")
                    .is_empty()
            );
        }
        for event in codec.feed(b"v").expect("decode new text") {
            assert!(matches!(
                mode.route(event, Instant::now(), true)
                    .expect("route new text")
                    .as_slice(),
                [PrefixAction::Input(_)]
            ));
        }
    }

    fn run(input: &[u8], chunk_size: usize) -> (Vec<LocalCommand>, Vec<u8>) {
        let mut codec = HostInputCodec::new();
        let mut mode = CommandMode::new();
        let mut commands = Vec::new();
        let mut forwarded = Vec::new();
        for chunk in input.chunks(chunk_size) {
            for event in codec.feed(chunk).expect("decode") {
                for action in mode.route(event, Instant::now(), true).expect("route") {
                    match action {
                        PrefixAction::Command(command) => commands.push(command),
                        PrefixAction::Input(event) => match event {
                            HostInputEvent::Bytes(bytes)
                            | HostInputEvent::ForwardedBytes(bytes)
                            | HostInputEvent::Opaque(bytes)
                            | HostInputEvent::Paste(bytes) => forwarded.extend(bytes),
                            HostInputEvent::EnhancedKey(key) => forwarded.extend(key.raw),
                            HostInputEvent::LegacyCtrlC => forwarded.push(3),
                            HostInputEvent::PageUp => forwarded.extend(PAGE_UP),
                            HostInputEvent::PageDown => forwarded.extend(PAGE_DOWN),
                            HostInputEvent::Mouse(mouse) => forwarded.extend(mouse.raw),
                            HostInputEvent::TerminalReply { .. } => {
                                panic!("physical reply must not be routed as input")
                            }
                        },
                    }
                }
            }
        }
        (commands, forwarded)
    }

    #[test]
    fn decoded_enhanced_prefix_and_period_dispatch_local_detach() {
        for input in [
            b"\x1d.".as_slice(),
            b"\x1b[93;5u\x1b[46u",
            b"\x1b[93;5u\x1b[93;5:3u.",
            b"\x1d\x1b[46u\x1b[46;1:3u",
            b"\x1b[93;5u\x1b[93;1:3u\x1b[46u\x1b[46;1:3u",
            b"\x1b[300::93;5u\x1b[46;1;12290u",
        ] {
            for chunk_size in [1, input.len()] {
                assert_eq!(
                    run(input, chunk_size),
                    (vec![LocalCommand::Detach], Vec::new())
                );
            }
        }
    }

    #[test]
    fn cancellation_consumes_unknown_key_lifecycle_and_whole_utf8_scalar() {
        for input in [
            b"\x1dxok".as_slice(),
            b"\x1d\xe3\x80\x82ok",
            b"\x1b[93;5u\x1b[120u\x1b[120;1:2u\x1b[93;5:3u\x1b[120;1:3uok",
            b"\x1d\x1b[120;1;46u\x1b[120;1:3uok",
            b"\x1d\x1b[99;0uok",
            b"\x1d\x1b[5~ok",
            b"\x1d\x1b.ok",
            b"\x1d\x1bOPok",
            b"\x1d\x1bO1;5Pok",
            b"\x1dX\x1b[120;2:3uok",
            b"\x1dx\x1b[120;1:3uok",
            b"\x1d\t\x1b[9;1:3uok",
            b"\x1d\r\x1b[13;1:3uok",
            b"\x1d\x7f\x1b[127;1:3uok",
            b"\x1d\x1b[5~\x1b[5;1:3~ok",
        ] {
            for chunk_size in [1, input.len()] {
                assert_eq!(run(input, chunk_size), (Vec::new(), b"ok".to_vec()));
            }
        }
    }

    #[test]
    fn quoting_is_a_second_press_not_a_repeat_and_modifiers_do_not_cancel() {
        assert_eq!(run(b"\x1d\x1d", 1), (Vec::new(), vec![0x1d]));
        assert_eq!(
            run(
                b"\x1b[93;5u\x1b[93;5:2u\x1b[93;5:3u\x1b[93;5u\x1b[93;5:3u",
                1
            ),
            (Vec::new(), b"\x1b[93;5u\x1b[93;5:3u".to_vec()),
        );
        assert_eq!(
            run(
                b"\x1b[57442;5u\x1b[93;5u\x1b[93;5:3u\x1b[57442;1:3u\x1b[46u",
                1
            ),
            (
                vec![LocalCommand::Detach],
                b"\x1b[57442;5u\x1b[57442;1:3u".to_vec()
            ),
        );
    }

    #[test]
    fn non_prefix_input_and_paste_remain_byte_preserving() {
        for input in [
            b"\x1b[93;7u.".as_slice(), // Ctrl+Alt+] is not Ctrl+].
            b"\x1b[93;13u.",           // Ctrl+Super+].
            b"\x1b[93;6u.",            // Ctrl+Shift+].
            b"\x1b[99;0u",
            b"\x1b[99;\x1du.", // Opaque sequence, not a raw prefix.
            b"\x1b\x1d.",      // Alt+Ctrl+] is one opaque legacy key.
            b"\x1b[200~\x1d.\x1b[93;5u\x1b[201~",
            b"\xe3\x80\x82\x03ordinary",
        ] {
            assert_eq!(run(input, 1), (Vec::new(), input.to_vec()));
        }
        let paste = b"\x1b[200~\x1d.\x1b[201~";
        let mut input = vec![0x1d];
        input.extend(paste);
        assert_eq!(run(&input, 1), (Vec::new(), paste.to_vec()));
    }

    #[test]
    fn missing_release_reports_do_not_accumulate_held_keys() {
        let mut mode = CommandMode::new();
        for code in 1000..1200 {
            for raw in [b"\x1b[93;5u".to_vec(), format!("\x1b[{code}u").into_bytes()] {
                let key = EnhancedKey::parse(raw).expect("key");
                assert!(
                    mode.route(HostInputEvent::EnhancedKey(key), Instant::now(), false)
                        .expect("no release reports requested")
                        .is_empty()
                );
            }
        }
    }

    #[test]
    fn held_command_key_reports_are_bounded_and_resettable() {
        let mut mode = CommandMode::new();
        for code in 1000..1064 {
            let prefix = EnhancedKey::parse(b"\x1b[93;5u".to_vec()).expect("prefix");
            mode.route(HostInputEvent::EnhancedKey(prefix), Instant::now(), true)
                .expect("prefix replaces its previous press");
            let key = EnhancedKey::parse(format!("\x1b[{code}u").into_bytes()).expect("key");
            let result = mode.route(HostInputEvent::EnhancedKey(key), Instant::now(), true);
            assert_eq!(result.is_err(), code == 1063);
        }
        mode.clear_pending();
        assert!(mode.feed(b"\x1d", Instant::now(), true).is_ok());
    }

    #[test]
    fn timeout_still_consumes_owned_releases_but_never_reinterprets_forwarded_bytes() {
        let now = Instant::now();
        let mut mode = CommandMode::new();
        mode.feed(b"\x1d", now, true).expect("prefix");
        let release = EnhancedKey::parse(b"\x1b[93;1:3u".to_vec()).expect("release");
        assert!(
            mode.route(
                HostInputEvent::EnhancedKey(release),
                now + CONTROL_PREFIX_TIMEOUT,
                true,
            )
            .expect("expired prefix release")
            .is_empty()
        );
        let forwarded = HostInputEvent::ForwardedBytes(b"\x1d.".to_vec());
        assert_eq!(
            mode.route(forwarded.clone(), now, true)
                .expect("encoded fallthrough"),
            vec![PrefixAction::Input(forwarded)]
        );
        assert!(mode.deadline().is_none());
    }

    #[test]
    fn timeout_and_epoch_reset_never_replay_a_prefix() {
        let now = Instant::now();
        let mut mode = CommandMode::new();
        assert!(mode.feed(b"\x1d", now, false).expect("prefix").is_empty());
        assert_eq!(mode.deadline(), Some(now + CONTROL_PREFIX_TIMEOUT));
        assert_eq!(
            mode.feed(b"x", now + CONTROL_PREFIX_TIMEOUT, false)
                .expect("timeout"),
            vec![PrefixAction::Input(HostInputEvent::ForwardedBytes(
                b"x".to_vec()
            ))]
        );
        assert_eq!(mode.deadline(), None);
        assert!(mode.feed(b"\x1d", now, false).expect("prefix").is_empty());
        mode.clear_pending();
        assert_eq!(
            mode.feed(b".", now, false).expect("new epoch"),
            vec![PrefixAction::Input(HostInputEvent::ForwardedBytes(
                b".".to_vec()
            ))]
        );
    }
}
