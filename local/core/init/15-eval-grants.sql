-- Applied against core_eval by core-grants. core_eval_app owns the evaluation data; the runtime role may use it.
GRANT USAGE, CREATE ON SCHEMA public TO core_eval_app;
GRANT ALL ON ALL TABLES IN SCHEMA public TO core_eval_app;
GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA public TO core_eval_app; -- runs_run_seq_seq etc.
GRANT SELECT, INSERT ON ALL TABLES IN SCHEMA public TO core_app;
REVOKE ALL ON DATABASE core_eval FROM exporter_ro;
