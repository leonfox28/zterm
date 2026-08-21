//! Side-effect-free Foundation command-line placeholder.

fn main() {
    let status = zterm_daemon::bootstrap_status();
    println!(
        "zterm {} ({}; {}/{}; bootstrap schema {})",
        status.version, status.phase, status.os, status.arch, status.schema_version
    );
}
