//! Adversarial bounds for PTY-controlled terminal input.

use zterm_core::terminal::{
    MAX_SIDE_EVENTS_PER_UPDATE, RejectedEffect, TerminalColor, TerminalSideEvent, TerminalSize,
    UnsupportedSequenceKind,
};
use zterm_terminal::{
    MAX_CELL_TEXT_BYTES, MAX_COMBINING_BYTES_PER_SESSION, MAX_COMBINING_CELLS_PER_SESSION,
    MAX_CONTROL_SEQUENCE_BYTES, MAX_CONTROL_STRING_BYTES, MAX_REPLY_BYTES_PER_UPDATE,
    TerminalError, TerminalModel,
};

fn visible_text(model: &TerminalModel) -> String {
    model
        .state()
        .cells
        .into_iter()
        .map(|cell| cell.contents)
        .collect()
}

#[test]
fn control_strings_are_bounded_chunk_invariant_and_content_redacted() {
    const SECRET: &str = "URI_SECRET_8f73";
    let uri = format!(
        "https://example.invalid/{SECRET}/{}",
        "x".repeat(MAX_CONTROL_STRING_BYTES * 2)
    );
    let mut input = format!(
        "before\x1b]8;;{uri}\x1b\\linked\x1b]8;;\x1b\\\x1bP{SECRET}{}\x1b\\",
        "d".repeat(MAX_CONTROL_STRING_BYTES * 2),
    )
    .into_bytes();
    input.extend_from_slice(b"\x9d8;;c1-secret\x9cafter");

    let mut whole = TerminalModel::new(TerminalSize::new(4, 80), 8).expect("model");
    let whole_update = whole.ingest(&input).expect("whole ingest");
    let mut chunked = TerminalModel::new(TerminalSize::new(4, 80), 8).expect("model");
    let mut events = Vec::new();
    for byte in &input {
        events.extend(
            chunked
                .ingest(std::slice::from_ref(byte))
                .expect("byte ingest")
                .events,
        );
    }

    assert_eq!(whole.state(), chunked.state());
    assert_eq!(whole_update.events, events);
    assert!(visible_text(&whole).contains("beforelinkedafter"));
    let surfaces = format!(
        "{:?}{:?}{:?}",
        whole.state(),
        whole.snapshot(),
        whole_update.events,
    );
    assert!(!surfaces.contains(SECRET));
    assert!(
        whole_update
            .events
            .contains(&TerminalSideEvent::UnsupportedSequence(
                UnsupportedSequenceKind::Osc,
            ))
    );
    assert!(
        whole_update
            .events
            .contains(&TerminalSideEvent::UnsupportedSequence(
                UnsupportedSequenceKind::Control,
            ))
    );
}

#[test]
fn cancelled_controls_filtered_attributes_and_malformed_utf8_stay_contained() {
    const SECRET: &str = "FILTERED_SECRET_d61a";
    let input = [
        b"before".as_slice(),
        format!("\x1b]8;;https://invalid/{SECRET}\x1b\\linked\x1b]8;;\x1b\\").as_bytes(),
        b"\x1b[58;2;1;2;3m\x1b[5b\x1b[>1u",
        format!("\x1bP{SECRET}\x18after").as_bytes(),
        b"\x1b[31;\x1acolorless\xffdone",
    ]
    .concat();

    let mut whole = TerminalModel::new(TerminalSize::new(4, 80), 8).expect("whole model");
    let whole_update = whole.ingest(&input).expect("whole hostile ingest");
    let mut chunked = TerminalModel::new(TerminalSize::new(4, 80), 8).expect("chunked model");
    let mut replies = Vec::new();
    let mut events = Vec::new();
    for byte in &input {
        let update = chunked
            .ingest(std::slice::from_ref(byte))
            .expect("byte hostile ingest");
        replies.extend(update.replies);
        events.extend(update.events);
    }

    assert_eq!(whole.state(), chunked.state());
    assert_eq!(whole_update.replies, replies);
    assert_eq!(whole_update.events, events);
    assert!(whole_update.events.iter().any(|event| matches!(
        event,
        TerminalSideEvent::UnsupportedSequence(UnsupportedSequenceKind::Osc)
    )));
    assert!(whole_update.events.iter().any(|event| matches!(
        event,
        TerminalSideEvent::UnsupportedSequence(UnsupportedSequenceKind::Csi)
    )));
    let surfaces = format!("{:?}{:?}", whole.state(), whole.snapshot());
    assert!(!surfaces.contains(SECRET));
    assert!(visible_text(&whole).contains("linkedaftercolorless"));
}

#[test]
fn combining_text_is_capped_before_it_can_grow_engine_cells() {
    let mut model = TerminalModel::new(TerminalSize::new(2, 16), 4).expect("model");
    let flood = format!("e{}", "\u{301}".repeat(4_000));
    let update = model
        .ingest(flood.as_bytes())
        .expect("combining flood is contained");
    let cell = model
        .state()
        .cells
        .into_iter()
        .find(|cell| cell.contents.starts_with('e'))
        .expect("base cell remains visible");
    assert!(cell.contents.len() <= MAX_CELL_TEXT_BYTES);
    assert!(update.events.iter().any(|event| matches!(
        event,
        TerminalSideEvent::UnsupportedSequence(UnsupportedSequenceKind::Character)
            | TerminalSideEvent::EventsDropped { .. }
    )));
}

#[test]
fn session_combining_cell_and_byte_limits_emit_bounded_classifications() {
    let mut cells = TerminalModel::new(TerminalSize::new(80, 80), 0).expect("model");
    let mut cell_fill = Vec::new();
    for index in 0..MAX_COMBINING_CELLS_PER_SESSION {
        let row = index / 80 + 1;
        let column = index % 80 + 1;
        cell_fill.extend_from_slice(format!("\x1b[{row};{column}He\u{301}").as_bytes());
    }
    assert!(
        cells
            .ingest(&cell_fill)
            .expect("fill cell budget")
            .events
            .is_empty()
    );
    let cell_overflow = cells
        .ingest(b"\x1b[80;80He\xcc\x82")
        .expect("cell overflow is contained");
    assert_eq!(
        cell_overflow.events,
        vec![TerminalSideEvent::UnsupportedSequence(
            UnsupportedSequenceKind::Character,
        )]
    );

    let mut bytes = TerminalModel::new(TerminalSize::new(80, 80), 0).expect("model");
    let combining = "\u{301}".repeat(
        MAX_COMBINING_BYTES_PER_SESSION / MAX_COMBINING_CELLS_PER_SESSION / '\u{301}'.len_utf8(),
    );
    let mut byte_fill = Vec::new();
    for index in 0..MAX_COMBINING_CELLS_PER_SESSION {
        let row = index / 80 + 1;
        let column = index % 80 + 1;
        byte_fill.extend_from_slice(format!("\x1b[{row};{column}He{combining}").as_bytes());
    }
    assert!(
        bytes
            .ingest(&byte_fill)
            .expect("fill byte budget")
            .events
            .is_empty()
    );
    let byte_overflow = bytes
        .ingest(b"\x1b[1;1H\xcc\x82")
        .expect("byte overflow is contained");
    assert_eq!(
        byte_overflow.events,
        vec![TerminalSideEvent::UnsupportedSequence(
            UnsupportedSequenceKind::Character,
        )]
    );
}

#[test]
fn disabled_effects_sync_updates_and_oversized_sequences_are_contained() {
    let mut model = TerminalModel::new(TerminalSize::new(4, 80), 8).expect("model");
    let title = "t".repeat(MAX_CONTROL_STRING_BYTES * 2);
    let long_csi = format!("\x1b[{}m", "1;".repeat(MAX_CONTROL_SEQUENCE_BYTES));
    let input = format!(
        "\x1b[?2026hvisible-now\x1b[?2026l{long_csi}\x1b]52;c;secret\x07\x1b]2;{title}\x07"
    );
    let update = model
        .ingest(input.as_bytes())
        .expect("hostile controls are contained");

    assert!(visible_text(&model).contains("visible-now"));
    assert!(update.events.contains(&TerminalSideEvent::EffectRejected(
        RejectedEffect::ClipboardWrite,
    )));
    assert!(update.events.iter().any(|event| matches!(
        event,
        TerminalSideEvent::UnsupportedSequence(UnsupportedSequenceKind::Csi)
    )));
    assert!(
        update
            .events
            .contains(&TerminalSideEvent::UnsupportedSequence(
                UnsupportedSequenceKind::Osc,
            ))
    );
    assert!(
        !update
            .events
            .iter()
            .any(|event| matches!(event, TerminalSideEvent::TitleChanged { .. }))
    );
}

#[test]
fn nested_escape_and_c1_introducers_cannot_bypass_ingress_policy() {
    let mut model = TerminalModel::new(TerminalSize::new(4, 80), 8).expect("model");
    let sync_input = b"\x1b[\x1b[?2026hvisible-immediately";

    let sync = model
        .ingest(sync_input)
        .expect("nested synchronized update is contained");
    let mut chunked = TerminalModel::new(TerminalSize::new(4, 80), 8).expect("chunked model");
    let mut chunked_events = Vec::new();
    for byte in sync_input {
        chunked_events.extend(
            chunked
                .ingest(std::slice::from_ref(byte))
                .expect("nested ESC restart remains framed across chunks")
                .events,
        );
    }

    assert_eq!(chunked.state(), model.state());
    assert_eq!(chunked_events, sync.events);
    assert!(visible_text(&model).contains("visible-immediately"));
    assert_eq!(
        sync.events,
        vec![TerminalSideEvent::UnsupportedSequence(
            UnsupportedSequenceKind::Csi,
        )]
    );

    let title = model
        .ingest(b"\x1b[\x1b]2;nested-title\x1b\\after-title")
        .expect("nested OSC is handled by the host policy");
    assert!(title.events.contains(&TerminalSideEvent::TitleChanged {
        title: "nested-title".to_owned(),
        truncated: false,
    }));
    assert!(visible_text(&model).contains("after-title"));

    let c1 = model
        .ingest(b"\x1b[\x9d8;;URI_SECRET_14d2\x9cafter-c1")
        .expect("nested C1 OSC is contained");
    assert!(c1.events.contains(&TerminalSideEvent::UnsupportedSequence(
        UnsupportedSequenceKind::Osc,
    )));
    let visible = visible_text(&model);
    assert!(visible.contains("after-c1"));
    assert!(!visible.contains("URI_SECRET_14d2"));
}

#[test]
fn embedded_controls_do_not_obscure_filtered_sequence_identity() {
    let input = b"\x1b[?2026\x07hvisible-now\x1b\x07]2;control-title\x1b\\after-title";
    let mut whole = TerminalModel::new(TerminalSize::new(4, 80), 8).expect("whole model");
    let whole_update = whole
        .ingest(input)
        .expect("embedded controls are handled without bypass");
    let mut chunked = TerminalModel::new(TerminalSize::new(4, 80), 8).expect("chunked model");
    let mut events = Vec::new();
    for byte in input {
        events.extend(
            chunked
                .ingest(std::slice::from_ref(byte))
                .expect("byte-wise embedded controls remain framed")
                .events,
        );
    }

    assert_eq!(chunked.state(), whole.state());
    assert_eq!(events, whole_update.events);
    assert!(visible_text(&whole).contains("visible-now"));
    assert!(visible_text(&whole).contains("after-title"));
    assert_eq!(
        whole_update.events,
        vec![
            TerminalSideEvent::AudibleBell,
            TerminalSideEvent::UnsupportedSequence(UnsupportedSequenceKind::Csi),
            TerminalSideEvent::AudibleBell,
            TerminalSideEvent::TitleChanged {
                title: "control-title".to_owned(),
                truncated: false,
            },
        ]
    );
}

#[test]
fn underline_color_filter_uses_sgr_parameters_without_blocking_rgb_components() {
    let mut ordinary_rgb = TerminalModel::new(TerminalSize::new(2, 16), 0).expect("RGB model");
    let rgb_update = ordinary_rgb
        .ingest(b"\x1b[38;2;58;59;60mRGB")
        .expect("ordinary RGB color remains supported");
    assert!(rgb_update.events.is_empty());
    assert!(ordinary_rgb.state().cells.iter().any(|cell| {
        cell.contents == "R" && cell.style.foreground == TerminalColor::Rgb(58, 59, 60)
    }));

    let mut underline = TerminalModel::new(TerminalSize::new(2, 16), 0).expect("underline model");
    let underline_update = underline
        .ingest(b"\x1b[058;5;1mcontained")
        .expect("leading-zero underline color is contained");
    assert_eq!(
        underline_update.events,
        vec![TerminalSideEvent::UnsupportedSequence(
            UnsupportedSequenceKind::Csi,
        )]
    );
}

#[test]
fn canonical_reply_stream_has_a_hard_per_update_limit() {
    let mut model = TerminalModel::new(TerminalSize::new(2, 8), 0).expect("model");
    let queries = b"\x1b[c".repeat(MAX_REPLY_BYTES_PER_UPDATE / 4 + 1);
    assert_eq!(model.ingest(&queries), Err(TerminalError::ReplyOverflow));
}

#[test]
fn side_event_flood_is_summarized_at_the_hard_limit() {
    let mut model = TerminalModel::new(TerminalSize::new(2, 8), 0).expect("model");
    let update = model
        .ingest(&b"\x1bg".repeat(MAX_SIDE_EVENTS_PER_UPDATE + 68))
        .expect("event flood is contained");
    assert_eq!(update.events.len(), MAX_SIDE_EVENTS_PER_UPDATE);
    assert_eq!(
        update.events.last(),
        Some(&TerminalSideEvent::EventsDropped { count: 69 })
    );
}
