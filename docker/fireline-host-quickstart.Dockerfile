FROM rust:1.89-bookworm AS builder

WORKDIR /app

COPY Cargo.toml ./
COPY crates ./crates
COPY src ./src
COPY verification ./verification

RUN cargo build --release --features anthropic-provider --bin fireline --bin fireline-streams --bin fireline-testy-load

FROM node:22-bookworm-slim AS embedded-spec-bootstrap

WORKDIR /opt/fireline-bootstrap

COPY docker/bin ./docker/bin
COPY packages/client/src ./packages/client/src

RUN npm init -y \
    && npm install --no-package-lock --omit=dev \
      tsx@4.20.6 \
      @agentclientprotocol/sdk@0.18.1 \
      ws@8.18.3 \
      @durable-streams/client@0.2.3

FROM debian:bookworm-slim

ARG SPEC=docker/specs/placeholder-spec.json

RUN apt-get update \
    && apt-get install -y --no-install-recommends bash ca-certificates curl socat \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --system --create-home --home-dir /var/lib/fireline --uid 10001 fireline

COPY --from=builder /app/target/release/fireline /usr/local/bin/fireline
COPY --from=builder /app/target/release/fireline-streams /usr/local/bin/fireline-streams
COPY --from=builder /app/target/release/fireline-testy-load /usr/local/bin/fireline-testy-load
COPY --from=embedded-spec-bootstrap /usr/local /usr/local
COPY --from=embedded-spec-bootstrap /opt/fireline-bootstrap /opt/fireline-bootstrap
COPY docker/bin/fireline-host-quickstart-entrypoint.sh /usr/local/bin/fireline-host-quickstart-entrypoint
COPY docker/bin/healthcheck-http.sh /usr/local/bin/healthcheck-http
COPY ${SPEC} /etc/fireline/spec.json

RUN chmod +x /usr/local/bin/fireline-host-quickstart-entrypoint /usr/local/bin/healthcheck-http \
    && mkdir -p /var/lib/fireline/durable-streams /etc/fireline \
    && chown -R fireline:fireline /var/lib/fireline

ENV FIRELINE_PORT=4440 \
    FIRELINE_HOST=0.0.0.0 \
    FIRELINE_NAME=hosted-fireline \
    FIRELINE_CONTROL_PLANE_PROVIDER=local \
    FIRELINE_DOCKERFILE=docker/fireline-runtime.Dockerfile \
    FIRELINE_DOCKER_IMAGE=fireline-runtime:dev \
    FIRELINE_EMBEDDED_SPEC_PATH=/etc/fireline/spec.json \
    FIRELINE_STREAMS_PORT=7474 \
    FIRELINE_STREAMS_INTERNAL_PORT=17474 \
    DS_STORAGE__MODE=file-durable \
    DS_STORAGE__DATA_DIR=/var/lib/fireline/durable-streams

WORKDIR /var/lib/fireline
USER fireline

VOLUME ["/var/lib/fireline"]
EXPOSE 4440 7474

HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
  CMD ["/usr/local/bin/healthcheck-http", "http://127.0.0.1:4440/healthz", "http://127.0.0.1:7474/healthz"]

ENTRYPOINT ["/usr/local/bin/fireline-host-quickstart-entrypoint"]
