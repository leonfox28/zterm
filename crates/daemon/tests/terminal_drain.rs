//! Zero-attachment drain and transport-drop lifecycle gate.

#[cfg(unix)]
#[path = "support/terminal_driver_fixture.rs"]
mod support;

fn main() {
    #[cfg(unix)]
    {
        if support::maybe_run_fixture_child() {
            return;
        }
        if let Err(error) = run() {
            eprintln!("terminal drain gate failed: {error}");
            std::process::exit(1);
        }
        println!("TERMINAL_DRAIN_GATE=PASS");
    }

    #[cfg(not(unix))]
    println!("TERMINAL_DRAIN_GATE=SKIPPED_NON_UNIX");
}

#[cfg(unix)]
fn run() -> Result<(), String> {
    use std::ffi::OsString;

    use support::{DEADLINE, TempMarker};
    use zterm_daemon::terminal_driver::{TerminalAttachment, TerminalDriverConfig};
    use zterm_platform::pty::PtyChildState;

    const BULK_BYTES: usize = 1024 * 1024;
    let config = TerminalDriverConfig {
        byte_channel_capacity: 2,
        read_chunk_bytes: 1024,
    };

    let marker = TempMarker::new("zero-attachment")?;
    let driver = support::spawn_driver(
        [
            OsString::from("bulk"),
            marker.argument(),
            BULK_BYTES.to_string().into(),
        ],
        config,
    )?;
    support::wait_for_marker(marker.path(), &driver)?;
    if driver
        .stats()
        .map_err(support::display_error)?
        .active_attachments
        != 0
    {
        return Err("bulk driver unexpectedly had an attachment".into());
    }
    if driver.try_wait().map_err(support::display_error)? != PtyChildState::Running {
        return Err("bulk child did not remain alive after zero-attachment output".into());
    }
    support::wait_for_text(&driver, "BULK-COMPLETE")?;
    let stats = driver.stats().map_err(support::display_error)?;
    if stats.processed_bytes < BULK_BYTES as u64 {
        return Err(format!(
            "driver processed only {} bulk bytes",
            stats.processed_bytes
        ));
    }
    if stats.maximum_pending_chunks > stats.byte_channel_capacity {
        return Err(format!("bounded channel exceeded capacity: {stats:?}"));
    }
    driver
        .write_input(b"exit\n")
        .map_err(support::display_error)?;
    support::wait_for_natural_exit(&driver)?;
    println!(
        "DRAIN_CASE=zero_attachment,bytes={},capacity={},max_pending={}",
        stats.processed_bytes, stats.byte_channel_capacity, stats.maximum_pending_chunks
    );

    struct SimulatedIrohConnectionGuard {
        _attachment: TerminalAttachment,
    }

    let driver = support::spawn_driver([OsString::from("transport")], config)?;
    support::wait_for_text(&driver, "TRANSPORT-READY")?;
    let guard = SimulatedIrohConnectionGuard {
        _attachment: driver.attach(),
    };
    drop(guard);
    if driver
        .stats()
        .map_err(support::display_error)?
        .active_attachments
        != 0
    {
        return Err("connection guard drop retained an attachment".into());
    }
    if driver.try_wait().map_err(support::display_error)? != PtyChildState::Running {
        return Err("connection guard drop terminated the child".into());
    }
    driver
        .write_input(b"probe\n")
        .map_err(support::display_error)?;
    support::wait_for_text(&driver, "CHILD-STILL-RUNNING")?;
    driver
        .write_input(b"exit\n")
        .map_err(support::display_error)?;
    support::wait_for_natural_exit(&driver)?;
    driver
        .wait_until_idle(DEADLINE)
        .map_err(support::display_error)?;
    println!("DRAIN_CASE=transport_drop,child_continued=true");

    let driver = std::sync::Arc::new(support::spawn_driver([OsString::from("query")], config)?);
    let waiting_driver = std::sync::Arc::clone(&driver);
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(waiting_driver.wait());
    });
    let status = receiver
        .recv_timeout(DEADLINE)
        .map_err(|error| format!("wait held the PTY reply lock: {error}"))?
        .map_err(support::display_error)?;
    if !status.success() {
        return Err(format!("query fixture did not exit cleanly: {status:?}"));
    }
    driver
        .wait_until_idle(DEADLINE)
        .map_err(support::display_error)?;
    println!("DRAIN_CASE=wait_releases_reply_lock,dsr_reply=true");
    Ok(())
}
