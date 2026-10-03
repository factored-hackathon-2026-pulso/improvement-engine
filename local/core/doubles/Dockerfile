# platform-sim DOUBLES image: the runtime image (agent-core wheel + fastapi/uvicorn/cryptography/rfc8785) plus the
# test-only deps the mocks need (jsonschema, referencing). Never used as evidence for `real_local`.
# Build: podman --connection pulso-dev build --build-arg BASE=<runtime image> -f local/core/doubles/Dockerfile -t <tag> local/core/doubles
ARG BASE
FROM ${BASE}
USER 0
RUN pip install --no-cache-dir "jsonschema>=4.23" "referencing>=0.35"
USER 10001
LABEL com.pulso.role=double
