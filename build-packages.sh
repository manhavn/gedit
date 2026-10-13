#!/usr/bin/env bash
#
# build-packages.sh — build gedit-rs for amd64 + aarch64 and package the
# results as .deb (Debian/Ubuntu), .rpm (Fedora/openSUSE) and .tar.gz
# (everything else) into dist/.
#
# Usage:
#   ./build-packages.sh            # everything (setup + build + package)
#   ./build-packages.sh setup      # toolchains/sysroots only
#   ./build-packages.sh build      # native + cross release binaries
#   ./build-packages.sh package    # dist/ artifacts from existing binaries
#
# Build-host requirements: bash, curl, zcat, apt/apt-cache, dpkg-deb,
# rustup (with target aarch64-unknown-linux-gnu), cargo and either a system
# C compiler or zig (for linking). Missing pieces are bootstrapped into
# .sysroot/ where possible; nfpm is downloaded to create .deb/.rpm.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -1)"
[ -n "$VERSION" ] || { echo "cannot read version from Cargo.toml" >&2; exit 1; }
APP_ID="com.geditrust.Gedit"
PKGNAME="gedit-rs"
SYSROOT="$ROOT/.sysroot"
A64="$SYSROOT/arm64"
DIST="$ROOT/dist"
NFPM_VERSION="v2.48.0"
NFPM="$SYSROOT/bin/nfpm"

CODENAME="${UBUNTU_CODENAME:-$(. /etc/os-release 2>/dev/null && echo "${VERSION_CODENAME:-}" || true)}"
CODENAME="${CODENAME:-noble}"
PORTS_MIRROR="${UBUNTU_PORTS_MIRROR:-http://ports.ubuntu.com/ubuntu-ports}"
ARCHIVE_MIRROR="${UBUNTU_ARCHIVE_MIRROR:-http://archive.ubuntu.com/ubuntu}"

ZIG_BIN="${ZIG:-$(command -v zig || true)}"
if [ -z "$ZIG_BIN" ]; then
    ZIG_BIN="$(ls -d "$HOME"/.local/share/mise/installs/zig/*/bin/zig 2>/dev/null | sort -V | tail -1 || true)"
fi

log() { printf '\n== %s\n' "$*"; }
die() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }

triplet_for() { # amd64 -> x86_64-linux-gnu, arm64 -> aarch64-linux-gnu
    case "$1" in
        amd64) echo x86_64-linux-gnu ;;
        arm64) echo aarch64-linux-gnu ;;
        *) die "unknown arch $1" ;;
    esac
}

# ---------------------------------------------------------------------------
# Debian package index handling (used to fetch foreign-arch packages without
# root: apt cannot download :arm64 unless the architecture is enabled).
# ---------------------------------------------------------------------------
apt_names() { # seed packages -> transitive package names on this host
    apt-cache depends --recurse --no-recommends --no-suggests --no-conflicts \
        --no-breaks --no-replaces --no-enhances "$@" 2>/dev/null |
        grep -vE '^[[:space:]]|^\|' | sed 's/:.*//' | grep -vE '^<' | sort -u
}

index_for() { # arch -> $SYSROOT/index-<arch>.tsv  (name<TAB>pool/path<TAB>sha256)
    local arch=$1 comp out="$SYSROOT/index-$1.tsv" url
    [ -s "$out" ] && return 0
    mkdir -p "$SYSROOT"
    : > "$out.tmp"
    local mirrors
    if [ "$arch" = arm64 ]; then mirrors="$PORTS_MIRROR"; else mirrors="$ARCHIVE_MIRROR"; fi
    for comp in main universe; do
        url="$mirrors/dists/$CODENAME/$comp/binary-$arch/Packages.gz"
        log "index: $url"
        curl -sf --max-time 180 "$url" | zcat |
            awk -F': ' '
                function flush(){ if (pkg != "" && file != "") print pkg "\t" file "\t" sha }
                /^Package: /{ flush(); pkg=$2; file=""; sha="" }
                /^Filename: /{ file=$2 }
                /^SHA256: /{ sha=$2 }
                END{ flush() }' >>"$out.tmp"
    done
    mv "$out.tmp" "$out"
}

fetch_debs() { # arch destdir rootdir pkg... — download + extract, idempotent
    local arch=$1 destdir=$2 rootdir=$3
    shift 3
    local trip missing=0 name rel sha url attempt
    trip="$(triplet_for "$arch")"
    mkdir -p "$destdir" "$rootdir"
    index_for "$arch"
    local base
    if [ "$arch" = arm64 ]; then base="$PORTS_MIRROR"; else base="$ARCHIVE_MIRROR"; fi
    for name in "$@"; do
        [ -e "$destdir/.done-$name" ] && continue
        local row
        row="$(grep -m1 -F "$name	" "$SYSROOT/index-$arch.tsv" || true)"
        if [ -z "$row" ]; then
            # Not a real package (virtual name) — fine unless it is core.
            case "$name" in
                libgtk-4-dev | libgtksourceview-5-dev | libgtk-4-1 | libgtksourceview-5-0)
                    die "$name not found in $arch index" ;;
                *) echo "  skip (virtual/absent): $name"; continue ;;
            esac
        fi
        rel="$(echo "$row" | cut -f2)"
        sha="$(echo "$row" | cut -f3)"
        url="$base/$rel"
        echo "  deb: $name"
        # Corrupt/partial downloads happen; verify against the index SHA256
        # and retry a few times before giving up.
        for attempt in 1 2 3; do
            curl -sf --max-time 300 -o "$destdir/$name.deb" "$url" ||
                { echo "    download attempt $attempt failed"; sleep 2; continue; }
            if [ -n "$sha" ]; then
                echo "$sha  $destdir/$name.deb" | sha256sum -c - >/dev/null 2>&1 ||
                    { echo "    checksum attempt $attempt failed"; rm -f "$destdir/$name.deb"; sleep 2; continue; }
            fi
            if dpkg-deb -x "$destdir/$name.deb" "$rootdir" 2>/dev/null; then
                touch "$destdir/.done-$name"
                break
            fi
            echo "    extract attempt $attempt failed"; rm -f "$destdir/$name.deb"; sleep 2
        done
        [ -e "$destdir/.done-$name" ] || die "could not install $name after retries"
    done
    fix_broken_symlinks "$rootdir" "$trip"
}

fix_broken_symlinks() { # sysroot-root-dir triplet
    local root="$1" trip="$2" l t
    while IFS= read -r l; do
        [ -e "$l" ] && continue
        t="$(readlink "$l")"
        case "$t" in
            /*)
                if [ -e "$root$t" ]; then ln -sfn "$root$t" "$l"; fi ;;
            *)
                if [ -e "$(dirname "$l")/$t" ]; then :
                elif [ -e "$root/usr/lib/$trip/$t" ]; then ln -sfn "$root/usr/lib/$trip/$t" "$l"
                fi ;;
        esac
    done < <(find "$root/usr/lib" "$root/lib" -maxdepth 3 -type l 2>/dev/null || true)
}

# ---------------------------------------------------------------------------
# Host setup: system pkg-config + cc when available, local .sysroot otherwise.
# ---------------------------------------------------------------------------
setup_host() {
    if command -v pkg-config >/dev/null 2>&1 && pkg-config --exists gtk4 gtksourceview-5 2>/dev/null; then
        log "host: using system pkg-config (gtk4 + gtksourceview-5 found)"
    elif [ -x "$SYSROOT/bin/pkg-config" ]; then
        log "host: reusing local sysroot pkg-config"
    else
        log "host: bootstrapping GTK dev sysroot (no system pkg-config)"
        local names
        names="$(apt_names libgtk-4-dev libgtksourceview-5-dev; echo pkgconf-bin; echo libpkgconf7)"
        fetch_debs amd64 "$SYSROOT/debs" "$SYSROOT/root" $names
        cp "$SYSROOT/root/usr/bin/pkgconf" "$SYSROOT/bin/pkg-config.bin"
        write_pkg_config_shim "$SYSROOT/bin/pkg-config" "$SYSROOT/root" amd64
    fi

    if command -v cc >/dev/null 2>&1; then
        log "host: using system C compiler"
        return 0
    fi
    [ -n "$ZIG_BIN" ] || die "no system cc and no zig found (install zig or build-essential)"
    log "host: no cc — installing zig cc shim + linklib"
    mkdir -p "$SYSROOT/bin" "$SYSROOT/linklib"
    # zig's driver warns "ignoring deprecated linker optimization setting"
    # about `-Wl,-O<n>` (rustc adds `-Wl,-O1` for optimized builds) and the
    # message then trips rustc's `linker_messages` lint. Zig ignores the
    # flag anyway, so strip it to keep builds warning-free.
    cat >"$SYSROOT/bin/cc" <<EOF
#!/bin/bash
args=()
for a in "\$@"; do
    case "\$a" in
        -Wl,-O[0-9]*) ;;
        *) args+=("\$a") ;;
    esac
done
exec "$ZIG_BIN" cc "\${args[@]}"
EOF
    chmod +x "$SYSROOT/bin/cc"
    local libdir=/usr/lib/x86_64-linux-gnu
    [ -e "$SYSROOT/linklib/libc.so" ] || ln -s "$libdir/libc.so" "$SYSROOT/linklib/libc.so"
    [ -e "$SYSROOT/linklib/libm.so" ] || ln -s "$libdir/libm.so" "$SYSROOT/linklib/libm.so"
    local n
    for n in pthread dl util rt; do
        [ -e "$SYSROOT/linklib/lib$n.so" ] || ln -s libc.so "$SYSROOT/linklib/lib$n.so"
    done
    [ -e "$SYSROOT/linklib/libgcc_s.so" ] || ln -s "$libdir/libgcc_s.so.1" "$SYSROOT/linklib/libgcc_s.so"
}

write_pkg_config_shim() { # path sysroot-root-dir arch
    local trip hostlib
    trip="$(triplet_for "$3")"
    # pkg-config.bin is a HOST binary: it needs the host sysroot libs
    # (libpkgconf…); the target lib dir is included for completeness.
    hostlib="$SYSROOT/root/usr/lib/x86_64-linux-gnu"
    cat >"$1" <<EOF
#!/bin/sh
# Generated by build-packages.sh — resolves .pc files from a local sysroot.
ROOT="$2"
export LD_LIBRARY_PATH="$hostlib:\$ROOT/usr/lib/$trip\${LD_LIBRARY_PATH:+:\$LD_LIBRARY_PATH}"
export PKG_CONFIG_PATH="\$ROOT/usr/lib/$trip/pkgconfig:\$ROOT/usr/share/pkgconfig"
export PKG_CONFIG_SYSROOT_DIR="\$ROOT"
exec "$SYSROOT/bin/pkg-config.bin" "\$@"
EOF
    chmod +x "$1"
}

# ---------------------------------------------------------------------------
# arm64 cross sysroot: GTK dev + runtime packages extracted from the Ubuntu
# ports mirror (no root needed), plus pkg-config / zig cross-cc shims.
# ---------------------------------------------------------------------------
setup_arm64() {
    local marker="$A64/.setup-done"
    [ -e "$marker" ] && { log "arm64: sysroot present"; return 0; }
    [ -n "$ZIG_BIN" ] || die "arm64: zig is required as cross linker (set ZIG=...)"
    command -v rustup >/dev/null || die "arm64: rustup not found"
    log "arm64: adding rust target aarch64-unknown-linux-gnu"
    rustup target add aarch64-unknown-linux-gnu

    log "arm64: fetching packages from $PORTS_MIRROR ($CODENAME)"
    local names trip
    trip="$(triplet_for arm64)"
    # GTK dev files (.pc/headers) + full runtime closure (for .so symlink
    # resolution at link time) + libc/libgcc development bits.
    names="$(apt_names libgtk-4-dev libgtksourceview-5-dev)
$(apt_names libgtk-4-1 libgtksourceview-5-0)
libc6-dev
libgcc-14-dev"
    names="$(echo "$names" | sort -u | grep -v '^$' || true)"
    fetch_debs arm64 "$A64/debs" "$A64/root" $names

    mkdir -p "$A64/bin" "$A64/linklib"
    cp "$SYSROOT/bin/pkg-config.bin" "$A64/bin/pkg-config.bin" 2>/dev/null ||
        cp "$SYSROOT/root/usr/bin/pkgconf" "$A64/bin/pkg-config.bin"
    write_pkg_config_shim "$A64/bin/pkg-config" "$A64/root" arm64

    # zig cc cross linker shim. rustc adds BFD's `--fix-cortex-a53-843419`
    # for aarch64 targets; zig's lld driver rejects it, so strip it (it only
    # patches a Cortex-A53 instruction-reorder hardware erratum).
    cat >"$A64/bin/cc-aarch64" <<EOF
#!/bin/bash
args=()
for a in "\$@"; do
    case "\$a" in
        --fix-cortex-a53-843419 | -Wl,--fix-cortex-a53-843419 | -Wl,-O[0-9]*) ;;
        *) args+=("\$a") ;;
    esac
done
exec "$ZIG_BIN" cc -target aarch64-linux-gnu "\${args[@]}"
EOF
    chmod +x "$A64/bin/cc-aarch64"

    # Link support files: rustc asks for -lpthread/-ldl/... which glibc >= 2.34
    # merged into libc — alias them; provide the real libgcc_s symlink.
    local libdir="$A64/root/usr/lib/$trip"
    local n
    for n in pthread dl util rt; do
        printf 'INPUT ( libc.so.6 )\n' >"$A64/linklib/lib$n.so"
    done
    [ -e "$libdir/libgcc_s.so.1" ] && ln -sfn "$libdir/libgcc_s.so.1" "$A64/linklib/libgcc_s.so"

    touch "$marker"
    log "arm64: sysroot ready"
}

# ---------------------------------------------------------------------------
# Builds
# ---------------------------------------------------------------------------
build_native() {
    log "build: native release"
    set -a
    . "$ROOT/env.sh"
    set +a
    cargo build --release
    file "$ROOT/target/release/$PKGNAME" | grep -q 'x86-64' ||
        die "native binary is not x86-64"
    echo "  $(file -b "$ROOT/target/release/$PKGNAME" | cut -d, -f1-2)"
}

build_arm64() {
    log "build: cross release (aarch64-unknown-linux-gnu)"
    export PKG_CONFIG="$A64/bin/pkg-config"
    export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER="$A64/bin/cc-aarch64"
    export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS="-L $A64/linklib"
    cargo build --release --target aarch64-unknown-linux-gnu
    local bin="$ROOT/target/aarch64-unknown-linux-gnu/release/$PKGNAME"
    file "$bin" | grep -q 'aarch64' || die "cross binary is not aarch64"
    echo "  $(file -b "$bin" | cut -d, -f1-2)"
}

# ---------------------------------------------------------------------------
# Packaging (nfpm creates deb + rpm from the same config; tar.gz is generic)
# ---------------------------------------------------------------------------
setup_nfpm() {
    [ -x "$NFPM" ] && return 0
    log "package: fetching nfpm $NFPM_VERSION"
    local base="https://github.com/goreleaser/nfpm/releases/download/$NFPM_VERSION"
    local tarball="nfpm_${NFPM_VERSION#v}_Linux_x86_64.tar.gz"
    mkdir -p "$SYSROOT/bin" "$SYSROOT/nfpm-dl"
    curl -sfL --max-time 180 -o "$SYSROOT/nfpm-dl/$tarball" "$base/$tarball" ||
        die "nfpm download failed"
    if curl -sfL --max-time 60 -o "$SYSROOT/nfpm-dl/checksums.txt" "$base/checksums.txt"; then
        (cd "$SYSROOT/nfpm-dl" && awk -v f="$tarball" '$2 == f' checksums.txt | sha256sum -c -) ||
            die "nfpm checksum mismatch"
    fi
    tar -xzf "$SYSROOT/nfpm-dl/$tarball" -C "$SYSROOT/nfpm-dl" nfpm
    mv "$SYSROOT/nfpm-dl/nfpm" "$NFPM"
    chmod +x "$NFPM"
}

write_nfpm_config() { # out-yaml deb-arch contents-src-bin
    local out=$1 arch=$2 bin=$3
    cat >"$out" <<EOF
name: $PKGNAME
arch: $arch
platform: linux
version: $VERSION
section: editors
priority: optional
maintainer: gedit-rs contributors <gedit-rs@localhost>
homepage: https://example.invalid/gedit-rs
description: gedit-like text editor for Linux (GTK4 + GtkSourceView 5)
license: MIT
contents:
  - src: $bin
    dst: /usr/bin/$PKGNAME
  - src: packaging/gedit-rs.desktop
    dst: /usr/share/applications/$APP_ID.desktop
  - src: packaging/gedit-rs.svg
    dst: /usr/share/icons/hicolor/scalable/apps/$APP_ID.svg
EOF
}

write_tarball() { # binary arch(deb-style) out.tar.gz
    local bin=$1 arch=$2 out=$3
    local stage="$SYSROOT/pkgstage/$PKGNAME-$VERSION-linux-$arch"
    rm -rf "$stage"
    mkdir -p "$stage/bin"
    cp "$bin" "$stage/bin/$PKGNAME"
    cp packaging/gedit-rs.desktop packaging/gedit-rs.svg "$stage/"
    cat >"$stage/install.sh" <<EOF
#!/bin/sh
# Installs gedit-rs system-wide (needs write access to /usr/local).
set -e
cd "\$(dirname "\$0")"
install -Dm755 bin/$PKGNAME /usr/local/bin/$PKGNAME
install -Dm644 $PKGNAME.desktop /usr/local/share/applications/$APP_ID.desktop
install -Dm644 $PKGNAME.svg /usr/local/share/icons/hicolor/scalable/apps/$APP_ID.svg
command -v update-desktop-database >/dev/null && update-desktop-database /usr/local/share/applications || true
command -v gtk-update-icon-cache   >/dev/null && gtk-update-icon-cache -q /usr/local/share/icons/hicolor || true
echo "$PKGNAME installed. Requires GTK4 + GtkSourceView 5 at runtime."
EOF
    chmod +x "$stage/install.sh"
    tar -C "$SYSROOT/pkgstage" -czf "$out" "$PKGNAME-$VERSION-linux-$arch"
    rm -rf "$stage"
}

package() {
    setup_nfpm
    mkdir -p "$DIST"
    log "package: building artifacts in dist/"

    local pair deb_arch bin
    for pair in amd64 arm64; do
        deb_arch=$pair
        if [ "$pair" = amd64 ]; then bin="$ROOT/target/release/$PKGNAME"
        else bin="$ROOT/target/aarch64-unknown-linux-gnu/release/$PKGNAME"
        fi
        [ -x "$bin" ] || die "missing $bin — run ./build-packages.sh build first"

        # --- .deb (architecture: amd64/arm64)
        write_nfpm_config "$SYSROOT/pkg-deb.yaml" "$deb_arch" "$bin"
        cat >>"$SYSROOT/pkg-deb.yaml" <<EOF
depends:
  - libgtk-4-1
  - libgtksourceview-5-0
EOF
        "$NFPM" pkg --config "$SYSROOT/pkg-deb.yaml" --packager deb --target "$DIST/.tmp.deb" >/dev/null
        mv "$DIST/.tmp.deb" "$DIST/${PKGNAME}_${VERSION}_${deb_arch}.deb"
        dpkg-deb -f "$DIST/${PKGNAME}_${VERSION}_${deb_arch}.deb" Package Version Architecture Depends |
            sed 's/^/    /'

        # --- .rpm (nfpm maps amd64->x86_64, arm64->aarch64 per packager)
        write_nfpm_config "$SYSROOT/pkg-rpm.yaml" "$deb_arch" "$bin"
        cat >>"$SYSROOT/pkg-rpm.yaml" <<EOF
depends:
  - gtk4
  - gtksourceview5
EOF
        "$NFPM" pkg --config "$SYSROOT/pkg-rpm.yaml" --packager rpm --target "$DIST/.tmp.rpm" >/dev/null
        mv "$DIST/.tmp.rpm" "$DIST/${PKGNAME}-${VERSION}-1.$( [ "$deb_arch" = amd64 ] && echo x86_64 || echo aarch64 ).rpm"
        echo "    rpm: $(basename "$DIST/${PKGNAME}-${VERSION}-1.$( [ "$deb_arch" = amd64 ] && echo x86_64 || echo aarch64 ).rpm") created"

        # --- .tar.gz (generic; includes install.sh)
        write_tarball "$bin" "$deb_arch" "$DIST/${PKGNAME}-${VERSION}-linux-${deb_arch}.tar.gz"
        echo "    tar: $(basename "$DIST/${PKGNAME}-${VERSION}-linux-${deb_arch}.tar.gz") created"
    done

    log "artifacts:"
    ls -la "$DIST"
    echo
    echo "Verify:"
    echo "  dpkg -i dist/${PKGNAME}_${VERSION}_amd64.deb     # Debian/Ubuntu amd64"
    echo "  rpm -ivh dist/${PKGNAME}-${VERSION}-1.x86_64.rpm  # Fedora/openSUSE x86_64"
    echo "  tar xzf dist/${PKGNAME}-${VERSION}-linux-arm64.tar.gz && sudo ./${PKGNAME}-${VERSION}-linux-arm64/install.sh"
}

cmd="${1:-all}"
case "$cmd" in
    setup)
        setup_host
        setup_arm64
        ;;
    build)
        setup_host
        setup_arm64
        build_native
        build_arm64
        ;;
    package)
        package
        ;;
    all)
        setup_host
        setup_arm64
        build_native
        build_arm64
        package
        ;;
    *)
        die "unknown command: $cmd (use: setup|build|package|all)"
        ;;
esac
