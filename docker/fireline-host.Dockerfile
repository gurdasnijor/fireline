FROM rust:1.89-bookworm AS builder

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY src ./src

RUN cargo build --release --features anthropic-provider --bin fireline

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends bash ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --system --create-home --home-dir /var/lib/fireline --uid 10001 fireline

COPY --from=builder /app/target/release/fireline /usr/local/bin/fireline
COPY docker/bin/fireline-host-entrypoint.sh /usr/local/bin/fireline-host-entrypoint
COPY docker/bin/healthcheck-http.sh /usr/local/bin/healthcheck-http

RUN chmod +x /usr/local/bin/fireline-host-entrypoint /usr/local/bin/healthcheck-http \
    && mkdir -p /var/lib/fireline \
    && chown -R fireline:fireline /var/lib/fireline

ENV FIRELINE_PORT=4440 \
    FIRELINE_HOST=0.0.0.0 \
    FIRELINE_NAME=hosted-fireline \
    FIRELINE_CONTROL_PLANE_PROVIDER=local \
    FIRELINE_DOCKERFILE=docker/fireline-runtime.Dockerfile \
    FIRELINE_DOCKER_IMAGE=fireline-runtime:dev

WORKDIR /var/lib/fireline
USER fireline

EXPOSE 4440

HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
  CMD ["/usr/local/bin/healthcheck-http", "http://127.0.0.1:4440/healthz"]

ENTRYPOINT ["/usr/local/bin/fireline-host-entrypoint"]
