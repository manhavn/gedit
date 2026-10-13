# gedit-rs

A gedit-like text editor for Linux, written in Rust with **GTK4** and **GtkSourceView 5**.

![status](https://img.shields.io/badge/status-working-green)

## Features

- Tabbed documents (one window, close button per tab)
- Syntax highlighting with automatic language detection (GtkSourceView)
- New / Open / Save / Save As, with unsaved-changes prompts on close
- Undo / Redo
- Find & replace with match highlighting, match-case and regex toggles (Ctrl+F / Ctrl+H)
- Go to line (click "Line X, Column Y" in the status bar)
- Language selector in the status bar
- Word wrap and dark mode (View menu, persisted)
- Preferences dialog (line numbers, current-line highlight, auto indent, spaces vs tabs, word wrap, dark mode)
- Keyboard shortcuts: Ctrl+N/O/S, Ctrl+Shift+S, Ctrl+Z, Ctrl+Shift+Z/Ctrl+Y, Ctrl+F, Ctrl+H, Ctrl+W, Ctrl+Q
- Single-instance application: launching it again with files opens new tabs in the
  running window; files can also be passed on the command line

## Building

```sh
cargo build --release
./target/release/gedit-rs [FILE...]
```

### System dependencies

Normal systems only need the GTK4 + GtkSourceView5 development packages and a
C compiler:

```sh
sudo apt install pkg-config libgtk-4-dev libgtksourceview-5-dev build-essential
```

This machine has no root access and no C compiler, so the repo carries a local
toolchain shim under `.sysroot/` (created once, kept out of git):

- `.sysroot/debs/*.deb` — the `-dev` packages (headers + `.pc` files) downloaded
  with `apt-get download` and extracted with `dpkg-deb -x` into `.sysroot/root/`.
- `.sysroot/bin/pkg-config` — wrapper that resolves `.pc` files from the extracted
  sysroot (`PKG_CONFIG_PATH` + `PKG_CONFIG_SYSROOT_DIR`). Wired up via
  `[env] PKG_CONFIG` in `.cargo/config.toml`.
- `.sysroot/bin/cc` — delegates to `zig cc`, which bundles glibc startfiles;
  needed because a bare `ld`/`rust-lld` link produces binaries without crt
  startup (`_start`), i.e. entry point `0x0` and a segfault at exec.
- `.sysroot/linklib/` — unversioned `.so` symlinks for libs whose dev symlinks
  are missing on the system (glibc 2.44 dropped the `libpthread`/`libdl`/`librt`
  stubs; they alias `libc.so`).

If you install the real packages via sudo, delete `.sysroot/` and the
`linker`/`PKG_CONFIG` lines in `.cargo/config.toml`.

## Configuration

Preferences are stored in `$XDG_CONFIG_HOME/gedit-rs.ini`
(`~/.config/gedit-rs.ini`).

## Known limitations

- Files must be valid UTF-8 (no charset conversion).
- Screenshot automation is blocked by the GNOME Wayland session
  (`org.gnome.Shell.Screenshot` returns `AccessDenied`), so UI verification is
  done via clean startup logs and D-Bus single-instance checks.

## Architecture

| File | Role |
|------|------|
| `src/main.rs` | Window, header bar, notebook tabs, actions/accels, status bar, dialogs |
| `src/document.rs` | `Document`: SourceView + SourceBuffer + backing path, load/save |
| `src/search.rs` | Find/replace bar backed by `GtkSourceSearchContext` |
| `src/prefs.rs` | Preferences + `GKeyFile` persistence + color scheme |
| `src/utils.rs` | Awaitable message dialogs |
