-- CPG (Team Claude): durable state of control-api LITE (PgStore), additive; nothing is renamed or dropped.
-- Every tenant-owned table is keyed by tenant_id first, so a read or write can never reach another tenant's row.
-- PRIMARY KEY / UNIQUE constraints are the single-winner guarantees (bindings, artifacts, replay set).

-- core-task-bindings: one binding per (tenant, command_key); a job id belongs to one command key.
CREATE TABLE IF NOT EXISTS pulso_ca_bindings (
    tenant_id TEXT NOT NULL CHECK (char_length(tenant_id) BETWEEN 1 AND 128),
    command_key TEXT NOT NULL CHECK (char_length(command_key) BETWEEN 1 AND 256),
    request_digest TEXT NOT NULL CHECK (char_length(request_digest) BETWEEN 1 AND 256),
    job_id TEXT NOT NULL CHECK (char_length(job_id) BETWEEN 1 AND 256),
    core_run_id TEXT NOT NULL CHECK (char_length(core_run_id) BETWEEN 1 AND 256),
    attempt BIGINT NOT NULL,
    task_binding_ref TEXT NOT NULL CHECK (char_length(task_binding_ref) BETWEEN 1 AND 256),
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (tenant_id, command_key),
    UNIQUE (tenant_id, job_id)
);

-- task_binding_ref -> owning tenant (authorization checks, grants); first owner wins, a ref is never re-assigned.
CREATE TABLE IF NOT EXISTS pulso_ca_binding_refs (
    binding_ref TEXT PRIMARY KEY CHECK (char_length(binding_ref) BETWEEN 1 AND 256),
    tenant_id TEXT NOT NULL CHECK (char_length(tenant_id) BETWEEN 1 AND 128)
);

-- Artifacts: idempotent by (tenant, artifact id); the id embeds the content digest.
CREATE TABLE IF NOT EXISTS pulso_ca_artifacts (
    tenant_id TEXT NOT NULL CHECK (char_length(tenant_id) BETWEEN 1 AND 128),
    artifact_id TEXT NOT NULL CHECK (char_length(artifact_id) BETWEEN 1 AND 256),
    artifact_ref JSONB NOT NULL,
    envelope JSONB NOT NULL CHECK (octet_length(envelope::text) <= 1114112),
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (tenant_id, artifact_id)
);

-- Namespaced tenant-scoped JSON documents: platform observation ledger / cursors / receipts / quarantine METADATA
-- (the ingest path never hands this table a raw event payload), grants, lab sessions/queries/results/receipts,
-- run events, wiki pages. The namespace is a closed set.
CREATE TABLE IF NOT EXISTS pulso_ca_docs (
    ns TEXT NOT NULL CHECK (ns IN ('ingest_ledger', 'ingest_cursor', 'ingest_receipt', 'ingest_quarantine', 'grant',
                                   'lab_session', 'lab_query', 'lab_result', 'lab_receipt', 'run_events', 'wiki')),
    tenant_id TEXT NOT NULL CHECK (char_length(tenant_id) BETWEEN 1 AND 128),
    doc_id TEXT NOT NULL CHECK (char_length(doc_id) BETWEEN 1 AND 1024),
    doc JSONB NOT NULL CHECK (octet_length(doc::text) <= 8388608),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (ns, tenant_id, doc_id)
);

-- Receiver-owned JWT jti replay set (per verifier scope); rows are evicted once expired.
CREATE TABLE IF NOT EXISTS pulso_ca_jti (
    scope TEXT NOT NULL CHECK (char_length(scope) BETWEEN 1 AND 64),
    iss TEXT NOT NULL CHECK (char_length(iss) BETWEEN 1 AND 256),
    jti TEXT NOT NULL CHECK (char_length(jti) BETWEEN 1 AND 256),
    expires_at DOUBLE PRECISION NOT NULL,
    PRIMARY KEY (scope, iss, jti)
);
CREATE INDEX IF NOT EXISTS pulso_ca_jti_expires ON pulso_ca_jti (expires_at);
