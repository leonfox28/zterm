# Output format proposal

Status: style accepted by the user on 2026-09-16 ("按这种风格统一吧").
PRD R5 owns the requirements; design.md maps the examples to available data.

## Evidence and direction

The existing renderers already use names-first text tables and a full ID on the
following line (`crates/cli/src/lib.rs`, `render_devices` / `render_sessions`).
Status currently repeats State and Daemon and mixes aligned/unstructured fields.
Confirmations already enumerate affected Sessions. Retain these useful data
owners and apply modest layout/wording changes rather than new output modes.

Accepted common style:

- English human-readable text, with no new output-format flags or JSON mode.
- Aligned field labels for a single object; names-first tables for lists.
- Full IDs remain copyable on a separate line; no shortened-ID selector promise.
- Ordinary output uses plain text, without boxes, emoji or animation. Pair QR
  retains the required fixed monochrome terminal rendering.
- Include the target on Session listings and target-specific results/errors.
- Short success messages; errors state what failed and give an actionable hint
  when the existing typed result supports one. Do not infer permission,
  reachability or successful mutation from incomplete observations.
- Progress, errors and interactive prompts use stderr; results use stdout.
  Doctor's check report and logs' record tail are command results on stdout.
  Pair create's approved non-TTY stdout contract remains exact ticket + newline.
- Preserve substantive fields and diagnostic coverage; layout is not permission
  to drop full identities, authorization direction, viewport or failure details.

The examples below are mock output using synthetic names, version and IDs.
Angle-bracket placeholders stand for complete values, not literal/truncated output.

## Status

```text
Device:          laptop
Version:         0.1.32
Setup:           Configured
Daemon:          Running
Infrastructure:  official-n0
Network:         Online
Sessions:        2
  main
  work

Device ID: <full-device-id>
```

Setup and daemon state distinguish unconfigured from configured-but-stopped.
Version keeps the existing observation meaning; no new version negotiation.
Offline/unknown network states must reflect actual observations, not probes.
`doctor` retains detailed checks and stable diagnostic reasons in `[ok]` /
`[error]` lines and labelled fields. `logs` remains a one-shot record tail.

## Session list

```text
Target: laptop

Name  State     Size
main  Attached  120x36
  ID: <full-session-id>
work  Detached  120x36
  ID: <full-session-id>
```

Size is columns x rows. Attached/Detached describes the controller, not whether
the Session or its process is running. Empty lists retain a target-specific
hint using `zterm connect <target>` with no explicit `--session`.

## Device list

```text
Name    Connection     Known host  Allowed here
laptop  Connected      Yes         No
  ID: <full-device-id>
phone   Not connected  No          Yes
  ID: <full-device-id>
```

`Known host` means a locally stored outbound host entry, not proof that the host
currently authorizes a connection. `Allowed here` reflects inbound authorization
with Yes / No / Revoked. These labels express the accepted style; both directions remain
separate. Never label a merely unconnected peer Offline or unreachable. Preserve
the alias/remote-name/unnamed fallback. Device rename still changes a local alias,
not the remote device's configured name.

## Results, errors and confirmation

```text
Renamed session 'work' to 'build' on laptop.
Session ID: <full-session-id>
```

```text
Error: Session 'work' was not found on laptop.
Run: zterm session list --target laptop
```

Hints must be generated from the confirmed error category and correctly quote
names/targets for shell use. Unknown operation outcome and partial-success errors
must remain explicit, including a created Session ID when follow-up attach fails.

```text
Reset this device: local
Device ID: <full-device-id>

This will:
  End 2 running sessions: main, work
  Delete local identity, configuration and pairing data
  Keep the zterm executable

Continue? [y/N]:
```

The reset preflight currently supplies identity and Session names, not a device
display name, so use `local` instead of adding another observation just for this
heading. Existing no-state success, confirmation rules,
mutation target freezing and no-implicit-setup behavior remain authoritative.

## Pair create

```text
Pair this device
Expires in 10 minutes. Can be used once.

<terminal QR>

To pair manually, run 'zterm pair accept' on the other device
and paste this ticket:
<full-ticket>

The accepting device will be allowed to control this device.
```

QR and manual text carry the same single ticket. Narrow-terminal fallback keeps
the text. Pipe/redirect stdout remains just the ticket; human guidance uses stderr.
No helper command includes the bearer ticket as an argument.

## Scope

The user accepted this general layout. The examples do not change terminal
rendering, diagnostic coverage, Session persistence, permission semantics or
the agreed command tree. Complete-plan review precedes task activation.
