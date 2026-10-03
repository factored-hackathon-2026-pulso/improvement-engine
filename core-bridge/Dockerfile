# pulso-core-runtime image (plan 17.3.2). Build:
#   podman build --build-context core=<agent-core checkout at the pin> -f core-bridge/Dockerfile \
#     --build-arg CORE_SHA=<40-hex> --build-arg PULSO_SHA=<sha> -t pulso-core-runtime:<core7>-<pulso7> core-bridge
# The `core` named context must be the pinned checkout; the build script verifies HEAD == CORE_SHA
# before building and the Dockerfile fails on a contracts/VERSION mismatch.

# ---- builder: wheelhouse from the pin + our package ----
FROM docker.io/library/python:3.12-slim AS builder
ARG CORE_SHA=789d6c89b2fca90fc10e2abf157da51dc81c5d51
ARG UV_VERSION=0.7.22
ENV UV_LINK_MODE=copy PIP_DISABLE_PIP_VERSION_CHECK=1
RUN pip install --no-cache-dir "uv==${UV_VERSION}"
COPY --from=core . /src/agent-core
COPY runtime-requirements.txt /tmp/runtime-requirements.txt
WORKDIR /src/agent-core
# contracts/VERSION must equal the pinned contracts version; the locked set is the constraint set.
RUN test "$(cat contracts/VERSION)" = "1.3.0" \
 && uv export --frozen --no-dev --no-emit-project --no-hashes -o /tmp/constraints.txt \
 && uv build --wheel --out-dir /wheelhouse . \
 && pip wheel --no-cache-dir -c /tmp/constraints.txt --wheel-dir /wheelhouse -r /tmp/constraints.txt \
 && pip wheel --no-cache-dir -c /tmp/constraints.txt --wheel-dir /wheelhouse -r /tmp/runtime-requirements.txt

# ---- runtime: slim, non-root, read-only rootfs friendly ----
FROM docker.io/library/python:3.12-slim AS runtime
ARG CORE_SHA=789d6c89b2fca90fc10e2abf157da51dc81c5d51
ARG PULSO_SHA=unknown
ENV PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1 PULSO_SHA=${PULSO_SHA} PULSO_CORE_SHA=${CORE_SHA} \
    PULSO_CALIBRATIONS_DIR=/opt/pulso/assets/calibrations PULSO_CLASSIFIER_DIR=/opt/pulso/assets/classifiers
COPY --from=builder /wheelhouse /wheelhouse
RUN pip install --no-cache-dir --no-index --find-links /wheelhouse agent-core jsonschema referencing \
 && rm -rf /wheelhouse \
 && useradd --system --uid 10001 --no-create-home pulso
# Only our package is copied (no testing/, tests/, contracts/ sources, dev group, Core compose).
COPY src/pulso_core_runtime /opt/pulso/lib/pulso_core_runtime
COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
ENV PYTHONPATH=/opt/pulso/lib
USER 10001
EXPOSE 8000
ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
CMD ["runtime"]
