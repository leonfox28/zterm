# Desktop clipboard upload design

## Dependency and ownership

Requires verified shared service child. Implement parent R1/R4/R5/R7-R11/R13-R14
as applicable; parent design owns all transfer constants and storage contracts.
This child owns CLI input/clipboard/rendering and Unix client facade consumption.

## Source acquisition

Add a desktop upload module under terminal_ui, with a small OS clipboard adapter
and injectable source reader for focused tests. Researched candidate arboard 3.6.1
provides file-list and decoded-image reads. Keep dependencies desktop-only and
pin verified versions; do not add a platform-helper shell script or Zterm-owned
unsafe code merely to read pasteboard data.

Read file-list before raw image. Reject multiple files, directories, unavailable
sources and text-only clipboard locally. Stage raw pixels as bounded encoded PNG;
ordinary files stream from one opened regular source and preserve bytes. Keep a
single bounded clipboard/source job whose late result cannot start an upload after
cancel/retarget. Clipboard contents and paths are absent from diagnostics.

## Input integration

Extend existing BINDINGS with plain v -> Upload and c -> CancelUpload. Do not bind
Ctrl+V or Ctrl+C. Refactor the Session command executor so non-ending commands
continue the same terminal loop; Detach remains view-only termination. Local-target
upload gives a recoverable remote-required notice before accessing the clipboard.

Upload controller records original attachment/epoch, source handle, cancellation,
progress and pause ownership. Use existing typed event routing: physical replies,
selection-copy priority and local prefix handling retain their established owners;
then the upload gate prevents new child input. Keep local scroll/selection reading
and resize/synchronization responsive. Decoder fragments, pending paste or held
key state from the pause must not revive after completion. No input replay queue.

Successful shared result is revalidated and inserted via the existing mode-aware
input path with zero modifiers, no Enter and exactly one action. Only that internal
insertion bypasses local upload pause. Cancel/error/stale authority ends the pause
without a path; same-origin synchronization uses the existing input barrier.

## Status rendering

Extend StatusRenderer with redacted transfer metadata and small transient notices.
Composition reserves right-hand display cells while left side retains target/route/
RTT as space allows. Use text_cells/Unicode widths to avoid cutting wide cells.
Choose full bar+percent+size+speed, then bar+percent, then percent-only by available
width. Error/success strings are fixed/localized content with safe filenames only.

All output goes through ComposedFrame and the sole presenter. Progress wakeups
mark the same paced render owner dirty; no stdout logging or second progress writer.
Starting/completing upload changes neither child PTY size nor reserved row count.
Parent design defines one-row handling and notice duration.

## Validation ownership

Test source-priority and exact source bytes, no fallback from multi-file clipboard,
clipboard errors/cancel, legacy/enhanced command events, preserved Ctrl+V/prefix,
input pause/no replay, stale insertion and display-cell layout. Verify source jobs
remain bounded under repeated triggers/cancel. Actual OS clipboard smoke evidence
is distinct from injected source tests; use isolated fixtures and restore clipboard
when a live test needs to replace it.

Runtime matrix: current macOS arm64 plus supported Linux X11/Wayland environments,
existing terminal modes/IME and narrow/alternate-screen status output. No Windows/
Intel macOS distribution expansion. Any unavailable runtime evidence stays explicit.
