# Small motion at the end of IME opening

The user reports that the row-layer phone build feels a little better, but the
keyboard appears to move vertically by a small amount just as opening completes.
They have taken the phone away and explicitly request emulator-only testing.
Do not query, inject input into, install on or capture the physical phone during
this investigation. Preserve the installed checked row-layer build and the
user's host daemon/session. The already-collected phone trace remains local.

Classification is **undetermined**. The reported keyboard motion can differ
from the measured final Main drawing burst. First distinguish system IME position,
app toolbar position, consumed inset changes and terminal pan/source adoption;
do not assume that the symptom is caused by row diff or that the toolbar is the
keyboard. Use the production Compose grid and real system IME on a newly owned
AVD, with a local alternate-screen fixture and known row metrics. Compare each
drawn terminal/footer position to the current inset throughout the final frames,
including the transition from running to settled layout.

The existing invariant is one-way layout from consumed parent pixels, interpolated
subrow remainder during motion and integral settled terminal height. The decor
animation owner, Compose inset consumer and grid measurement must agree on the
same endpoint. Candidate owners are `TerminalGeometry.kt`, `TerminalGridLayout`
in `TerminalScreen.kt`, and `ImeAnimationState.kt`; `TerminalView` is relevant
only if its editor anchor/source behavior provably changes the system keyboard.
Expected initial changes are a temporary diagnostic based on the existing
real-IME test plus task evidence, not a new product mechanism. Choose a product
boundary after the actual sequence is observed. Preserve early requests in both
directions, retained source handoff, correct bottom anchoring and bounded row
layers. No new animation clock, timer/settle debounce, parser, wire schema or
native row-sharing change belongs to this symptom alone.

Validation should reproduce a coordinate discontinuity before a correction,
then verify continuity on the existing real-IME and geometry/rendering fixtures.
An AOSP emulator result cannot by itself prove the phone vendor's keyboard motion
fixed. Record that limit explicitly. Retire only this investigation's owned AVD
and remove temporary diagnostic hooks before producing a reviewable build.

## Android 36 observations

The task-owned `Zterm_ImeEnd_0909` AVD uses the installed Android 36 Google APIs
arm64 image. A temporary extension of
`productionLayoutReportsTheTargetBeforeTheSystemImeFinishes` observes the real
decor callbacks, current/source/target Compose insets, native View bounds and
toolbar bounds at draw traversal. It buffers content-free samples and logs them
after the scenario. The wrapper forwards the production animation owner's edges;
no production code is changed. These are layout/control observations, not a
recording of the OEM keyboard's composited pixels or a phone performance result.

1. With the AVD animator scale at 3, both directions pass. During opening the
   final IME inset is 882 → 883 px, terminal height 1148 → 1147 px, toolbar bottom
   1540 → 1539 px. After the end callback these coordinates stay fixed for all
   six observed post-end draws. Root screen Y remains zero; current/source/target
   converge to 883/883/883. No extra reversal or whole-row snap occurs.
2. At normal animator scale, use a width-matched local alternate-screen frame
   with a visible cursor and two footer rows. Post a target-size candidate when
   the early request is observed, keeping the same input epoch. Both directions
   pass again. Opening ends with inset 881 → 883 and toolbar bottom 1541 → 1539,
   then stays fixed; closing returns to the original toolbar position. This
   covers candidate adoption and CursorAnchorInfo updates, though its source
   handle is deliberately null and it does not exercise real native source work.

Logs: `/tmp/zterm-ime-end-probe-test.txt`, `/tmp/zterm-ime-end-probe-frames.txt`,
`/tmp/zterm-ime-end-candidate-test.txt`, `/tmp/zterm-ime-end-candidate-frames.txt`.
The original test file is saved at
`/tmp/zterm-ime-end-TerminalRenderingTest-before.kt` for removal of the temporary
diagnostic. The frames distinguish app layout from IME inset progress; they do
not justify a speculative terminal remainder or cursor-anchor patch.

The phone is API 37. The official SDK manager exposes
`system-images;android-37.0;google_apis;arm64-v8a` revision 6; install that image
for one matching-API check on a separate owned AVD. This is a distinct callback
compatibility check, not an unbounded emulator matrix or a substitute for the
phone vendor's IME implementation.

## Matching API result and explicit deferral

The same candidate/visible-cursor scenario passes on the owned Android 37.0
Google APIs arm64 revision-6 AVD (`Zterm_ImeEnd37_0909`). The final opening inset
is 881 → 882 → 883 px; toolbar bottom is 1541 → 1540 → 1539 px, then remains at
1539. Root Y stays zero; closing also settles without reversal. Logs:
`/tmp/zterm-ime-end37-candidate-test.txt` and
`/tmp/zterm-ime-end37-candidate-frames.txt`. The temporary test extension is
archived as `ime-end-probe.patch` and removed from the regular test source.

The user further describes a tiny overshoot beyond the opening endpoint followed
by a return, and explicitly says to record it and work on other items if it cannot
be found. **Defer this symptom**: neither API 36 nor API 37's emulator reproduction
shows it. This does not prove the real report false or establish that it is an
OEM animation feature. No production offset, remainder, anchor or IME-lifetime
change is justified by this evidence. Resume when a physical-window recording
can distinguish keyboard-surface overshoot from app toolbar motion; the phone
remains unavailable and must not be operated.

Continue the separate measured final-drawing burst in `ime-phone-trace.md` using
the owned emulator, attributing its cost before choosing the next implementation.
