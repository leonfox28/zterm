//! Notification framing must compose with ordinary output and other OSC owners.

use zterm_core::terminal::{
    TerminalHostEffect, TerminalHostEffects, TerminalNotification, TerminalSideEvent, TerminalSize,
};
use zterm_terminal::TerminalModel;

fn model() -> TerminalModel {
    TerminalModel::new(TerminalSize::new(3, 32), 10).expect("valid viewport")
}

#[test]
fn notification_forms_survive_every_chunk_boundary_and_c1_shaped_utf8() {
    let mut bytes = b"before\x1b]9;".to_vec();
    bytes.extend_from_slice("结果: done".as_bytes());
    bytes.extend_from_slice(b"\x07\x9d777;notify;");
    bytes.extend_from_slice("标题;结果;body".as_bytes());
    bytes.extend_from_slice(b"\x9c\x1b]9;same\x1b\\\x1b]9;same\x1b\\after");
    let expected = [
        TerminalNotification::osc9("结果: done".into()).expect("OSC 9"),
        TerminalNotification::osc777("标题".into(), "结果;body".into()).expect("OSC 777"),
        TerminalNotification::osc9("same".into()).expect("repeated"),
        TerminalNotification::osc9("same".into()).expect("repeated"),
    ];
    let mut baseline = model();
    baseline.ingest(b"beforeafter").expect("baseline");
    for width in 1..=bytes.len() {
        let mut terminal = model();
        let mut effects = TerminalHostEffects::default();
        for chunk in bytes.chunks(width) {
            let mut update = terminal.ingest(chunk).expect("ingest");
            assert!(update.replies.is_empty());
            assert!(update.events.is_empty());
            while let Some(effect) = update.host_effects.pop() {
                effects.push(effect);
            }
        }
        for value in &expected {
            assert_eq!(
                effects.pop(),
                Some(TerminalHostEffect::Notification(value.clone()))
            );
        }
        assert!(effects.is_empty());
        let screen = terminal.snapshot().surface;
        assert_eq!(screen.rows, baseline.snapshot().surface.rows);
        assert_eq!(screen.cursor, baseline.snapshot().surface.cursor);
    }
}

#[test]
fn invalid_notifications_are_consumed_without_effect_or_screen_residue() {
    let mut cases = vec![
        b"\x1b]9;4;1;20\x07".to_vec(),
        b"\x1b]9;4\x1b\\".to_vec(),
        b"\x1b]9;\x07".to_vec(),
        b"\x1b]9;invalid\xffutf8\x07".to_vec(),
        b"\x1b]9;embedded\x1b[31m\x07".to_vec(),
        b"\x1b]9;a\x00b\x07".to_vec(),
        "\x1b]9;a\u{9c}b\x07".as_bytes().to_vec(),
        b"\x1b]777;notify;missing-body-delimiter\x07".to_vec(),
        b"\x1b]777;notify;;\x07".to_vec(),
        b"\x1b]777;other;title;body\x07".to_vec(),
        b"\x1b]99;body\x07".to_vec(),
        b"\x1b]9;cancel\x18".to_vec(),
        b"\x1b]777;notify;cancel;body\x1a".to_vec(),
    ];
    cases.push(format!("\x1b]9;{}\x1b\\", "x".repeat(1023)).into_bytes());
    for mut bytes in cases {
        bytes.extend_from_slice(b"OK");
        let mut terminal = model();
        for byte in bytes {
            let update = terminal.ingest(&[byte]).expect("contained input");
            assert!(update.host_effects.is_empty());
            assert!(update.replies.is_empty());
        }
        let mut baseline = model();
        baseline.ingest(b"OK").expect("baseline");
        assert_eq!(
            terminal.snapshot().surface.rows,
            baseline.snapshot().surface.rows
        );
    }
}

#[test]
fn notifications_bypass_synchronized_screen_publication_and_keep_other_osc_behavior() {
    let mut terminal = model();
    terminal.ingest(b"\x1b[?1049h").expect("alternate screen");
    terminal.ingest(b"\x1b[?2026h").expect("begin hold");
    let before = terminal.snapshot();
    let mut update = terminal.ingest(
        "hidden\x1b]2;结果\x1b\\\x1b]9;结果\x07\x1b]52;c;Y2xpcA==\x07\x1b]777;notify;title;\x1b\\".as_bytes()
    ).expect("held output");
    assert_eq!(terminal.snapshot(), before);
    assert!(update.events.contains(&TerminalSideEvent::TitleChanged {
        title: "结果".into(),
        truncated: false
    }));
    let Some(TerminalHostEffect::ClipboardWrite(write)) = update.host_effects.pop() else {
        panic!("clipboard remains independent");
    };
    assert_eq!(write.as_str(), "clip");
    for expected in [
        TerminalNotification::osc9("结果".into()),
        TerminalNotification::osc777("title".into(), "".into()),
    ] {
        assert_eq!(
            update.host_effects.pop(),
            Some(TerminalHostEffect::Notification(
                expected.expect("notification")
            ))
        );
    }
    assert!(update.host_effects.is_empty());
    let end = terminal.ingest(b"\x1b[?2026l").expect("release");
    assert!(end.host_effects.is_empty());
}
