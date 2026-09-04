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

    use support::{DEADLINE, TempMarker};
    use zterm_core::terminal::TerminalSurfaceDeltaResult;
    use zterm_daemon::terminal_driver::TerminalDriverConfig;

    let marker = TempMarker::new("slow-attachment")?;
    let config = TerminalDriverConfig {
        byte_channel_capacity: 3,
        read_chunk_bytes: 512,
    };
    let driver = support::spawn_driver([OsString::from("burst"), marker.argument()], config)?;
    support::wait_for_text(&driver, "BASELINE-STATE")?;

    let mut slow = driver.attach();
    let initial = match slow.sync_latest().map_err(support::display_error)? {
        TerminalSurfaceDeltaResult::Resync(snapshot) => snapshot,
        TerminalSurfaceDeltaResult::Delta(_) => {
            return Err("initial attachment unexpectedly returned delta".into());
        }
    };
    let initial_revision = initial.revision;
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
        TerminalSurfaceDeltaResult::Resync(snapshot) => snapshot,
        TerminalSurfaceDeltaResult::Delta(_) => {
            return Err("discarded slow watermark did not force a full resync".into());
        }
    };
    let latest = driver
        .attach()
        .latest_snapshot()
        .map_err(support::display_error)?;
    if snapshot.surface != latest.surface {
        return Err("slow attachment resync was not semantically latest".into());
    }
    if !support::snapshot_text(&snapshot)?.contains("LATEST-STATE") {
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
