//! Shared helpers: fixture paths and schema validation through the repo's own minischema (python).
#![allow(dead_code)]
use std::path::PathBuf;
use std::process::Command;

pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stp1").join(name)
}
pub fn read_fixture(name: &str) -> String {
    std::fs::read_to_string(fixture(name)).unwrap()
}
fn contracts() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../contracts/engine-steps")
}
/// Validate `doc` against schemas/<schema>.schema.json; returns the error lines (empty = valid).
pub fn schema_errors(schema: &str, doc: &str) -> Vec<String> {
    let c = contracts();
    let script = "import sys,json;sys.path.insert(0,sys.argv[1]);import minischema;\
        s=json.load(open(sys.argv[2]));d=json.loads(sys.argv[3]);print('\\n'.join(minischema.validate(d,s)))";
    let schema_path = c.join("schemas").join(format!("{schema}.schema.json"));
    let out = Command::new("python")
        .args(["-c", script, c.to_str().unwrap(), schema_path.to_str().unwrap(), doc])
        .output()
        .expect("python needed for schema validation");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).lines().filter(|l| !l.is_empty()).map(String::from).collect()
}
pub fn assert_valid(schema: &str, doc: &str) {
    let e = schema_errors(schema, doc);
    assert!(e.is_empty(), "schema {schema}: {e:?}\n{doc}");
}
