# Build environment for gedit-rs — source it from the repository root:
#
#   . ./env.sh && cargo build --release
#
# On a machine with system GTK4 development packages this is optional.
# On machines without them it wires up the local .sysroot/ shims:
#   - .sysroot/bin/  provides `pkg-config` and `cc` (they are found via PATH)
#   - .sysroot/linklib provides unversioned .so names the linker needs
export PATH="$PWD/.sysroot/bin:$PATH"
export PKG_CONFIG="${PKG_CONFIG:-pkg-config}"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="${CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER:-cc}"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS="${CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS:--L $PWD/.sysroot/linklib}"
