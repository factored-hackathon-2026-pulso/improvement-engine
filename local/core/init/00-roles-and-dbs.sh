#!/bin/sh
# Runs once on a fresh data directory (docker-entrypoint-initdb.d). Passwords arrive as env names, never as literals.
set -eu
psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" \
  -v core_app_pw="$CORE_APP_PASSWORD" -v core_eval_app_pw="$CORE_EVAL_APP_PASSWORD" \
  -v exporter_ro_pw="$EXPORTER_RO_PASSWORD" -f /opt/pulso-init/00-roles-and-dbs.sql
