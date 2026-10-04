-- X-LEARN: append-only canonical replay checkpoint revisions with a CAS head.
-- The payload contains commitments and protocol metadata, never raw replay IDs.

CREATE TABLE IF NOT EXISTS pulso_replay_campaign_checkpoint_heads (
    campaign_digest TEXT PRIMARY KEY
        CHECK (campaign_digest ~ '^sha256:[0-9a-f]{64}$'),
    current_revision BIGINT NOT NULL DEFAULT 0 CHECK (current_revision >= 0),
    cursor BIGINT NOT NULL DEFAULT 0 CHECK (cursor >= 0),
    payload_digest TEXT NOT NULL
        CHECK (payload_digest ~ '^sha256:[0-9a-f]{64}$'),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS pulso_replay_campaign_checkpoint_revisions (
    campaign_digest TEXT NOT NULL
        REFERENCES pulso_replay_campaign_checkpoint_heads(campaign_digest)
        ON DELETE CASCADE,
    revision BIGINT NOT NULL CHECK (revision > 0),
    cursor BIGINT NOT NULL CHECK (cursor >= 0),
    wire_version SMALLINT NOT NULL CHECK (wire_version > 0),
    checkpoint_payload BYTEA NOT NULL,
    payload_digest TEXT NOT NULL
        CHECK (payload_digest ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (campaign_digest, revision),
    UNIQUE (campaign_digest, revision, payload_digest)
);

CREATE INDEX IF NOT EXISTS pulso_replay_campaign_checkpoint_history_idx
    ON pulso_replay_campaign_checkpoint_revisions (campaign_digest, revision DESC);

CREATE OR REPLACE FUNCTION pulso_reject_replay_checkpoint_history_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'replay checkpoint revisions are append-only';
END;
$$;

DROP TRIGGER IF EXISTS pulso_replay_checkpoint_history_immutable
    ON pulso_replay_campaign_checkpoint_revisions;
CREATE TRIGGER pulso_replay_checkpoint_history_immutable
    BEFORE UPDATE OR DELETE ON pulso_replay_campaign_checkpoint_revisions
    FOR EACH ROW EXECUTE FUNCTION pulso_reject_replay_checkpoint_history_mutation();
