# pulso-local-identity: sandbox-only human issuer (plan 16.13.2 / 17.3.2, DR-08). Never pushed to a remote registry,
# never referenced by staging/prod configuration (contract scan: local_identity.contract).
# The image deliberately does NOT bake the local profile switch: the operator (compose) must opt in explicitly.
#   podman build -t pulso-local-identity:<sha7> local-identity
FROM docker.io/library/python:3.12-slim AS builder
ARG UV_VERSION=0.7.22
ENV UV_LINK_MODE=copy PIP_DISABLE_PIP_VERSION_CHECK=1
RUN pip install --no-cache-dir "uv==${UV_VERSION}"
WORKDIR /src
COPY pyproject.toml uv.lock ./
RUN uv export --frozen --no-dev --no-emit-project --no-hashes -o /tmp/requirements.txt \
 && pip wheel --no-cache-dir --wheel-dir /wheelhouse -r /tmp/requirements.txt

FROM docker.io/library/python:3.12-slim AS runtime
ENV PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1 LOCAL_IDENTITY_HOST=0.0.0.0 \
    LOCAL_IDENTITY_REPLAY_DB=/var/lib/local-identity/replay.sqlite3
COPY --from=builder /wheelhouse /wheelhouse
RUN pip install --no-cache-dir --no-index --find-links /wheelhouse /wheelhouse/*.whl \
 && rm -rf /wheelhouse \
 && useradd --system --uid 10002 --no-create-home identity \
 && mkdir -p /var/lib/local-identity && chown 10002 /var/lib/local-identity
COPY src/local_identity /opt/local-identity/local_identity
ENV PYTHONPATH=/opt/local-identity
USER 10002
EXPOSE 8083
HEALTHCHECK --interval=10s --timeout=3s CMD python -c "import urllib.request,sys; sys.exit(0 if urllib.request.urlopen('http://127.0.0.1:8083/readyz').status==200 else 1)"
ENTRYPOINT ["python", "-m", "local_identity"]
