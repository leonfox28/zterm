# Remote OS clipboard alternative

> Historical research: current decisions are in ../prd.md and file-upload-scope.md.
> The image-only scope, Ctrl+V proposal and remote clipboard alternative are superseded.

Date: 2026-09-09. User asks whether uploading into the remote clipboard is simpler
and more native than uploading to a file and inserting its path. This is an
alternative under discussion, not approval to implement a clipboard backend.

## What can work

If the host already has a reachable graphical clipboard and both zterm's writer
and the AI process connect to that same graphical session, zterm can transfer
image bytes, publish them as an image clipboard format, then forward the original
paste key. The AI application owns decoding and attachment creation. This can
preserve a tool's native attachment UI better than generic path text.

The writer must confirm readiness before forwarding the key; sending the original
Ctrl+V immediately can cause the application to read the previous clipboard.
Successful clipboard publication does not itself acknowledge that the application
has consumed the image.

## Evidence and constraints

- Pi 0.85.1 `dist/utils/clipboard-native.js:6` enables its Linux native clipboard
  only when `DISPLAY` or `WAYLAND_DISPLAY` is present, and exports no native
  clipboard otherwise (`:18`). This decision is made at module initialization.
  Source inspected under the local installed package, not a remote machine.
- `dist/utils/clipboard-image.js:209` uses that native backend and/or `wl-paste`
  and `xclip`. The helpers access display-system clipboards; installing a helper
  does not create a clipboard server.
- Wayland clipboard tooling exposes image copy, display selection, seat selection
  and background data serving. Primary documentation:
  https://github.com/bugaevc/wl-clipboard
  https://github.com/bugaevc/wl-clipboard/blob/master/data/wl-clipboard.1
- Xvfb can run an X server without physical display/input hardware. Thus headless
  clipboard compatibility is possible, but needs a virtual display service,
  lifecycle/authorization handling, a clipboard owner, and AI processes launched
  with the correct display environment. Xvfb alone does not publish image data.
  Primary documentation:
  https://xorg.freedesktop.org/archive/X11R7.0/doc/html/Xvfb.1.html
  and https://github.com/astrand/xclip
- Native clipboard scope is a graphical session/seat, not a zterm Session. Multiple
  zterm Sessions using one clipboard can overwrite each other's image before the
  receiving program asynchronously reads it. Independent files avoid this shared
  slot. Isolated clipboard/display instances can mitigate this at extra cost.
- A private zterm in-memory "clipboard" is not automatically visible to unmodified
  Pi/Codex. It would need a supported OS clipboard protocol/backend or application
  integration. Replacing only `xclip`/`wl-paste` commands cannot cover tools using
  native clipboard libraries.
- Existing zterm OSC 52 support is bounded host-to-controller UTF-8 clipboard
  writes. It is not an image upload channel, and the current terminal contract
  consumes clipboard reads without replying. See
  `.trellis/spec/backend/local-daemon-ipc.md:797` and
  `.trellis/spec/backend/core-wire-domain.md`.

## Assessment

Remote clipboard is a viable native integration on a host with a suitable existing
graphical session. It is not universally simpler for server-oriented zterm: a plain
headless host has no X11/Wayland clipboard service for these readers. Providing
one would add deployment and process-environment requirements beyond uploading a
file. Do not assume the user's specific server is headless without inspection.

Recommend file upload + absolute path as the general first implementation. Native
clipboard integration could be a later explicit host capability if preserving an
AI tool's exact attachment UI becomes a firm requirement. Do not implement both
or silently choose between them in this task without resolving product scope.

The desktop Ctrl+V ownership decision remains open. A clipboard-first variant
still needs a trigger for local image acquisition/transfer and consumes/delays a
paste gesture while waiting for remote publication; it does not automatically
resolve generic terminal shortcut conflicts.

No graphical service was installed or started, no local/remote clipboard was read
or overwritten, and no remote AI runtime test was performed during this review.
