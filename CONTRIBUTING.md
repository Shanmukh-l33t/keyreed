# Contributing

KeyReed is a Windows voice input application written in Rust. Small, tested changes are welcome. Architectural rewrites are not the point of a pull request.

## Before you open a pull request

- Open an issue first for anything large, and read the code you are changing.
- Do not remove a writing mode, a catalog model or an integrity pin to make a change smaller.
- Do not add a dependency for something the standard library or the existing crates already do.
- Run `cargo test --workspace` when the change is in Rust.
- Run `node scripts/check-site.mjs` when the change is in `website/`. If you changed the Rust files shown on the site, run `python3 scripts/gen-excerpts.py` too.
- Do not commit `vm/`, `vendor/`, `target/`, `dist/`, model files or private keys.
- Do not claim a feature, measurement or privacy property in docs or on the website that the code does not back up.

Setup instructions are in [docs/development.md](docs/development.md); how the pieces fit is in [docs/architecture.md](docs/architecture.md).

## Commits

Say what changed and why. [Conventional Commits](https://www.conventionalcommits.org/) are welcome (`fix(asr): ...`, `docs: ...`), and a one-line subject is enough when the diff is small.

## Windows packages

The installer is built with NSIS from `scripts/keyreed.nsi` against `dist/keyreed-win-x64`. Do not replace that package with a diagnostic tool or a test launcher.

## Reporting bugs

Use the [bug template](https://github.com/Shanmukh-l33t/keyreed/issues/new?template=bug.md). Include your Windows version and steps. Do not attach recordings of other people, passwords or clipboard contents from real documents. Report security problems privately, as described in [SECURITY.md](SECURITY.md).

## Conduct

See [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
