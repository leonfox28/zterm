//! Exact query replies from the authoritative terminal state.
use zterm_core::terminal::TerminalSize;
use zterm_terminal::{MAX_REPLY_BYTES_PER_UPDATE, TerminalError, TerminalModel};

#[test]
fn extended_unknown_fields_are_encoded_in_order_across_chunks() {
    let input = "\x1b]21;foreground=?;a-b=?;未知=?;cursor=?\x1b\\".as_bytes();
    for split in 0..=input.len() {
        let mut model = TerminalModel::new(TerminalSize::new(3, 20), 0).expect("model");
        let mut replies = model.ingest(&input[..split]).expect("first").replies;
        replies.extend(model.ingest(&input[split..]).expect("second").replies);
        assert_eq!(
            replies,
            b"\x1b]21;foreground=;unknown=YS1i;unknown=5pyq55+l;cursor=\x1b\\"
        );
    }
}

#[test]
fn character_size_query_uses_actual_size_even_during_synchronized_output() {
    let mut model = TerminalModel::new(TerminalSize::new(3, 20), 0).expect("model");
    assert_eq!(
        model.ingest(b"\x1b[18t").expect("query").replies,
        b"\x1b[8;3;20t"
    );
    model.resize(TerminalSize::new(7, 41)).expect("resize");
    model.ingest(b"\x1b[?2026h").expect("begin");
    let frozen = model.snapshot();
    let reply = model.ingest(b"\x1b[8;1;2t\x9b18t").expect("held query");
    assert_eq!(reply.replies, b"\x1b[8;7;41t");
    assert_eq!(model.snapshot().surface, frozen.surface);
    for input in [b"\x1b[18;0t".as_slice(), b"\x1b[?18t", b"\x1b[14t"] {
        assert!(
            model
                .ingest(input)
                .expect("unsupported query")
                .replies
                .is_empty()
        );
    }
}

#[test]
fn encoded_unknown_queries_keep_the_reply_budget() {
    let mut model = TerminalModel::new(TerminalSize::new(3, 20), 0).expect("model");
    let query = format!("\x1b]21;{}=?\x07", "x".repeat(900));
    let input = query.repeat(MAX_REPLY_BYTES_PER_UPDATE / 900 + 1);
    assert!(matches!(
        model.ingest(input.as_bytes()),
        Err(TerminalError::ReplyOverflow)
    ));
}
