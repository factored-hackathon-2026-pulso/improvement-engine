-- MIG0 (L-PGJS): job claim/commit support, additive. pulso_jobs.lease_version is the fence token and
-- pulso_jobs.attempt the claim counter (both bumped once per claim by the same UPDATE).
-- Nothing is renamed or dropped; existing rows stay valid.
ALTER TABLE pulso_jobs ADD COLUMN IF NOT EXISTS effect_state TEXT NOT NULL DEFAULT 'no_effect'
    CHECK (effect_state IN ('no_effect', 'unknown_pending_reconciliation', 'applied_acknowledged',
                            'completed_no_effect', 'cancelled_before_effect'));

-- out/N of a job: one row per (job, step); the PRIMARY KEY is the single-winner guarantee.
CREATE TABLE IF NOT EXISTS pulso_job_outputs (
    tenant_id TEXT NOT NULL,
    job_id UUID NOT NULL,
    step_index INTEGER NOT NULL CHECK (step_index >= 0),
    fence_token BIGINT NOT NULL CHECK (fence_token > 0),
    worker_id TEXT NOT NULL CHECK (char_length(worker_id) BETWEEN 1 AND 128),
    record TEXT NOT NULL,
    committed_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (tenant_id, job_id, step_index),
    FOREIGN KEY (tenant_id, job_id) REFERENCES pulso_jobs (tenant_id, id)
);

-- Engine key/value store (E1/E2 `JobStore`: lease, in/N, eff/N, out/N) keyed by an engine job reference.
CREATE TABLE IF NOT EXISTS pulso_job_kv (
    tenant_id TEXT NOT NULL CHECK (char_length(tenant_id) BETWEEN 1 AND 128),
    job_ref TEXT NOT NULL CHECK (char_length(job_ref) BETWEEN 1 AND 128),
    key TEXT NOT NULL CHECK (char_length(key) BETWEEN 1 AND 128),
    version BIGINT NOT NULL CHECK (version > 0),
    value TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (tenant_id, job_ref, key)
);
