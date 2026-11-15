# Release builds with stellar-cli (Docker)

soroban-sdk 28 refuses to compile a contract unless the build is driven by
**stellar-cli v25.2.0+** (spec shaking). Tests (`cargo test`) and `cargo clippy`
do **not** need it; only release `.wasm` builds do.

`stellar-build.Dockerfile` builds stellar-cli **from crates.io source** on the
official Rust image, so release builds are reproducible and need no changes to
the host toolchain.

## 1. Build the image (once)

```bash
docker build -f docker/stellar-build.Dockerfile -t constella-stellar-build:30.0.1 docker
```

## 2. Build all contracts

Run from the repository root. `CARGO_TARGET_DIR` overrides the Windows-only
`target-dir` in `.cargo/config.toml`; output lands in the gitignored
`contracts/target/`. The named volume caches downloaded crates between runs.

```bash
docker run --rm \
  -v "$(pwd):/work" -w /work/contracts \
  -e CARGO_TARGET_DIR=/work/contracts/target \
  -v constella-cargo-registry:/usr/local/cargo/registry \
  constella-stellar-build:30.0.1 \
  stellar contract build --locked
```

`--locked` fails the build if `contracts/Cargo.lock` would change, so every
release WASM is built from exactly the committed, reviewed dependency set. To
upgrade a dependency, change `Cargo.toml`, regenerate the lock, run the full
test suite, and commit both files together.

Artifacts: `contracts/target/wasm32v1-none/release/constella_*.wasm`.

## Pinned versions

| Component | Version | Why |
|---|---|---|
| Rust | 1.93.1 | stellar-cli 30.0.1 requires rustc 1.93+ (soroban-sdk 28 itself needs 1.91+) |
| stellar-cli | 30.0.1 | latest release (verified 15 Nov 2026); ≥ 25.2.0 required by soroban-sdk 28 |
| soroban-sdk | 28.0.0 | matches OpenZeppelin stellar-contracts v0.8.0 |
