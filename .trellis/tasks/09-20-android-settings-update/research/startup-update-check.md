# Startup update check decision — 2026-09-20

The user proposed checking silently once when the app starts, showing the update
dialog only if a newer version exists, and asked how other apps handle this.
The user approved the recommended startup behavior and 24-hour same-version
reminder suppression with `按你说的来`. This extends the earlier manual-update
scope. No product code was edited during this planning decision.

## Evidence

- `MainActivity.kt:12` constructs the UI in `onCreate`, which is not a unique
  process-start signal: Activity recreation must not start another request.
- `ZtermApplication.kt:6` already owns long-lived app objects. The planned
  application update owner can deduplicate startup/manual requests independently
  of terminal busy state and Activity lifetime.
- [Google Play app updates](https://support.google.com/googleplay/answer/113412)
  supports store-managed manual and automatic updates.
- [Play in-app updates](https://developer.android.com/guide/playcore/in-app-updates)
  separates flexible and immediate update flows; the latter is intended for
  changes critical to core functionality.
- [Android update guidance](https://developer.android.com/guide/playcore/in-app-updates/kotlin-java#start_an_update)
  explicitly advises controlling how often update prompts are requested. Its
  staleness/priority examples are configurable policies, not a universal timer.

The Play APIs describe store-distributed applications. This task retains zterm's
existing GitHub APK distribution and signed-release verification. Borrowing the
interaction principles does not mean integrating Play update APIs.

## Approved policy

- Once per cold process launch after the first usable foreground screen, start
  an asynchronous metadata check. Never block first rendering or terminal work.
  Background/foreground transitions, rotations, and theme changes do not count
  as new launches. Do not introduce periodic/background scheduling.
- Automatic checks produce no loading overlay, latest-version Toast, or error
  Toast. Offline, rate-limit, and verification failures stay quiet and must not
  be interpreted internally as already current.
- A verified newer version can show the approved optional update modal when the
  app is resumed and no permission/error modal is already taking precedence.
  Never download an APK before the existing explicit confirmation.
- Persist dismissal per version: after Later, do not automatically
  prompt for that version again for 24 hours. The next eligible cold launch can
  prompt after the interval. A newly released higher version is not suppressed
  by the older version's dismissal. This is the user-approved product policy,
  not an Android platform requirement.
- Manual checks always give the original explicit feedback and bypass reminder
  suppression. Deduplicate concurrent startup/manual requests; if a manual check
  joins one in flight, its result must use manual feedback once.
- The terminal notification switch controls OSC notification delivery only;
  update dialogs are app UI and remain independent of notification permission.

The shared owner will distinguish startup/manual intent, host its modal at app
level and persist reminder metadata without racing preferences. The updated
execution plan covers silent success/failure, one check per launch, lifecycle-safe
prompts, manual bypass and suppression across restarts. Penpot reuses the approved
update modal and adds an editor annotation for the startup flow. Requirements,
design and execution planning now include this policy.
