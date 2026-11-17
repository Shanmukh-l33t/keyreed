#!/usr/bin/env bash
# KeyReed release pipeline in one script: bump every version string, build, package, hash, patch the download
# blocks, and print the `gh release create` command. It never publishes anything.
#
#   scripts/release-2.0.sh <version> [options]          e.g.  scripts/release-2.0.sh 2.0.0-rc.0
#
#   --skip-build        use the exe and packages that already exist (dist/keyreed-win-x64*, target/release)
#   --skip-rehearsal    do not install the sandbox installer and run the packaged build
#   --dry-run           print what would change and stop (touches nothing)
#   --scratch DIR       where the rehearsal installs and extracts (default: $TMPDIR/keyreed-release-rehearsal)
#
# Environment (all optional):
#   KEYREED_TOOLCHAIN      llvm-mingw + nsis folder (default ~/.local/toolchain); build-release.sh and build-installer.sh need it
#   KEYREED_RUNTIME_ROOT   a packaged runtime to take llama and sherpa-onnx DLLs and models from (package-win.sh)
#   KEYREED_GRAMMAR_SRC    the grammar model folder (model.onnx, tokenizer.json, ..., LICENSE, NOTICE)
#   CARGO_TARGET_DIR       build output folder (the packager and the rehearsal look in it)
#
# Version strings it sets (the places release 0.5.1 changed, commit 5718940, plus the ones that commit missed):
#   Cargo.toml [workspace.package], Cargo.lock, scripts/keyreed.nsi (APP_VERSION, VIProductVersion),
#   README.md (status line, installer size, full-archive name and size), docs/installation.md (table rows, hash block),
#   website/index.html (JSON-LD, release pill, download block incl. static SHA-256 fallbacks), website/excerpts.js
#   (regenerated), CHANGELOG.md (heading and asset table).
set -euo pipefail

die() { echo "FATAL: $*" >&2; exit 1; }
step() { echo; echo "== $* =="; }

VERSION="${1:-}"; [[ -n "$VERSION" ]] || die "usage: scripts/release-2.0.sh <version> [--skip-build] [--skip-rehearsal] [--dry-run] [--scratch DIR]"
shift
SKIP_BUILD=0; SKIP_REHEARSAL=0; DRY=0; SCRATCH="${TMPDIR:-/tmp}/keyreed-release-rehearsal"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1 ;;
    --skip-rehearsal) SKIP_REHEARSAL=1 ;;
    --dry-run) DRY=1 ;;
    --scratch) shift; SCRATCH="${1:?--scratch needs a folder}" ;;
    *) die "unknown option $1" ;;
  esac
  shift
done
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] || die "version must look like 2.0.0 or 2.0.0-rc.0 (got $VERSION)"

cd "$(dirname "${BASH_SOURCE[0]}")/.."
ROOT="$(pwd)"
PY="$(command -v python3 || command -v python)" || die "python is needed (hashing, zipping, editing the docs)"
CORE="${VERSION%%-*}"; IFS=. read -r MAJ MIN PAT <<<"$CORE"
NSIS_VERSION="$MAJ.$MIN.$PAT.0"
TAG="v$VERSION"
PRE=0; [[ "$VERSION" == *-* ]] && PRE=1
REPO="https://github.com/Shanmukh-l33t/keyreed"
OUT="dist/release-$VERSION"
SETUP="KeyReed-Setup-x64.exe"
PORTABLE="keyreed-$VERSION-windows-x86_64-portable.zip"
FULL="keyreed-$VERSION-windows-x64-full-offline.zip"
GRAMMAR_ZIP="keyreed-$VERSION-grammar-model-gector-bert-en.zip"
TARGET="${CARGO_TARGET_DIR:-$ROOT/target}"

step "plan"
echo "version $VERSION (tag $TAG, installer file version $NSIS_VERSION, prerelease=$PRE)"
echo "assets  $SETUP, $PORTABLE, $FULL, $GRAMMAR_ZIP, SHA256SUMS.txt  ->  $OUT"
echo "git     $(git rev-parse --short HEAD) on $(git branch --show-current), $(git status --porcelain | wc -l) uncommitted paths"
if [[ "$DRY" == 1 ]]; then echo "(dry run: nothing changed)"; exit 0; fi

# ---------------------------------------------------------------------------------------------------------------
step "1/6 bump version strings"
sed -i -E 's/^version = "[^"]*"/version = "'"$VERSION"'"/' Cargo.toml
grep -q "^version = \"$VERSION\"" Cargo.toml || die "Cargo.toml was not updated"
sed -i -E 's/^!define APP_VERSION "[^"]*"/!define APP_VERSION "'"$VERSION"'"/; s/^VIProductVersion "[^"]*"/VIProductVersion "'"$NSIS_VERSION"'"/' scripts/keyreed.nsi
grep -q "APP_VERSION \"$VERSION\"" scripts/keyreed.nsi || die "keyreed.nsi was not updated"
# Cargo.lock: the six workspace packages follow the workspace version; cargo rewrites them.
cargo update --workspace --offline || die "cargo could not update Cargo.lock offline"
grep -c "^version = \"$VERSION\"" Cargo.lock | xargs echo "Cargo.lock packages at $VERSION:"

# ---------------------------------------------------------------------------------------------------------------
if [[ "$SKIP_BUILD" == 0 ]]; then
  step "2/6 build the exe, the slim and full bundles, and the installer"
  scripts/build-release.sh
  EXE="$TARGET/release/keyreed.exe"
  [[ -f "$EXE" ]] || die "build-release.sh did not produce $EXE (is CARGO_TARGET_DIR set the same way?)"
  GOT="$("$EXE" --version)"; echo "built: $GOT"
  [[ "$GOT" == *"$VERSION"* ]] || die "the built exe reports '$GOT', not $VERSION"
  scripts/package-win.sh slim
  scripts/package-win.sh default
  scripts/build-installer.sh
  scripts/build-installer.sh --test
else
  step "2/6 build skipped (--skip-build)"
fi
for f in dist/keyreed-win-x64-slim/keyreed.exe dist/keyreed-win-x64/keyreed.exe "dist/$SETUP" dist/KeyReed-Setup-TEST.exe; do [[ -f "$f" ]] || die "$f is missing"; done
[[ -f dist/keyreed-win-x64/models/gector-bert-en/model.onnx && -f dist/keyreed-win-x64/models/gector-bert-en/LICENSE && -f dist/keyreed-win-x64/models/gector-bert-en/NOTICE ]] \
  || die "the full bundle has no grammar model with LICENSE and NOTICE (set KEYREED_GRAMMAR_SRC and rerun package-win.sh default)"

# ---------------------------------------------------------------------------------------------------------------
step "3/6 archives, sizes, SHA-256"
mkdir -p "$OUT"
"$PY" - "$VERSION" "$OUT" "$PORTABLE" "$FULL" "$GRAMMAR_ZIP" <<'PYEOF'
import hashlib, json, os, shutil, sys, zipfile

version, out, portable, full, grammar = sys.argv[1:6]

def zip_dir(src, top, name):
    path = os.path.join(out, name)
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED, compresslevel=6) as z:
        for base, _, files in os.walk(src):
            for f in sorted(files):
                full = os.path.join(base, f)
                z.write(full, os.path.join(top, os.path.relpath(full, src)).replace(os.sep, "/"))
    return path

zip_dir("dist/keyreed-win-x64-slim", f"keyreed-{version}-portable", portable)
zip_dir("dist/keyreed-win-x64", f"keyreed-{version}-full-offline", full)
zip_dir("dist/keyreed-win-x64/models/gector-bert-en", "gector-bert-en", grammar)
shutil.copyfile("dist/KeyReed-Setup-x64.exe", os.path.join(out, "KeyReed-Setup-x64.exe"))

info = {}
lines = []
for name in ("KeyReed-Setup-x64.exe", portable, full, grammar):
    p = os.path.join(out, name)
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    info[name] = {"bytes": os.path.getsize(p), "sha256": h.hexdigest()}
    lines.append(f"{h.hexdigest()} *{name}")
    print(f"{info[name]['bytes']:>13,} bytes  {h.hexdigest()}  {name}")
open(os.path.join(out, "SHA256SUMS.txt"), "w", newline="\n").write("\n".join(lines) + "\n")
json.dump(info, open(os.path.join(out, "release-info.json"), "w"), indent=1)
PYEOF

# ---------------------------------------------------------------------------------------------------------------
step "4/6 patch README, install guide, website and changelog"
"$PY" - "$VERSION" "$OUT" "$SETUP" "$PORTABLE" "$FULL" "$GRAMMAR_ZIP" "$REPO" "$TAG" <<'PYEOF'
import json, os, re, sys
from datetime import date

version, out, setup, portable, full, grammar, repo, tag = sys.argv[1:9]
info = json.load(open(os.path.join(out, "release-info.json")))

def size(n):  # decimal units, as the website and 0.5.1 show them (23114430 bytes -> 23.1 MB)
    return f"{n / 1e9:.2f} GB" if n >= 1e9 else f"{n / 1e6:.1f} MB"

def edit(path, subs):
    raw = open(path, encoding="utf-8", newline="").read()
    nl = "\r\n" if "\r\n" in raw else "\n"
    text = raw.replace("\r\n", "\n")
    for pattern, repl, expect in subs:
        text, n = re.subn(pattern, repl, text, flags=re.M)
        if n != expect:
            sys.exit(f"FATAL: {path}: {pattern!r} matched {n} times, expected {expect}")
    open(path, "w", encoding="utf-8", newline="").write(text.replace("\n", nl))

sh = {k: v["sha256"] for k, v in info.items()}
sz = {k: size(v["bytes"]) for k, v in info.items()}
ver_re = r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.]+)?"

edit("README.md", [
    (r"release \*\*" + ver_re + r"\*\*", f"release **{version}**", 1),
    (r"(Download \*\*`KeyReed-Setup-x64.exe`\*\* \(`)[0-9.]+ [KMG]B(`\))", rf"\g<1>{sz[setup]}\g<2>", 1),
    (r"`keyreed-" + ver_re + r"-windows-x64-full-offline.zip` \([0-9.]+ [KMG]B\)", f"`{full}` ({sz[full]})", 1),
])
edit("docs/installation.md", [
    (r"(\| `KeyReed-Setup-x64.exe` \|[^|]*\| Lightweight `)[0-9.]+ [KMG]B(`)", rf"\g<1>{sz[setup]}\g<2>", 1),
    (r"(\| `)keyreed-" + ver_re + r"(-windows-x86_64-portable.zip` \|[^|]*\| `)[0-9.]+ [KMG]B(`)", rf"\g<1>keyreed-{version}\g<2>{sz[portable]}\g<3>", 1),
    (r"(\| `)keyreed-" + ver_re + r"(-windows-x64-full-offline.zip` \|[^|]*\| `)[0-9.]+ [KMG]B(`)", rf"\g<1>keyreed-{version}\g<2>{sz[full]}\g<3>", 1),
    (r"^For release " + ver_re + ":", f"For release {version}:", 1),
    (r"^[0-9a-f]{64}  KeyReed-Setup-x64.exe$", f"{sh[setup]}  {setup}", 1),
    (r"^[0-9a-f]{64}  keyreed-" + ver_re + r"-windows-x86_64-portable.zip$", f"{sh[portable]}  {portable}", 1),
    (r"^[0-9a-f]{64}  keyreed-" + ver_re + r"-windows-x64-full-offline.zip$", f"{sh[full]}  {full}", 1),
])
# The grammar model row and hash line are new in 2.0: add them once, update them afterwards.
doc = open("docs/installation.md", encoding="utf-8", newline="").read()
nl = "\r\n" if "\r\n" in doc else "\n"
doc = doc.replace("\r\n", "\n")
row = f"| `{grammar}` | You want the grammar model for Quick Fix as a separate file. | `{sz[grammar]}` archive of the Apache-2.0 `gector-bert-en` model (int8) with its LICENSE and NOTICE. Settings downloads it for you; use this to install it by hand. |"
doc, n = re.subn(r"^\| `keyreed-[^`]*-grammar-model-gector-bert-en.zip` \|.*$", row, doc, flags=re.M)
if n == 0:
    doc = re.sub(r"(^\| `keyreed-[^`]*-windows-x64-full-offline.zip` \|.*$)", lambda m: m.group(1) + "\n" + row, doc, count=1, flags=re.M)
line = f"{sh[grammar]}  {grammar}"
doc, n = re.subn(r"^[0-9a-f]{64}  keyreed-[^ ]*-grammar-model-gector-bert-en.zip$", line, doc, flags=re.M)
if n == 0:
    doc = re.sub(r"(^[0-9a-f]{64}  keyreed-[^ ]*-windows-x64-full-offline.zip$)", lambda m: m.group(1) + "\n" + line, doc, count=1, flags=re.M)
open("docs/installation.md", "w", encoding="utf-8", newline="").write(doc.replace("\n", nl))

edit("website/index.html", [
    (r'("softwareVersion": ")' + ver_re + '"', rf'\g<1>{version}"', 1),
    (r'(<span id="pill-ver">KeyReed )' + ver_re + r"(</span>)", rf"\g<1>{version}\g<2>", 1),
    (r'(<span id="dl-ver">Version )' + ver_re + r"(</span>)", rf"\g<1>{version}\g<2>", 1),
    (r'(<span data-bind="installer-size">)[0-9.]+ [KMG]B(</span>)', rf"\g<1>{sz[setup]}\g<2>", 2),
    (r'(<span data-bind="full-size">)[0-9.]+ [KMG]B(</span>)', rf"\g<1>{sz[full]}\g<2>", 1),
    (r'(<span id="full-name">)keyreed-[^<]*(</span>)', rf"\g<1>{full}\g<2>", 1),
    (r'(id="dl-full" href=")[^"]*"', rf'\g<1>{repo}/releases/latest/download/{full}"', 1),
    (r'(<code id="sha-installer">)[0-9a-f]{64}(</code>)', rf"\g<1>{sh[setup]}\g<2>", 1),
    (r'(<code id="sha-full">)[0-9a-f]{64}(</code>)', rf"\g<1>{sh[full]}\g<2>", 1),
])

# CHANGELOG: rename "Unreleased" (or add a heading) and keep one asset table per release.
log = open("CHANGELOG.md", encoding="utf-8", newline="").read()
nl = "\r\n" if "\r\n" in log else "\n"
log = log.replace("\r\n", "\n")
today = date.today().isoformat()
heading = f"## {version} ({today})"
if re.search(r"^## Unreleased\b", log, flags=re.M):
    log = re.sub(r"^## Unreleased.*$", heading, log, count=1, flags=re.M)
elif not re.search(r"^## " + re.escape(version) + r"\b", log, flags=re.M):
    log = log.replace("# Changelog\n", f"# Changelog\n\n{heading}\n\nWrite this release's notes here.\n", 1)
table = ["", "| File | Size | SHA-256 |", "| --- | --- | --- |"] + [f"| `{n}` | {sz[n]} | `{sh[n]}` |" for n in (setup, portable, full, grammar)]
block = "\n".join(table) + "\n"
section = re.search(r"^## " + re.escape(version) + r"\b.*?(?=^## |\Z)", log, flags=re.M | re.S)
body = section.group(0)
body = re.sub(r"\n\| File \| Size \| SHA-256 \|\n\| --- \| --- \| --- \|\n(?:\|.*\|\n)+", "\n", body)
body = body.rstrip("\n") + "\n" + block + "\n"
log = log[:section.start()] + body + log[section.end():]
open("CHANGELOG.md", "w", encoding="utf-8", newline="").write(log.replace("\n", nl))
open(os.path.join(out, "release-notes.md"), "w", encoding="utf-8", newline="\n").write(re.sub(r"^## [^\n]*\n\n?", "", body, count=1))
print("patched README.md, docs/installation.md, website/index.html, CHANGELOG.md; notes in", os.path.join(out, "release-notes.md"))
PYEOF
"$PY" scripts/gen-excerpts.py
git diff --stat | tail -12

# ---------------------------------------------------------------------------------------------------------------
if [[ "$SKIP_REHEARSAL" == 0 ]]; then
  step "5/6 rehearsal in $SCRATCH (the installed copy under AppData is never touched)"
  rm -rf "$SCRATCH"; mkdir -p "$SCRATCH"
  # The sandbox installer variant keeps its own registry keys and folders; the real installer would overwrite the
  # owner's InstallDir registration.
  INSTALL_DIR="$(cygpath -w "$SCRATCH/installed")"
  "$ROOT/dist/KeyReed-Setup-TEST.exe" /S "/D=$INSTALL_DIR"
  for _ in $(seq 1 60); do [[ -f "$SCRATCH/installed/keyreed.exe" ]] && break; sleep 1; done
  [[ -f "$SCRATCH/installed/keyreed.exe" ]] || die "the sandbox installer did not produce keyreed.exe"
  echo "installed: $("$SCRATCH/installed/keyreed.exe" --version)"
  [[ "$("$SCRATCH/installed/keyreed.exe" --version)" == *"$VERSION"* ]] || die "the installed exe does not report $VERSION"
  echo "$(find "$SCRATCH/installed" -type f | wc -l) files installed"
  # The full bundle, extracted: runs with the stripped PATH, finds its runtime and the grammar model beside it.
  "$PY" -c "import zipfile,sys; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])" "$OUT/$FULL" "$SCRATCH/full"
  FULLDIR="$SCRATCH/full/keyreed-$VERSION-full-offline"
  printf '%s\n' '{"id":"r1","input":"This are a test sentence with recieve in it.","app_context":"email"}' > "$SCRATCH/case.jsonl"
  env -i SystemRoot="${SYSTEMROOT:-C:\\Windows}" PATH="/c/Windows/System32" "$FULLDIR/keyreed.exe" --version
  env -i SystemRoot="${SYSTEMROOT:-C:\\Windows}" PATH="/c/Windows/System32" "$FULLDIR/keyreed.exe" --eval-quickfix-jsonl "$SCRATCH/case.jsonl" "$SCRATCH/out.jsonl" --spell
  echo "quick fix in the extracted bundle: $(cat "$SCRATCH/out.jsonl" | cut -c1-200)"
  grep -q '"output"' "$SCRATCH/out.jsonl" || die "Quick Fix did not run from the extracted bundle"
  # Uninstall the sandbox copy again (silently; _?= keeps the uninstaller in the foreground).
  if [[ -f "$SCRATCH/installed/uninstall.exe" ]]; then "$SCRATCH/installed/uninstall.exe" /S "_?=$INSTALL_DIR" || true; fi
  echo "rehearsal passed"
else
  step "5/6 rehearsal skipped (--skip-rehearsal)"
fi

# ---------------------------------------------------------------------------------------------------------------
step "6/6 what to run to publish (nothing has been pushed or published)"
HEAD_SHA="$(git rev-parse HEAD)"
FLAGS="--target $HEAD_SHA"
if [[ "$PRE" == 1 ]]; then FLAGS="$FLAGS --prerelease"; else FLAGS="$FLAGS --latest"; fi
cat <<EOF
Commit the bumped files first (the commit becomes the release target), push, then:

gh release create $TAG $FLAGS --title "KeyReed $VERSION" --notes-file $OUT/release-notes.md \\
  $OUT/KeyReed-Setup-x64.exe \\
  $OUT/$PORTABLE \\
  $OUT/$FULL \\
  $OUT/$GRAMMAR_ZIP \\
  $OUT/SHA256SUMS.txt

(--target must be the full 40-character SHA of the pushed commit; HEAD is $HEAD_SHA now.)
After it is published: fetch $REPO/releases/latest/download/$SETUP and the other asset URLs and compare the
SHA-256 with $OUT/SHA256SUMS.txt.
EOF
