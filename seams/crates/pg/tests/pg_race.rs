//! Database-level races: a reclaim racing a fence-checked commit, on two connections, many iterations.
//! Triggers widen the window INSIDE the transactions (random pg_sleep) and act as the oracle: when out/N is
//! inserted they record the CURRENT lease holder; a commit by a superseded fence is a recorded violation.
mod common;
use common::{migrations_dir, TempDb};
use engine::{CommitGuard, JobStore};
use pg::pgrepo::PgRepo;
use pg::pgstore::PgJobStore;
use pg::repo::{JobRepository, RepoError};
use std::sync::{Arc, Barrier};

const ITER: u64 = 90;

fn db() -> Option<TempDb> {
    let db = TempDb::create()?;
    pg::migrate::migrate(&mut db.connect(), &pg::migrate::discover(&migrations_dir()).unwrap()).unwrap();
    Some(db)
}

#[test]
fn reclaim_racing_commit_output_never_lets_a_superseded_fence_commit() {
    let Some(db) = db() else { return };
    db.connect()
        .batch_execute(
            "CREATE TABLE race_viol (note TEXT);
             CREATE FUNCTION race_claim() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
               IF NEW.lease_version > OLD.lease_version AND NEW.lease_owner = 'racer' THEN PERFORM pg_sleep(random() * 0.02); END IF;
               RETURN NEW; END $$;
             CREATE TRIGGER race_claim BEFORE UPDATE ON pulso_jobs FOR EACH ROW EXECUTE FUNCTION race_claim();
             CREATE FUNCTION race_out() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
               PERFORM pg_sleep(random() * 0.02);
               IF (SELECT lease_version FROM pulso_jobs WHERE tenant_id = NEW.tenant_id AND id = NEW.job_id) <> NEW.fence_token
               THEN INSERT INTO race_viol VALUES ('out/' || NEW.step_index || ' fence ' || NEW.fence_token || ' committed after reclaim'); END IF;
               RETURN NEW; END $$;
             CREATE TRIGGER race_out BEFORE INSERT ON pulso_job_outputs FOR EACH ROW EXECUTE FUNCTION race_out();",
        )
        .unwrap();
    let repo = Arc::new(PgRepo::new(db.config()));
    let (mut commit_won, mut reclaim_won_first, mut skipped) = (0, 0, 0);
    for i in 0..ITER {
        let job = repo.admit("t").unwrap();
        let c1 = repo.claim_next("t", "w1", 100, 30).unwrap().unwrap();
        assert_eq!(c1.fence_token, 1);
        let gate = Arc::new(Barrier::new(2));
        let a = {
            let (repo, job, gate) = (repo.clone(), job.clone(), gate.clone());
            std::thread::spawn(move || {
                gate.wait();
                std::thread::sleep(std::time::Duration::from_millis((i % 9) * 3));
                repo.commit_output("t", &job, 0, "w1", 1, 129, "P stale?")
            })
        };
        let b = {
            let (repo, gate) = (repo.clone(), gate.clone());
            std::thread::spawn(move || {
                gate.wait();
                repo.claim_next("t", "racer", 130, 30)
            })
        };
        let (ra, rb) = (a.join().unwrap(), b.join().unwrap());
        let out = repo.output("t", &job, 0).unwrap();
        match &ra {
            Ok(()) => {
                commit_won += 1;
                assert_eq!(out.as_deref(), Some("P stale?"));
            }
            Err(RepoError::StaleFence) => {
                reclaim_won_first += 1;
                assert_eq!(out, None, "iteration {i}: rejected commit left an output");
            }
            Err(e) => panic!("iteration {i}: {e:?}"),
        }
        // B either reclaimed (fence 2) or skipped the row locked by the in-flight commit, and then reclaims
        let c2 = match rb.unwrap() {
            Some(c) => c,
            None => {
                skipped += 1;
                repo.claim_next("t", "racer", 130, 30).unwrap().expect("reclaim after the commit finished")
            }
        };
        assert_eq!((c2.fence_token, c2.attempt), (2, 2), "iteration {i}");
    }
    let viol: Vec<String> = db.connect().query("SELECT note FROM race_viol", &[]).unwrap().iter().map(|r| r.get(0)).collect();
    assert!(viol.is_empty(), "superseded fence committed: {viol:?}");
    eprintln!("repo race: commit first {commit_won}, reclaim first {reclaim_won_first}, claim skipped a locked row {skipped}");
    assert!(commit_won > 0 && reclaim_won_first > 0, "race did not exercise both orders");
}

#[test]
fn reclaim_racing_commit_guarded_never_lets_a_superseded_fence_commit() {
    let Some(db) = db() else { return };
    db.connect()
        .batch_execute(
            "CREATE TABLE race_viol (note TEXT);
             CREATE FUNCTION race_out() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
               IF NEW.key LIKE 'out/%' THEN
                 PERFORM pg_sleep(random() * 0.02);
                 IF (SELECT value FROM pulso_job_kv WHERE tenant_id = NEW.tenant_id AND job_ref = NEW.job_ref AND key = 'lease') NOT LIKE 'w1|1|%'
                 THEN INSERT INTO race_viol VALUES (NEW.job_ref || ' ' || NEW.key || ' committed after reclaim'); END IF;
               END IF;
               RETURN NEW; END $$;
             CREATE TRIGGER race_out BEFORE INSERT ON pulso_job_kv FOR EACH ROW EXECUTE FUNCTION race_out();",
        )
        .unwrap();
    let cfg = db.config();
    let (mut commit_won, mut reclaim_won) = (0, 0);
    for i in 0..ITER {
        let job = format!("job-{i}");
        let setup = PgJobStore::open(&cfg, "t", &job).unwrap();
        setup.cas("lease", 0, "w1|1|1|130").unwrap();
        let gate = Arc::new(Barrier::new(2));
        let a = {
            let (s, gate) = (PgJobStore::open(&cfg, "t", &job).unwrap(), gate.clone());
            std::thread::spawn(move || {
                gate.wait();
                std::thread::sleep(std::time::Duration::from_millis((i % 9) * 3));
                s.commit_guarded("out/0", "P x", &CommitGuard { worker_id: "w1", fence_token: 1, now: 129 })
            })
        };
        let b = {
            let (s, gate) = (PgJobStore::open(&cfg, "t", &job).unwrap(), gate.clone());
            std::thread::spawn(move || {
                gate.wait();
                s.cas("lease", 1, "w2|2|2|160")
            })
        };
        let (ra, rb) = (a.join().unwrap(), b.join().unwrap());
        assert_eq!(rb, Ok(2), "iteration {i}: the reclaim must always land (before or after the commit)");
        let out = setup.get("out/0").unwrap();
        match ra {
            Ok(_) => {
                commit_won += 1;
                assert!(out.is_some());
            }
            Err(_) => {
                reclaim_won += 1;
                assert_eq!(out, None, "iteration {i}: rejected commit left an output");
            }
        }
    }
    let viol: Vec<String> = db.connect().query("SELECT note FROM race_viol", &[]).unwrap().iter().map(|r| r.get(0)).collect();
    assert!(viol.is_empty(), "superseded fence committed: {viol:?}");
    eprintln!("store race: commit first {commit_won}, reclaim first {reclaim_won}");
    assert!(commit_won > 0 && reclaim_won > 0, "race did not exercise both orders");
}
