# syntax=docker/dockerfile:1
FROM rust:1.95.0-slim AS builder
WORKDIR /build
RUN apt-get update -q && apt-get install -y -q protobuf-compiler && rm -rf /var/lib/apt/lists/*
COPY . .
# target-cpu=native: enables NEON + crypto extensions on arm64 (M-series), AVX2 on x86_64.
# lto=thin: cross-crate inlining without full-LTO link time cost.
# codegen-units=1: single CGU for maximum intra-crate optimization.
ENV RUSTFLAGS="-C target-cpu=native"
RUN cargo build --release -p konfig --bin konfig

FROM gcr.io/distroless/cc-debian13:nonroot
COPY --from=builder --chmod=755 /build/target/release/konfig /konfig
EXPOSE 50051 9090
ENTRYPOINT ["/konfig"]
