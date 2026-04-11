FROM rust:1.89-bookworm AS builder

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY src ./src

RUN cargo build --release --bin fireline --bin fireline-testy

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/fireline /usr/local/bin/fireline
COPY --from=builder /app/target/release/fireline-testy /usr/local/bin/fireline-testy

ENTRYPOINT ["/usr/local/bin/fireline"]
