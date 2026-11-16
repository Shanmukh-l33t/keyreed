#!/usr/bin/env bash
# Build the KeyReed Windows installer: KeyReedSetup UI plugin -> makensis.
#
#   scripts/build-installer.sh                      # dist/KeyReed-Setup-x64.exe from dist/keyreed-win-x64-slim
#   scripts/build-installer.sh --test               # dist/KeyReed-Setup-TEST.exe (own registry keys/folders, for QA only)
#   scripts/build-installer.sh [--test] C:/path/to/payload   # use another payload folder
#
# Needs llvm-mingw (i686 clang, for the 32-bit NSIS plugin) and NSIS 3.x under ~/.local/toolchain.
# Fails loudly if either is missing.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
TOOLS="${KEYREED_TOOLCHAIN:-$HOME/.local/toolchain}"
export PATH="$TOOLS/llvm-mingw/bin:$TOOLS/nsis:$PATH"
for t in i686-w64-mingw32-clang makensis; do
  command -v "$t" >/dev/null || { echo "FATAL: $t not found on PATH (toolchain: $TOOLS)"; exit 1; }
done

DEFS=()
if [[ "${1:-}" == "--test" ]]; then DEFS+=(-DSANDBOX_TEST); shift; fi
[[ -n "${1:-}" ]] && DEFS+=("-DPAYLOAD_DIR=$(cygpath -w "$1")")

NSIS_DIR="$(cygpath -m "$(dirname "$(command -v makensis)")")"
mkdir -p scripts/installer/plugins
rm -f scripts/installer/plugins/KeyReedWave.dll
i686-w64-mingw32-clang -shared -O2 -s -Wall -DUNICODE -D_UNICODE -I"$NSIS_DIR/Examples/Plugin" \
  scripts/installer/kr_setup.c -o scripts/installer/plugins/KeyReedSetup.dll \
  -Wl,--kill-at -lgdi32 -luser32 -lole32 -luuid -lm
mkdir -p dist
(cd scripts && makensis -V2 -INPUTCHARSET UTF8 "${DEFS[@]}" keyreed.nsi)
