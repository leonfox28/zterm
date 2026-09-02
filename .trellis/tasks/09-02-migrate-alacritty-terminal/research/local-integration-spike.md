# Local libghostty-vt Integration Spike

> Re-audit note (2026-09-02): this document proves only the candidate build/API probe. The tested
> `5988a0b...` revision still contains the open safe-API capacity issue #70, so it is not an approved
> production cutover pin. See `plan-reaudit-2026-09-02.md` for Gate A.

## Scope

This spike was intentionally built under `/tmp` and did not modify product
code. It answers whether Zterm's pinned Rust toolchain can consume the current
safe Rust wrapper, statically link `libghostty-vt`, and exercise the APIs needed
for the migration on the available macOS arm64 host.

## Inputs

- Host: macOS arm64.
- Rust: the workspace-pinned Rust 1.98.0 toolchain.
- Zig: 0.16.0 arm64 macOS archive, SHA-256
  `b23d70deaa879b5c2d486ed3316f7eaa53e84acf6fc9cc747de152450d401489`.
- Rust wrapper: `https://github.com/Uzaaft/libghostty-rs.git` at exact revision
  `5988a0b78b4aa804d1c12e66bbfe662bd97d81c0`, with default features disabled.
- Ghostty source selected by that wrapper: exact commit
  `22d13172cde98a0a4dda05d3d6a3fcb0dd8ed018`.

The published crates.io `libghostty-vt = 0.2.1` was tried first. Its build
selected the older Ghostty commit
`a887df42c56f6de86c0fe6da9c4eeca37931e083` and required Zig 0.15.2, while the
repository at the same nominal crate version requires Zig 0.16.0 and contains
newer scrollback, snapshot, iOS, Android, and Windows work. The published crate
therefore cannot be treated as equivalent to the current Git source.

## Exercised Boundary

The temporary Rust executable used only the safe `libghostty-vt` crate. It did
not import `libghostty-vt-sys`, declare `extern` functions, or contain Zterm
product changes. It exercised:

- terminal create and drop;
- a four-line scrollback limit;
- ordered VT writes;
- primary-screen, mouse, focus, and SGR mouse-mode reads;
- synchronous PTY-response and bell callbacks;
- VT formatting of terminal state;
- plain formatting including retained history; and
- a bounded two-row history selection addressed in history coordinates.

The run produced:

```text
LIBGHOSTTY_PROBE=PASS scrollback_rows=3 replies=6 formatted=61 full_text="one\ntwo\nthree\nfour\nfive"
```

The generated debug executable was 5.7 MiB and depended dynamically only on
`/usr/lib/libSystem.B.dylib`. The generated `libghostty-vt.a` was 14 MiB. The
complete temporary Cargo target directory was 610 MiB, which is build-cache
evidence rather than shipped-size evidence.

## Conclusions and Limits

The spike establishes that the current safe wrapper and static C library can
work with Zterm's Rust version on one supported host and that the APIs needed
for authoritative scrollback, history selection, modes, callbacks, and
formatting exist. It does not establish:

- either macOS x86_64 or Linux release target;
- the macOS 13 or glibc 2.28 deployment floors;
- Windows runtime behavior;
- iOS or Android linkage;
- parity with Zterm's complete corpus, snapshot/delta wire contract, resource
  limits, or daemon concurrency model; or
- release reproducibility and SBOM/license integration.

Those remain explicit implementation acceptance gates. The probe also exposed
two architecture constraints:

1. all safe wrapper handle types are deliberately `!Send + !Sync`, so the
   Ghostty terminal must be created, used, and destroyed on one owner thread;
2. the wrapper build may fetch its pinned Ghostty source during a Cargo build,
   so CI/release must pin Zig and make the exact native-source acquisition and
   cache boundary explicit rather than relying on an ambient toolchain.
