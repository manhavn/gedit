# gedit-rs

A gedit-like text editor for Linux, written in Rust with **GTK4** and **GtkSourceView 5**.

![status](https://img.shields.io/badge/status-working-green)

## Features

- Tabbed documents (one window, close button per tab)
- Tab and window titles track the current document (file name, or the
  numbered "Untitled Document N"); hovering a tab shows the document's full path
- Syntax highlighting with automatic language detection (GtkSourceView)
- New / Open / Save / Save As, with unsaved-changes prompts on close
- Saving a file the user can't write (e.g. root-owned) retries through the
  system's PolicyKit authentication dialog (`pkexec`); the text is piped over
  stdin, nothing touches the disk before authentication succeeds
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
source env.sh                     # wires up .sysroot shims if needed
cargo build --release
./target/release/gedit-rs [FILE...]
```

### Packaging for distribution

`build-packages.sh` builds release binaries for **amd64** and **aarch64**
and packages them into `dist/`:

```sh
./build-packages.sh              # setup + build + package
./build-packages.sh setup        # toolchains / foreign-arch sysroots only
./build-packages.sh build        # native + cross binaries
./build-packages.sh package      # dist/ artifacts from built binaries
```

| Artifact | Target | Installs on |
|----------|--------|-------------|
| `gedit-rs_<v>_amd64.deb` | x86_64 | Debian, Ubuntu, Mint… |
| `gedit-rs_<v>_arm64.deb` | aarch64 | Debian, Ubuntu, Mint… (ARM boards) |
| `gedit-rs-<v>-1.x86_64.rpm` | x86_64 | Fedora, openSUSE, RHEL… |
| `gedit-rs-<v>-1.aarch64.rpm` | aarch64 | Fedora, openSUSE, RHEL… (ARM) |
| `gedit-rs-<v>-linux-{amd64,arm64}.tar.gz` | both | any distro (`./install.sh`) |

`.deb`/`.rpm` are produced with [nfpm](https://nfpm.goreleaser.com)
(downloaded automatically into `.sysroot/bin/`), and both declare runtime
dependencies on GTK4 + GtkSourceView 5. The cross build uses
`rustup target add aarch64-unknown-linux-gnu`, Zig as the cross linker, and
an arm64 sysroot fetched from the Ubuntu ports mirror (no root required).

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

## Debugging: chrome snapshot

The GNOME Wayland session blocks external screenshot tools
(`org.gnome.Shell.Screenshot` returns `AccessDenied`), so the app can render
its own window chrome to a PNG for visual verification:

```sh
./target/debug/gedit-rs --chrome-snapshot /tmp/chrome.png
./target/debug/gedit-rs FILE… --chrome-snapshot /tmp/chrome.png   # with files open
```

It draws the header bar plus the window body (tab strip, editor, status bar)
onto a magenta backdrop and exits. Widgets that paint no background of their
own (plain Boxes) show magenta — on screen they inherit the window
background, which a child-only snapshot cannot capture. The snapshot fires on
both `activate` (no arguments) and `open` (file arguments), so document/tab
text can be checked with real files.

## Theme workaround: registering `gtk.gresource`

This system's Yaru GTK4 theme stores its CSS inside `gtk.gresource`, with
`gtk.css`/`gtk-dark.css` on disk being only `@import url("resource://…")`
wrappers. GTK loads `gtk-dark.css` first under a dark preference but only
registers the gresource as a side effect of loading `gtk.css` — so every
`resource://` import fails ("resource … does not exist"), the theme rules
never apply, and the header bar / tab strip / status bar render fully
transparent (you see the desktop behind them). `register_theme_gresource()`
in `src/main.rs` registers the active theme's `gtk.gresource` with GLib
before GTK parses any CSS, fixing the ordering.

## Known limitations

- Files must be valid UTF-8 (no charset conversion).
- The editor uses GtkSourceView's light style scheme regardless of the GTK
  color scheme.

## Architecture

| File | Role |
|------|------|
| `src/main.rs` | Window, header bar, notebook tabs, actions/accels, status bar, dialogs |
| `src/document.rs` | `Document`: SourceView + SourceBuffer + backing path, load, text access |
| `src/search.rs` | Find/replace bar backed by `GtkSourceSearchContext` |
| `src/prefs.rs` | Preferences + `GKeyFile` persistence + color scheme |
| `src/utils.rs` | Awaitable message dialogs + elevated (`pkexec`) writes |
