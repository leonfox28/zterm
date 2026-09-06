"""Private real-CLI reattach probe; --verify-fix asserts post-fix acceptance."""
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import pty
import re
import select
import shlex
import struct
import subprocess
import sys
import tempfile
import termios
import time

sys.dont_write_bytecode = True
ROOT = next(p for p in Path(__file__).resolve().parents if (p / 'Cargo.toml').is_file())
ARCHIVED = ROOT / '.trellis/tasks/archive/2026-09/09-05-fix-terminal-sync-scroll/research'
OUTPUT = ROOT / 'target/reattach-input-probe'
if '--verify-fix' in sys.argv:
    OUTPUT = OUTPUT / 'fixed'
HERDR = Path('/opt/homebrew/bin/herdr')


def build():
    OUTPUT.mkdir(parents=True, exist_ok=True)
    spec = importlib.util.spec_from_file_location('previous', ARCHIVED / 'run_cli_herdr_probe.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.build(ROOT, ARCHIVED, OUTPUT, '--observed' in sys.argv)


def run(binary, name, flags=7, herdr=False, alternate=False, reattach_columns=140):
    with tempfile.TemporaryDirectory(prefix='zt-rea-', dir='/tmp') as temporary:
        case = Path(temporary)
        config = case / 'herdr.toml'
        config.write_text('onboarding = false\n[terminal]\ndefault_shell = "/bin/sh"\nshell_mode = "non_login"\nnew_cwd = "current"\n')
        env = dict(os.environ, TERM='xterm-256color', COLORTERM='truecolor',
                   XDG_CONFIG_HOME=str(case/'config'), XDG_STATE_HOME=str(case/'state'),
                   HERDR_CONFIG_PATH=str(config), HERDR_SOCKET_PATH=str(case/'herdr.sock'),
                   HERDR_DISABLE_SOUND='1')
        if '--observed' in sys.argv:
            env['ZTERM_CAUSAL_TRACE'] = str(OUTPUT/f'{name}-trace.log')
            Path(env['ZTERM_CAUSAL_TRACE']).unlink(missing_ok=True)
        command = [str(binary), str(case)]
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 40, 140, 0, 0))
        original_termios = termios.tcgetattr(slave)
        child = None
        capture = bytearray()
        result = {'case': name, 'same_outer_pty': True, 'alternate': alternate, 'reattach_columns': reattach_columns}
        def drain(seconds, predicate=None):
            end = time.monotonic() + seconds
            while time.monotonic() < end:
                if predicate and predicate():
                    return
                if select.select([master], [], [], 0.03)[0]:
                    try:
                        data = os.read(master, 65536)
                    except OSError:
                        return
                    if not data:
                        return
                    capture.extend(data)
                elif child is not None and child.poll() is not None:
                    return
        def start():
            return subprocess.Popen(command + ['connect', 'local'], env=env,
                                    stdin=slave, stdout=slave, stderr=slave,
                                    start_new_session=True)
        def mode():
            values = re.findall(rb'\x1b\[=(\d+)u', capture)
            return int(values[-1]) if values else 0
        def detach():
            payload = (b'\x1b[93;5u\x1b[93;5:3u' + (b'\x1b[46u' if mode() & 8 else b'.')) if mode() else b'\x1d.'
            os.write(master, payload)
            drain(3)
            status = child.poll()
            if status is None:
                os.write(master, b'\x1d.')
                drain(1)
            return status
        try:
            subprocess.run(command + ['setup', '--name', 'reattach-probe', '--profile', 'official-n0'],
                           env=env, capture_output=True, timeout=20, check=True)
            child = start()
            for _ in range(12):
                os.write(master, b"printf '\\132\\124\\137\\122\\105\\101\\104\\131\\n'\r")
                drain(.25, lambda: b'ZT_READY' in capture)
                if b'ZT_READY' in capture:
                    break
            assert b'ZT_READY' in capture, 'initial shell not interactive'
            if herdr:
                launch = shlex.join(['env', f'HERDR_CONFIG_PATH={config}',
                    f'HERDR_SOCKET_PATH={case / "herdr.sock"}', 'HERDR_DISABLE_SOUND=1',
                    'SHELL=/bin/sh', str(HERDR)])
            else:
                fixture = case/'raw_child.py'
                fixture.write_text("import os, pathlib, tty\ntty.setraw(0)\n"
                    + ("os.write(1, b'\\x1b[?1049h')\n" if alternate else "")
                    + f"os.write(1, b'\\x1b[>{flags}uMODE_READY')\n"
                    f"with pathlib.Path({str(case/'received.bin')!r}).open('wb', buffering=0) as f:\n"
                    " while True:\n  data=os.read(0, 4096)\n  if not data: break\n  f.write(data)\n  os.write(1,b'INPUT:'+data.hex().encode()+b'\\r\\n')\n")
                launch = shlex.join([sys.executable, str(fixture)])
            os.write(master, launch.encode()+b'\r')
            drain(8, lambda: mode() != 0 if herdr else b'MODE_READY' in capture)
            drain(.3)
            result['initial_flags'] = mode()
            result['initial_herdr_visible'] = b'spaces' in capture if herdr else None
            if herdr:
                os.write(master, f'printf before > {case}/before.txt'.encode()+b'\x1b[13u')
                drain(1)
                result['first_input_effect'] = (case/'before.txt').exists()
            else:
                os.write(master, b'a\x1b[A\x1b[13u\x1b[99;5u')
                drain(.3)
                result['first_received_hex'] = (case/'received.bin').read_bytes().hex()
            result['first_detach_exit'] = detach()
            actual_termios = termios.tcgetattr(slave)
            result['termios_restored'] = actual_termios == original_termios
            result['termios_flag_changes'] = {str(i): [original_termios[i], actual_termios[i]] for i in range(6) if original_termios[i] != actual_termios[i]}
            actual_termios[3] &= ~getattr(termios, 'PENDIN', 0)
            expected_termios = list(original_termios)
            expected_termios[3] &= ~getattr(termios, 'PENDIN', 0)
            result['termios_restored_except_kernel_pendin'] = actual_termios == expected_termios
            assert child.poll() == 0, 'detach failed'
            result['session_list_detached'] = subprocess.run(command+['session','list','local'],
                env=env,capture_output=True,text=True,timeout=10,check=True).stdout
            capture.extend(b'\nPROBE_REATTACH_BOUNDARY\n')
            reattach_start = len(capture)
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 40, reattach_columns, 0, 0))
            child = start()
            drain(1)
            result['reattach_flags'] = mode()
            result['reattach_visible'] = len(capture)-reattach_start
            if herdr:
                os.write(master, f'printf after > {case}/after.txt'.encode()+b'\x1b[13u')
                drain(1)
                result['reattach_input_effect'] = (case/'after.txt').exists()
                # Herdr's own quit command provides an independent visible effect.
                os.write(master, b'\x1b[98;5u\x1b[98;5:3uq')
                drain(1)
                result['after_herdr_quit_flags'] = mode()
                os.write(master, f'printf shell > {case}/shell.txt'.encode()+b'\r')
                drain(.5)
                result['post_quit_shell_effect'] = (case/'shell.txt').exists()
            else:
                before = (case/'received.bin').read_bytes()
                os.write(master, b'b\x1b[B\x1b[13u\x1b[99;5u')
                drain(.5)
                result['reattach_received_hex'] = (case/'received.bin').read_bytes()[len(before):].hex()
            result['session_list_reattached'] = subprocess.run(command+['session','list','local'], env=env,capture_output=True,text=True,timeout=10,check=True).stdout
            result['second_detach_exit'] = detach()
            result['errors_visible'] = b'PROBE_CLI_ERROR' in capture
        finally:
            if child and child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait(timeout=5)
            if herdr and (case/'herdr.sock').exists():
                stopped = subprocess.run([str(HERDR),'server','stop'],env=env,cwd=case,
                                         capture_output=True,timeout=10)
                result['herdr_cleanup'] = stopped.returncode
            stopped = subprocess.run(command+['daemon','stop','--yes'],env=env,
                                     capture_output=True,timeout=20)
            result['daemon_cleanup'] = stopped.returncode
            os.close(master)
            os.close(slave)
            (OUTPUT/f'{name}.ansi').write_bytes(capture)
            (OUTPUT/f'{name}.json').write_text(json.dumps(result,indent=2)+'\n')
            print(json.dumps(result),flush=True)
            assert stopped.returncode == 0, 'private daemon cleanup failed'
        if '--verify-fix' in sys.argv:
            assert result['first_detach_exit'] == result['second_detach_exit'] == 0
            assert result['termios_restored_except_kernel_pendin']
            assert not result['errors_visible']
            before_id = re.search(r'ID: (\w+)', result['session_list_detached']).group(1)
            after_id = re.search(r'ID: (\w+)', result['session_list_reattached']).group(1)
            assert before_id == after_id, 'reattach replaced the retained Session'
            if herdr:
                assert result['first_input_effect'] and result['reattach_input_effect']
                assert result['post_quit_shell_effect'], 'Herdr quit or returned shell is unresponsive'
                assert result['herdr_cleanup'] == 0
            else:
                assert result['first_received_hex'] == b'a\x1b[A\x1b[13u\x1b[99;5u'.hex()
                assert result['reattach_received_hex'] == b'b\x1b[B\x1b[13u\x1b[99;5u'.hex()
                expected_columns = reattach_columns if alternate else reattach_columns - 1
                assert f'{expected_columns}x39' in result['session_list_reattached']
        return result


if __name__ == '__main__':
    binary = build()
    if '--verify-fix' in sys.argv:
        assert '--observed' not in sys.argv, 'acceptance uses the actual product build'
        results = [run(binary, 'generic-main-flags-7', 7),
                   run(binary, 'generic-main-flags-15', 15),
                   run(binary, 'generic-alternate-legacy', 0, alternate=True),
                   run(binary, 'generic-alternate-flags-7', 7, alternate=True),
                   run(binary, 'generic-alternate-flags-15', 15, alternate=True),
                   run(binary, 'generic-main-new-width', 7, reattach_columns=150),
                   run(binary, 'generic-alternate-new-width', 7, alternate=True, reattach_columns=150),
                   run(binary, 'herdr-reattach', herdr=True)]
        filename = 'post-fix-outcomes.json'
    elif '--matrix' in sys.argv:
        results = [run(binary, 'generic-alternate-legacy', 0, alternate=True),
                   run(binary, 'generic-main-new-width', 7, reattach_columns=150)]
        filename = 'matrix-outcomes.json'
    elif '--alternate-only' in sys.argv:
        results = [run(binary, 'generic-alternate-flags-7', 7, alternate=True)]
        filename = 'observed-outcomes.json' if '--observed' in sys.argv else 'alternate-outcomes.json'
    else:
        results = [run(binary,'generic-flags-7',7),run(binary,'generic-flags-15',15),
                   run(binary,'herdr-reattach',herdr=True)]
        filename = 'outcomes.json'
    (OUTPUT/filename).write_text(json.dumps(results,indent=2)+'\n')
