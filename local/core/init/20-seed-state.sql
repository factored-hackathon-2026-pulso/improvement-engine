-- Marker table for idempotent seeding: a second run with the same assets digest writes nothing.
CREATE TABLE IF NOT EXISTS pulso_seed_state (
    name text PRIMARY KEY,
    assets_digest text NOT NULL,
    seeded_at timestamptz NOT NULL DEFAULT now()
);
GRANT SELECT, INSERT, UPDATE ON pulso_seed_state TO core_app;
-- exporter_ro must not see it (least privilege is verified by the exporter).
REVOKE ALL ON pulso_seed_state FROM exporter_ro;
