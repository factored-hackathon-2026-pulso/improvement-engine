-- R1M (L-PG range, Team Claude): one durable watermark per monitored source, additive; nothing is renamed or dropped.
-- A source id is bound to ONE data mode and ONE adapter, so dataset and platform sources never share a row or a cursor.
-- `watermark` is the adapter cursor as text: `seq:<event_log.sequence>` (product) or `ds:<_ingested_at>|<_batch_id>|<row key>` (datasets).
-- The monitor commits it AFTER the batch it names was processed (at-least-once; downstream work is keyed by the batch, so replay is idempotent).
CREATE SCHEMA IF NOT EXISTS pulso;

CREATE TABLE IF NOT EXISTS pulso.source_watermark (
    source_id TEXT PRIMARY KEY CHECK (source_id ~ '^(dataset|platform):[a-z0-9._:-]{1,80}$'),
    data_mode TEXT NOT NULL CHECK (data_mode IN ('dataset', 'platform')),
    adapter TEXT NOT NULL CHECK (adapter IN ('product-sqlite', 'product-postgres', 'dataset-pg')),
    watermark TEXT NOT NULL CHECK (char_length(watermark) BETWEEN 1 AND 512),
    first_event_time TEXT,
    cases_opened BIGINT NOT NULL DEFAULT 0 CHECK (cases_opened >= 0),
    batches BIGINT NOT NULL DEFAULT 0 CHECK (batches >= 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK (source_id LIKE data_mode || ':%'),
    CHECK ((data_mode = 'dataset') = (adapter = 'dataset-pg')),
    CHECK ((data_mode = 'dataset') = (watermark LIKE 'ds:%'))
);

DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'pulso_app') THEN
        GRANT USAGE ON SCHEMA pulso TO pulso_app;
        GRANT SELECT, INSERT, UPDATE ON pulso.source_watermark TO pulso_app;
    END IF;
END $$;
