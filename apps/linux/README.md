# OpenClaw Linux app (dev)

This is the Linux desktop app scaffold. The UI is GTK4 with a WebKitGTK
webview for Canvas. The gateway is not bundled; the app spawns/attaches to the
installed `openclaw` CLI (mirrors macOS behavior).

## Quick run

From `apps/linux/`:

```bash
cargo run -p openclaw-linux
```

## Dependencies (non-Nix)

You need a Rust toolchain (stable, edition 2024) and GTK4 + WebKitGTK development packages.
Package names vary by distro; examples:

- Ubuntu/Debian: `libgtk-4-dev`, `libwebkitgtk-6.0-dev`, `pkg-config`
- Fedora: `gtk4-devel`, `webkitgtk6-devel`, `pkgconf-pkg-config`
- Arch: `gtk4`, `webkitgtk`, `pkgconf`

If the build cannot find GTK/WebKit, verify `pkg-config` is installed and that
the development packages include `.pc` files in the default search paths.
For HTTPS/WSS support in WebKitGTK at runtime, install `glib-networking` (or
the distro equivalent). If missing, Canvas may show "TLS support is not
available" for `https://` targets.

## Optional: Nix dev shell

From `apps/linux/`:

```bash
nix-shell
cargo run -p openclaw-linux
```

The Nix shell is optional; the project does not require Nix.
