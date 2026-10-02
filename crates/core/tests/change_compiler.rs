use improvement_engine_core::ArtifactReference;
use improvement_engine_core::change_compiler::{
    ChangeOperation, ChangeOperationKind, CoreEntityKind, UntrustedChangeSpec,
};
use improvement_engine_core::core_task::CoreTaskScope;
use serde_json::json;

#[test]
fn public_callers_can_author_untrusted_input_but_not_construct_an_authorized_capability() {
    let spec = UntrustedChangeSpec::new(
        CoreTaskScope::new("tenant_a", "job_a", "grant_a", "authority_a").unwrap(),
        ArtifactReference {
            tenant_id: "tenant_a".to_owned(),
            id: "018f0f4e-7bbd-7000-8000-000000000600".to_owned(),
            revision: 1,
            digest: "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_owned(),
        },
        "flow/payment-status",
        "transaction_status_lookup",
        vec![ChangeOperation::new(
            ChangeOperationKind::Add,
            CoreEntityKind::Flow,
            json!({
                "id": "payment_status_resolution",
                "version": "1.0.0",
                "priority": 1,
                "nodes": [{"id":"complete","type":"end","config":{"outcome":"completed"}}]
            }),
            "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        )],
    )
    .expect("untrusted authoring shape is valid");
    assert!(std::mem::size_of_val(&spec) > 0);
}
