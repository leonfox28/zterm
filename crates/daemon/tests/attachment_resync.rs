//! Slow-attachment latest-state resynchronization gate.

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
            eprintln!("attachment resync gate failed: {error}");
            std::process::exit(1);
        }
        println!("ATTACHMENT_RESYNC_GATE=PASS");
    }

    #[cfg(not(unix))]
    println!("ATTACHMENT_RESYNC_GATE=SKIPPED_NON_UNIX");
}

#[cfg(unix)]
fn run() -> Result<(), String> {
    use std::ffi::OsString;

    use support::{DEADLINE, INITIAL_SIZE, SCROLLBACK_ROWS, TempMarker};
    use zterm_core::terminal::TerminalDeltaResult;
    use zterm_daemon::terminal_driver::TerminalDriverConfig;
    use zterm_terminal::TerminalModel;

    let marker = TempMarker::new("slow-attachment")?;
    let config = TerminalDriverConfig {
        byte_channel_capacity: 3,
        read_chunk_bytes: 512,
    };
    let driver = support::spawn_driver([OsString::from("burst"), marker.argument()], config)?;
    support::wait_for_text(&driver, "BASELINE-STATE")?;

    let mut slow = driver.attach();
    let initial = match slow.sync_latest().map_err(support::display_error)? {
        TerminalDeltaResult::Resync(snapshot) => snapshot,
        TerminalDeltaResult::Delta(_) => {
            return Err("initial attachment unexpectedly returned delta".into());
        }
    };
    let initial_revision = initial.revision;
    let mut client =
        TerminalModel::new(INITIAL_SIZE, SCROLLBACK_ROWS).map_err(support::display_error)?;
    client
        .ingest(&initial.recent_history_ansi)
        .map_err(support::display_error)?;
    client
        .ingest(&initial.screen_ansi)
        .map_err(support::display_error)?;

    driver
        .write_input(b"burst\n")
        .map_err(support::display_error)?;
    support::wait_for_marker(marker.path(), &driver)?;
    support::wait_for_text(&driver, "LATEST-STATE")?;
    let latest_revision = slow
        .wait_for_revision_after(initial_revision, DEADLINE)
        .map_err(support::display_error)?;

    slow.discard_checkpoint();
    let resync = slow.sync_latest().map_err(support::display_error)?;
    let snapshot = match resync {
        TerminalDeltaResult::Resync(snapshot) => snapshot,
        TerminalDeltaResult::Delta(_) => {
            return Err("discarded slow watermark did not force a full resync".into());
        }
    };
    client = TerminalModel::new(snapshot.size, SCROLLBACK_ROWS).map_err(support::display_error)?;
    client
        .ingest(&snapshot.recent_history_ansi)
        .map_err(support::display_error)?;
    client
        .ingest(&snapshot.screen_ansi)
        .map_err(support::display_error)?;

    let latest = driver
        .attach()
        .latest_snapshot()
        .map_err(support::display_error)?;
    let mut authoritative_replay =
        TerminalModel::new(latest.size, SCROLLBACK_ROWS).map_err(support::display_error)?;
    authoritative_replay
        .ingest(&latest.recent_history_ansi)
        .map_err(support::display_error)?;
    authoritative_replay
        .ingest(&latest.screen_ansi)
        .map_err(support::display_error)?;
    if client.state() != authoritative_replay.state() {
        return Err("slow attachment resync was not semantically latest".into());
    }
    if !support::state_text(&client).contains("LATEST-STATE") {
        return Err("resynchronized client omitted latest completion state".into());
    }

    let stats = driver.stats().map_err(support::display_error)?;
    if stats.maximum_pending_chunks > stats.byte_channel_capacity {
        return Err(format!("bounded channel exceeded capacity: {stats:?}"));
    }
    if stats.processed_chunks <= u64::from(stats.byte_channel_capacity as u32) {
        return Err(format!(
            "fixture did not outrun one queue window: {stats:?}"
        ));
    }
    if latest_revision <= initial_revision {
        return Err("slow attachment did not observe a newer revision".into());
    }

    driver
        .write_input(b"exit\n")
        .map_err(support::display_error)?;
    support::wait_for_natural_exit(&driver)?;
    driver
        .wait_until_idle(DEADLINE)
        .map_err(support::display_error)?;
    println!(
        "RESYNC_CASE=slow_attachment,from={},to={},processed_chunks={},capacity={},max_pending={}",
        initial_revision,
        latest_revision,
        stats.processed_chunks,
        stats.byte_channel_capacity,
        stats.maximum_pending_chunks
    );
    Ok(())
}
