# seams

Claude-owned seams between the Python host and Codex's Rust engine. This is a
nested Cargo workspace with its own `[workspace]` and `Cargo.lock`; the root
`Cargo.toml` does not list it and Codex's `crates/**` stay untouched.

- Members: `crates/*`. Each lane adds its own crate directory.
- ABI rule: the `abi` crate and `core-client` must never depend on Codex's
  `crates/core`. A graph test (`core-client/tests/graph.rs`) enforces this.
- K0 crates are std-only so the build works offline.

## Build and test

Use a private target dir (lane convention: `scripts/env/lane-target-dir.ps1 -Lane claude-seams`):

    CARGO_TARGET_DIR=D:/cargo-targets/claude-seams cargo test --offline -j 2 --manifest-path seams/Cargo.toml

Run one cargo process at a time on this machine.
