#!/usr/bin/env bash
# Build the KeyReed Windows installer: waveform plugin -> artwork -> makensis.
#
#   scripts/build-installer.sh                      # dist/KeyReed-Setup-x64.exe from dist/keyreed-win-x64-slim
#   scripts/build-installer.sh --test               # dist/KeyReed-Setup-TEST.exe (own registry keys/folders, for QA only)
#   scripts/build-installer.sh [--test] C:/path/to/payload   # use another payload folder
#
# Needs llvm-mingw (i686 clang, for the 32-bit NSIS plugin) and NSIS 3.x under ~/.local/toolchain,
# plus ffmpeg and python3 for the artwork. Fails loudly if any of them is missing.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
TOOLS="${KEYREED_TOOLCHAIN:-$HOME/.local/toolchain}"
export PATH="$TOOLS/llvm-mingw/bin:$TOOLS/nsis:$PATH"
for t in i686-w64-mingw32-clang makensis ffmpeg python3; do
  command -v "$t" >/dev/null || { echo "FATAL: $t not found on PATH (toolchain: $TOOLS)"; exit 1; }
done

DEFS=()
if [[ "${1:-}" == "--test" ]]; then DEFS+=(-DSANDBOX_TEST); shift; fi
[[ -n "${1:-}" ]] && DEFS+=("-DPAYLOAD_DIR=$(cygpath -w "$1")")

NSIS_DIR="$(cygpath -m "$(dirname "$(command -v makensis)")")"
mkdir -p scripts/installer/plugins
i686-w64-mingw32-clang -shared -O2 -s -DUNICODE -D_UNICODE -I"$NSIS_DIR/Examples/Plugin" \
  scripts/installer/kr_wave.c -o scripts/installer/plugins/KeyReedWave.dll \
  -Wl,--kill-at -lgdi32 -luser32 -lm
mkdir -p dist
python3 scripts/installer/gen_art.py
(cd scripts && makensis -V2 "${DEFS[@]}" keyreed.nsi)
