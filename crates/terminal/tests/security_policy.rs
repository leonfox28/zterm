//! Adversarial bounds for PTY-controlled terminal input.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use zterm_core::terminal::{
    MAX_SIDE_EVENTS_PER_UPDATE, MAX_TERMINAL_CLIPBOARD_BYTES, RejectedEffect, TerminalColor,
    TerminalHostEffect, TerminalKeyboardFlags, TerminalSideEvent, TerminalSize,
    UnsupportedSequenceKind,
};
use zterm_terminal::{
    MAX_CELL_TEXT_BYTES, MAX_COMBINING_BYTES_PER_SESSION, MAX_COMBINING_CELLS_PER_SESSION,
    MAX_CONTROL_SEQUENCE_BYTES, MAX_CONTROL_STRING_BYTES, MAX_OSC52_BASE64_BYTES,
    MAX_REPLY_BYTES_PER_UPDATE, TerminalError, TerminalModel,
};

fn visible_text(model: &TerminalModel) -> String {
    model
        .snapshot()
        .surface
        .rows
        .into_iter()
        .flat_map(|row| row.cells)
        .map(|cell| cell.contents)
        .collect()
}

fn assert_same_presented_surface(left: &TerminalModel, right: &TerminalModel) {
    let left = left.snapshot().surface;
    let right = right.snapshot().surface;
    assert_eq!(left.size, right.size);
    assert_eq!(left.active_screen, right.active_screen);
    assert_eq!(left.rows, right.rows);
    assert_eq!(left.cursor, right.cursor);
    assert_eq!(left.modes, right.modes);
    assert_eq!(
        left.scroll_metrics.map(|metrics| (
            metrics.epoch,
            metrics.offset_from_bottom,
            metrics.max_offset_from_bottom,
            metrics.viewport_rows,
        )),
        right.scroll_metrics.map(|metrics| (
            metrics.epoch,
            metrics.offset_from_bottom,
            metrics.max_offset_from_bottom,
            metrics.viewport_rows,
        )),
    );
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

    assert_same_presented_surface(&whole, &chunked);
    assert_eq!(whole_update.events, events);
    assert!(visible_text(&whole).contains("beforelinkedafter"));
    let surfaces = format!(
        "{:?}{:?}{:?}",
        whole.snapshot(),
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

    assert_same_presented_surface(&whole, &chunked);
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
    let surfaces = format!("{:?}", whole.snapshot());
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
        .snapshot()
        .surface
        .rows
        .into_iter()
        .flat_map(|row| row.cells)
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

    assert_same_presented_surface(&chunked, &model);
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

    assert_same_presented_surface(&chunked, &whole);
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
    assert!(
        ordinary_rgb
            .snapshot()
            .surface
            .rows
            .iter()
            .flat_map(|row| &row.cells)
            .any(|cell| {
                cell.contents == "R" && cell.style.foreground == TerminalColor::Rgb(58, 59, 60)
            })
    );

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

#[test]
fn osc52_write_is_strict_chunk_invariant_redacted_and_latest_only() {
    const FIRST: &str = "CLIPBOARD_FIRST_18af\n界";
    const SECOND: &str = "CLIPBOARD_SECOND_29bc\tvalue";
    let first = BASE64_STANDARD.encode(FIRST);
    let second = BASE64_STANDARD.encode(SECOND);
    let mut input =
        format!("before\x1b]52;c;{first}\x1b\\\x1b]52;c;not-canonical\x07").into_bytes();
    input.push(0x9d);
    input.extend_from_slice(format!("52;c;{second}").as_bytes());
    input.push(0x9c);
    input.extend_from_slice(b"after");

    let mut whole = TerminalModel::new(TerminalSize::new(2, 32), 0).expect("whole model");
    let whole_update = whole.ingest(&input).expect("whole OSC 52 ingest");
    let mut chunked = TerminalModel::new(TerminalSize::new(2, 32), 0).expect("chunked model");
    let mut chunk_effect = None;
    let mut chunk_events = Vec::new();
    for chunk in input.chunks(7) {
        let update = chunked.ingest(chunk).expect("chunked OSC 52 ingest");
        if update.host_effect.is_some() {
            chunk_effect = update.host_effect;
        }
        chunk_events.extend(update.events);
    }

    assert_same_presented_surface(&whole, &chunked);
    assert_eq!(whole_update.host_effect, chunk_effect);
    assert_eq!(whole_update.events, chunk_events);
    let TerminalHostEffect::ClipboardWrite(value) = whole_update
        .host_effect
        .as_ref()
        .expect("latest valid write");
    assert_eq!(value.as_str(), SECOND);
    assert_eq!(
        whole_update.events,
        vec![TerminalSideEvent::EffectRejected(
            RejectedEffect::ClipboardWrite
        )]
    );
    let debug = format!("{whole_update:?}");
    assert!(!debug.contains(FIRST));
    assert!(!debug.contains(SECOND));
    assert!(!debug.contains(&first));
    assert!(!debug.contains(&second));
    assert!(visible_text(&whole).contains("beforeafter"));
}

#[test]
fn osc52_exact_cap_and_both_overflow_paths_are_atomic() {
    let exact_text = "x".repeat(MAX_TERMINAL_CLIPBOARD_BYTES);
    let exact_encoded = BASE64_STANDARD.encode(&exact_text);
    assert_eq!(exact_encoded.len(), MAX_OSC52_BASE64_BYTES);
    let mut exact = TerminalModel::new(TerminalSize::new(2, 16), 0).expect("exact model");
    let exact_update = exact
        .ingest(format!("\x1b]52;c;{exact_encoded}\x07").as_bytes())
        .expect("exact cap accepted");
    let TerminalHostEffect::ClipboardWrite(value) =
        exact_update.host_effect.expect("exact cap write");
    assert_eq!(value.as_str().len(), MAX_TERMINAL_CLIPBOARD_BYTES);

    let decoded_over = BASE64_STANDARD.encode("y".repeat(MAX_TERMINAL_CLIPBOARD_BYTES + 1));
    assert_eq!(decoded_over.len(), MAX_OSC52_BASE64_BYTES);
    let decoded_over_update = exact
        .ingest(format!("\x1b]52;c;{decoded_over}\x07visible").as_bytes())
        .expect("decoded overflow contained");
    assert!(decoded_over_update.host_effect.is_none());
    assert_eq!(
        decoded_over_update.events,
        vec![TerminalSideEvent::EffectRejected(
            RejectedEffect::ClipboardWrite
        )]
    );
    assert!(visible_text(&exact).contains("visible"));

    let encoded_over = "A".repeat(MAX_OSC52_BASE64_BYTES + 1);
    let encoded_over_update = exact
        .ingest(format!("\x1b]52;c;{encoded_over}\x1b\\tail").as_bytes())
        .expect("encoded overflow contained through terminator");
    assert!(encoded_over_update.host_effect.is_none());
    assert_eq!(
        encoded_over_update.events,
        vec![TerminalSideEvent::EffectRejected(
            RejectedEffect::ClipboardWrite
        )]
    );
    assert!(visible_text(&exact).contains("tail"));
}

#[test]
fn osc52_rejects_reads_selectors_empty_noncanonical_utf8_nul_and_cancelled_input() {
    let invalid_utf8 = BASE64_STANDARD.encode([0xff]);
    let nul = BASE64_STANDARD.encode(b"a\0b");
    let cases = [
        "\x1b]52;c;?\x07".to_owned(),
        "\x1b]52;;YQ==\x07".to_owned(),
        "\x1b]52;p;YQ==\x07".to_owned(),
        "\x1b]52;c,p;YQ==\x07".to_owned(),
        "\x1b]52;c;\x07".to_owned(),
        "\x1b]52;c;YQ\x07".to_owned(),
        "\x1b]52;c;Zh==\x07".to_owned(),
        format!("\x1b]52;c;{invalid_utf8}\x07"),
        format!("\x1b]52;c;{nul}\x07"),
    ];
    for input in cases {
        let mut model = TerminalModel::new(TerminalSize::new(2, 8), 0).expect("case model");
        let update = model.ingest(input.as_bytes()).expect("rejected OSC 52");
        assert!(update.host_effect.is_none());
        assert_eq!(update.events.len(), 1);
        assert!(matches!(
            update.events[0],
            TerminalSideEvent::EffectRejected(_)
        ));
        assert!(visible_text(&model).is_empty());
    }

    let mut cancelled = TerminalModel::new(TerminalSize::new(2, 8), 0).expect("cancel model");
    let update = cancelled
        .ingest(b"\x1b]52;c;YQ==\x18ok")
        .expect("cancelled OSC 52");
    assert!(update.host_effect.is_none());
    assert!(update.events.is_empty());
    assert!(visible_text(&cancelled).contains("ok"));
}

#[test]
fn kitty_keyboard_stack_is_strict_projected_and_queryable() {
    let mut model = TerminalModel::new(TerminalSize::new(2, 16), 0).expect("keyboard model");
    let set = model.ingest(b"\x1b[=3u").expect("set flags");
    assert!(set.events.is_empty());
    assert_eq!(
        model.snapshot().surface.modes.keyboard_flags,
        TerminalKeyboardFlags::DISAMBIGUATE_ESCAPE_CODES
            .union(TerminalKeyboardFlags::REPORT_EVENT_TYPES)
    );
    let checkpoint = model.checkpoint();

    model.ingest(b"\x1b[>4u").expect("push flags");
    assert_eq!(model.snapshot().surface.modes.keyboard_flags.bits(), 4);
    let query = model.ingest(b"\x1b[?u").expect("query flags");
    assert_eq!(query.replies, b"\x1b[?4u");
    model.ingest(b"\x1b[<u").expect("pop flags");
    assert!(model.snapshot().surface.modes.keyboard_flags.is_empty());
    model.ingest(b"\x1b[=8;2u").expect("union flags");
    assert_eq!(model.snapshot().surface.modes.keyboard_flags.bits(), 8);
    model.ingest(b"\x1b[=1;3u").expect("difference flags");
    assert_eq!(model.snapshot().surface.modes.keyboard_flags.bits(), 8);
    let delta = model.delta_or_resync(&checkpoint);
    let zterm_core::terminal::TerminalSurfaceDeltaResult::Delta(delta) = delta else {
        panic!("keyboard-only change remains a semantic delta");
    };
    assert_eq!(delta.modes.keyboard_flags.bits(), 8);

    model.ingest(b"\x1b[=u").expect("default set flags");
    assert!(model.snapshot().surface.modes.keyboard_flags.is_empty());
    model.ingest(b"\x1b[>u").expect("default push flags");
    assert!(model.snapshot().surface.modes.keyboard_flags.is_empty());
    model.ingest(b"\x1b[=4;0u").expect("default set behavior");
    assert_eq!(model.snapshot().surface.modes.keyboard_flags.bits(), 4);
    model.ingest(b"\x1b[<0u").expect("default pop count");
    assert!(model.snapshot().surface.modes.keyboard_flags.is_empty());

    model.ingest(b"\x1b[>1u").expect("push before large pop");
    let large_pop = model.ingest(b"\x1b[<4097u").expect("large pop count");
    assert!(large_pop.events.is_empty());
    assert!(model.snapshot().surface.modes.keyboard_flags.is_empty());

    for malformed in [b"\x1b[=32u".as_slice(), b"\x1b[=1;4u", b"\x1b[>1;2u"] {
        let before = model.snapshot().surface.modes.keyboard_flags;
        let update = model.ingest(malformed).expect("malformed flags contained");
        assert_eq!(
            update.events,
            vec![TerminalSideEvent::UnsupportedSequence(
                UnsupportedSequenceKind::Csi
            )]
        );
        assert_eq!(model.snapshot().surface.modes.keyboard_flags, before);
    }

    model.ingest(b"\x1bc").expect("terminal reset");
    assert!(model.snapshot().surface.modes.keyboard_flags.is_empty());

    model.ingest(b"\x1b[>1u").expect("push main flags");
    model
        .ingest(b"\x1b[?1049h")
        .expect("enter alternate screen");
    assert!(model.snapshot().surface.modes.keyboard_flags.is_empty());
    model.ingest(b"\x1b[>2u").expect("push alternate flags");
    model.ingest(b"\x1b[?1049l").expect("return to main screen");
    assert_eq!(model.snapshot().surface.modes.keyboard_flags.bits(), 1);
    model
        .ingest(b"\x1b[?1049h")
        .expect("restore alternate screen");
    assert_eq!(model.snapshot().surface.modes.keyboard_flags.bits(), 2);
    model.ingest(b"\x1bc").expect("reset both screen stacks");
    assert!(model.snapshot().surface.modes.keyboard_flags.is_empty());
}
