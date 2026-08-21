//! Fixed ANSI compatibility corpus for the zterm-owned terminal boundary.

use zterm_core::terminal::{
    ActiveScreen, MAX_SIDE_EVENTS_PER_UPDATE, MAX_TITLE_BYTES, RejectedEffect, TerminalColor,
    TerminalDeltaResult, TerminalModel, TerminalMouseEncoding, TerminalMouseMode,
    TerminalSideEvent, TerminalSize, TerminalState, UnsupportedSequenceKind,
};

const SIZE: TerminalSize = TerminalSize::new(8, 40);

#[derive(Clone, Copy, Debug)]
enum Chunking {
    Whole,
    OneByte,
    Fixed(usize),
    PseudoRandom,
}

#[derive(Debug, Eq, PartialEq)]
struct Run {
    state: TerminalState,
    replies: Vec<u8>,
    events: Vec<TerminalSideEvent>,
}

fn chunks(bytes: &[u8], chunking: Chunking) -> Vec<&[u8]> {
    if bytes.is_empty() {
        return Vec::new();
    }

    let mut result = Vec::new();
    let mut offset = 0;
    let mut random = 0x6d2b_79f5_u32;
    while offset < bytes.len() {
        let requested = match chunking {
            Chunking::Whole => bytes.len(),
            Chunking::OneByte => 1,
            Chunking::Fixed(width) => width,
            Chunking::PseudoRandom => {
                random = random.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                usize::try_from((random % 11) + 1).expect("chunk width fits usize")
            }
        };
        let end = offset.saturating_add(requested).min(bytes.len());
        result.push(&bytes[offset..end]);
        offset = end;
    }
    result
}

fn run(bytes: &[u8], chunking: Chunking) -> Run {
    let mut model = TerminalModel::new(SIZE, 32).expect("corpus size is valid");
    let mut replies = Vec::new();
    let mut events = Vec::new();
    for chunk in chunks(bytes, chunking) {
        let update = model.ingest(chunk).expect("corpus ingest succeeds");
        replies.extend(update.replies);
        events.extend(update.events);
    }
    Run {
        state: model.state(),
        replies,
        events,
    }
}

fn assert_chunk_invariant(bytes: &[u8]) -> Run {
    let expected = run(bytes, Chunking::Whole);
    for chunking in [
        Chunking::OneByte,
        Chunking::Fixed(5),
        Chunking::PseudoRandom,
    ] {
        assert_eq!(run(bytes, chunking), expected, "chunking {chunking:?}");
    }
    expected
}

#[test]
fn fixed_ansi_corpus_is_chunk_boundary_invariant() {
    let screen = assert_chunk_invariant(
        concat!(
            "\x1b[2J\x1b[Hmain\r\n",
            "\x1b[2;6r\x1b[6;1Hscroll-one\r\nscroll-two\x1b[r",
            "\x1b[3;4H\x1b[38;5;196mindexed",
            "\x1b[48;2;1;2;3mtruecolor\x1b[0m",
            "\x1b[5;2H界e\u{301}",
        )
        .as_bytes(),
    );
    assert_eq!(screen.state.active_screen, ActiveScreen::Main);
    assert!(screen.state.cells.iter().any(|cell| cell.wide));
    assert!(
        screen
            .state
            .cells
            .iter()
            .any(|cell| cell.contents == "e\u{301}")
    );
    assert!(screen.state.cells.iter().any(|cell| {
        cell.style.foreground == TerminalColor::Indexed(196) && cell.contents.contains("i")
    }));
    assert!(screen.state.cells.iter().any(|cell| {
        cell.style.background == TerminalColor::Rgb(1, 2, 3) && !cell.contents.is_empty()
    }));

    let alternate = assert_chunk_invariant(
        concat!(
            "main-before-alt",
            "\x1b[?1049h\x1b[2J\x1b[Halternate",
            "\x1b[?1h\x1b=\x1b[?2004h\x1b[?1003h\x1b[?1006h\x1b[?1004h",
        )
        .as_bytes(),
    );
    assert_eq!(alternate.state.active_screen, ActiveScreen::Alternate);
    assert!(alternate.state.modes.application_cursor);
    assert!(alternate.state.modes.application_keypad);
    assert!(alternate.state.modes.bracketed_paste);
    assert!(alternate.state.modes.focus_reporting);
    assert_eq!(
        alternate.state.modes.mouse_mode,
        TerminalMouseMode::AnyMotion
    );
    assert_eq!(
        alternate.state.modes.mouse_encoding,
        TerminalMouseEncoding::Sgr
    );

    let restored = assert_chunk_invariant(b"main-restored\x1b[?1049hdiscarded-alt\x1b[?1049l!");
    assert_eq!(restored.state.active_screen, ActiveScreen::Main);
    let visible: String = restored
        .state
        .cells
        .iter()
        .map(|cell| cell.contents.as_str())
        .collect();
    assert!(visible.contains("main-restored!"));
    assert!(!visible.contains("discarded-alt"));
}

#[test]
fn query_replies_match_the_documented_vt100_capability() {
    let run = assert_chunk_invariant(b"\x1b[3;4H\x1b[c\x1b[5n\x1b[6n\x1b[?6n");
    assert_eq!(run.replies, b"\x1b[?1;2c\x1b[0n\x1b[3;4R\x1b[?3;4R");
    assert!(run.events.is_empty());

    let unsupported = assert_chunk_invariant(b"\x1b[>6n");
    assert!(unsupported.replies.is_empty());
    assert_eq!(
        unsupported.events,
        vec![TerminalSideEvent::UnsupportedSequence(
            UnsupportedSequenceKind::Csi,
        )]
    );

    let last_column = assert_chunk_invariant(b"1234567890123456789012345678901234567890\x1b[6n");
    assert_eq!(last_column.replies, b"\x1b[1;40R");
}

#[test]
fn allowed_side_events_are_bounded_and_unsafe_effects_are_contained() {
    let bytes = concat!(
        "safe-before",
        "\x07\x1bg",
        "\x1b]2;bounded title\x07",
        "\x1b]1;bounded icon\x07",
        "\x1b]52;c;c2VjcmV0LWJ5dGVz\x07",
        "\x1b]52;c;?\x07",
        "\x1b]777;unknown-osc-secret\x07",
        "\x1bPunknown-dcs-secret\x1b\\",
        "\x1b_unknown-apc-secret\x1b\\",
        "safe-after",
    )
    .as_bytes();
    let run = assert_chunk_invariant(bytes);

    assert!(run.events.contains(&TerminalSideEvent::AudibleBell));
    assert!(run.events.contains(&TerminalSideEvent::VisualBell));
    assert!(run.events.contains(&TerminalSideEvent::TitleChanged {
        title: "bounded title".to_owned(),
        truncated: false,
    }));
    assert!(run.events.contains(&TerminalSideEvent::IconNameChanged {
        icon_name: "bounded icon".to_owned(),
        truncated: false,
    }));
    assert!(run.events.contains(&TerminalSideEvent::EffectRejected(
        RejectedEffect::ClipboardWrite,
    )));
    assert!(run.events.contains(&TerminalSideEvent::EffectRejected(
        RejectedEffect::ClipboardRead,
    )));
    assert!(run.events.contains(&TerminalSideEvent::UnsupportedSequence(
        UnsupportedSequenceKind::Osc,
    )));
    assert!(run.replies.is_empty());

    let mut model = TerminalModel::new(SIZE, 32).expect("corpus size is valid");
    let checkpoint = model.checkpoint();
    let update = model.ingest(bytes).expect("safety corpus ingests");
    let snapshot = model.snapshot();
    let delta_ansi = match model.delta_or_resync(&checkpoint) {
        TerminalDeltaResult::Delta(delta) => delta.ansi,
        TerminalDeltaResult::Resync(snapshot) => snapshot.screen_ansi,
    };
    let serialized = format!(
        "{state:?}{events:?}{replies:?}{screen:?}{history:?}{delta:?}",
        state = model.state(),
        events = update.events,
        replies = update.replies,
        screen = snapshot.screen_ansi,
        history = snapshot.recent_history_ansi,
        delta = delta_ansi,
    );
    for secret in [
        "secret-bytes",
        "c2VjcmV0LWJ5dGVz",
        "unknown-osc-secret",
        "unknown-dcs-secret",
        "unknown-apc-secret",
    ] {
        assert!(
            !serialized.contains(secret),
            "unsafe payload leaked: {secret}"
        );
    }
}

#[test]
fn side_event_volume_and_payloads_have_explicit_bounds() {
    let mut model = TerminalModel::new(SIZE, 0).expect("corpus size is valid");
    let bells = vec![7; 100];
    let update = model.ingest(&bells).expect("bell burst ingests");
    assert_eq!(update.events.len(), MAX_SIDE_EVENTS_PER_UPDATE);
    assert_eq!(
        update.events.last(),
        Some(&TerminalSideEvent::EventsDropped { count: 69 })
    );

    let long_title = "t".repeat(MAX_TITLE_BYTES + 50);
    let update = model
        .ingest(format!("\x1b]2;{long_title}\x07").as_bytes())
        .expect("long title ingests");
    assert_eq!(
        update.events,
        vec![TerminalSideEvent::TitleChanged {
            title: "t".repeat(MAX_TITLE_BYTES),
            truncated: true,
        }]
    );
}

#[test]
fn repeated_resize_preserves_chunk_independent_state() {
    fn resized_run(chunking: Chunking) -> Run {
        let mut model =
            TerminalModel::new(TerminalSize::new(4, 12), 8).expect("initial size is valid");
        let mut replies = Vec::new();
        let mut events = Vec::new();
        for (bytes, resize) in [
            (b"before-resize".as_slice(), Some(TerminalSize::new(6, 18))),
            (
                "\x1b[6;18Hwide=界".as_bytes(),
                Some(TerminalSize::new(3, 9)),
            ),
            (b"\x1b[Hfinal".as_slice(), None),
        ] {
            for chunk in chunks(bytes, chunking) {
                let update = model.ingest(chunk).expect("resize corpus ingests");
                replies.extend(update.replies);
                events.extend(update.events);
            }
            if let Some(size) = resize {
                let update = model.resize(size).expect("resize succeeds");
                replies.extend(update.replies);
                events.extend(update.events);
            }
        }
        Run {
            state: model.state(),
            replies,
            events,
        }
    }

    let expected = resized_run(Chunking::Whole);
    for chunking in [
        Chunking::OneByte,
        Chunking::Fixed(4),
        Chunking::PseudoRandom,
    ] {
        assert_eq!(resized_run(chunking), expected, "chunking {chunking:?}");
    }
    assert_eq!(expected.state.size, TerminalSize::new(3, 9));
}
