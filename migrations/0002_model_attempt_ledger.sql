-- U10 durable model-attempt state. Applied in the same PostgreSQL control
-- plane as U06 jobs; no FK to immutable source data.
CREATE TABLE IF NOT EXISTS pulso_model_attempts (
  tenant_id text NOT NULL,
  job_id text NOT NULL,
  attempt_id text NOT NULL,
  policy_digest text NOT NULL,
  input_commitment text NOT NULL,
  lifecycle text NOT NULL CHECK (lifecycle IN ('terminal','retryable_before_dispatch','dispatching','reconcile_before_retry')),
  retries_used integer NOT NULL CHECK (retries_used >= 0),
  retry_exhausted boolean NOT NULL,
  budget_reservation_id text,
    state_json jsonb NOT NULL,
  revision bigint NOT NULL DEFAULT 1,
  PRIMARY KEY (tenant_id, job_id, attempt_id)
);

-- Production adapter must use UPDATE ... WHERE revision = expected_revision
-- (or equivalent serializable/CAS function) for every lifecycle transition.
