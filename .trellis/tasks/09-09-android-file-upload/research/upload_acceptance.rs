//! Disposable real-network fixture. Ticket is written only to a private file.
use std::{fs, io::Write, os::unix::fs::OpenOptionsExt, time::Duration};
use zterm_daemon::{bootstrap::bootstrap, config::{ValidatedInfrastructure, validate_setup_input}, lifecycle::run_daemon, local_ipc::{LocalClient, LocalPairingClient}};
use zterm_platform::user_state::UserPaths;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let folder = std::path::PathBuf::from(std::env::args().nth(1).ok_or("fixture directory")?);
    fs::create_dir_all(folder.join("home"))?;
    let paths = UserPaths::for_test(nix::unistd::geteuid().as_raw(), folder.join("home"), folder.join("home/.zterm"), folder.join("run"));
    bootstrap(&paths, &validate_setup_input("upload-acceptance", ValidatedInfrastructure::OfficialN0)?)?;
    let owned = paths.clone();
    let daemon = std::thread::spawn(move || run_daemon(&owned));
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async {
        let client = LocalClient::new(paths.socket());
        for _ in 0..100 { if client.readiness().await.is_ok() { break; } tokio::time::sleep(Duration::from_millis(100)).await; }
        let pairing = LocalPairingClient::new(paths.socket());
        let ticket = tokio::time::timeout(Duration::from_secs(45), async {
            loop { match pairing.create(600).await { Ok(ticket) => break ticket, Err(_) => tokio::time::sleep(Duration::from_secs(1)).await } }
        }).await?;
        let mut file = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(folder.join("ticket.txt"))?;
        file.write_all(ticket.expose().as_bytes())?;
        println!("UPLOAD_FIXTURE_READY");
        while !folder.join("stop").is_file() { tokio::time::sleep(Duration::from_millis(250)).await; }
        client.stop(true).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })?;
    daemon.join().map_err(|_| "fixture daemon join")??;
    Ok(())
}
