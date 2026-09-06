"""Add observation only to isolated daemon/CLI source copies; never correct behavior."""

import os
from pathlib import Path
import shutil
import subprocess
import tomllib

TRACE_HELPER = r'''
pub(crate) fn causal_trace(args: std::fmt::Arguments<'_>) {
    use std::io::Write;
    static FILE: std::sync::OnceLock<std::sync::Mutex<std::fs::File>> = std::sync::OnceLock::new();
    let Some(path) = std::env::var_os("ZTERM_CAUSAL_TRACE") else { return; };
    let file = FILE.get_or_init(|| std::sync::Mutex::new(
        std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap()));
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_micros();
    let line = format!("{} {} {}\n", stamp, std::process::id(), args);
    file.lock().unwrap().write_all(line.as_bytes()).unwrap();
}
'''


def replace(path, before, after):
    source = path.read_text()
    assert source.count(before) == 1, (path, before, source.count(before))
    path.write_text(source.replace(before, after, 1))


def trace_statement(message):
    return f'crate::causal_trace(format_args!({message}));\n'


def instrument(root, output, libraries, native_paths):
    copied = output / "observed-source"
    for package in ("daemon", "cli"):
        shutil.copytree(root / f"crates/{package}/src", copied / package, dirs_exist_ok=True)
        with (copied / package / "lib.rs").open("a") as f:
            f.write(TRACE_HELPER)
    ui = copied / "cli/terminal_ui.rs"
    before = "        let latest_size = latest_layout.child;\n"
    replace(ui, before, before + trace_statement('"UI INITIAL provisional={:?} snapshot={:?} desired={:?} screen={:?}", initial_size, prepared.initial_snapshot().surface.size, latest_size, prepared.initial_snapshot().surface.active_screen'))
    session = copied / "cli/terminal_ui/session.rs"
    before = "        let (next, pending_resize) = self.resize_coalescer.enter_transport_state(next);\n"
    replace(session, before, before + trace_statement('"UI STATE previous={:?} effective={:?} resize={:?}", previous, next, pending_resize'))
    before = "            TerminalViewEvent::Snapshot(snapshot) => {\n"
    replace(session, before, before + trace_statement('"UI SNAPSHOT revision={} size={:?} state={:?}", snapshot.revision.get(), snapshot.surface.size, self.transport_state'))
    before = "            TerminalViewEvent::Delta(delta) | TerminalViewEvent::ResumeDelta(delta) => {\n"
    replace(session, before, before + trace_statement('"UI DELTA from={} to={} size={:?} state={:?} barrier={}", delta.from_revision.get(), delta.to_revision.get(), delta.size, self.transport_state, resume_barrier'))
    before = "                            // A paced history frame may have committed a new\n"
    replace(session, before, trace_statement('"UI INPUT state={:?} len={}", self.transport_state, bytes.len()') + before)
    client = copied / "daemon/client/session.rs"
    source = client.read_text()
    start = source.index("    async fn send<Message: prost::Message>(")
    before = "        let request_id = self.next_request_id;\n"
    index = source.index(before, start) + len(before)
    source = source[:index] + trace_statement('"CLIENT SEND {:?}", kind') + source[index:]
    client.write_text(source)
    server = copied / "daemon/session.rs"
    before = "    validate_viewport(actor.limits, size)?;\n    require_resize_controller(runtime, attachment_id)?;\n"
    replace(server, before, trace_statement('"SERVER RESIZE current={:?} requested={:?}", runtime.viewport, size') + before)
    environment = dict(os.environ)
    environment["CARGO_PKG_VERSION"] = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    observed_libraries = dict(libraries)
    dependency_dir = Path(libraries["zterm_core"]).parent
    for package in ("daemon", "cli"):
        crate_name = f"zterm_{package}"
        binary = copied / f"lib{crate_name}.rlib"
        command = ["rustc", "+1.98.0", "--edition=2024", "--crate-type=rlib", "--crate-name", crate_name,
                   "-C", "metadata=causal_observer", str(copied / package / "lib.rs"), "-o", str(binary),
                   "-L", f"dependency={dependency_dir}", "-L", f"dependency={copied}"]
        manifest = tomllib.loads((root / f"crates/{package}/Cargo.toml").read_text())
        dependencies = dict(manifest["dependencies"])
        dependencies.update(manifest.get("target", {}).get("cfg(unix)", {}).get("dependencies", {}))
        for name in dependencies:
            rust_name = name.replace("-", "_")
            command.extend(["--extern", f"{rust_name}={observed_libraries[rust_name]}"])
        for path in sorted(native_paths):
            command.extend(["-L", path])
        subprocess.run(command, env=environment, check=True)
        observed_libraries[crate_name] = str(binary)
    return observed_libraries, copied
