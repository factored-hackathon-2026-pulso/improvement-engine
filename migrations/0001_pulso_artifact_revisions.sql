-- U02: append-only Pulso-owned artifact revisions. Source dataset tables remain untouched.
CREATE TABLE IF NOT EXISTS pulso_artifact_revisions (
    tenant_id TEXT NOT NULL CHECK (char_length(tenant_id) BETWEEN 1 AND 128),
    artifact_id UUID NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0),
    kind TEXT NOT NULL CHECK (kind IN (
        'signal', 'opportunity', 'proposal', 'capability_bundle', 'scenario_set',
        'evaluation', 'memory_wiki', 'detector', 'run_config', 'source_snapshot'
    )),
    digest TEXT NOT NULL CHECK (digest ~ '^sha256:[0-9a-f]{64}$'),
    payload JSONB NOT NULL,
    source_snapshot_tenant_id TEXT,
    source_snapshot_artifact_id UUID,
    source_snapshot_revision BIGINT,
    source_snapshot_digest TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (tenant_id, artifact_id, revision),
    CHECK ((source_snapshot_tenant_id IS NULL)
        = (source_snapshot_artifact_id IS NULL)
        AND (source_snapshot_artifact_id IS NULL)
        = (source_snapshot_revision IS NULL)
        AND (source_snapshot_revision IS NULL)
        = (source_snapshot_digest IS NULL))
);

CREATE INDEX IF NOT EXISTS pulso_artifact_revisions_head_idx
    ON pulso_artifact_revisions (tenant_id, artifact_id, revision DESC);

-- A separate head makes a compare-and-swap a single row-level transaction.
CREATE TABLE IF NOT EXISTS pulso_artifact_heads (
    tenant_id TEXT NOT NULL CHECK (char_length(tenant_id) BETWEEN 1 AND 128),
    artifact_id UUID NOT NULL,
    -- Zero is an internal, lockable uninitialized head; externally it is NULL.
    head_revision BIGINT NOT NULL CHECK (head_revision >= 0),
    PRIMARY KEY (tenant_id, artifact_id)
);

CREATE OR REPLACE FUNCTION pulso_reject_artifact_revision_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'pulso artifact revisions are immutable';
END;
$$;

DROP TRIGGER IF EXISTS pulso_artifact_revisions_immutable ON pulso_artifact_revisions;
CREATE TRIGGER pulso_artifact_revisions_immutable
    BEFORE UPDATE OR DELETE ON pulso_artifact_revisions
    FOR EACH ROW EXECUTE FUNCTION pulso_reject_artifact_revision_mutation();

-- This is the only intended runtime write path. The runtime role receives EXECUTE
-- only; migrations/owners retain DDL privileges outside of the service boundary.
CREATE OR REPLACE FUNCTION pulso_append_artifact_revision(
    p_tenant_id TEXT,
    p_artifact_id UUID,
    p_expected_head_revision BIGINT,
    p_revision BIGINT,
    p_kind TEXT,
    p_digest TEXT,
    p_payload JSONB,
    p_source_snapshot_tenant_id TEXT DEFAULT NULL,
    p_source_snapshot_artifact_id UUID DEFAULT NULL,
    p_source_snapshot_revision BIGINT DEFAULT NULL,
    p_source_snapshot_digest TEXT DEFAULT NULL
)
RETURNS VOID
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
    v_actual_head_revision BIGINT;
    v_snapshot_kind TEXT;
    v_snapshot_digest TEXT;
BEGIN
    -- Materialize a lockable sentinel so concurrent first writes have the same
    -- CAS path as later writes. A normal service role gets EXECUTE, not DML.
    INSERT INTO public.pulso_artifact_heads (tenant_id, artifact_id, head_revision)
    VALUES (p_tenant_id, p_artifact_id, 0)
    ON CONFLICT (tenant_id, artifact_id) DO NOTHING;

    SELECT head_revision INTO v_actual_head_revision
    FROM public.pulso_artifact_heads
    WHERE tenant_id = p_tenant_id AND artifact_id = p_artifact_id
    FOR UPDATE;

    IF p_expected_head_revision IS DISTINCT FROM NULLIF(v_actual_head_revision, 0) THEN
        RAISE EXCEPTION 'pulso artifact head conflict' USING ERRCODE = '40001';
    END IF;
    IF p_revision <> v_actual_head_revision + 1 THEN
        RAISE EXCEPTION 'pulso artifact revision is not next' USING ERRCODE = '23514';
    END IF;

    IF p_source_snapshot_tenant_id IS NOT NULL THEN
        IF p_source_snapshot_tenant_id <> p_tenant_id THEN
            RAISE EXCEPTION 'source snapshot tenant differs' USING ERRCODE = '42501';
        END IF;
        SELECT kind, digest INTO v_snapshot_kind, v_snapshot_digest
        FROM public.pulso_artifact_revisions
        WHERE tenant_id = p_source_snapshot_tenant_id
          AND artifact_id = p_source_snapshot_artifact_id
          AND revision = p_source_snapshot_revision;
        IF NOT FOUND OR v_snapshot_kind <> 'source_snapshot'
            OR v_snapshot_digest <> p_source_snapshot_digest THEN
            RAISE EXCEPTION 'invalid source snapshot reference' USING ERRCODE = '23503';
        END IF;
    END IF;

    INSERT INTO public.pulso_artifact_revisions (
        tenant_id, artifact_id, revision, kind, digest, payload,
        source_snapshot_tenant_id, source_snapshot_artifact_id,
        source_snapshot_revision, source_snapshot_digest
    ) VALUES (
        p_tenant_id, p_artifact_id, p_revision, p_kind, p_digest, p_payload,
        p_source_snapshot_tenant_id, p_source_snapshot_artifact_id,
        p_source_snapshot_revision, p_source_snapshot_digest
    );

    UPDATE public.pulso_artifact_heads
    SET head_revision = p_revision
    WHERE tenant_id = p_tenant_id AND artifact_id = p_artifact_id;
END;
$$;

-- Least-privilege read port. It returns the resolved snapshot identity that the
-- Rust adapter needs for defensive readback validation without table SELECT.
CREATE OR REPLACE FUNCTION pulso_get_artifact_revision(
    p_tenant_id TEXT,
    p_artifact_id UUID,
    p_revision BIGINT
)
RETURNS TABLE (
    tenant_id TEXT,
    artifact_id TEXT,
    revision BIGINT,
    kind TEXT,
    digest TEXT,
    payload JSONB,
    source_snapshot_tenant_id TEXT,
    source_snapshot_artifact_id TEXT,
    source_snapshot_revision BIGINT,
    source_snapshot_digest TEXT,
    resolved_source_snapshot_kind TEXT,
    resolved_source_snapshot_digest TEXT
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
    SELECT r.tenant_id, r.artifact_id::TEXT, r.revision, r.kind, r.digest, r.payload,
        r.source_snapshot_tenant_id, r.source_snapshot_artifact_id::TEXT,
        r.source_snapshot_revision, r.source_snapshot_digest,
        snapshot.kind, snapshot.digest
    FROM public.pulso_artifact_revisions r
    LEFT JOIN public.pulso_artifact_revisions snapshot
      ON snapshot.tenant_id = r.source_snapshot_tenant_id
     AND snapshot.artifact_id = r.source_snapshot_artifact_id
     AND snapshot.revision = r.source_snapshot_revision
    WHERE r.tenant_id = p_tenant_id
      AND r.artifact_id = p_artifact_id
      AND r.revision = p_revision;
$$;

REVOKE ALL ON pulso_artifact_revisions, pulso_artifact_heads FROM PUBLIC;
REVOKE ALL ON FUNCTION pulso_append_artifact_revision(
    TEXT, UUID, BIGINT, BIGINT, TEXT, TEXT, JSONB, TEXT, UUID, BIGINT, TEXT
) FROM PUBLIC;
REVOKE ALL ON FUNCTION pulso_get_artifact_revision(TEXT, UUID, BIGINT) FROM PUBLIC;
