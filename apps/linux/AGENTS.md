# Linux App Agent Guide

This guide is for agents working in `apps/linux`.

## Goal

- Build Linux app behavior with parity to the macOS client where applicable.
- Reuse existing Gateway/Node patterns already implemented by the macOS app.

## macOS Client Docs (Reference)

Use these as the canonical product/behavior docs for the macOS client:

- https://docs.openclaw.ai/platforms/macos
- https://docs.openclaw.ai/platforms/mac/menu-bar
- https://docs.openclaw.ai/platforms/mac/health
- https://docs.openclaw.ai/platforms/mac/webchat
- https://docs.openclaw.ai/platforms/mac/canvas
- https://docs.openclaw.ai/platforms/mac/voicewake
- https://docs.openclaw.ai/platforms/mac/remote
- https://docs.openclaw.ai/platforms/mac/child-process
- https://docs.openclaw.ai/platforms/mac/bundled-gateway
- https://docs.openclaw.ai/platforms/mac/xpc
- https://docs.openclaw.ai/platforms/mac/permissions
- https://docs.openclaw.ai/platforms/mac/skills
- https://docs.openclaw.ai/platforms/mac/peekaboo
- https://docs.openclaw.ai/platforms/mac/logging
- https://docs.openclaw.ai/platforms/mac/dev-setup

## macOS App Source (Reference)

Read these before porting behavior into Linux:

- `apps/macos/README.md`
- `apps/macos/Package.swift`
- `apps/macos/Sources/OpenClaw/`
- `apps/macos/Sources/OpenClawIPC/IPC.swift`
- `apps/macos/Sources/OpenClawDiscovery/`
- `apps/macos/Tests/OpenClawIPCTests/`

## Linux App Source (You Own This)

- `apps/linux/openclaw-linux/`
- `apps/linux/openclaw-ui-gtk/`
- `apps/linux/openclaw-core/`

## Crate Responsibilities

- `openclaw-core`
  - Platform-agnostic core contracts and runtime logic.
  - Owns node runtime command semantics (`src/node_runtime.rs`).
  - Owns UI abstraction contracts (`src/ui.rs`): `UiApp`, `UiControl`, `UiEventSink`, `NodeStatusView`.
  - Must not depend on GTK/WebKit or Linux-specific UI crates.
- `openclaw-ui-gtk`
  - GTK/WebKit implementation of the `openclaw-core` UI contracts.
  - Exposes `create_app()` that returns `Box<dyn UiApp>`.
  - Contains all GTK/WebKit code paths and canvas window rendering.
- `openclaw-linux`
  - Linux host/orchestrator binary.
  - Wires gateway/node session and runtime to UI via `openclaw-core` interfaces.
  - Depends on `openclaw-ui-gtk` only as a UI provider entrypoint, not for GTK types.

## Architecture Guardrails

- Dependency direction is one-way:
  - `openclaw-core` <- `openclaw-ui-gtk`
  - `openclaw-core` <- `openclaw-linux`
  - `openclaw-linux` -> `openclaw-ui-gtk` (factory/wiring only).
- Do not import `gtk4` or `webkit6` in `openclaw-linux`.
- Do not reference `openclaw-ui-gtk` concrete widget/command types in core or Linux runtime logic.
- New host-to-UI behavior must be added to `openclaw-core/src/ui.rs` first, then implemented in `openclaw-ui-gtk`.
- New node runtime command semantics should live in `openclaw-core/src/node_runtime.rs`; adapters in `openclaw-linux` should remain thin.
- Keep UI implementation details private to `openclaw-ui-gtk`; expose only trait-based interfaces across crate boundaries.
- If a change forces bidirectional crate coupling, stop and redesign around core traits/events before shipping.

## Change Checklist (Linux UI/Core)

1. Define or update interface contracts in `openclaw-core`.
2. Implement the contract in `openclaw-ui-gtk`.
3. Wire usage in `openclaw-linux` without importing GTK/WebKit types.
4. Add/adjust tests in `openclaw-core` for runtime semantics and in `openclaw-ui-gtk` for UI command planning.

## Parity Checklist (When Adding Features)

1. Find the matching macOS doc page first.
2. Read the corresponding macOS source implementation.
3. Mirror behavior in Linux unless platform constraints require a divergence.
4. Document any intentional Linux-specific divergence in PR notes or inline comments.
