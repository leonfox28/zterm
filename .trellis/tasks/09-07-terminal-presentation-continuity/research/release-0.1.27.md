# v0.1.27 release — 2026-09-08

User instruction after explicit disclosure of the visual evidence gap:
“先走发布流程吧”. This authorizes the necessary work commits, version preparation,
PR push/review/merge, exact-main tag, normal protected signing approval and formal
publication. Preserve repository protections. Do not update/restart the existing
Mac daemon or touch its `main` Session.

Scope: coherent DEC 2026 host publication and bounded history progress; healthy
resize input/IME lifetime; independent coordinate generation; Android integral
layout and direct animation events; desktop final-cell diff/full blank coverage.
Unchanged wire format and current native/Android publication target set.

Known issue: Android keyboard-close can still expose a temporary blank top region
before history refill. Endpoint pixel equality is verified, but full-transition
flicker improvement is not accepted. Real Herdr and desktop GUI visual acceptance
were not completed. Publish the current improvements without claiming zero flicker,
and keep the task active for this follow-up.

Baseline/upstream main: be66a16. Prior latest: v0.1.26. Next: v0.1.27, Android
versionCode 102799. `just doctor`, full `just check`, final Android build/lint,
focused native/desktop checks and emulator acceptance passed as detailed in
`runtime-acceptance.md`. No product source changed after those final owner checks.

The canonical operator will update only Cargo.toml/Cargo.lock, push this feature
branch and create the single combined feature/version PR. Live release evidence
is recorded after publication; no remote artifact is replaced or force-moved.
