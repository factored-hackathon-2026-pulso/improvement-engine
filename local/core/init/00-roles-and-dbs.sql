-- One PG16 instance, two databases (core_runtime, core_eval), distinct least-privilege roles (plan 17.3.8).
-- psql variables only (-v); no literal credentials. Idempotent.
SELECT format('CREATE ROLE core_app LOGIN PASSWORD %L NOSUPERUSER NOCREATEDB NOCREATEROLE', :'core_app_pw')
 WHERE NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'core_app') \gexec
SELECT format('CREATE ROLE core_eval_app LOGIN PASSWORD %L NOSUPERUSER NOCREATEDB NOCREATEROLE', :'core_eval_app_pw')
 WHERE NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'core_eval_app') \gexec
SELECT format('CREATE ROLE exporter_ro LOGIN PASSWORD %L NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION', :'exporter_ro_pw')
 WHERE NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'exporter_ro') \gexec

SELECT 'CREATE DATABASE core_eval OWNER postgres'
 WHERE NOT EXISTS (SELECT 1 FROM pg_database WHERE datname = 'core_eval') \gexec

REVOKE ALL ON DATABASE core_runtime FROM PUBLIC;
REVOKE ALL ON DATABASE core_eval FROM PUBLIC;
GRANT CONNECT ON DATABASE core_runtime TO core_app, exporter_ro;
GRANT CONNECT ON DATABASE core_eval TO core_eval_app, core_app;
-- core_app also needs the eval database for the runtime's evaluation store.
GRANT USAGE ON SCHEMA public TO core_app, exporter_ro;
