//! The 11 `/internal/v1` operations. `tests/pins.rs` checks the table against `contract.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Idem {
    /// Header mandatory (refused client-side when absent).
    Required,
    Optional,
    None,
}

#[derive(Debug, Clone, Copy)]
pub struct Route {
    pub id: &'static str,
    pub method: &'static str,
    /// Template relative to `BASE_PATH`; `{x}` segments are filled from `path_params`.
    pub path: &'static str,
    pub purpose: &'static str,
    pub tenant_required: bool,
    pub idem: Idem,
}

const fn r(id: &'static str, method: &'static str, path: &'static str, purpose: &'static str, tenant: bool, idem: Idem) -> Route {
    Route { id, method, path, purpose, tenant_required: tenant, idem }
}

pub const AUTHORING_DRY_RUN: Route = r("authoring_dry_run", "POST", "/core-authoring/dry-run", "authoring_dry_run", true, Idem::None);
pub const ISSUE_CREDENTIAL: Route = r("issue_credential", "POST", "/core-credentials/issue", "credential_issue", true, Idem::None);
pub const READ_ALIAS: Route = r("read_alias", "GET", "/core-state/aliases/{agent_id}/{alias}", "alias_read", true, Idem::None);
pub const INVOKE: Route = r("invoke", "POST", "/core-tasks/invoke", "core_task_invoke", true, Idem::Required);
pub const READ_TASK: Route = r("read_task", "GET", "/core-tasks/{task_id}", "core_task_read", true, Idem::None);
pub const ADMIT_EVALUATION: Route = r("admit_evaluation", "POST", "/evaluation/admissions", "evaluation_admit", true, Idem::Optional);
pub const READ_ARM_BY_KEY: Route = r("read_arm", "GET", "/evaluation/arms/by-key/{key}", "evaluation_arm_read", true, Idem::None);
pub const RUN_ARM: Route = r("run_arm", "POST", "/evaluation/arms/run", "evaluation_arm_run", true, Idem::Required);
pub const READ_ARM: Route = r("read_arm", "GET", "/evaluation/arms/{arm_id}", "evaluation_arm_read", true, Idem::None);
pub const RUN_ARM_BY_ID: Route = r("run_arm", "POST", "/evaluation/arms/{arm_id}/run", "evaluation_arm_run", true, Idem::Required);
pub const VERSION: Route = r("version", "GET", "/version", "version_probe", false, Idem::None);

pub const ALL: &[&Route] = &[
    &AUTHORING_DRY_RUN,
    &ISSUE_CREDENTIAL,
    &READ_ALIAS,
    &INVOKE,
    &READ_TASK,
    &ADMIT_EVALUATION,
    &READ_ARM_BY_KEY,
    &RUN_ARM,
    &READ_ARM,
    &RUN_ARM_BY_ID,
    &VERSION,
];
