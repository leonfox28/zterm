use zterm_core::{Revision, terminal::TerminalSize};

// Reuse the product's sole terminal model through a disposable SessionService.
// The outer ANSI is replayed verbatim by a PTY child; no test ANSI interpreter or
// direct CLI -> terminal-engine dependency is introduced.
pub(crate) fn project_outer_child_rows(
    bytes: &[u8],
) -> Vec<Vec<zterm_core::terminal::TerminalCell>> {
    project_outer_rows(bytes)
        .into_iter()
        .take(23)
        .map(|row| row[..79].to_vec())
        .collect()
}

// The last six cells of row 24 are reserved for the replay completion marker.
pub(crate) fn project_outer_rows(bytes: &[u8]) -> Vec<Vec<zterm_core::terminal::TerminalCell>> {
    use zterm_core::{AttachmentId, DeviceId, DomainErrorKind, ResourceLimits};
    use zterm_daemon::error::DaemonError;
    use zterm_daemon::session::SessionService;
    use zterm_platform::pty::{ExplicitPtyCommand, PtyHost, PtySize};

    let temporary = tempfile::tempdir().expect("outer frame fixture");
    let transcript = temporary.path().join("frame.ansi");
    std::fs::write(&transcript, bytes).expect("retain exact outer output");
    let cwd = temporary.path().to_path_buf();
    let sessions = SessionService::with_spawner(
        DeviceId::from_array([0x42; 32]),
        ResourceLimits::default(),
        move |size, _| {
            let command = ExplicitPtyCommand::new("/bin/sh", &cwd)
                .arg("-c")
                // Replayed color/status queries receive real terminal replies.
                // They are input to this fixture, not echoed presentation.
                .arg(r#"stty -echo; cat "$1"; printf '\033[24;75HSYNCED'; read -r hold"#)
                .arg("outer-frame")
                .arg(&transcript);
            let pty = PtyHost::new()
                .spawn(command, PtySize::new(size.rows, size.columns))
                .map_err(|error| {
                    DaemonError::new(DomainErrorKind::InvalidWorkingDirectory, error.to_string())
                })?;
            Ok((pty, cwd.clone()))
        },
    );
    let principal = sessions.local_principal(AttachmentId::from_array([0x43; 16]));
    let prepared = sessions
        .prepare_attach(
            principal,
            None,
            true,
            false,
            Some(TerminalSize::new(24, 80)),
        )
        .expect("outer presentation fixture succeeds");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let rows = loop {
        let snapshot = prepared
            .attachment
            .sync_latest(Revision::ZERO)
            .expect("outer presentation fixture succeeds");
        let status: String = snapshot.surface.rows[23]
            .cells
            .iter()
            .map(|cell| cell.contents.as_str())
            .collect();
        if status.ends_with("SYNCED") {
            break snapshot
                .surface
                .rows
                .iter()
                .map(|row| row.cells.clone())
                .collect();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "outer frame replay did not finish"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    drop(prepared);
    sessions
        .shutdown()
        .expect("outer presentation fixture succeeds");
    rows
}
