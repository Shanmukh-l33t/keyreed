#!/usr/bin/env bash
# Reproducible release build. The windows-gnu Rust toolchain needs the llvm-mingw (UCRT) binutils
# (windres, dlltool, gcc) at BUILD time only; the produced exe imports system DLLs only.
# Usage: KEYREED_TOOLCHAIN=/c/path/to/toolchain scripts/build-release.sh
set -euo pipefail
TC="${KEYREED_TOOLCHAIN:-$HOME/.local/toolchain}"
export PATH="$TC/llvm-mingw/bin:$TC/nsis:$PATH"
for t in windres dlltool x86_64-w64-mingw32-gcc; do
  command -v "$t" >/dev/null || { echo "FATAL: $t not found (set KEYREED_TOOLCHAIN)"; exit 1; }
done
cd "$(dirname "${BASH_SOURCE[0]}")/.."
cargo build --release -p vi-app
E="${CARGO_TARGET_DIR:-target}/release/keyreed.exe"
# Smoke test with a stripped PATH: proves no developer toolchain is needed at runtime.
env -i SystemRoot="${SYSTEMROOT:-C:\Windows}" PATH="/c/Windows/System32" "$E" --version
# Imports must be system DLLs only (no msys/mingw runtime).
if llvm-objdump -p "$E" | grep -iE 'DLL Name: (libgcc|libstdc|libwinpthread|msys|libssp)'; then echo "FATAL: toolchain runtime DLL imported"; exit 1; fi
