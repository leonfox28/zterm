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
        lib = copied / package / "lib.rs"
        with lib.open("a") as destination:
            destination.write(TRACE_HELPER)

    ui = copied / "cli/terminal_ui/session.rs"
    before = "            TerminalViewEvent::Snapshot(snapshot) => {\n"
    replace(ui, before, before + trace_statement('"UI SNAPSHOT rev={} state={:?}", snapshot.revision.get(), self.transport_state'))
    before = "                    delta_acknowledges_existing_sync(self.transport_state);\n"
    replace(ui, before, before + trace_statement('"UI DELTA from={} to={} state={:?} ack={} screen={:?}", delta.from_revision.get(), delta.to_revision.get(), self.transport_state, acknowledges_existing_sync, delta.active_screen'))
    before = "                            if let Some(size) = mode_resize {\n"
    replace(ui, before, before + trace_statement('"UI MODE_RESIZE {}x{}", size.rows, size.columns'))
    before = "        let previous = self.transport_state;\n"
    replace(ui, before, before + trace_statement('"UI STATE {:?}->{:?}", previous, next'))

    client = copied / "daemon/client/session.rs"
    before = "        let request_id = self.next_request_id;\n"
    # Restrict to the shared send function; establishment has its own request IDs.
    source = client.read_text()
    start = source.index("    async fn send<Message: prost::Message>(")
    position = source.index(before, start)
    source = source[:position] + before + trace_statement('"CLIENT SEND {:?} request={} applied={}", kind, request_id, self.applied_revision.load(Ordering::Acquire)') + source[position + len(before):]
    client.write_text(source)
    before = "                Ok(LocalAttachmentEvent::Delta(semantic))\n"
    replace(client, before, trace_statement('"CLIENT DELTA from={} to={} screen={:?}", semantic.from_revision.get(), semantic.to_revision.get(), semantic.active_screen') + before)
    before = "                Ok(LocalAttachmentEvent::Snapshot(surface))\n"
    replace(client, before, trace_statement('"CLIENT SNAPSHOT rev={}", surface.revision.get()') + before)

    view = copied / "daemon/client/view.rs"
    before = "                permit.send(event);\n"
    replace(view, before, '''                match &event {
                    Ok(TerminalViewEvent::Delta(delta)) => crate::causal_trace(format_args!("DRIVER QUEUE_DELTA from={} to={} state={:?}", delta.from_revision.get(), delta.to_revision.get(), last_state)),
                    Ok(TerminalViewEvent::Snapshot(snapshot)) => crate::causal_trace(format_args!("DRIVER QUEUE_SNAPSHOT rev={}", snapshot.revision.get())),
                    _ => {},
                }
''' + before)

    server = copied / "daemon/session.rs"
    before = "    let AttachmentSync::Awaiting {\n        revision: expected,\n        target,\n    } = attachment.sync\n"
    replace(server, before, '''    let expected_probe = match attachment.sync { AttachmentSync::Awaiting { revision, .. } => Some(revision.get()), _ => None };
    crate::causal_trace(format_args!("SERVER ACK received={} awaiting={:?}", revision.get(), expected_probe));
''' + before)
    before = "    validate_viewport(actor.limits, size)?;\n    require_resize_controller(runtime, attachment_id)?;\n"
    replace(server, before, trace_statement('"SERVER RESIZE {}x{}", size.rows, size.columns') + before)
    before = "        Some(TerminalSurfaceDeltaResult::Resync(snapshot)) => {\n            attachment.sync = AttachmentSync::Awaiting {\n"
    replace(server, before, "        Some(TerminalSurfaceDeltaResult::Resync(snapshot)) => {\n" + trace_statement('"SERVER AWAITING rev={}", snapshot.revision.get()') + "            attachment.sync = AttachmentSync::Awaiting {\n")

    wire = copied / "daemon/session_wire.rs"
    before = "            let bytes = encode_delta(0, attachment.attachment_id(), delta)?;\n"
    replace(wire, before, trace_statement('"SERVER EMIT_DELTA from={} to={} screen={:?}", delta.from_revision.get(), delta.to_revision.get(), delta.active_screen') + before)

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
