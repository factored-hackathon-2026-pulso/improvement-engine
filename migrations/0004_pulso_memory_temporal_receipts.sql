-- P4 durable seam: bind governed U23 use receipts to the triggering event and
-- persist the temporal commitment used by U22/U23. This does not itself mint
-- authorization; only the trusted service composition may invoke the function.

ALTER TABLE pulso_memory_use_receipts
    ADD COLUMN IF NOT EXISTS temporal_commitment TEXT,
    ADD COLUMN IF NOT EXISTS event_ref TEXT;

ALTER TABLE pulso_memory_use_receipts
    DROP CONSTRAINT IF EXISTS pulso_memory_use_temporal_commitment_check;
ALTER TABLE pulso_memory_use_receipts
    ADD CONSTRAINT pulso_memory_use_temporal_commitment_check
    CHECK (temporal_commitment IS NULL OR temporal_commitment ~ '^sha256:[0-9a-f]{64}$');

ALTER TABLE pulso_memory_use_receipts
    DROP CONSTRAINT IF EXISTS pulso_memory_use_event_ref_check;
ALTER TABLE pulso_memory_use_receipts
    ADD CONSTRAINT pulso_memory_use_event_ref_check
    CHECK (event_ref IS NULL OR char_length(event_ref) BETWEEN 1 AND 256);

DROP INDEX IF EXISTS pulso_memory_use_event_scope_uq;
CREATE UNIQUE INDEX IF NOT EXISTS pulso_memory_use_event_uq
    ON pulso_memory_use_receipts (tenant_id, event_ref)
    WHERE event_ref IS NOT NULL;

-- Revoke and use admission lock the same current-head row(s). This gives the
-- tombstone predicate a transaction ordering: either the use receipt commits
-- before revocation, or revocation commits first and the use is rejected.
CREATE OR REPLACE FUNCTION pulso_revoke_memory_snapshot(
    p_tenant_id TEXT,
    p_artifact_id UUID,
    p_artifact_revision BIGINT,
    p_artifact_digest TEXT,
    p_reason_code TEXT
)
RETURNS VOID
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
    v_head public.pulso_memory_heads%ROWTYPE;
    v_bound BOOLEAN := FALSE;
BEGIN
    FOR v_head IN
        WITH RECURSIVE descendants(artifact_id, revision, digest) AS (
            SELECT p_artifact_id, p_artifact_revision, p_artifact_digest
            UNION
            SELECT edge.child_artifact_id, edge.child_revision, edge.child_digest
            FROM public.pulso_memory_lineage edge
            JOIN descendants parent ON edge.tenant_id = p_tenant_id
                AND edge.parent_artifact_id = parent.artifact_id
                AND edge.parent_revision = parent.revision
                AND edge.parent_digest = parent.digest
        )
        SELECT head.*
        FROM public.pulso_memory_heads head
        JOIN descendants current ON head.tenant_id = p_tenant_id
            AND head.artifact_id = current.artifact_id
            AND head.artifact_revision = current.revision
            AND head.artifact_digest = current.digest
        FOR UPDATE OF head
    LOOP
        v_bound := TRUE;
    END LOOP;

    IF NOT v_bound AND NOT EXISTS (
        SELECT 1 FROM public.pulso_memory_lineage
        WHERE tenant_id = p_tenant_id
          AND ((child_artifact_id = p_artifact_id AND child_revision = p_artifact_revision AND child_digest = p_artifact_digest)
            OR (parent_artifact_id = p_artifact_id AND parent_revision = p_artifact_revision AND parent_digest = p_artifact_digest))
    ) THEN
        RAISE EXCEPTION 'memory snapshot is not bound to a memory scope' USING ERRCODE = '23503';
    END IF;

    IF p_reason_code IS NULL OR char_length(p_reason_code) NOT BETWEEN 1 AND 128 THEN
        RAISE EXCEPTION 'invalid memory revocation reason' USING ERRCODE = '22023';
    END IF;
    INSERT INTO public.pulso_memory_tombstones (
        tenant_id, artifact_id, artifact_revision, artifact_digest, reason_code
    ) VALUES (
        p_tenant_id, p_artifact_id, p_artifact_revision, p_artifact_digest, p_reason_code
    ) ON CONFLICT (tenant_id, artifact_id, artifact_revision, artifact_digest) DO NOTHING;
END;
$$;

CREATE OR REPLACE FUNCTION pulso_record_memory_use_temporal(
    p_tenant_id TEXT,
    p_receipt_digest TEXT,
    p_temporal_commitment TEXT,
    p_event_ref TEXT,
    p_purpose TEXT,
    p_world_ref TEXT,
    p_campaign_ref TEXT,
    p_protocol_ref TEXT,
    p_partition_ref TEXT,
    p_artifact_id UUID,
    p_artifact_revision BIGINT,
    p_artifact_digest TEXT,
    p_expected_head_version BIGINT,
    p_run_ref TEXT,
    p_grant_ref TEXT,
    p_allowed_at_unix_seconds BIGINT,
    p_replay_cutoff_unix_seconds BIGINT
)
RETURNS VOID
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
    v_head public.pulso_memory_heads%ROWTYPE;
    v_existing public.pulso_memory_use_receipts%ROWTYPE;
    v_kind TEXT;
    v_digest TEXT;
    v_payload JSONB;
    v_available_at NUMERIC;
    v_inserted INTEGER;
BEGIN
    IF p_tenant_id IS NULL OR char_length(p_tenant_id) NOT BETWEEN 1 AND 128
       OR p_receipt_digest IS NULL OR p_receipt_digest !~ '^sha256:[0-9a-f]{64}$'
       OR p_temporal_commitment IS NULL
       OR p_temporal_commitment !~ '^sha256:[0-9a-f]{64}$'
       OR p_event_ref IS NULL OR char_length(p_event_ref) NOT BETWEEN 1 AND 256
       OR p_run_ref IS NULL OR p_run_ref = '' OR p_grant_ref IS NULL OR p_grant_ref = ''
       OR p_artifact_id IS NULL OR p_artifact_revision IS NULL OR p_artifact_revision <= 0
       OR p_artifact_digest IS NULL OR p_expected_head_version IS NULL OR p_expected_head_version <= 0
       OR p_allowed_at_unix_seconds IS NULL OR p_replay_cutoff_unix_seconds IS NULL
       OR p_allowed_at_unix_seconds < 0 OR p_replay_cutoff_unix_seconds < 0
       OR p_allowed_at_unix_seconds > p_replay_cutoff_unix_seconds THEN
        RAISE EXCEPTION 'invalid temporal memory-use request' USING ERRCODE = '22023';
    END IF;

    SELECT * INTO v_head
    FROM public.pulso_memory_heads
    WHERE tenant_id = p_tenant_id AND purpose = p_purpose AND world_ref = p_world_ref
      AND campaign_ref = p_campaign_ref AND protocol_ref = p_protocol_ref
      AND partition_ref = p_partition_ref
    FOR UPDATE;
    IF NOT FOUND OR v_head.head_version <> p_expected_head_version
       OR v_head.artifact_id <> p_artifact_id
       OR v_head.artifact_revision <> p_artifact_revision
       OR v_head.artifact_digest <> p_artifact_digest THEN
        RAISE EXCEPTION 'memory use does not match current scoped head' USING ERRCODE = '40001';
    END IF;

    IF public.pulso_memory_snapshot_revoked(
        p_tenant_id, p_artifact_id, p_artifact_revision, p_artifact_digest
    ) THEN
        RAISE EXCEPTION 'memory snapshot revoked' USING ERRCODE = '42501';
    END IF;

    SELECT kind, digest, payload INTO v_kind, v_digest, v_payload
    FROM public.pulso_artifact_revisions
    WHERE tenant_id = p_tenant_id AND artifact_id = p_artifact_id
      AND revision = p_artifact_revision;
    IF NOT FOUND OR v_kind <> 'memory_wiki' OR v_digest <> p_artifact_digest
       OR v_payload->>'purpose' <> p_purpose
       OR jsonb_typeof(v_payload->'available_at_unix_seconds') <> 'number'
       OR NOT public.pulso_memory_pages_valid(v_payload->'pages') THEN
        RAISE EXCEPTION 'memory snapshot is unavailable at governed cutoff' USING ERRCODE = '42501';
    END IF;
    v_available_at := (v_payload->>'available_at_unix_seconds')::NUMERIC;
    IF v_available_at < 0 OR trunc(v_available_at) <> v_available_at
       OR v_available_at > p_allowed_at_unix_seconds
       OR v_available_at > p_replay_cutoff_unix_seconds THEN
        RAISE EXCEPTION 'memory snapshot is unavailable at governed cutoff' USING ERRCODE = '42501';
    END IF;

    SELECT * INTO v_existing
    FROM public.pulso_memory_use_receipts
    WHERE tenant_id = p_tenant_id AND event_ref = p_event_ref;
    IF FOUND THEN
        IF v_existing.receipt_digest IS NOT DISTINCT FROM p_receipt_digest
           AND v_existing.temporal_commitment IS NOT DISTINCT FROM p_temporal_commitment
           AND v_existing.artifact_id IS NOT DISTINCT FROM p_artifact_id
           AND v_existing.artifact_revision IS NOT DISTINCT FROM p_artifact_revision
           AND v_existing.artifact_digest IS NOT DISTINCT FROM p_artifact_digest
           AND v_existing.head_version IS NOT DISTINCT FROM p_expected_head_version
           AND v_existing.run_ref IS NOT DISTINCT FROM p_run_ref
           AND v_existing.grant_ref IS NOT DISTINCT FROM p_grant_ref
           AND v_existing.purpose IS NOT DISTINCT FROM p_purpose
           AND v_existing.world_ref IS NOT DISTINCT FROM p_world_ref
           AND v_existing.campaign_ref IS NOT DISTINCT FROM p_campaign_ref
           AND v_existing.protocol_ref IS NOT DISTINCT FROM p_protocol_ref
           AND v_existing.partition_ref IS NOT DISTINCT FROM p_partition_ref
           AND v_existing.allowed_at_unix_seconds IS NOT DISTINCT FROM p_allowed_at_unix_seconds THEN
            RETURN;
        END IF;
        RAISE EXCEPTION 'event already admitted with different memory-use identity' USING ERRCODE = '23505';
    END IF;

    INSERT INTO public.pulso_memory_use_receipts (
        tenant_id, receipt_digest, purpose, world_ref, campaign_ref, protocol_ref,
        partition_ref, artifact_id, artifact_revision, artifact_digest, head_version,
        run_ref, grant_ref, allowed_at_unix_seconds, temporal_commitment, event_ref
    ) VALUES (
        p_tenant_id, p_receipt_digest, p_purpose, p_world_ref, p_campaign_ref,
        p_protocol_ref, p_partition_ref, p_artifact_id, p_artifact_revision,
        p_artifact_digest, p_expected_head_version, p_run_ref, p_grant_ref,
        p_allowed_at_unix_seconds, p_temporal_commitment, p_event_ref
    ) ON CONFLICT (tenant_id, event_ref) WHERE event_ref IS NOT NULL DO NOTHING;
    GET DIAGNOSTICS v_inserted = ROW_COUNT;
    IF v_inserted = 0 THEN
        SELECT * INTO v_existing
        FROM public.pulso_memory_use_receipts
        WHERE tenant_id = p_tenant_id AND event_ref = p_event_ref;
        IF NOT FOUND
           OR v_existing.receipt_digest IS DISTINCT FROM p_receipt_digest
           OR v_existing.temporal_commitment IS DISTINCT FROM p_temporal_commitment
           OR v_existing.purpose IS DISTINCT FROM p_purpose
           OR v_existing.world_ref IS DISTINCT FROM p_world_ref
           OR v_existing.campaign_ref IS DISTINCT FROM p_campaign_ref
           OR v_existing.protocol_ref IS DISTINCT FROM p_protocol_ref
           OR v_existing.partition_ref IS DISTINCT FROM p_partition_ref
           OR v_existing.artifact_id IS DISTINCT FROM p_artifact_id
           OR v_existing.artifact_revision IS DISTINCT FROM p_artifact_revision
           OR v_existing.artifact_digest IS DISTINCT FROM p_artifact_digest
           OR v_existing.head_version IS DISTINCT FROM p_expected_head_version
           OR v_existing.run_ref IS DISTINCT FROM p_run_ref
           OR v_existing.grant_ref IS DISTINCT FROM p_grant_ref
           OR v_existing.allowed_at_unix_seconds IS DISTINCT FROM p_allowed_at_unix_seconds THEN
            RAISE EXCEPTION 'event already admitted with different memory-use identity' USING ERRCODE = '23505';
        END IF;
    END IF;
END;
$$;

REVOKE ALL ON FUNCTION pulso_record_memory_use_temporal(
    TEXT, TEXT, TEXT, TEXT, TEXT, TEXT, TEXT, TEXT, TEXT, UUID, BIGINT, TEXT,
    BIGINT, TEXT, TEXT, BIGINT, BIGINT
) FROM PUBLIC;
