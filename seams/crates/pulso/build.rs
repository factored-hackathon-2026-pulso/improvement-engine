//! Embeds the repo `migrations/*.sql` into the binary so the container image is ONE executable
//! (no migrations directory to ship, and none to drift from the code that expects it).
use std::{env, fs, path::PathBuf};

fn main() {
    let dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../../migrations");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "sql"))
        .collect();
    files.sort();
    let mut out = String::from("pub static MIGRATIONS: &[(&str, &str)] = &[\n");
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
        let name = f.file_name().unwrap().to_str().unwrap();
        out.push_str(&format!("    ({name:?}, include_str!({:?})),\n", f.canonicalize().unwrap().display().to_string().trim_start_matches(r"\\?\")));
    }
    out.push_str("];\n");
    fs::write(PathBuf::from(env::var("OUT_DIR").unwrap()).join("migrations.rs"), out).unwrap();
}
