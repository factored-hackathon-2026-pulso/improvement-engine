# pulso-platform-exporter image (PL-0008). Build (context = this directory):
#   podman build --build-arg PULSO_SHA=<sha> -t pulso-platform-exporter:<sha7> platform-exporter
# Dependencies come only from uv.lock (uv export --frozen, no dev group). Non-root uid 10001, read-only root fs friendly:
# the only writable paths are /run/pulso-keys (key files, ADR 0009 of core-bridge), the state dir and /tmp, all of which
# are pre-created and owned by the app uid because a Fargate ephemeral volume inherits ownership from the image path.

# ---- builder: wheelhouse from the lock ----
FROM docker.io/library/python:3.12-slim AS builder
ARG UV_VERSION=0.7.22
ENV UV_LINK_MODE=copy PIP_DISABLE_PIP_VERSION_CHECK=1
RUN pip install --no-cache-dir "uv==${UV_VERSION}"
WORKDIR /src
COPY pyproject.toml uv.lock ./
RUN uv export --frozen --no-dev --no-emit-project --no-hashes -o /tmp/constraints.txt \
 && pip wheel --no-cache-dir -c /tmp/constraints.txt --wheel-dir /wheelhouse -r /tmp/constraints.txt

# ---- runtime ----
FROM docker.io/library/python:3.12-slim AS runtime
ARG PULSO_SHA=unknown
ENV PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1 PULSO_SHA=${PULSO_SHA} PYTHONPATH=/opt/pulso/lib \
    EXPORTER_STATE_PATH=/var/lib/pulso-platform-exporter/state.sqlite
COPY --from=builder /wheelhouse /wheelhouse
COPY --from=builder /tmp/constraints.txt /tmp/constraints.txt
RUN pip install --no-cache-dir --no-index --find-links /wheelhouse -r /tmp/constraints.txt \
 && rm -rf /wheelhouse /tmp/constraints.txt \
 && useradd --system --uid 10001 --no-create-home pulso \
 && install -d -m 0700 -o 10001 /run/pulso-keys /var/lib/pulso-platform-exporter
# Only the package is copied (no tests/, docs/, simulator, dev group).
COPY src/platform_exporter /opt/pulso/lib/platform_exporter
COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
USER 10001
ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
