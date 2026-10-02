-- U07 durable V2 job/run event ledger. Source dataset tables are untouched.
-- Event IDs are supplied by the caller as UUIDv7; sequence is allocated by
-- locking the root job row in the same transaction as the job transition.

CREATE TABLE IF NOT EXISTS pulso_jobs (
    id UUID NOT NULL,
    tenant_id TEXT NOT NULL CHECK (char_length(tenant_id) BETWEEN 1 AND 128),
    run_ref UUID NOT NULL,
    kind TEXT NOT NULL CHECK (char_length(kind) BETWEEN 1 AND 80),
    logical_key TEXT NOT NULL CHECK (char_length(logical_key) BETWEEN 1 AND 256),
    generation BIGINT NOT NULL CHECK (generation >= 0),
    parent_job_id UUID NOT NULL,
    status TEXT NOT NULL CHECK (status IN (
        'queued', 'running', 'retry_wait', 'waiting_dependency', 'complete',
        'deferred', 'dead', 'superseded', 'cancelled'
    )),
    lane TEXT NOT NULL CHECK (char_length(lane) BETWEEN 1 AND 80),
    priority INTEGER NOT NULL DEFAULT 0,
    due_at TIMESTAMPTZ NOT NULL,
    attempt INTEGER NOT NULL DEFAULT 0 CHECK (attempt >= 0),
    lease_owner TEXT,
    lease_until TIMESTAMPTZ,
    lease_version BIGINT NOT NULL DEFAULT 0 CHECK (lease_version >= 0),
    last_event_sequence BIGINT NOT NULL DEFAULT 0 CHECK (last_event_sequence >= 0),
    input_ref TEXT NOT NULL,
    config_ref TEXT NOT NULL,
    reserved_cost_usd NUMERIC(18, 6) NOT NULL DEFAULT 0 CHECK (reserved_cost_usd >= 0),
    actual_cost_usd NUMERIC(18, 6) NOT NULL DEFAULT 0 CHECK (actual_cost_usd >= 0),
    error_code TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (tenant_id, id),
    UNIQUE (tenant_id, kind, logical_key, generation),
    UNIQUE (tenant_id, id, run_ref),
    CHECK (get_byte(uuid_send(id), 6) >> 4 = 7),
    CHECK (get_byte(uuid_send(id), 8) >> 6 = 2),
    FOREIGN KEY (tenant_id, run_ref)
        REFERENCES pulso_jobs (tenant_id, id) DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (tenant_id, parent_job_id)
        REFERENCES pulso_jobs (tenant_id, id) DEFERRABLE INITIALLY DEFERRED,
    CHECK ((parent_job_id = id AND run_ref = id) OR parent_job_id <> id),
    CHECK (parent_job_id = id OR last_event_sequence = 0)
);

CREATE INDEX IF NOT EXISTS pulso_jobs_run_ref_status_idx
    ON pulso_jobs (tenant_id, run_ref, status, id);
CREATE INDEX IF NOT EXISTS pulso_jobs_pending_idx
    ON pulso_jobs (tenant_id, lane, priority DESC, due_at, id)
    WHERE status = 'queued';

CREATE TABLE IF NOT EXISTS pulso_run_events (
    id UUID NOT NULL,
    tenant_id TEXT NOT NULL CHECK (char_length(tenant_id) BETWEEN 1 AND 128),
    run_ref UUID NOT NULL,
    job_ref UUID,
    sequence BIGINT NOT NULL CHECK (sequence > 0),
    event_at TIMESTAMPTZ NOT NULL,
    stage TEXT NOT NULL CHECK (stage ~ '^[a-z0-9_]{1,80}$'),
    event_code TEXT NOT NULL CHECK (event_code ~ '^[a-z0-9_]{1,80}$'),
    status TEXT NOT NULL CHECK (status ~ '^[a-z0-9_]{1,80}$'),
    reason_code TEXT CHECK (reason_code IS NULL OR reason_code ~ '^[a-z0-9_]{1,80}$'),
    artifact_ref TEXT CHECK (artifact_ref IS NULL OR char_length(artifact_ref) BETWEEN 1 AND 256),
    trace_id TEXT CHECK (trace_id IS NULL OR char_length(trace_id) BETWEEN 1 AND 256),
    details_ref TEXT CHECK (details_ref IS NULL OR char_length(details_ref) BETWEEN 1 AND 256),
    PRIMARY KEY (tenant_id, id),
    UNIQUE (tenant_id, run_ref, sequence),
    CHECK (get_byte(uuid_send(id), 6) >> 4 = 7),
    CHECK (get_byte(uuid_send(id), 8) >> 6 = 2),
    FOREIGN KEY (tenant_id, run_ref)
        REFERENCES pulso_jobs (tenant_id, id) DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (tenant_id, job_ref, run_ref)
        REFERENCES pulso_jobs (tenant_id, id, run_ref) DEFERRABLE INITIALLY DEFERRED
);

CREATE INDEX IF NOT EXISTS pulso_run_events_run_sequence_idx
    ON pulso_run_events (tenant_id, run_ref, sequence DESC);

CREATE OR REPLACE FUNCTION pulso_reject_run_event_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'pulso run events are append-only';
END;
$$;

DROP TRIGGER IF EXISTS pulso_run_events_immutable ON pulso_run_events;
CREATE TRIGGER pulso_run_events_immutable
    BEFORE UPDATE OR DELETE ON pulso_run_events
    FOR EACH ROW EXECUTE FUNCTION pulso_reject_run_event_mutation();

CREATE OR REPLACE FUNCTION pulso_validate_job_run_root()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    v_parent_run UUID;
BEGIN
    IF NEW.parent_job_id = NEW.id THEN
        IF NEW.run_ref <> NEW.id THEN
            RAISE EXCEPTION 'root job run_ref must equal its id';
        END IF;
    ELSE
        SELECT run_ref INTO v_parent_run
        FROM public.pulso_jobs
        WHERE tenant_id = NEW.tenant_id AND id = NEW.parent_job_id;
        IF NOT FOUND OR v_parent_run <> NEW.run_ref THEN
            RAISE EXCEPTION 'parent job must belong to the same run';
        END IF;
        IF NEW.last_event_sequence <> 0 THEN
            RAISE EXCEPTION 'only root job may own run event sequence';
        END IF;
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS pulso_jobs_validate_run_root ON pulso_jobs;
CREATE CONSTRAINT TRIGGER pulso_jobs_validate_run_root
AFTER INSERT OR UPDATE
ON pulso_jobs DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION pulso_validate_job_run_root();

CREATE OR REPLACE FUNCTION pulso_validate_run_event_scope()
RETURNS TRIGGER
LANGUAGE plpgsql
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

DROP TRIGGER IF EXISTS pulso_run_events_validate_scope ON pulso_run_events;
CREATE CONSTRAINT TRIGGER pulso_run_events_validate_scope
AFTER INSERT OR UPDATE
ON pulso_run_events DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION pulso_validate_run_event_scope();

REVOKE ALL ON pulso_jobs, pulso_run_events FROM PUBLIC;
