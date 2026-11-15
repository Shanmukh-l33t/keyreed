#!/usr/bin/env bash
# Reproducible Windows bundle assembler for Keyreed. Run from repo root:
#   scripts/package-win.sh [default|all]
# Assembles dist/keyreed-win-x64/ with the exe, runtime DLLs, and the model set that
# matches AppConfig defaults (Parakeet v2 ASR + Qwen3.5-0.8B SLM). Uses hardlinks for the
# large model trees (same filesystem) so the bundle costs ~0 extra disk. Emits CHECKSUMS.sha256.
set -euo pipefail

MODE="${1:-default}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="$ROOT/dist/keyreed-win-x64"
VENDOR="$ROOT/vendor"
BENCH="$ROOT/bench/models"
TARGET_EXE="$ROOT/target/x86_64-pc-windows-msvc/release/keyreed.exe"

DEFAULT_ASR=(sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8)
ALL_ASR=(sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8 sherpa-onnx-moonshine-base-en-int8 sherpa-onnx-moonshine-tiny-en-int8)
DEFAULT_LLM=(Qwen3.5-0.8B-Q4_K_M.gguf)
ALL_LLM=(Qwen3.5-0.8B-Q4_K_M.gguf Qwen3-0.6B-Q4_K_M.gguf)

if [[ "$MODE" == "all" ]]; then ASR_SET=("${ALL_ASR[@]}"); LLM_SET=("${ALL_LLM[@]}"); else ASR_SET=("${DEFAULT_ASR[@]}"); LLM_SET=("${DEFAULT_LLM[@]}"); fi

# link_or_copy: hardlink when possible (same fs, zero cost), else copy.
loc() { ln "$2" "$1" 2>/dev/null || cp -f "$2" "$1"; }

echo "== Keyreed Windows packager (mode=$MODE) =="
[[ -f "$TARGET_EXE" ]] || { echo "FATAL: exe not built: $TARGET_EXE"; exit 1; }

rm -rf "$DIST"
mkdir -p "$DIST/models" "$DIST/runtime/sherpa-onnx" "$DIST/runtime/llama"

# 1) binary
cp -f "$TARGET_EXE" "$DIST/keyreed.exe"

# 2) sherpa runtime DLLs
cp -f "$VENDOR"/sherpa-onnx/win-x64/lib/*.dll "$DIST/runtime/sherpa-onnx/"

# 3) llama runtime: copy ONLY the curated set validated in the Windows VM E2E (server + its
#    impl/backing DLLs + MSVC CRT), not the 18 unused llama-*.exe CLI tools from the vendor drop.
LLAMA_SRC=""
for cand in "$VENDOR/llama-cpp/win-x64" "$VENDOR/llama" "$VENDOR/llama.cpp"; do
  [[ -f "$cand/llama-server.exe" ]] && LLAMA_SRC="$cand" && break
done
if [[ -z "$LLAMA_SRC" ]]; then
  echo "FATAL: llama-server.exe not found under vendor; cannot ship Tier-1 SLM"; exit 1
fi
LLAMA_FILES=(
  ggml-base.dll ggml-cpu-alderlake.dll ggml-cpu-cannonlake.dll ggml-cpu-cascadelake.dll
  ggml-cpu-cooperlake.dll ggml-cpu-haswell.dll ggml-cpu-icelake.dll ggml-cpu-ivybridge.dll
  ggml-cpu-piledriver.dll ggml-cpu-sandybridge.dll ggml-cpu-sapphirerapids.dll ggml-cpu-skylakex.dll
  ggml-cpu-sse42.dll ggml-cpu-x64.dll ggml-cpu-zen4.dll ggml-rpc.dll ggml.dll libomp.dll
  llama-batched-bench-impl.dll llama-bench-impl.dll llama-cli-impl.dll llama-common.dll
  llama-completion-impl.dll llama-fit-params-impl.dll llama-perplexity-impl.dll llama-quantize-impl.dll
  llama-server-impl.dll llama-server.exe llama.dll
  msvcp140.dll msvcp140_1.dll mtmd.dll vcruntime140.dll vcruntime140_1.dll
)
CRT_FILES=(msvcp140.dll msvcp140_1.dll vcruntime140.dll vcruntime140_1.dll)
CRT_PROV=""
for f in "${LLAMA_FILES[@]}"; do
  src="$LLAMA_SRC/$f"
  # MSVC CRT is not in the llama drop; resolve it from a vendored copy or the local
  # redistributable image, and record provenance (threat model T12: these must be code-signed
  # / from a trusted redistributable before public release).
  if [[ ! -f "$src" ]] && printf '%s\n' "${CRT_FILES[@]}" | grep -qx "$f"; then
    for cand in "$VENDOR/msvc-crt/$f" "/root/.wine/drive_c/windows/system32/$f"; do
      [[ -f "$cand" ]] && src="$cand" && CRT_PROV="$cand" && break
    done
  fi
  [[ -f "$src" ]] || { echo "FATAL: llama runtime file missing: $f"; exit 1; }
  cp -f "$src" "$DIST/runtime/llama/$f"
done

# 4) models: VAD + ASR folders + GGUFs
VAD="$BENCH/silero_vad.onnx"
[[ -f "$VAD" ]] || VAD="$(find "$BENCH" "$VENDOR" -name silero_vad.onnx 2>/dev/null | head -1)"
[[ -f "$VAD" ]] || { echo "FATAL: silero_vad.onnx not found"; exit 1; }
cp -f "$VAD" "$DIST/models/silero_vad.onnx"

for folder in "${ASR_SET[@]}"; do
  src="$BENCH/$folder"
  [[ -d "$src" ]] || { echo "FATAL: missing extracted ASR model $src (download+extract first)"; exit 1; }
  # hardlink the whole tree; strip test_wavs to keep bundle lean
  cp -al "$src" "$DIST/models/$folder"
  rm -rf "$DIST/models/$folder/test_wavs"
done

for gguf in "${LLM_SET[@]}"; do
  src="$VENDOR/models/$gguf"
  [[ -f "$src" ]] || { echo "FATAL: missing GGUF $src"; exit 1; }
  loc "$DIST/models/$gguf" "$src"
done

# 5) provenance: license, readme, checksums
[[ -f "$ROOT/LICENSE" ]] && cp -f "$ROOT/LICENSE" "$DIST/LICENSE" || true
cat > "$DIST/README.txt" <<EOF
Keyreed Windows bundle (offline-first push-to-talk dictation).
Built: $(date -u +%Y-%m-%dT%H:%M:%SZ)
ASR models: ${ASR_SET[*]}
SLM models: ${LLM_SET[*]}
Selection rationale: research/01-speech-recognition.md, research/03-ai-correction-context.md.
All model archives are SHA-256 pinned in crates/vi-asr/src/catalog.rs (threat model T9).
Runtime DLLs (onnxruntime, llama) must be code-signed before public release (threat model T12).
MSVC CRT provenance: ${CRT_PROV:-bundled from vendor/llama-cpp drop}
NOTE: app-local MSVC CRT DLLs are a stopgap. Production installs should run vc_redist.x64.exe
from Microsoft instead of shipping these DLLs side-by-side.
EOF

( cd "$DIST" && find . -type f ! -name CHECKSUMS.sha256 -print0 | sort -z | xargs -0 sha256sum > CHECKSUMS.sha256 )

echo "== bundle assembled =="
du -sh "$DIST"
find "$DIST" -type f | wc -l | xargs echo "files:"
echo "checksum manifest: $DIST/CHECKSUMS.sha256 ($(wc -l < "$DIST/CHECKSUMS.sha256") entries)"
