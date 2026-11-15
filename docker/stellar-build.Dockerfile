# Reproducible release-build environment for Constella Soroban contracts.
#
# soroban-sdk 28 refuses to build contracts unless the build is driven by
# stellar-cli v25.2.0+ (mandatory spec shaking). This image builds stellar-cli
# from crates.io source on the official Rust image, so release WASMs are built
# identically on any machine without changing the host toolchain.
#
# Build:  docker build -f docker/stellar-build.Dockerfile -t constella-stellar-build:30.0.1 docker
# Use:    see docker/README-stellar-build.md
FROM rust:1.93.1-bookworm

# System libraries stellar-cli links against (keyring / hardware-wallet support).
RUN apt-get update \
    && apt-get install -y --no-install-recommends libdbus-1-dev libudev-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Soroban contracts target wasm32v1-none.
RUN rustup target add wasm32v1-none

ARG STELLAR_CLI_VERSION=30.0.1
RUN cargo install --locked stellar-cli --version ${STELLAR_CLI_VERSION} \
    && stellar --version

WORKDIR /work
