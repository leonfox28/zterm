# Pre-change CLI inventory

Inspected for the 2026-09-15 task at workspace version 0.1.32 (`Cargo.toml:17`).
This records the existing implementation, not the approved target syntax.

## Evidence

- `crates/cli/src/lib.rs:31`: help/version and disabled help subcommand.
- `crates/cli/src/lib.rs:38`: five hidden internal entry flags.
- `crates/cli/src/lib.rs:148`: twelve public top-level commands.
- `crates/cli/src/lib.rs:233`, `:264`, `:303`, `:383`: nested operation enums.
- `crates/cli/src/lib.rs:625`, `:632`: identical status handlers.
- `crates/cli/src/lib.rs:648`: bare setup-sensitive invocation.
- `crates/cli/src/lib.rs:769`, `:818`: connect versus existing-only attach.
- `README.md:29`: complete public syntax.

## Existing public syntax (21 operations)

```text
zterm setup [--name <name>] [--profile <official-n0|self-hosted>] [--relay-url <https-url>]
zterm status
zterm doctor
zterm pair create [--ttl <duration-with-s|m|h-suffix>] [--qr | --qr-image <new.png>]
zterm pair accept [--stdin] [--alias <alias>]
zterm device list
zterm device rename <device> <alias>
zterm device revoke <device> [-y|--yes]
zterm connect <device|local> [--session <name-or-id>] [--takeover]
zterm session list [<device|local>]
zterm session new <device|local> <name> [--cwd <host-path>]
zterm session attach <device|local> <session> [--takeover]
zterm session rename <device|local> <session> <new-name>
zterm session close <device|local> <session> [-y|--yes]
zterm daemon status
zterm daemon stop [-y|--yes]
zterm daemon restart [-y|--yes]
zterm logs [-n|--lines <n>]
zterm reset --identity [-y|--yes]
zterm update [--version <vSEMVER>] [-y|--yes]
zterm uninstall [-y|--yes]
```

Bare invocation prints setup guidance before configuration; afterwards it
creates/attaches local main. Public output is human-readable, without JSON.
Top-level help/version are `-h/--help` and `-V/--version`.

Hidden entries remain outside public grammar: `--internal-daemon`,
`--internal-update`, `--internal-release-self-check`,
`--internal-release-verify <MANIFEST> <SIGNATURE>`, and
`--internal-release-install <DESTINATION>`.
