# Cargo slot protocol (G0e)

1. Each lane builds into its own `CARGO_TARGET_DIR`, taken from
   `scripts/env/lane-target-dir.ps1 -Lane <lane>` (default root `D:\cargo-targets`; never C:).
2. Before a session starts, run `scripts/env/check-target-dirs.ps1 -Assignment lane=dir ...`;
   it exits 1 when two lanes share a dir or a dir is not on D:.
3. Only one cargo build/test runs at a time (one cargo slot): create
   `<CARGO_TARGET_DIR>\..\cargo-slot.lock` before cargo, delete it after; if it exists, wait.
4. Put the lane's `CARGO_TARGET_DIR` line in the lane brief.
