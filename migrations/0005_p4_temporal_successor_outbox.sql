-- P4-U33 admitted-use composition: one tenant-scoped source event owns one
-- immutable memory-use receipt and one durable queued successor request.
-- Agent execution/admission remains a later U06 responsibility.

ALTER TABLE pulso_jobs
    ADD COLUMN IF NOT EXISTS source_event_ref TEXT,
    ADD COLUMN IF NOT EXISTS request_digest TEXT;

ALTER TABLE pulso_jobs
    DROP CONSTRAINT IF EXISTS pulso_jobs_source_event_pair_check;
ALTER TABLE pulso_jobs
    ADD CONSTRAINT pulso_jobs_source_event_pair_check
    CHECK (
        (source_event_ref IS NULL) = (request_digest IS NULL)
        AND (source_event_ref IS NULL OR char_length(source_event_ref) BETWEEN 1 AND 256)
        AND (request_digest IS NULL OR request_digest ~ '^sha256:[0-9a-f]{64}$')
    );

CREATE UNIQUE INDEX IF NOT EXISTS pulso_jobs_source_event_uq
    ON pulso_jobs (tenant_id, source_event_ref)
    WHERE source_event_ref IS NOT NULL;

-- This scope trigger is deferred until transaction commit. The committing
-- caller is the least-privilege runtime role (which has no direct pulso_jobs
-- read access), so its validation must use a fixed, privileged boundary.
CREATE OR REPLACE FUNCTION public.pulso_validate_run_event_scope()
RETURNS TRIGGER
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
    v_root_run UUID;
    v_job_run UUID;
BEGIN
    SELECT run_ref INTO v_root_run
    FROM public.pulso_jobs
    WHERE tenant_id = NEW.tenant_id AND id = NEW.run_ref;
    IF NOT FOUND OR v_root_run <> NEW.run_ref THEN
        RAISE EXCEPTION 'run event reference must identify its run root';
    END IF;

    IF NEW.job_ref IS NOT NULL THEN
        SELECT run_ref INTO v_job_run
        FROM public.pulso_jobs
        WHERE tenant_id = NEW.tenant_id AND id = NEW.job_ref;
        IF NOT FOUND OR v_job_run <> NEW.run_ref THEN
            RAISE EXCEPTION 'run event job must belong to the referenced run';
        END IF;
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION public.pulso_validate_run_event_scope() FROM PUBLIC;

CREATE OR REPLACE FUNCTION pulso_record_temporal_memory_successor(
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
    p_source_run_ref TEXT,
    p_grant_ref TEXT,
    p_allowed_at_unix_seconds BIGINT,
    p_replay_cutoff_unix_seconds BIGINT,
    p_successor_job_id UUID,
    p_request_digest TEXT,
    p_event_occurred_at_unix_ms BIGINT
)
RETURNS TABLE(receipt_digest TEXT, successor_job_id UUID, successor_status TEXT)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
DECLARE
    v_receipt public.pulso_memory_use_receipts%ROWTYPE;
    v_job public.pulso_jobs%ROWTYPE;
    v_event public.pulso_run_events%ROWTYPE;
    v_has_receipt BOOLEAN;
    v_has_job BOOLEAN;
    v_has_event BOOLEAN;
BEGIN
    IF p_tenant_id IS NULL OR char_length(p_tenant_id) NOT BETWEEN 1 AND 128
       OR p_event_ref IS NULL OR p_event_ref !~ '^sha256_[0-9a-f]{56}$'
       OR p_request_digest IS NULL OR p_request_digest !~ '^sha256:[0-9a-f]{64}$'
       OR p_successor_job_id IS NULL
       OR get_byte(uuid_send(p_successor_job_id), 6) >> 4 <> 7
       OR get_byte(uuid_send(p_successor_job_id), 8) >> 6 <> 2
       OR p_event_occurred_at_unix_ms IS NULL OR p_event_occurred_at_unix_ms < 0
       OR (
            (get_byte(uuid_send(p_successor_job_id), 0)::BIGINT << 40)
          | (get_byte(uuid_send(p_successor_job_id), 1)::BIGINT << 32)
          | (get_byte(uuid_send(p_successor_job_id), 2)::BIGINT << 24)
          | (get_byte(uuid_send(p_successor_job_id), 3)::BIGINT << 16)
          | (get_byte(uuid_send(p_successor_job_id), 4)::BIGINT << 8)
          | get_byte(uuid_send(p_successor_job_id), 5)::BIGINT
       ) IS DISTINCT FROM p_event_occurred_at_unix_ms THEN
        RAISE EXCEPTION 'invalid temporal successor request' USING ERRCODE = '22023';
    END IF;

    -- Serialize all retries/competing scopes for this tenant event before
    -- inspecting either side of the receipt/outbox pair.
    PERFORM pg_advisory_xact_lock(
        hashtextextended(p_tenant_id || chr(31) || p_event_ref, 0)
    );

    SELECT * INTO v_receipt
    FROM public.pulso_memory_use_receipts
    WHERE tenant_id = p_tenant_id AND event_ref = p_event_ref
    FOR UPDATE;
    v_has_receipt := FOUND;

    SELECT * INTO v_job
    FROM public.pulso_jobs
    WHERE tenant_id = p_tenant_id AND source_event_ref = p_event_ref
    FOR UPDATE;
    v_has_job := FOUND;

    SELECT * INTO v_event
    FROM public.pulso_run_events
    WHERE tenant_id = p_tenant_id
      AND run_ref = COALESCE(v_job.id, p_successor_job_id)
      AND job_ref = COALESCE(v_job.id, p_successor_job_id)
      AND id = COALESCE(v_job.id, p_successor_job_id)
    FOR UPDATE;
    v_has_event := FOUND;

    IF v_has_receipt OR v_has_job OR v_has_event THEN
        IF NOT v_has_receipt OR NOT v_has_job OR NOT v_has_event THEN
            RAISE EXCEPTION 'temporal successor pair requires reconciliation'
                USING ERRCODE = '55000';
        END IF;

        -- U33 checks the current live scoped head and tombstone before its own
        -- idempotent receipt return. Revalidate after confirming the durable
        -- pair is complete, but before returning it, so revoked/replaced memory
        -- cannot turn an old exact event replay into success.
        PERFORM public.pulso_record_memory_use_temporal(
            p_tenant_id, p_receipt_digest, p_temporal_commitment, p_event_ref,
            p_purpose, p_world_ref, p_campaign_ref, p_protocol_ref, p_partition_ref,
            p_artifact_id, p_artifact_revision, p_artifact_digest,
            p_expected_head_version, p_source_run_ref, p_grant_ref,
            p_allowed_at_unix_seconds, p_replay_cutoff_unix_seconds
        );

        IF v_receipt.receipt_digest IS DISTINCT FROM p_receipt_digest
           OR v_receipt.temporal_commitment IS DISTINCT FROM p_temporal_commitment
           OR v_receipt.event_ref IS DISTINCT FROM p_event_ref
           OR v_receipt.purpose IS DISTINCT FROM p_purpose
           OR v_receipt.world_ref IS DISTINCT FROM p_world_ref
           OR v_receipt.campaign_ref IS DISTINCT FROM p_campaign_ref
           OR v_receipt.protocol_ref IS DISTINCT FROM p_protocol_ref
           OR v_receipt.partition_ref IS DISTINCT FROM p_partition_ref
           OR v_receipt.artifact_id IS DISTINCT FROM p_artifact_id
           OR v_receipt.artifact_revision IS DISTINCT FROM p_artifact_revision
           OR v_receipt.artifact_digest IS DISTINCT FROM p_artifact_digest
           OR v_receipt.head_version IS DISTINCT FROM p_expected_head_version
           OR v_receipt.run_ref IS DISTINCT FROM p_source_run_ref
           OR v_receipt.grant_ref IS DISTINCT FROM p_grant_ref
           OR v_receipt.allowed_at_unix_seconds IS DISTINCT FROM p_allowed_at_unix_seconds
           OR v_job.id IS DISTINCT FROM p_successor_job_id
           OR v_job.run_ref IS DISTINCT FROM p_successor_job_id
           OR v_job.kind IS DISTINCT FROM 'memory_successor'
           OR v_job.logical_key IS DISTINCT FROM p_event_ref
           OR v_job.generation IS DISTINCT FROM 0
           OR v_job.request_digest IS DISTINCT FROM p_request_digest
           OR v_event.sequence IS DISTINCT FROM 1
           OR v_event.stage IS DISTINCT FROM 'admission'
           OR v_event.event_code IS DISTINCT FROM 'job_queued'
           OR v_event.status IS DISTINCT FROM 'queued'
           OR v_event.reason_code IS DISTINCT FROM 'temporal_memory_successor'
           OR v_event.details_ref IS DISTINCT FROM 'request:' || p_request_digest THEN
            RAISE EXCEPTION 'event already has a different temporal successor request'
                USING ERRCODE = '23505';
        END IF;

        RETURN QUERY SELECT v_receipt.receipt_digest, v_job.id, v_job.status;
        RETURN;
    END IF;

    -- U33 owns scoped-head, exact artifact, cutoff and revocation checks. Its
    -- receipt insert and this successor outbox insert share this transaction.
    PERFORM public.pulso_record_memory_use_temporal(
        p_tenant_id, p_receipt_digest, p_temporal_commitment, p_event_ref,
        p_purpose, p_world_ref, p_campaign_ref, p_protocol_ref, p_partition_ref,
        p_artifact_id, p_artifact_revision, p_artifact_digest,
        p_expected_head_version, p_source_run_ref, p_grant_ref,
        p_allowed_at_unix_seconds, p_replay_cutoff_unix_seconds
    );

    INSERT INTO public.pulso_jobs (
        id, tenant_id, run_ref, kind, logical_key, generation, parent_job_id,
        status, lane, due_at, input_ref, config_ref, source_event_ref, request_digest
    ) VALUES (
        p_successor_job_id, p_tenant_id, p_successor_job_id,
        'memory_successor', p_event_ref, 0, p_successor_job_id,
        'queued', 'memory_successor', CURRENT_TIMESTAMP,
        'source_event:' || p_event_ref,
        'memory_temporal_request:' || p_request_digest,
        p_event_ref, p_request_digest
    );

    UPDATE public.pulso_jobs
    SET last_event_sequence = 1
    WHERE tenant_id = p_tenant_id AND id = p_successor_job_id;

    INSERT INTO public.pulso_run_events (
        id, tenant_id, run_ref, job_ref, sequence, event_at, stage, event_code,
        status, reason_code, artifact_ref, details_ref
    ) VALUES (
        p_successor_job_id, p_tenant_id, p_successor_job_id, p_successor_job_id,
        1, CURRENT_TIMESTAMP, 'admission', 'job_queued', 'queued',
        'temporal_memory_successor',
        'memory:' || p_artifact_id::TEXT || ':' || p_artifact_revision::TEXT || ':' || p_artifact_digest,
        'request:' || p_request_digest
    );

    RETURN QUERY SELECT p_receipt_digest, p_successor_job_id, 'queued'::TEXT;
END;
$$;

REVOKE ALL ON FUNCTION pulso_record_temporal_memory_successor(
    TEXT, TEXT, TEXT, TEXT, TEXT, TEXT, TEXT, TEXT, TEXT, UUID, BIGINT, TEXT,
    BIGINT, TEXT, TEXT, BIGINT, BIGINT, UUID, TEXT, BIGINT
) FROM PUBLIC;
