//! The terminal `complete` transition and keyed (idempotent) admission of the job port.
//! Before this a job whose runner returned Ok stayed `leased` and was reclaimed after its lease lapsed (double execution).
use pg::repo::{JobRepository, MemRepo, RepoError};

const T: &str = "tenant-a";

#[test]
fn a_completed_job_is_never_claimed_again_even_after_the_lease_would_have_lapsed() {
    let r = MemRepo::new();
    let id = r.admit(T).unwrap();
    let c = r.claim_next(T, "w1", 100, 30).unwrap().unwrap();
    r.commit_output(T, &id, 0, "w1", c.fence_token, 101, "done").unwrap();
    r.complete(T, &id, "w1", c.fence_token, 102).unwrap();
    for now in [103, 130, 10_000] {
        assert_eq!(r.claim_next(T, "w2", now, 30).unwrap(), None, "complete job claimed at {now}");
    }
    assert_eq!(r.output(T, &id, 0).unwrap().as_deref(), Some("done"), "complete keeps the outputs");
}

#[test]
fn only_the_current_unexpired_holder_can_complete() {
    let r = MemRepo::new();
    let id = r.admit(T).unwrap();
    let c1 = r.claim_next(T, "w1", 100, 30).unwrap().unwrap();
    assert_eq!(r.complete(T, &id, "w2", c1.fence_token, 101), Err(RepoError::StaleFence), "wrong worker");
    assert_eq!(r.complete(T, &id, "w1", c1.fence_token + 1, 101), Err(RepoError::StaleFence), "wrong fence");
    assert_eq!(r.complete("tenant-b", &id, "w1", c1.fence_token, 101), Err(RepoError::StaleFence), "other tenant");
    assert_eq!(r.complete(T, &id, "w1", c1.fence_token, 130), Err(RepoError::StaleFence), "expired lease");
    // the refused attempts changed nothing: the job is still reclaimable after expiry, and the new holder can complete
    let c2 = r.claim_next(T, "w2", 130, 30).unwrap().expect("not complete: reclaimable");
    assert_eq!(r.complete(T, &id, "w1", c1.fence_token, 131), Err(RepoError::StaleFence), "superseded holder");
    r.complete(T, &id, "w2", c2.fence_token, 131).unwrap();
    assert_eq!(r.complete(T, &id, "w2", c2.fence_token, 132), Err(RepoError::StaleFence), "already complete");
}

#[test]
fn keyed_admission_is_idempotent_and_the_key_is_readable() {
    let r = MemRepo::new();
    let a = r.admit_keyed(T, "monitor:mon-0123456789abcdef").unwrap();
    let again = r.admit_keyed(T, "monitor:mon-0123456789abcdef").unwrap();
    assert_eq!(a, again, "same key, same job");
    let other = r.admit_keyed(T, "monitor:mon-fedcba9876543210").unwrap();
    assert_ne!(a, other);
    let foreign = r.admit_keyed("tenant-b", "monitor:mon-0123456789abcdef").unwrap();
    assert_ne!(a, foreign, "keys are per tenant");
    assert_eq!(r.job_key(T, &a).unwrap().as_deref(), Some("monitor:mon-0123456789abcdef"));
    assert_eq!(r.job_key("tenant-b", &a).unwrap(), None, "a job id of another tenant is not readable");
    assert_eq!(r.admit(T).map(|id| r.job_key(T, &id).unwrap()), Ok(None), "an unkeyed job has no key");
    // exactly two keyed jobs exist for tenant-a: the duplicate admission did not queue a second one
    let (x, y) = (r.claim_next(T, "w", 1, 30).unwrap().unwrap(), r.claim_next(T, "w", 1, 30).unwrap().unwrap());
    assert_eq!((x.job, y.job), (a, other));
}

#[test]
fn keyed_admission_refuses_unusable_keys() {
    let r = MemRepo::new();
    for bad in ["", "a\nb", &"k".repeat(257)] {
        assert!(matches!(r.admit_keyed(T, bad), Err(RepoError::InvalidId(_))), "{bad:?}");
    }
}
