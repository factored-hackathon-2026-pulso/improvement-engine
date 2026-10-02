-- U29: treated platform observations and transport cursors owned by Pulso.
-- JSONB contains only the allowlisted typed projection; no source payload text.
CREATE TABLE IF NOT EXISTS pulso_platform_observation_blobs (
    tenant_id TEXT NOT NULL,
    digest TEXT NOT NULL CHECK (digest ~ '^sha256:[0-9a-f]{64}$'),
    treated_bytes BYTEA NOT NULL,
    retention_until_ms BIGINT NOT NULL,
    PRIMARY KEY (tenant_id, digest),
    CHECK (octet_length(treated_bytes) <= 262144),
    CHECK (retention_until_ms > 0)
);

CREATE TABLE IF NOT EXISTS pulso_platform_observation_cursors (
    tenant_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    partition_id TEXT NOT NULL,
    last_to_seq BIGINT,
    last_batch_digest TEXT,
    PRIMARY KEY (tenant_id, source_id, partition_id),
    CHECK (last_to_seq IS NULL OR last_to_seq >= 0)
);

CREATE TABLE IF NOT EXISTS pulso_platform_observation_batches (
    tenant_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    partition_id TEXT NOT NULL,
    cursor_key TEXT NOT NULL,
    from_seq BIGINT,
    to_seq BIGINT,
    contract_ref TEXT NOT NULL,
    batch_digest TEXT NOT NULL CHECK (batch_digest ~ '^sha256:[0-9a-f]{64}$'),
    event_blob_ref TEXT NOT NULL CHECK (event_blob_ref ~ '^sha256:[0-9a-f]{64}$'),
    retention_until_ms BIGINT NOT NULL,
    coverage JSONB NOT NULL,
    event_count BIGINT NOT NULL CHECK (event_count >= 0),
    core_verification_refs JSONB NOT NULL DEFAULT '[]'::jsonb,
    verified_complete_capability BOOLEAN NOT NULL DEFAULT FALSE,
    max_received_at_ms BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (tenant_id, source_id, partition_id, cursor_key)
);

ALTER TABLE pulso_platform_observation_batches
    ADD COLUMN IF NOT EXISTS core_verification_refs JSONB NOT NULL DEFAULT '[]'::jsonb;
ALTER TABLE pulso_platform_observation_batches
    ADD COLUMN IF NOT EXISTS max_received_at_ms BIGINT NOT NULL DEFAULT 0;
ALTER TABLE pulso_platform_observation_batches
    ADD COLUMN IF NOT EXISTS verified_complete_capability BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE pulso_platform_observation_batches
    DROP CONSTRAINT IF EXISTS pulso_platform_observation_batches_event_count_check;
ALTER TABLE pulso_platform_observation_batches
    ADD CONSTRAINT pulso_platform_observation_batches_event_count_check CHECK (event_count >= 0);

CREATE TABLE IF NOT EXISTS pulso_platform_observation_events (
    tenant_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    source_event_id TEXT NOT NULL,
    event_digest TEXT NOT NULL CHECK (event_digest ~ '^sha256:[0-9a-f]{64}$'),
    event_json JSONB NOT NULL,
    occurred_at_ms BIGINT NOT NULL,
    received_at_ms BIGINT NOT NULL,
    batch_digest TEXT NOT NULL,
    PRIMARY KEY (tenant_id, source_id, source_event_id)
);

CREATE INDEX IF NOT EXISTS pulso_platform_observation_events_time_idx
    ON pulso_platform_observation_events (tenant_id, occurred_at_ms, received_at_ms);
CREATE INDEX IF NOT EXISTS pulso_platform_observation_events_batch_idx
    ON pulso_platform_observation_events (tenant_id, source_id, batch_digest);
CREATE INDEX IF NOT EXISTS pulso_platform_observation_batches_window_idx
    ON pulso_platform_observation_batches
    (tenant_id, ((coverage->>'window_start_ms')::bigint),
     ((coverage->>'window_end_ms')::bigint), max_received_at_ms);

-- Control-plane-owned entitlement. Runtime roles may never mutate this table.
-- A role must be restricted to the tenant(s) intentionally assigned by the
-- DBA; a caller-supplied tenant or custom GUC alone never grants access.
CREATE TABLE IF NOT EXISTS pulso_observation_role_entitlements (
    role_name NAME NOT NULL,
    tenant_id TEXT NOT NULL,
    grant_id TEXT NOT NULL,
    purpose TEXT NOT NULL,
    PRIMARY KEY (role_name, tenant_id, grant_id, purpose)
);
REVOKE ALL ON pulso_observation_role_entitlements FROM PUBLIC;

CREATE OR REPLACE FUNCTION pulso_observation_role_can_access(row_tenant TEXT, requester NAME)
RETURNS BOOLEAN LANGUAGE sql STABLE SECURITY DEFINER
SET search_path = pg_catalog, public AS $$
    SELECT EXISTS (
        SELECT 1 FROM public.pulso_observation_role_entitlements e
        WHERE e.role_name = requester
          AND e.tenant_id = row_tenant
          AND e.grant_id = current_setting('pulso.observation_grant', true)
          AND e.purpose = current_setting('pulso.observation_purpose', true)
    );
$$;

ALTER TABLE pulso_platform_observation_blobs ENABLE ROW LEVEL SECURITY;
ALTER TABLE pulso_platform_observation_blobs FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS pulso_observation_tenant_access ON pulso_platform_observation_blobs;
CREATE POLICY pulso_observation_tenant_access ON pulso_platform_observation_blobs
    USING (pulso_observation_role_can_access(tenant_id, current_user))
    WITH CHECK (pulso_observation_role_can_access(tenant_id, current_user));

ALTER TABLE pulso_platform_observation_cursors ENABLE ROW LEVEL SECURITY;
ALTER TABLE pulso_platform_observation_cursors FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS pulso_observation_tenant_access ON pulso_platform_observation_cursors;
CREATE POLICY pulso_observation_tenant_access ON pulso_platform_observation_cursors
    USING (pulso_observation_role_can_access(tenant_id, current_user))
    WITH CHECK (pulso_observation_role_can_access(tenant_id, current_user));

ALTER TABLE pulso_platform_observation_batches ENABLE ROW LEVEL SECURITY;
ALTER TABLE pulso_platform_observation_batches FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS pulso_observation_tenant_access ON pulso_platform_observation_batches;
CREATE POLICY pulso_observation_tenant_access ON pulso_platform_observation_batches
    USING (pulso_observation_role_can_access(tenant_id, current_user))
    WITH CHECK (pulso_observation_role_can_access(tenant_id, current_user));

ALTER TABLE pulso_platform_observation_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE pulso_platform_observation_events FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS pulso_observation_tenant_access ON pulso_platform_observation_events;
CREATE POLICY pulso_observation_tenant_access ON pulso_platform_observation_events
    USING (pulso_observation_role_can_access(tenant_id, current_user))
    WITH CHECK (pulso_observation_role_can_access(tenant_id, current_user));

CREATE OR REPLACE FUNCTION pulso_reject_platform_observation_mutation()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'platform observations are immutable';
END;
$$;

DROP TRIGGER IF EXISTS pulso_platform_observation_batches_immutable ON pulso_platform_observation_batches;
CREATE TRIGGER pulso_platform_observation_batches_immutable
    BEFORE UPDATE OR DELETE ON pulso_platform_observation_batches
    FOR EACH ROW EXECUTE FUNCTION pulso_reject_platform_observation_mutation();

DROP TRIGGER IF EXISTS pulso_platform_observation_events_immutable ON pulso_platform_observation_events;
CREATE TRIGGER pulso_platform_observation_events_immutable
    BEFORE UPDATE OR DELETE ON pulso_platform_observation_events
    FOR EACH ROW EXECUTE FUNCTION pulso_reject_platform_observation_mutation();

DROP TRIGGER IF EXISTS pulso_platform_observation_blobs_immutable ON pulso_platform_observation_blobs;
CREATE TRIGGER pulso_platform_observation_blobs_immutable
    BEFORE UPDATE OR DELETE ON pulso_platform_observation_blobs
    FOR EACH ROW EXECUTE FUNCTION pulso_reject_platform_observation_mutation();
