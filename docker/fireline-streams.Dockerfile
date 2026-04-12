FROM rust:1.89-bookworm AS builder

WORKDIR /app

COPY Cargo.toml ./
COPY crates ./crates
COPY src ./src

RUN cargo build --release --bin fireline-streams

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends bash ca-certificates curl socat \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --system --create-home --home-dir /var/lib/fireline --uid 10001 fireline

COPY --from=builder /app/target/release/fireline-streams /usr/local/bin/fireline-streams
COPY docker/bin/fireline-streams-entrypoint.sh /usr/local/bin/fireline-streams-entrypoint
COPY docker/bin/healthcheck-http.sh /usr/local/bin/healthcheck-http

RUN chmod +x /usr/local/bin/fireline-streams-entrypoint /usr/local/bin/healthcheck-http \
    && mkdir -p /var/lib/fireline/durable-streams \
    && chown -R fireline:fireline /var/lib/fireline

ENV FIRELINE_STREAMS_PORT=7474 \
    FIRELINE_STREAMS_INTERNAL_PORT=17474 \
    DS_STORAGE__MODE=file-durable \
    DS_STORAGE__DATA_DIR=/var/lib/fireline/durable-streams

WORKDIR /var/lib/fireline
USER fireline

VOLUME ["/var/lib/fireline"]
EXPOSE 7474

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD ["/usr/local/bin/healthcheck-http", "http://127.0.0.1:7474/healthz"]

ENTRYPOINT ["/usr/local/bin/fireline-streams-entrypoint"]
