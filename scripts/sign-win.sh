#!/usr/bin/env bash
# Production Authenticode signer for the Keyreed Windows bundle.
# Fails closed. Does not generate a self-signed certificate. A self-signed
# signature does not satisfy SmartScreen and must not be shipped.
#
# Usage:
#   scripts/sign-win.sh status
#   KEYREED_SIGN_PFX=/path/to/cert.pfx KEYREED_SIGN_PFX_PASS='***' scripts/sign-win.sh sign
#
# Requires osslsigncode. Timestamp URL defaults to the DigiCert RFC3161 server.
# Microsoft-signed CRT DLLs (msvcp140, vcruntime140) are left untouched.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="${KEYREED_DIST:-$ROOT/dist/keyreed-win-x64}"
MODE="${1:-status}"
TS_URL="${KEYREED_TIMESTAMP_URL:-http://timestamp.digicert.com}"

need() { command -v "$1" >/dev/null || { echo "FATAL: $1 not installed"; exit 1; }; }
need osslsigncode

status_one() {
  local f="$1"
  local out
  out="$(osslsigncode verify -in "$f" 2>&1 || true)"
  if printf '%s' "$out" | grep -q 'No signature found'; then
    echo "UNSIGNED  $f"
  elif printf '%s' "$out" | grep -q 'Signature verification: ok'; then
    echo "VALID     $f"
  else
    echo "OTHER     $f"
    printf '%s\n' "$out" | head -3 | sed 's/^/          /'
  fi
}

if [[ "$MODE" == "gate" ]]; then
  [[ -d "$DIST" ]] || { echo "FATAL: bundle missing: $DIST"; exit 1; }
  fail=0
  while read -r f; do
    base="$(basename "$f")"
    case "$base" in
      msvcp140.dll|msvcp140_1.dll|vcruntime140.dll|vcruntime140_1.dll) continue ;;
    esac
    out="$(osslsigncode verify -in "$f" 2>&1 || true)"
    if printf '%s' "$out" | grep -q 'Signature verification: ok'; then
      echo "VALID     $f"
    else
      echo "UNSIGNED  $f"
      fail=1
    fi
  done < <(find "$DIST" -type f \( -name '*.exe' -o -name '*.dll' \) -print | sort)
  if [[ "$fail" -ne 0 ]]; then
    echo "RELEASE GATE FAILED: required binaries are unsigned. A production certificate is not installed."
    exit 1
  fi
  echo "RELEASE GATE PASSED"
  exit 0
fi

if [[ "$MODE" == "status" ]]; then
  [[ -d "$DIST" ]] || { echo "FATAL: bundle missing: $DIST"; exit 1; }
  echo "Signature status for $DIST"
  find "$DIST" -type f \( -name '*.exe' -o -name '*.dll' \) -print | sort | while read -r f; do
    status_one "$f"
  done
  exit 0
fi

if [[ "$MODE" != "sign" && "$MODE" != "status" && "$MODE" != "gate" ]]; then
  echo "usage: scripts/sign-win.sh status|gate|sign"
  exit 2
fi

PFX="${KEYREED_SIGN_PFX:-}"
PASS="${KEYREED_SIGN_PFX_PASS:-}"
if [[ -z "$PFX" || ! -f "$PFX" ]]; then
  echo "FATAL: refusing to sign. Set KEYREED_SIGN_PFX to a real code-signing PFX."
  echo "A self-signed certificate will not be generated. Unsigned public distribution is not production-ready."
  exit 1
fi
if [[ -z "$PASS" ]]; then
  echo "FATAL: KEYREED_SIGN_PFX_PASS is empty."
  exit 1
fi

# Never re-sign Microsoft redistributable CRT binaries. Their existing signature is the trust anchor.
is_ms_crt() {
  case "$(basename "$1")" in
    msvcp140.dll|msvcp140_1.dll|vcruntime140.dll|vcruntime140_1.dll) return 0 ;;
    *) return 1 ;;
  esac
}

sign_one() {
  local f="$1"
  local tmp="${f}.signed"
  osslsigncode sign \
    -pkcs12 "$PFX" -pass "$PASS" \
    -n "Keyreed" -i "https://keyreed.local" \
    -t "$TS_URL" \
    -in "$f" -out "$tmp"
  mv -f "$tmp" "$f"
  echo "SIGNED    $f"
}

find "$DIST" -type f \( -name '*.exe' -o -name '*.dll' \) -print | sort | while read -r f; do
  if is_ms_crt "$f"; then
    echo "SKIP_CRT  $f"
    continue
  fi
  sign_one "$f"
done

echo "Re-run: scripts/sign-win.sh status"
