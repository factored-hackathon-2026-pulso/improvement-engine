# Improvement engine image: ONE `pulso` executable (+ the synthetic runner it spawns) and the static debug console.
# Build context = this repo root (infra's release-engine.ps1 / aws-prod.ps1 use `Dockerfile`, context `.`).
# Build with `podman build --format docker ...`: the default OCI format silently drops HEALTHCHECK.
# Config is environment-only (see docs/dev/ENGINE_IMAGE.md). No secret is baked: tokens and DSNs arrive at run time.
ARG NODE_IMAGE=node:22.16-alpine
ARG RUST_IMAGE=rust:1-bookworm
ARG RUNTIME_IMAGE=debian:bookworm-slim

# --- stage 1: debug console (static SPA), served by `pulso` via PULSO_CONSOLE_DIR ---
FROM ${NODE_IMAGE} AS console
WORKDIR /app
COPY debug-console/package.json debug-console/package-lock.json ./
RUN npm ci
COPY debug-console/tsconfig.json debug-console/vite.config.ts debug-console/index.html ./
COPY debug-console/public ./public
COPY debug-console/src ./src
COPY debug-console/fixtures/demo-world.json ./fixtures/demo-world.json
RUN npx tsc --noEmit && npx vite build

# --- stage 2: engine binaries (release profile, locked). seams/crates/pulso/build.rs embeds ../../../migrations ---
FROM ${RUST_IMAGE} AS build
WORKDIR /src
ARG CARGO_BUILD_JOBS=1
ENV CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}
# Engine crates include_str! files from sibling top-level dirs (../../../<dir>), so those dirs travel with seams/.
COPY seams ./seams
COPY migrations ./migrations
COPY contracts ./contracts
COPY bridge-contract ./bridge-contract
COPY platform-contract ./platform-contract
RUN cargo build --release --locked --manifest-path seams/Cargo.toml -p pulso --bins \
    && install -D seams/target/release/pulso /out/pulso \
    && install -D seams/target/release/pulso-synth-runner /out/pulso-synth-runner     && cargo build --release --locked --manifest-path seams/Cargo.toml -p steps --bin steps_cli     && install -D seams/target/release/steps_cli /out/steps_cli
# steps_cli (last build above): the sensor of the improvement loop that the loop driver runs on the engine host.

# --- stage 3: minimal runtime, non-root ---
FROM ${RUNTIME_IMAGE}
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin pulso \
    && install -d -o 10001 -g 10001 /var/lib/pulso /var/lib/pulso/work /var/lib/pulso/store
COPY --from=build /out/pulso /out/pulso-synth-runner /out/steps_cli /usr/local/bin/
COPY --from=console /app/dist /opt/pulso/console
# The cells aggregator the AWS loader runs after an upload (stdlib only; needs python3 above): one file, not tests or the other aggregators.
COPY scripts/aggregate/bank_cells.py /opt/pulso/aggregate/bank_cells.py
USER 10001:10001
# Non-secret defaults only. A non-loopback bind makes `pulso run` demand PULSO_DEBUG_TOKEN and PULSO_ADMIN_TOKEN (>= 24 chars,
# different) at run time; without them the container refuses to start (exit 2) rather than serve unauthenticated.
ENV TMPDIR=/tmp \
    PULSO_LISTEN_ADDR=0.0.0.0:8080 \
    PULSO_ALLOW_NON_LOOPBACK=1 \
    PULSO_CONSOLE_DIR=/opt/pulso/console \
    PULSO_WORK_DIR=/var/lib/pulso/work \
    PULSO_STORE_DIR=/var/lib/pulso/store
VOLUME ["/var/lib/pulso"]
EXPOSE 8080
HEALTHCHECK --interval=15s --timeout=5s --start-period=60s --retries=5 CMD ["/usr/local/bin/pulso", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/pulso"]
CMD ["run"]
