//! Deterministic schema snapshot of the `public` schema: tables, columns, constraints, indexes,
//! functions and triggers as sorted text lines. The runner's own bookkeeping table is excluded.
use postgres::Client;

/// Name of the runner's bookkeeping table (excluded from snapshots).
pub const MIGRATIONS_TABLE: &str = "pulso_schema_migrations";

const QUERIES: [&str; 14] = [
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
    // column order, comments, ACLs, owners, RLS, policies, extensions, enum/domain types,
    // function owner/ACL and trigger enablement.
    "SELECT 'colorder ' || c.relname || '.' || a.attname || ' ' || row_number() OVER (PARTITION BY c.oid ORDER BY a.attnum) \
     FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid JOIN pg_namespace n ON n.oid = c.relnamespace \
     WHERE n.nspname = 'public' AND c.relkind IN ('r','p') AND a.attnum > 0 AND NOT a.attisdropped AND c.relname <> $1",
    "SELECT 'comment ' || c.relkind::text || ' ' || c.relname || '.' || d.objsubid || ' ' || d.description \
     FROM pg_description d JOIN pg_class c ON c.oid = d.objoid AND d.classoid = 'pg_class'::regclass \
     JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = 'public' AND c.relname <> $1",
    "SELECT 'rel ' || c.relkind::text || ' ' || c.relname || ' owner=' || pg_get_userbyid(c.relowner) \
     || ' rls=' || c.relrowsecurity::text || '/' || c.relforcerowsecurity::text || ' acl=' || COALESCE(c.relacl::text, '') \
     FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
     WHERE n.nspname = 'public' AND c.relkind IN ('r','p','S','v','m') AND c.relname <> $1",
    "SELECT 'policy ' || c.relname || ' ' || p.polname || ' ' || p.polcmd::text || ' ' || p.polpermissive::text \
     || ' ' || COALESCE(pg_get_expr(p.polqual, p.polrelid), '') || ' ' || COALESCE(pg_get_expr(p.polwithcheck, p.polrelid), '') \
     FROM pg_policy p JOIN pg_class c ON c.oid = p.polrelid JOIN pg_namespace n ON n.oid = c.relnamespace \
     WHERE n.nspname = 'public' AND c.relname <> $1",
    "SELECT 'extension ' || extname || ' ' || extversion FROM pg_extension WHERE $1::text IS NOT NULL",
    "SELECT 'type ' || t.typname || ' ' || t.typtype::text || ' ' || COALESCE((SELECT string_agg(e.enumlabel, ',' ORDER BY e.enumsortorder) \
     FROM pg_enum e WHERE e.enumtypid = t.oid), '') FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace \
     WHERE n.nspname = 'public' AND t.typtype IN ('e','d') AND $1::text IS NOT NULL",
    "SELECT 'fnmeta ' || p.proname || '(' || pg_get_function_identity_arguments(p.oid) || ') owner=' || pg_get_userbyid(p.proowner) \
     || ' acl=' || COALESCE(p.proacl::text, '') FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
     WHERE n.nspname = 'public' AND $1::text IS NOT NULL",
    "SELECT 'triggerstate ' || c.relname || ' ' || t.tgname || ' ' || t.tgenabled::text FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid \
     JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = 'public' AND NOT t.tgisinternal AND c.relname <> $1",
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
