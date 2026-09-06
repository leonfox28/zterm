# Nested application color mismatch: initial diagnosis

## User evidence

On 2026-09-06, the user supplied two screenshots from `zterm connect dev`:

- The shell view has a near-white background, dark default text, and a dark
  reverse-video zterm status row.
- After opening Herdr, its sidebar remains near-white while the pane containing
  Pi v0.85.0 has a pale cream background. Several cyan/yellow/gray text elements
  have visibly low contrast. The zterm status row retains the same appearance.

No screenshot of the same remote Herdr session through another connection is
available yet. The deployed zterm/Herdr versions, theme settings, and original
pane creation environment are not established by these screenshots.

## Confirmed zterm behavior

1. Default colors remain semantic defaults, indexed colors remain indices,
   and RGB values remain explicit values through the terminal projection
   (`crates/terminal/src/projection.rs:187,199`).
2. The presenter emits default SGR 39/49, indexed SGR, or explicit RGB SGR;
   it performs no dark/light color conversion
   (`crates/cli/src/terminal_ui/ansi_presenter.rs:342`). Thus only defaults
   and indexed colors depend on the outer terminal's configured colors.
3. OSC 4/10/11 palette/default-color queries are outside the ingress allowlist
   (`crates/terminal/src/ingress.rs:524`). Private color-scheme query CSI ?996n
   is also unsupported by the DSR handler (`ingress.rs:419`). Engine color
   callbacks are classified unsupported rather than answered (`engine.rs:109`).
4. `TERM=xterm-256color` and `COLORTERM=truecolor` declare color capability;
   they do not synchronize the viewing client's actual background or appearance
   (`crates/platform/src/pty.rs:730`).

## Relevant upstream evidence

Herdr source was inspected at the repository's earlier integration reference
`cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6` (not asserted to be the user's installed
version):

- [Terminal theme protocol](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/terminal_theme.rs#L52)
  defines OSC 10/11 queries, optional OSC 4 palette queries, a CSI ?996n appearance
  query, and appearance change reporting. Background luminance can infer appearance.
- [Theme synchronization](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/app/theme_sync.rs#L4)
  maintains host appearance separately from concrete host colors, updates its UI
  theme, and applies host colors/appearance to nested pane runtimes.
- [Pane theme handling](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/pane/terminal.rs#L1154)
  applies host palette/defaults while tracking child-owned default-color overrides.
- The [current configuration documentation](https://herdr.dev/docs/configuration/#theme)
  distinguishes Herdr's own UI theme, an option to use the host ANSI palette,
  optional automatic appearance switching, and a sidebar background that can
  retain the host background. This is context, not evidence of the user's settings.

## Classification and confidence

**Reported screenshot root cause: undetermined.**

There is a confirmed missing color/appearance negotiation capability at the
zterm terminal boundary. A nested application relying on those queries cannot
learn the outer terminal's colors through this path. If faithful nested theme
awareness is required, that is a missing boundary contract rather than a
Herdr-specific rendering rule.

It is not yet proven that this missing capability caused the photographed cream
background or low-contrast text. Independently configured application colors,
retained pane palette/default-color state, and application appearance fallback
remain plausible. A background difference alone does not prove an error because
Herdr's sidebar and embedded terminal have separate style ownership.

No evidence currently establishes zterm performing an incorrect global dark/light
switch or numerically altering explicit RGB values. Screenshots cannot establish
which escape sequences originally produced a cell color.

## Next discriminating evidence

- Compare the same Herdr session on `dev` through ordinary SSH and through zterm,
  with the same outer terminal theme. Reusing the same pane reduces configuration
  and application-version differences. Account for reattachment changing shared
  Herdr host-theme state when interpreting the comparison.
- A later application-neutral probe should compare responses to OSC 10/11/4
  and CSI ?996n, plus default/indexed/RGB color swatches. No such runtime probe
  or remote reproduction was executed in this turn.
- Once the failing boundary is established, specify which viewer owns the
  effective appearance of a retained Session and how reattachment/theme changes
  are reported. Preserve explicit child RGB and deliberate color overrides.

## Planning implication (proposal)

Investigate nested application color/appearance compatibility before treating
the feature as only theme presets and UI styling. This adds a candidate scope
decision, not implementation approval or a commitment to blindly forwarding
terminal queries across the remote connection.
