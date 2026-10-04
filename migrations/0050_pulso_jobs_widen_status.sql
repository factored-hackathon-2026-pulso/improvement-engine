-- MIG0 (L-PG): widen pulso_jobs.status additively. Every status admitted by
-- 0003_pulso_run_events.sql stays valid; 'leased' (claimed by a worker, lease
-- held, not yet running) and 'waiting_human' (durable approval wait) are added.
-- Nothing is renamed or dropped; existing rows remain valid.
ALTER TABLE pulso_jobs DROP CONSTRAINT IF EXISTS pulso_jobs_status_check;
ALTER TABLE pulso_jobs ADD CONSTRAINT pulso_jobs_status_check CHECK (status IN (
    'queued', 'running', 'retry_wait', 'waiting_dependency', 'complete',
    'deferred', 'dead', 'superseded', 'cancelled',
    'leased', 'waiting_human'
));
