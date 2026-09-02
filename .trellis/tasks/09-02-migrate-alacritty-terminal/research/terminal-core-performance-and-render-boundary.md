# Terminal Core Performance and Rendering Boundary

> **Decision update (2026-09-02):** 本文的候选性能 qualification 建议已被用户明确取消。
> 最终选型为官方 `alacritty_terminal 0.26.0`，迁移任务不运行或新增 throughput/latency/
> CPU/RSS benchmark，也不据此作性能结论。本文仅保留“terminal state core 不是 pixel
> renderer”的研究历史，不进入实施上下文。

Date: 2026-09-02

## Finding

`alacritty_terminal` 和 `libghostty-vt` 都是 terminal-emulation/state cores，不是最终的
pixel renderers。性能讨论必须区分 parser throughput、state mutation、render-state
projection、Zterm wire encoding/network 和客户端 drawing；不同层的 benchmark 不能互相
替代。

## Layer Model

```text
PTY output bytes
    │
    ├─ 1. VT parser: ANSI/OSC/DCS/APC → actions
    ├─ 2. terminal state: grid, cursor, modes, scrollback, reflow
    ├─ 3. presentation state: visible cells, styles, damage, snapshot/delta
    └─ 4. renderer: shaping, glyph cache/atlas, GPU/CPU drawing, window/view
```

| Component | Layers supplied | Not supplied |
| --- | --- | --- |
| `alacritty_terminal` | 1, 2, and layer-3 `RenderableContent`/`TermDamage` primitives | Alacritty app renderer, fonts, GPU, window/UI |
| `libghostty-vt` | 1, 2, and a richer retained layer-3 render-state/dirty-row API | drawing, shaping, GPU, window/UI |
| Full Alacritty app | terminal core plus separate `alacritty/src/renderer` | Zterm wire/session semantics |
| Full Ghostty app | terminal core plus separate `src/renderer` and platform UI | Zterm wire/session semantics |
| Zterm daemon | 1–2 via selected engine; Zterm-owned layer-3 snapshot/delta/history | pixels |
| Zterm CLI/mobile client | consumes Zterm wire; outer terminal or future mobile view performs layer 4 | authoritative child VT parsing |

Ghostty's official Ghostling example states explicitly that `libghostty-vt` contains no renderer
drawing or windowing code. Its `render.h` is named for data prepared for a renderer: update from a
terminal, inspect cells/cursor/colors/dirty rows, then let the caller draw. Alacritty likewise keeps
its application renderer outside the `alacritty_terminal` crate.

The terminology is confusing because terminal multiplexers also say they “render” when they convert
their internal grid back into ANSI for a parent terminal. That is projection/encoding, not pixel
drawing.

## Performance Evidence Assessment

### What supports the Ghostty performance claim

- Ghostty documents CPU-specific SIMD parsing and a multi-threaded read/write/render architecture.
- `libghostty-vt` shares the production Ghostty core and its page/memory/Unicode work.
- Ghostty maintains separate `TerminalParser` and `TerminalStream` benchmarks. The first measures
  parser actions only; the second applies realistic byte chunks to a full terminal state.

It is therefore technically plausible, and likely for some workloads, that Ghostty's parser/state
engine outperforms `alacritty_terminal`, especially for bulk ASCII/ANSI output or many concurrent
sessions.

### What is not established

- Ghostty's in-tree benchmarks do not compare `libghostty-vt` and `alacritty_terminal` in one harness.
- Parser-only throughput omits grid writes, scrollback, Unicode, reflow, damage, projection and FFI.
- Full Ghostty-vs-Alacritty GUI benchmarks also include different renderer, windowing, frame pacing,
  font caches and threading; those are not used by the Zterm daemon.
- No inspected result uses Zterm's real limits, hostile-output corpus, snapshots/deltas, one-session-
  one-PTY model or multi-session RSS budget.

Ghostty's own current README describes full Ghostty and Alacritty as usually within a few percentage
points and in the same high-performance category. This is an upstream characterization, not an
independent guarantee, but it is enough to reject the unqualified statement that Alacritty is simply
“slow.”

## Zterm-Relevant Qualification Race

Do not select an engine using a standalone parser microbenchmark. A single release-mode harness must
separate and then combine:

1. raw PTY bytes → fully updated terminal state;
2. visible cells/damage extraction;
3. snapshot, delta-or-resync and history-page encoding;
4. resize/reflow at the 240x80 viewport and 2,000-row history limits;
5. plain output, ANSI-heavy TUIs, alternate screen, Unicode/wide/combining text and adversarial long
   control strings;
6. one hot session and many concurrent sessions;
7. throughput, CPU time, allocations, peak/steady RSS and p95/p99 ingest-to-wire latency.

Run current `vt100` and `alacritty_terminal` in the same safe Rust harness. A standalone non-product
C/Zig executable may measure official `libghostty-vt` as a reference ceiling; it does not authorize
FFI in Zterm product crates. Use identical corpus, dimensions, chunking, hardware, compiler profile,
warm-up and output verification. Debug builds are invalid for comparison.

This gate answers the product question that matters: whether an engine meets Zterm's service and
resource envelope. Winning parser MB/s alone is insufficient, and a slower parser is acceptable only
while end-to-end latency and capacity remain within the agreed budget.

## Sources

- Ghostty architecture/performance and lib scope: https://github.com/ghostty-org/ghostty/blob/main/README.md
- Ghostty parser benchmark: https://github.com/ghostty-org/ghostty/blob/main/src/benchmark/TerminalParser.zig
- Ghostty full-stream benchmark: https://github.com/ghostty-org/ghostty/blob/main/src/benchmark/TerminalStream.zig
- Ghostty render-state API: https://github.com/ghostty-org/ghostty/blob/main/include/ghostty/vt/render.h
- Official Ghostling boundary: https://github.com/ghostty-org/ghostling#what-is-libghostty
- Alacritty terminal core: https://github.com/alacritty/alacritty/tree/v0.17.0/alacritty_terminal
- Alacritty application renderer: https://github.com/alacritty/alacritty/tree/v0.17.0/alacritty/src/renderer
- Zterm current model: `crates/core/src/terminal.rs`
