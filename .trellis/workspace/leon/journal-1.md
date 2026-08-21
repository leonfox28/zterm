# Journal - leon (Part 1)

> AI development session journal
> Started: 2026-08-20

---



## Session 1: Phase Zero relay bootstrap and v0.1.0 release

**Date**: 2026-08-21
**Task**: Phase Zero relay bootstrap and v0.1.0 release
**Branch**: `main`

### Summary

Completed the Rust 1.98.0 workspace and quality gates, published unified product version 0.1.0 with isolated GHCR production/development channels, deployed the official Iroh 1.0.3 relay by immutable production digest under the 1Panel Compose root, verified public authenticated relay fallback with QAD disabled, and exercised rollback plus production restore.

### Git Commits

| Hash | Message |
|------|---------|
| `43b06ff` | (see git log) |
| `a346a45` | (see git log) |
| `b9cac37` | (see git log) |
| `3b10aa8` | (see git log) |

### Status

[OK] **Completed**


## Session 2: Simplify relay release and deployment

**Date**: 2026-08-21
**Task**: Simplify relay release and deployment
**Branch**: `main`

### Summary

Simplified Relay version mapping and deployment, released v0.1.1 to GHCR, migrated the default server to the minimal zterm-relay Compose, verified one authenticated handshake, and removed duplicate tag-triggered CI.

### Git Commits

| Hash | Message |
|------|---------|
| `c2b574d` | (see git log) |
| `92dda0e` | (see git log) |
| `55563c3` | (see git log) |

### Status

[OK] **Completed**
