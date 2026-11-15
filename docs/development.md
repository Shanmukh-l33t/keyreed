# Development

## Prerequisites

- Rust 1.80 or newer (`rustup`)
- Node 18 or newer, only for the website checks
- Python 3, only for `scripts/gen-excerpts.py`
- For Windows packaging: NSIS and the Windows toolchain. Packaging is scripted for a Linux host cross-building to `x86_64-pc-windows-msvc`; see `scripts/package-win.sh`.

## Clone, test, build

```bash
git clone https://github.com/Shanmukh-l33t/keyreed.git
cd keyreed
cargo test --workspace
cargo build --release -p vi-app
```

The workspace is six crates under `crates/`:

| Crate | Role |
| --- | --- |
| `vi-core` | Config, types, push-to-talk state machine, dictionary, history |
| `vi-asr` | Speech model catalog, integrity checks, sherpa-onnx FFI, optional cloud STT |
| `vi-rewrite` | Tier 0 rules, local grammar model supervisor, guards, optional cloud rewrite |
| `vi-audio` | Microphone capture and resampling |
| `vi-win32` | Hotkey hook, clipboard, typing, tray and settings UI, overlay, foreground-app context |
| `vi-app` | The `keyreed` binary that wires everything together |

Do not commit `vm/`, `vendor/`, `target/`, `dist/`, model files or private keys. They are in `.gitignore`.

## Website

The site in `website/` is plain HTML, CSS and JavaScript. There is no build step and no dependencies.

```bash
python3 -m http.server 8080 --directory website
node scripts/check-site.mjs          # static checks: anchors, ARIA, SEO, CSP hash, secrets
python3 scripts/gen-excerpts.py      # regenerate website/excerpts.js from the real Rust files
```

The site ships a strict Content-Security-Policy (`script-src 'self'`), so do not add inline scripts; put code in a file. `check-site.mjs` fails if one appears. Keep the `headers` block identical in `website/vercel.json` and the root `vercel.json`.

The theme (light by default, dark optional) is set by `website/theme.js` before first paint and stored in `localStorage` under `keyreed-theme`. The colour tokens live at the top of `website/styles.css`; the product scene and source viewer use `.keep-dark` and stay dark in both themes.

Vercel serves `website/` via the root `vercel.json`, which copies it to `public/`.

## Brand images

`assets/src/brand.html` is the source for the README banner, the GitHub social preview and the website Open Graph image. Render them with headless Chromium as described in the comment at the top of that file.

## Checks before a pull request

```bash
cargo test --workspace
node scripts/check-site.mjs   # if you touched website/
```

CI runs the same checks. See [CONTRIBUTING.md](../CONTRIBUTING.md).
