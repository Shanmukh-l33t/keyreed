# Contributing

KeyReed is a Windows voice input application written in Rust. Small, tested changes are welcome. Architectural rewrites are not the point of a pull request.

## Before you open a pull request

- Read the code you are changing.
- Do not remove a writing mode, a catalog model, or an integrity pin to make a change smaller.
- Do not add a dependency for something the standard library or the existing crates already do.
- Run `cargo test --workspace --offline` when the change is in Rust.
- Do not commit `vm/`, `vendor/`, `target/`, `dist/`, model files, or private keys.

## Commits

Say what changed and why. A one-line subject is enough when the diff is small.

## Windows packages

The installer is built with NSIS from `scripts/keyreed.nsi` against `dist/keyreed-win-x64`. Do not replace that package with a diagnostic tool or a test launcher.
