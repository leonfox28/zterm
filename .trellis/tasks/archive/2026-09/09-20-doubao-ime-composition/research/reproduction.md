# Doubao Pinyin reproduction — 2026-09-20

## Result

Reproduced on the unchanged zterm Android client with the supplied Doubao APK.
The distinguishing setting is **布局与显示 → 输入拼音显示 → 显示在输入框**
(display Pinyin in the editor). The default **显示在候选栏** (candidate bar)
works in this experiment. The user confirmed 26-key Pinyin; their display-setting
value has not been independently confirmed.

Typing `nihao`, then selecting `你好`, sends `nnini hni ha你好 ` to the host,
including a trailing space. Chinese does arrive, but earlier Pinyin fragments
have already been submitted. This is not just the expected local preedit preview.

Classification: **local implementation defect** in
`TerminalView.TerminalInputConnection`. Its existing preedit owner is appropriate;
the Editable selection/composing metadata and query results are inconsistent.
This report captures the unchanged baseline. The subsequently approved fix and
post-fix acceptance are recorded in [verification.md](../verification.md).

## Environment

| Item | Tested value |
| --- | --- |
| Source | Clean `main` at `796e13c` |
| zterm | Fresh debug build, 0.1.33 / 103399, `io.github.leonfox28.zterm.dev` |
| APK SHA-256 | `349d9962595ef7719078a6e376b5681c81b668a977d196a79939c347bd853ba2` |
| IME | `com.bytedance.android.doubaoime/.ImeService`, 1.4.5 / 100405008 |
| IME APK | `/Users/huyuanzhe/Downloads/doubaoime_v1.4.5_100405008_official_arm64_release.apk` |
| IME SHA-256 | `33a026a9ef21d86ae9a78c421c0b2f6036382c60b0859a1b052fccdcddeebf15` |
| AVD | New `Zterm_Doubao_0920`, `emulator-5560`, Pixel 9, arm64, API 36 / Android 16 |
| OS fingerprint | `google/sdk_gphone64_arm64/emu64a:16/BE2A.250530.026.F3/13894323:userdebug/dev-keys` |
| Display | 1080 × 2424, 420 dpi; software keyboard enabled with hardware keyboard present |
| IME choices | Basic/offline mode, 26-key Pinyin, merged number/symbol layout |
| Host | Rebuilt existing `presentation_fixture` helper; independent `/tmp/zterm-doubao-0920` state, daemon and Session |

The user's daemon, existing AVDs and physical phone were not operated. Basic
mode suffices to reproduce; smart/cloud mode and physical-phone behavior are
untested. An earlier relay-connected attempt and the final direct-connected run
produced the same Pinyin corruption.

## Reproduction and controls

1. Install/select the supplied IME, choose 26-key Pinyin and enable software
   keyboard display with hardware keyboard present.
2. Set Doubao **布局与显示 → 输入拼音显示 → 显示在输入框**.
3. Open the zterm terminal and its explicit keyboard button.
4. In the disposable Session, run
   `clear; stty -icanon -echo; tee /tmp/zterm-doubao-0920/repro-final.txt`.
   This records actual incoming bytes without line buffering and echoes them.
5. Tap `n`, `i`, `h`, `a`, `o` on the software keyboard, then the first candidate
   `你好`. Do not press Enter.
6. Observe the prefix corruption on screen and in the host file.

At 1080 × 2424, tap centers were `n=(753,2048)`, `i=(806,1736)`,
`h=(647,1896)`, `a=(113,1896)`, `o=(913,1736)`, candidate=`(95,1605)`.
The recorded run waited one second after each tap. Rediscover coordinates if
the display or keyboard layout changes. `adb input text` prepared only the
shell command; all Pinyin input came from actual software-key taps.

| Taps completed | Cumulative UTF-8 received by host |
| --- | --- |
| `n` | empty |
| `ni` | `n` |
| `nih` | `nni` |
| `niha` | `nnini h` |
| `nihao` | `nnini hni ha` |
| Select `你好` | `nnini hni ha你好 ` |

Exact final hex: `6e6e696e6920686e69206861e4bda0e5a5bd20`.
Durable raw observations: [repro-observations.json](./repro-observations.json).

| Editor | Pinyin display | Result |
| --- | --- | --- |
| Android Settings search | Candidate bar | Exactly `你好` after selection |
| Android Settings search | In editor | Exactly `你好`, no prefix |
| zterm raw receiver | Candidate bar | Zero bytes while composing; exactly `你好` (`e4bda0e5a5bd`) after selection |
| zterm raw receiver | In editor | Repeated Pinyin prefixes, then Chinese and a trailing space |

**Verified workaround:** select **显示在候选栏** until the editor is corrected.

## Callback evidence and root cause

[ImeTrace.java](./ImeTrace.java) is a research-only JDI observer: it reads fields
and observes method entry/exit without invoking methods in or mutating the app.
Full trace: [callbacks.log](./callbacks.log). Observed `n`, `i`, candidate sequence:

```text
setComposingText exits: buffer="n", spans=[]
getTextBeforeCursor exits: result="", buffer="n", spans=[]
[next key]
finishComposingText enters: buffer="n", spans=[]
finishComposingText exits: buffer="", spans=[]
setComposingText exits: buffer="ni", spans=[]
getTextBeforeCursor exits: result="", buffer="ni", spans=[]
[candidate selection]
getTextAfterCursor exits: result="ni", buffer="ni", spans=[]
commitText (twice), then setSelection
```

ART did not reliably expose arguments at method entry: `args=[]` is not proof
of a zero-argument call. Buffer contents and return values are directly observed.
JDI adds overhead, so the trace proves order/state, not timing. Uninstrumented
runs before and after tracing independently reproduced the host-byte failure.

1. `apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalView.kt:60`
   creates the Editable without selection metadata. At `:624`,
   `setComposingText` replaces characters but ignores `newCursorPosition`, adds
   no composing span and never calls `updateSelection`. The trace confirms no
   spans after composing updates.
2. At `:623`, inherited BaseInputConnection queries read this buffer. Installed
   Android 36 SDK `android/view/inputmethod/BaseInputConnection.java:545` uses
   Selection spans and returns empty text for a nonpositive cursor; `:605`
   treats a negative selection end as zero for the after-cursor query. Thus
   nonempty Pinyin is reported entirely after an unpositioned cursor.
3. `TerminalView.kt:594` and `:614` report buffer length as the cursor through
   EditorInfo/anchor metadata, but do not update the actual Editable. The editor
   exposes contradictory positions.
4. Doubao subsequently calls `finishComposingText`. Its internal reset policy
   is inferred from preceding invalid context, not inspected inside the IME;
   the actual call and buffer clearing are observed.
5. `TerminalView.kt:629` submits old Pinyin through `commit` (`:598`),
   `AppRepository.kt:336` queues it as committed input, and
   `crates/android/src/terminal.rs:534` admits `Action::Text`. The raw host file
   establishes that these prefixes reach the child, beyond the local preview.

Android's [InputConnection contract](https://developer.android.com/reference/android/view/inputmethod/InputConnection#setComposingText(java.lang.CharSequence,int))
requires coherent composing text, requested cursor position and selection
notifications. The [BaseInputConnection query contract](https://developer.android.com/reference/android/view/inputmethod/BaseInputConnection#getTextBeforeCursor(int,int))
reads the Editable cursor; a Canvas anchor cannot provide that state.

The project invariant already exists at `.trellis/spec/frontend/android-app.md:307`:
local preedit belongs to InputConnection and Unicode is committed once. A single
local implementation violates it; no competing host/native composition owner is
evident. Correct the editor contract before changing finalization semantics.

## Coverage gap and follow-up

`TerminalUiTest.kt:138` directly sets complete composing strings and at `:145`
finishes twice. It covers lifetime/idempotence, but does not query editor context
between Pinyin updates. That successful path misses the real IME interaction.

A focused regression should assert selection, composing spans and before/after
queries after each update, zero child bytes before confirmation and exactly one
Chinese commit. Preserve explicit finish, Unicode, rejection and input-epoch
semantics. Do not simply ignore finish calls or add IME-brand-specific branches.
See [design.md](../design.md) and [implement.md](../implement.md).

## Artifacts and checks

Local files in `target/doubao-ime-investigation/`:

- `repro.mp4`: actual software-keyboard reproduction (14-second recording limit;
  verified H.264 stream duration 9.822 seconds, 1080 × 2424).
- `repro-final.png`: clean failure after selecting the candidate.
- `layout-inline.png`: selected input-display setting.
- `control-inline-final.png`: same setting works in the ordinary field.
- `terminal-nihao-after-select.png`: candidate-bar terminal control succeeds.
- `repro-observations.json`, `build.log`, `host-build.log`, `pair.log`, `apk-sha256.txt`.

Passed: fresh debug/test APK build; isolated host rebuild; pairing instrumentation
(1 test); real-keyboard controls and repeated failing runs; raw byte and callback
capture. No product change or broader-suite result is claimed. Test processes
are shut down after collection; the AVD, disposable host state and evidence remain
available. The investigation was completed in planning; the user later approved implementation.
See the task verification report for current status.
