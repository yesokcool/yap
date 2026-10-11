#!/usr/bin/env bash
# Build the release binaries: Windows x64 and Linux x64 cross-built from macOS
# (or Linux), plus macOS (native, on a Mac).
#
#   desktop/scripts/build-cross.sh            # all three (macOS only on a Mac)
#   desktop/scripts/build-cross.sh linux      # just one
#   desktop/scripts/build-cross.sh windows
#   desktop/scripts/build-cross.sh macos
#   desktop/scripts/build-cross.sh key        # only say which update key a build would use
#
# Self-update: the builds trust the Ed25519 public key in YAP_UPDATE_PUBKEY, else
# the one in desktop/update-pubkey.txt (gitignored; not a secret, just this
# instance's). With neither, the builds don't update themselves.
#
# Needs: rustup targets x86_64-unknown-linux-gnu + x86_64-pc-windows-msvc,
# `cargo install cargo-zigbuild cargo-xwin`, and `brew install zig pkgconf llvm nasm`.
# nasm assembles OpenH264's x86 SIMD for the Linux and Windows builds (screen
# sharing's encoder and decoder); without it they quietly fall back to plain C++.
# Linux links libasound (ALSA) dynamically; its headers/stub come from Debian's
# packages, unpacked into target/linux-sysroot (nothing is installed system-wide).
# Output (bare executables, no archives): target/dist/yap-linux-x86_64,
# target/dist/yap-windows-x86_64.exe, target/dist/yap-macos-<arch> (ad-hoc signed).
# These names match the update manifest's platform keys (linux-x86_64, …).
set -euo pipefail
cd "$(dirname "$0")/.."
CARGO="${CARGO:-$HOME/.cargo/bin/cargo}"
export PATH="$HOME/.cargo/bin:/opt/homebrew/opt/llvm/bin:/opt/homebrew/bin:$PATH"
TARGET_DIR="target/cross"

# The update key, as base64 of the raw 32-byte Ed25519 public key.
update_key() {
  if [ -n "${YAP_UPDATE_PUBKEY:-}" ]; then
    echo "update key: from YAP_UPDATE_PUBKEY"
  elif [ -f update-pubkey.txt ]; then
    YAP_UPDATE_PUBKEY="$(tr -d ' \t\r\n' < update-pubkey.txt)"
    echo "update key: from desktop/update-pubkey.txt"
  else
    unset YAP_UPDATE_PUBKEY
    echo "no update key: this build won't self-update"
    return
  fi
  if ! [[ "$YAP_UPDATE_PUBKEY" =~ ^[A-Za-z0-9+/]{43}=$ ]]; then
    echo "The update key must be a raw 32-byte Ed25519 public key in base64 (44 characters, ending in =)." >&2
    exit 1
  fi
  export YAP_UPDATE_PUBKEY
  echo "  $YAP_UPDATE_PUBKEY"
}
update_key
[ "${1:-all}" = "key" ] && exit 0
mkdir -p target/dist

# Say whether the x86 builds get OpenH264's assembly. (A build cached from before nasm
# was installed stays plain C++: `cargo clean -p openh264-sys2 --release --target <t>
# --target-dir target/cross` rebuilds it.)
nasm_check() {
  if command -v nasm >/dev/null 2>&1; then
    echo "OpenH264 x86 assembly: $(nasm -v)"
  else
    echo "warning: no nasm (brew install nasm): OpenH264 is plain C++ in this build (slower screen sharing)" >&2
  fi
}

linux_sysroot() {
  local root="target/linux-sysroot"
  if [ -f "$root/usr/lib/x86_64-linux-gnu/pkgconfig/alsa.pc" ]; then echo "$root"; return; fi
  mkdir -p "$root" target/linux-sysroot-dl
  local base="http://deb.debian.org/debian/pool/main/a/alsa-lib"
  for pkg in libasound2_1.2.8-1+b1_amd64.deb libasound2-dev_1.2.8-1+b1_amd64.deb; do
    [ -f "target/linux-sysroot-dl/$pkg" ] || curl -sSfL -o "target/linux-sysroot-dl/$pkg" "$base/$pkg"
    (cd target/linux-sysroot-dl && rm -rf x && mkdir x && cd x && ar x "../$pkg" && tar -xf data.tar.* -C "../../linux-sysroot")
  done
  echo "$root"
}

build_linux() {
  local root; root="$(cd "$(linux_sysroot)" && pwd)"
  echo "== linux x86_64 (zig, sysroot $root)"
  nasm_check
  # libasound.so here was built against a newer glibc than the 2.28 we target;
  # at runtime the user's own libasound (built for their glibc) is loaded, so
  # don't make the linker resolve libasound's own glibc references.
  CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS="-C link-arg=-Wl,--allow-shlib-undefined" \
  PKG_CONFIG_ALLOW_CROSS=1 \
  PKG_CONFIG_SYSROOT_DIR="$root" \
  PKG_CONFIG_PATH="$root/usr/lib/x86_64-linux-gnu/pkgconfig:$root/usr/share/pkgconfig" \
  PKG_CONFIG_LIBDIR="$root/usr/lib/x86_64-linux-gnu/pkgconfig" \
    "$CARGO" zigbuild --release --target x86_64-unknown-linux-gnu.2.28 --target-dir "$TARGET_DIR"
  cp "$TARGET_DIR/x86_64-unknown-linux-gnu/release/yap" target/dist/yap-linux-x86_64
  ls -l target/dist/yap-linux-x86_64
}

build_windows() {
  echo "== windows x86_64 (cargo-xwin, MSVC)"
  nasm_check
  # Static CRT (also in .cargo/config.toml; set here too in case cargo-xwin's own flags win).
  CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS="-C target-feature=+crt-static" \
    "$CARGO" xwin build --release --target x86_64-pc-windows-msvc --target-dir "$TARGET_DIR"
  cp "$TARGET_DIR/x86_64-pc-windows-msvc/release/yap.exe" target/dist/yap-windows-x86_64.exe
  ls -l target/dist/yap-windows-x86_64.exe
}

build_macos() {
  local arch; arch="$(uname -m)"
  [ "$arch" = "arm64" ] && arch="aarch64"
  echo "== macos $arch (native)"
  "$CARGO" build --release
  cp target/release/yap "target/dist/yap-macos-$arch"
  # Ad-hoc signature (Apple Silicon refuses to run unsigned code).
  codesign --force --sign - "target/dist/yap-macos-$arch"
  ls -l "target/dist/yap-macos-$arch"
}

case "${1:-all}" in
  linux) build_linux ;;
  windows) build_windows ;;
  macos) build_macos ;;
  all)
    build_linux
    build_windows
    if [ "$(uname -s)" = "Darwin" ]; then build_macos; fi
    ;;
  *) echo "usage: $0 [linux|windows|macos|all|key]"; exit 2 ;;
esac
