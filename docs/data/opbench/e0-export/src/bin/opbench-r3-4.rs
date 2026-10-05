#[path = "../r3_4.rs"]
mod r3_4;

use std::{
    collections::BTreeMap,
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process,
};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn main() {
    if let Err(message) = run() {
        eprintln!("R3-4 failed: {message}");
        process::exit(2);
    }
}

fn run() -> Result<(), &'static str> {
    let args = parse_args(env::args().skip(1))?;
    let bank_root = args
        .get("bank-data-root")
        .ok_or("missing --bank-data-root")?;
    let e0_dir = args.get("e0-data-dir").ok_or("missing --e0-data-dir")?;
    let output_dir = args.get("output-dir").ok_or("missing --output-dir")?;
    let repo_root = args.get("repo-root").ok_or("missing --repo-root")?;
    let prereg_commit = args.get("prereg-commit").ok_or("missing --prereg-commit")?;
    let code_revision = args.get("code-revision").ok_or("missing --code-revision")?;
    validate_sha(prereg_commit)
        .map_err(|_| "--prereg-commit must be a full 40-character commit SHA")?;
    validate_sha(code_revision)
        .map_err(|_| "--code-revision must be a full 40-character commit SHA")?;

    let bank_root = fs::canonicalize(bank_root).map_err(|_| "could not resolve bank data root")?;
    let e0_dir = fs::canonicalize(e0_dir).map_err(|_| "could not resolve E0 data directory")?;
    let repo_root = fs::canonicalize(repo_root).map_err(|_| "could not resolve repository root")?;
    if !repo_root.join(".git").exists() {
        return Err("--repo-root must identify a Git worktree");
    }
    let output_dir = resolve_new_output(output_dir, &repo_root, &[&bank_root, &e0_dir])?;

    let digests = r3_4::digest_inputs(&bank_root, &e0_dir)?;
    let schema_fingerprints = r3_4::schema_fingerprints(&bank_root, &e0_dir)?;
    let (cases, queries, closes) = r3_4::read_e0_inputs(&e0_dir)?;
    let complaints = r3_4::read_bank_complaints(&bank_root)?;
    let row_counts = json!({
        "e0_cases": cases.len(),
        "e0_copilot_queries": queries.len(),
        "e0_case_closes": closes.len(),
        "bank_complaints": complaints.len()
    });

    fs::create_dir(&output_dir).map_err(|_| "output directory must be new and writable")?;

    // Freeze the registration, input snapshot digests, schema contract, and row
    // counts outside Git before calculating or serializing any result tables.
    let manifest_path = output_dir.join("run_manifest.json");
    let mut manifest = json!({
        "benchmark": "OPBENCH-lite-R3-4",
        "generator_version": concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")),
        "preregistration_commit": prereg_commit.to_ascii_lowercase(),
        "code_revision": code_revision.to_ascii_lowercase(),
        "result_schema_version": "2",
        "configuration": {"k": 10},
        "schema_contract_version": "r3-4-allowlist-v1",
        "analysis_cutoff_utc": null,
        "cutoff_policy": "full registered source snapshots; no temporal filter",
        "ordering": {
            "bank_csv_partitions": "lexicographic relative path",
            "aggregate_rows": "category then subcategory, lexicographic"
        },
        "schema_fingerprints": schema_fingerprints,
        "input_digests": digests,
        "row_counts": row_counts
    });
    write_new_json(&manifest_path, &manifest)?;

    if cases.len() != 2_000 {
        return Err("E0 case count differs from the preregistered 2,000-case sample");
    }
    if !all_cases_link_exactly(&cases, &complaints) {
        return Err("E0 contains a null or orphan complaint link");
    }

    let results = r3_4::aggregate(
        &r3_4::Input {
            cases,
            complaints,
            queries,
            closes,
        },
        10,
    )?;
    let result_bytes =
        serde_json::to_vec_pretty(&results).map_err(|_| "could not encode aggregate results")?;
    write_new_bytes(&output_dir.join("results.json"), &result_bytes)?;
    manifest["result_sha256"] = json!(format!("{:x}", Sha256::digest(&result_bytes)));
    replace_json(&manifest_path, &manifest)?;
    Ok(())
}

fn parse_args(
    args: impl Iterator<Item = String>,
) -> Result<BTreeMap<String, String>, &'static str> {
    let mut parsed = BTreeMap::new();
    let mut values = args;
    while let Some(flag) = values.next() {
        let key = flag
            .strip_prefix("--")
            .ok_or("arguments must use --name value form")?;
        if ![
            "bank-data-root",
            "e0-data-dir",
            "output-dir",
            "repo-root",
            "prereg-commit",
            "code-revision",
        ]
        .contains(&key)
        {
            return Err("unknown command-line argument");
        }
        let value = values
            .next()
            .ok_or("command-line argument is missing its value")?;
        if value.is_empty() || parsed.insert(key.to_owned(), value).is_some() {
            return Err("command-line arguments must be non-empty and unique");
        }
    }
    if parsed.len() != 6 {
        return Err("all six explicit command-line arguments are required");
    }
    Ok(parsed)
}

fn validate_sha(value: &str) -> Result<(), ()> {
    if value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(())
    }
}

fn all_cases_link_exactly(cases: &[r3_4::E0Case], complaints: &[r3_4::BankComplaint]) -> bool {
    let complaint_ids = complaints
        .iter()
        .map(|complaint| complaint.complaint_id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut linked_ids = std::collections::HashSet::new();
    cases.iter().all(|case| {
        case.complaint_id
            .as_deref()
            .is_some_and(|id| complaint_ids.contains(id) && linked_ids.insert(id))
    })
}

fn resolve_new_output(
    path: &str,
    repo_root: &Path,
    inputs: &[&Path],
) -> Result<PathBuf, &'static str> {
    let requested = Path::new(path);
    if requested.exists() {
        return Err("output directory must not already exist");
    }
    let parent = requested
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent =
        fs::canonicalize(parent).map_err(|_| "output parent directory must already exist")?;
    let name = requested
        .file_name()
        .ok_or("output directory path is invalid")?;
    let resolved = parent.join(name);
    if resolved.starts_with(repo_root) || inputs.iter().any(|input| resolved.starts_with(input)) {
        return Err("output directory must be outside the repository and source data roots");
    }
    Ok(resolved)
}

fn write_new_json(path: &Path, value: &Value) -> Result<(), &'static str> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|_| "could not encode run manifest")?;
    write_new_bytes(path, &bytes)
}

fn write_new_bytes(path: &Path, bytes: &[u8]) -> Result<(), &'static str> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| "could not create a new output file")?;
    file.write_all(bytes)
        .map_err(|_| "could not write aggregate output")
}

fn replace_json(path: &Path, value: &Value) -> Result<(), &'static str> {
    let temp_path = path.with_extension("json.tmp");
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|_| "could not encode final run manifest")?;
    write_new_bytes(&temp_path, &bytes)?;
    fs::rename(temp_path, path).map_err(|_| "could not finalize run manifest")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_requires_all_explicit_paths_and_versions() {
        let missing = parse_args(["--bank-data-root", "data"].map(str::to_owned).into_iter());
        assert_eq!(
            missing.unwrap_err(),
            "all six explicit command-line arguments are required"
        );
    }

    #[test]
    fn runner_rejects_unknown_or_duplicate_arguments() {
        let unknown = parse_args(["--surprise", "x"].map(str::to_owned).into_iter());
        assert_eq!(unknown.unwrap_err(), "unknown command-line argument");

        let duplicate = parse_args(
            ["--bank-data-root", "a", "--bank-data-root", "b"]
                .map(str::to_owned)
                .into_iter(),
        );
        assert_eq!(
            duplicate.unwrap_err(),
            "command-line arguments must be non-empty and unique"
        );
    }

    #[test]
    fn runner_accepts_only_full_git_commit_shas() {
        assert!(validate_sha("0123456789abcdef0123456789abcdef01234567").is_ok());
        assert!(validate_sha("deadbeef").is_err());
        assert!(validate_sha("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz").is_err());
    }

    #[test]
    fn rejects_null_and_orphan_complaint_links_before_result_creation() {
        let complaints = vec![r3_4::BankComplaint {
            complaint_id: "SYN-COMPLAINT-1".into(),
            category: "C".into(),
            subcategory: "S".into(),
            status: "Open".into(),
            sla_breached: None,
            resolution: None,
            resolution_days: None,
        }];
        let cases = vec![r3_4::E0Case {
            case_id: "SYN-CASE-1".into(),
            complaint_id: Some("SYN-ORPHAN".into()),
            opened_at: 0,
        }];
        assert!(!all_cases_link_exactly(&cases, &complaints));
        let cases = vec![r3_4::E0Case {
            case_id: "SYN-CASE-1".into(),
            complaint_id: None,
            opened_at: 0,
        }];
        assert!(!all_cases_link_exactly(&cases, &complaints));
    }

    #[test]
    fn output_cannot_be_placed_inside_the_git_worktree_from_any_caller_directory() {
        let executable = env::current_exe().expect("test executable path");
        let repo_root = executable
            .ancestors()
            .find(|path| path.join(".git").exists())
            .expect("test should run from a Git worktree");
        let repo_root = fs::canonicalize(repo_root).expect("canonical worktree path");
        let output = repo_root.join("docs").join(".r3-4-output-must-not-exist");
        assert_eq!(
            resolve_new_output(output.to_str().expect("UTF-8 test path"), &repo_root, &[])
                .unwrap_err(),
            "output directory must be outside the repository and source data roots"
        );
    }
}

