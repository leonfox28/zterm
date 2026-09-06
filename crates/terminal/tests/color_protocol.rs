//! Application-neutral color conformance at the authoritative model boundary.
use zterm_core::terminal::*;
use zterm_terminal::TerminalModel;

fn model() -> TerminalModel {
    TerminalModel::new(TerminalSize::new(3, 32), 4).expect("color fixture operation succeeds")
}
fn send(model: &mut TerminalModel, input: &[u8]) -> String {
    String::from_utf8(
        model
            .ingest(input)
            .expect("color fixture operation succeeds")
            .replies,
    )
    .expect("color fixture operation succeeds")
}
fn rgb(r: u8, g: u8, b: u8) -> TerminalColorValue {
    TerminalColorValue::Rgb(r, g, b)
}
fn base() -> TerminalColorProfile {
    let mut p = TerminalColorProfile {
        appearance: TerminalAppearance::Light,
        ..Default::default()
    };
    p.values[COLOR_FOREGROUND] = rgb(20, 30, 40);
    p.values[COLOR_BACKGROUND] = rgb(240, 230, 220);
    p.values[1] = rgb(120, 20, 30);
    p
}
#[test]
fn queries_execute_at_their_stream_position_across_every_split() {
    let bytes = b"\x1b]4;1;#123456;1;?;1;rgb:f/0/a;1;?\x07\x1b]10;rgbi:.5/1/0;?\x1b\\\x1b]11;?\x07";
    let expected = "\x1b]4;1;rgb:1212/3434/5656\x07\x1b]4;1;rgb:ffff/0000/aaaa\x07\x1b]11;rgb:f0f0/e6e6/dcdc\x1b\\\x1b]11;rgb:f0f0/e6e6/dcdc\x07";
    for split in 0..=bytes.len() {
        let mut m = model();
        m.update_base_colors(base())
            .expect("color fixture operation succeeds");
        let replies = send(&mut m, &bytes[..split]) + &send(&mut m, &bytes[split..]);
        assert_eq!(replies, expected, "split {split}");
        assert_eq!(
            m.snapshot().surface.colors.profile.values[COLOR_FOREGROUND],
            rgb(127, 255, 0)
        );
    }
}
#[test]
fn all_palette_and_special_roles_set_query_and_reset_to_latest_base() {
    let mut m = model();
    m.update_base_colors(base())
        .expect("color fixture operation succeeds");
    for index in 0..=255 {
        let reply = send(
            &mut m,
            format!("\x1b]4;{index};red;{index};?\x1b\\").as_bytes(),
        );
        assert_eq!(reply, format!("\x1b]4;{index};rgb:ffff/0000/0000\x1b\\"));
    }
    for code in [10, 11, 12, 17, 19] {
        assert_eq!(
            send(
                &mut m,
                format!("\x1b]{code};#456\x07\x1b]{code};?\x07").as_bytes()
            ),
            format!("\x1b]{code};rgb:4040/5050/6060\x07")
        );
    }
    // Reset palettes and roles independently; unknown legacy values stay silent.
    send(
        &mut m,
        b"\x1b]104\x07\x1b]110\x07\x1b]111\x07\x1b]112\x07\x1b]117\x07\x1b]119\x07",
    );
    assert_eq!(m.snapshot().surface.colors.profile, base());
}
#[test]
fn extended_colors_distinguish_unknown_dynamic_and_unsupported_and_ignore_bad_pairs() {
    let mut m = model();
    assert_eq!(
        send(&mut m, b"\x1b]21;foreground=?;cursor=?;unsupported=?\x07"),
        "\x1b]21;foreground=;cursor=;unsupported=?\x07"
    );
    send(
        &mut m,
        b"\x1b]21;cursor=;cursor_text=#123456;foreground=;0=rgb:1/2/3;256=#ffffff\x07",
    );
    let colors = m.snapshot().surface.colors;
    assert!(colors.custom_cursor);
    assert_eq!(
        colors.profile.values[COLOR_CURSOR],
        TerminalColorValue::Dynamic
    );
    assert_eq!(colors.profile.values[COLOR_CURSOR_TEXT], rgb(18, 52, 86));
    assert_eq!(colors.profile.values[0], rgb(17, 34, 51));
    assert_eq!(
        colors.profile.values[COLOR_FOREGROUND],
        TerminalColorValue::Unknown
    );
    send(
        &mut m,
        b"\x1b]4;1;bad;2;#abcdef;3;#fff@0.5;4;rgb:0/0/0@1\x07",
    );
    assert_eq!(
        m.snapshot().surface.colors.profile.values[2],
        rgb(171, 205, 239)
    );
    assert_eq!(
        m.snapshot().surface.colors.profile.values[3],
        TerminalColorValue::Unknown
    );
    send(&mut m, b"\x1b]21;cursor;cursor_text\x07");
    assert!(!m.snapshot().surface.colors.custom_cursor);
}
#[test]
fn palette_only_updates_retain_grid_history_epoch_and_carry_color_revision() {
    let mut m = model();
    send(&mut m, b"1\r\n2\r\n3\r\n4\r\n5\r\n6\r\n7\r\n8");
    let before = m.snapshot();
    let checkpoint = m.checkpoint();
    send(&mut m, b"\x1b]4;1;red\x07");
    let TerminalSurfaceDeltaResult::Delta(delta) = m.delta_or_resync(&checkpoint) else {
        panic!("palette delta")
    };
    assert!(delta.row_patches.is_empty());
    assert_eq!(
        delta
            .scroll_metrics
            .expect("color fixture operation succeeds")
            .epoch,
        before
            .surface
            .scroll_metrics
            .expect("color fixture operation succeeds")
            .epoch
    );
    assert_eq!(delta.colors.changed_at, delta.to_revision);
    let mut applied = before.surface;
    delta
        .apply_to(before.revision, &mut applied)
        .expect("color fixture operation succeeds");
    assert_eq!(applied, m.snapshot().surface);
}
#[test]
fn appearance_notifications_only_follow_external_base_changes() {
    let mut m = model();
    assert!(send(&mut m, b"\x1b[?996n").is_empty());
    m.update_base_colors(base())
        .expect("color fixture operation succeeds");
    assert_eq!(
        send(&mut m, b"\x1b[?996n\x1b[?2031$p\x1b[?2031h\x1b[?2031$p"),
        "\x1b[?997;2n\x1b[?2031;2$y\x1b[?2031;1$y"
    );
    assert!(send(&mut m, b"\x1b]11;red\x07\x1b[#P\x1b[#Q\x1b]111\x07\x1b[?5h").is_empty());
    let mut next = base();
    next.values[2] = rgb(9, 8, 7);
    assert_eq!(
        m.update_base_colors(next.clone())
            .expect("color fixture operation succeeds")
            .replies,
        b"\x1b[?997;2n"
    );
    assert!(
        m.update_base_colors(next.clone())
            .expect("color fixture operation succeeds")
            .replies
            .is_empty()
    );
    next.appearance = TerminalAppearance::Dark;
    assert_eq!(
        m.update_base_colors(next)
            .expect("color fixture operation succeeds")
            .replies,
        b"\x1b[?997;1n"
    );
}
#[test]
fn reverse_maps_named_sources_once_and_keeps_literal_rendition() {
    let mut m = model();
    let mut p = base();
    p.values[0] = rgb(1, 2, 3);
    p.values[7] = rgb(7, 8, 9);
    m.update_base_colors(p)
        .expect("color fixture operation succeeds");
    send(&mut m, b"\x1b[38;2;1;2;3mX\x1b[?5h");
    let s = m.snapshot().surface;
    assert_eq!(
        s.rows[0].cells[0].style.foreground,
        TerminalColor::Rgb(1, 2, 3)
    );
    assert!(!s.rows[0].cells[0].style.inverse);
    assert_eq!(s.colors.profile.values[0], rgb(7, 8, 9));
    assert_eq!(
        s.colors.profile.values[COLOR_FOREGROUND],
        rgb(240, 230, 220)
    );
    send(&mut m, b"\x1b]10;#abcdef\x07\x1b[?5l");
    assert_eq!(
        m.snapshot().surface.colors.profile.values[COLOR_BACKGROUND],
        rgb(171, 205, 239)
    );
}
#[test]
fn stack_aliases_share_ten_slots_and_retain_unobserved_sources() {
    let mut m = model();
    send(&mut m, b"\x1b[?5h\x1b]30001\x07\x1b[?5l\x1b]30101\x07");
    let colors = m.snapshot().surface.colors;
    assert_eq!(colors.source(COLOR_FOREGROUND), COLOR_BACKGROUND);
    assert_eq!(
        colors.profile.values[COLOR_FOREGROUND],
        TerminalColorValue::Unknown
    );
    assert_eq!(send(&mut m, b"\x1b[#R"), "\x1b[?0;1#Q");
    send(&mut m, b"\x1b]10;red\x07\x1b[10#P\x1b]10;blue\x07\x1b[10#Q");
    assert_eq!(
        m.snapshot().surface.colors.profile.values[COLOR_FOREGROUND],
        rgb(255, 0, 0)
    );
    assert_eq!(send(&mut m, b"\x1b[#R"), "\x1b[?0;10#Q");
    for _ in 0..11 {
        send(&mut m, b"\x1b[#P");
    }
    assert_eq!(send(&mut m, b"\x1b[#R"), "\x1b[?10;10#Q");
    for _ in 0..11 {
        send(&mut m, b"\x1b[#Q");
    }
    assert_eq!(send(&mut m, b"\x1b[#R"), "\x1b[?0;10#Q");
    let mut m = model();
    m.update_base_colors(base()).expect("initial base");
    send(&mut m, b"\x1b[#P");
    let mut next = base();
    next.values[COLOR_BACKGROUND] = rgb(10, 20, 30);
    next.appearance = TerminalAppearance::Dark;
    m.update_base_colors(next.clone())
        .expect("replacement controller base");
    send(&mut m, b"\x1b[#Q");
    assert_eq!(
        m.snapshot().surface.colors.profile.values[COLOR_BACKGROUND],
        base().values[COLOR_BACKGROUND]
    );
    assert_eq!(
        m.snapshot().surface.colors.profile.appearance,
        TerminalAppearance::Dark
    );
    send(&mut m, b"\x1b]111\x07");
    assert_eq!(
        m.snapshot().surface.colors.profile.values[COLOR_BACKGROUND],
        next.values[COLOR_BACKGROUND]
    );
}
#[test]
fn reset_and_alternate_screen_lifetimes_preserve_the_controller_base() {
    let mut m = model();
    m.update_base_colors(base())
        .expect("color fixture operation succeeds");
    send(
        &mut m,
        b"\x1b]11;red\x07\x1b[?2031h\x1b[#P\x1b[?1049h\x1b[?1049l\x1b[!p",
    );
    assert_eq!(
        m.snapshot().surface.colors.profile.values[COLOR_BACKGROUND],
        rgb(255, 0, 0)
    );
    let mut next = base();
    next.values[COLOR_BACKGROUND] = rgb(10, 20, 30);
    m.update_base_colors(next.clone())
        .expect("color fixture operation succeeds");
    send(&mut m, b"\x1bc");
    assert_eq!(m.snapshot().surface.colors.profile, next);
    assert_eq!(
        send(&mut m, b"\x1b[?5$p\x1b[?2031$p\x1b[#R"),
        "\x1b[?5;2$y\x1b[?2031;2$y\x1b[?0;0#Q"
    );
}
#[test]
fn underline_shapes_and_colors_preserve_mixed_sgr_siblings_and_report_current_pen() {
    let mut mixed = model();
    send(&mut mixed, b"\x1b[31;58;5;2mX\x1b[0;59;48;2;1;2;3mY");
    let surface = mixed.snapshot().surface;
    assert_eq!(
        surface.rows[0].cells[0].style.foreground,
        TerminalColor::Indexed(1)
    );
    assert_eq!(
        surface.rows[0].cells[0].style.underline_color,
        TerminalColor::Indexed(2)
    );
    assert_eq!(
        surface.rows[0].cells[1].style.foreground,
        TerminalColor::Default
    );
    assert_eq!(
        surface.rows[0].cells[1].style.background,
        TerminalColor::Rgb(1, 2, 3)
    );
    for (code, style) in [
        (0, TerminalUnderline::None),
        (1, TerminalUnderline::Single),
        (2, TerminalUnderline::Double),
        (3, TerminalUnderline::Curly),
        (4, TerminalUnderline::Dotted),
        (5, TerminalUnderline::Dashed),
    ] {
        let mut m = model();
        send(
            &mut m,
            format!("\x1b[1;38;2;58;59;60;4:{code};58:2::1:2:3mX").as_bytes(),
        );
        let cell = &m.snapshot().surface.rows[0].cells[0];
        assert!(cell.style.bold);
        assert_eq!(cell.style.underline, style);
        assert_eq!(cell.style.foreground, TerminalColor::Rgb(58, 59, 60));
        assert_eq!(cell.style.underline_color, TerminalColor::Rgb(1, 2, 3));
        let reply = send(&mut m, b"\x1bP$qm\x1b\\");
        assert!(reply.contains("38;2;58;59;60"));
        assert!(reply.contains("58;2;1;2;3"));
        send(&mut m, b"\x1b[59;3;24;48;5;4mY");
        let cell = &m.snapshot().surface.rows[0].cells[1];
        assert!(cell.style.italic);
        assert_eq!(cell.style.underline, TerminalUnderline::None);
        assert_eq!(cell.style.underline_color, TerminalColor::Default);
        assert_eq!(cell.style.background, TerminalColor::Indexed(4));
    }
}
#[test]
fn malformed_colors_c1_and_capability_responses_remain_bounded_and_truthful() {
    let mut m = model();
    assert_eq!(
        send(&mut m, b"\x9d4;2;red;2;?\x9c"),
        "\x1b]4;2;rgb:ffff/0000/0000\x1b\\"
    );
    assert_eq!(
        send(
            &mut m,
            b"\x1bP$qx\x1b\\\x1bP+q436f;524742;5463;626164;544e\x1b\\"
        ),
        "\x1bP0$r\x1b\\\x1bP1+r436F=323536\x1b\\\x1bP1+r524742=38\x1b\\\x1bP1+r5463\x1b\\\x1bP0+r\x1b\\"
    );
    assert_eq!(send(&mut m, b"\x1b[?9999$p"), "\x1b[?9999;0$y");
}
