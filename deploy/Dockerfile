# Build from the repository root: docker build -f deploy/Dockerfile -t rusthouse-cli .
FROM rust:1.85-bookworm AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release --locked -p rusthouse-cli

FROM debian:bookworm-slim
RUN useradd --system --uid 10001 --create-home rusthouse
COPY --from=builder /src/target/release/rusthouse /usr/local/bin/rusthouse
USER 10001:10001
WORKDIR /workspace
ENTRYPOINT ["/usr/local/bin/rusthouse"]
CMD ["--help"]
