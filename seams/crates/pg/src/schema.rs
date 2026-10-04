//! Deterministic schema snapshot of the `public` schema: tables, columns, constraints, indexes,
//! functions and triggers as sorted text lines. The runner's own bookkeeping table is excluded.
use postgres::Client;

/// Name of the runner's bookkeeping table (excluded from snapshots).
pub const MIGRATIONS_TABLE: &str = "pulso_schema_migrations";

const QUERIES: [&str; 6] = [
    "SELECT 'table ' || c.relname FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
     WHERE n.nspname = 'public' AND c.relkind IN ('r','p') AND c.relname <> $1",
    "SELECT 'column ' || c.relname || '.' || a.attname || ' ' || format_type(a.atttypid, a.atttypmod) \
     || CASE WHEN a.attnotnull THEN ' not null' ELSE '' END \
     || COALESCE(' default ' || pg_get_expr(d.adbin, d.adrelid), '') \
     FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid JOIN pg_namespace n ON n.oid = c.relnamespace \
     LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
     WHERE n.nspname = 'public' AND c.relkind IN ('r','p') AND a.attnum > 0 AND NOT a.attisdropped AND c.relname <> $1",
    "SELECT 'constraint ' || c.relname || ' ' || k.conname || ' ' || pg_get_constraintdef(k.oid) \
     FROM pg_constraint k JOIN pg_class c ON c.oid = k.conrelid JOIN pg_namespace n ON n.oid = c.relnamespace \
     WHERE n.nspname = 'public' AND c.relname <> $1",
    "SELECT 'index ' || indexdef FROM pg_indexes WHERE schemaname = 'public' AND tablename <> $1",
    "SELECT 'function ' || pg_get_functiondef(p.oid) FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
     WHERE n.nspname = 'public' AND p.prokind = 'f' AND $1::text IS NOT NULL",
    "SELECT 'trigger ' || pg_get_triggerdef(t.oid) FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid \
     JOIN pg_namespace n ON n.oid = c.relnamespace \
     WHERE n.nspname = 'public' AND NOT t.tgisinternal AND c.relname <> $1",
];

/// Sorted, de-duplicated snapshot lines (multi-line function bodies are kept as one entry).
pub fn snapshot(client: &mut Client) -> Result<Vec<String>, postgres::Error> {
    let mut lines = Vec::new();
    for q in QUERIES {
        for row in client.query(q, &[&MIGRATIONS_TABLE])? {
            lines.push(row.get::<_, String>(0));
        }
    }
    lines.sort();
    lines.dedup();
    Ok(lines)
}
