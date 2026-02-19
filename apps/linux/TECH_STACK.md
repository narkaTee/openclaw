# OpenClaw Linux app tech stack

This document summarizes the initial Linux desktop tech choices.

## Goals
- Native UI at least on KDE and GNOME
- Single-process app with Rust core + UI.
- Reuse existing OpenClaw gateway via the CLI (no embedded gateway binary).
- Canvas uses an embedded webview (mirrors macOS).
- Portable distribution with a preference for AppImage.

## Selected stack
- Language: Rust (edition 2024).
- UI: GTK4 (native widgets).
- Canvas webview: WebKitGTK (embedded web content).
- Process model: single binary that spawns/attaches to the `openclaw` CLI gateway.
- Packaging: AppImage-first, with a flexible path to add additional options later.

## Rationale (brief)
- GTK4 gives a native Linux UI with broad distro support and GNOME parity while still usable on KDE.
- WebKitGTK is the most common embedded webview on Linux for GTK apps.
- Spawning the existing `openclaw` CLI matches macOS behavior and avoids bundling.
- AppImage provides the simplest "download and run" experience.

## Notes
- Tray integration is deferred; initial builds run as a normal windowed app.
- If GTK maturity or integration becomes limiting, keep the core/UI boundary clean to allow a future Qt UI.
