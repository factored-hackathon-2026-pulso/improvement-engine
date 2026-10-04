//! Job store port plus a file-backed implementation (std only).
//! CAS is serialised across threads and processes by an exclusive lock file (create_new);
//! records are written to a unique temp file, fsynced, then renamed over the key file.
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub trait JobStore {
    /// (version, value); version 0 never exists.
    fn get(&self, key: &str) -> Result<Option<(u64, String)>, String>;
    /// Compare-and-set: `expected` 0 = key must be absent. Returns the new version.
    fn cas(&self, key: &str, expected: u64, value: &str) -> Result<u64, String>;
    /// Create `key` (must be absent) iff the `lease` record currently names `guard` as an unexpired holder.
    /// Stores that can make check-and-write ONE atomic step (a transaction, the file lock) must override this;
    /// the default is read-then-write and leaves a window between the two.
    fn commit_guarded(&self, key: &str, value: &str, guard: &CommitGuard) -> Result<u64, String> {
        match self.get("lease")? {
            Some((_, l)) if guard.holds(&l) => self.cas(key, 0, value),
            _ => Err("stale fence".into()),
        }
    }
}

/// Who is committing: the worker, the fence it was issued and its clock (unix seconds).
pub struct CommitGuard<'a> {
    pub worker_id: &'a str,
    pub fence_token: u64,
    pub now: u64,
}

impl CommitGuard<'_> {
    /// True when the `lease` record (`worker|fence|attempt|expires`) is this guard's, current and unexpired.
    pub fn holds(&self, lease_record: &str) -> bool {
        let p: Vec<&str> = lease_record.split('|').collect();
        p.len() == 4
            && p[0] == self.worker_id
            && p[1].parse::<u64>() == Ok(self.fence_token)
            && p[3].parse::<u64>().is_ok_and(|exp| self.now < exp)
    }
}

pub struct FileStore {
    dir: PathBuf,
}

static SEQ: AtomicU64 = AtomicU64::new(0);

fn check_key(key: &str) -> Result<(), String> {
    let ok = !key.is_empty()
        && key.len() <= 128
        && !key.starts_with('/')
        && key.split('/').all(|seg| !seg.is_empty() && seg != "." && seg != "..")
        && key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-'));
    if ok { Ok(()) } else { Err(format!("invalid store key {key:?}")) }
}

impl FileStore {
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, String> {
        let dir = dir.into();
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        Ok(Self { dir })
    }
    fn path(&self, key: &str) -> PathBuf {
        // '/' -> '~' is injective because '~' is not an allowed key character.
        self.dir.join(format!("kv_{}", key.replace('/', "~")))
    }
    fn lock(&self) -> Result<Lock, String> {
        let p = self.dir.join("cas.lock");
        let start = Instant::now();
        loop {
            match fs::OpenOptions::new().write(true).create_new(true).open(&p) {
                Ok(_) => return Ok(Lock(p)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // a lock older than 10s belongs to a killed process: break it
                    let stale = fs::metadata(&p)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .is_some_and(|d| d > Duration::from_secs(10));
                    if stale {
                        let _ = fs::remove_file(&p);
                    } else if start.elapsed() > Duration::from_secs(30) {
                        return Err("cas lock timeout".into());
                    } else {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                }
                Err(e) => return Err(e.to_string()),
            }
        }
    }
}

struct Lock(PathBuf);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

impl JobStore for FileStore {
    fn get(&self, key: &str) -> Result<Option<(u64, String)>, String> {
        check_key(key)?;
        match fs::read_to_string(self.path(key)) {
            Ok(s) => {
                let (v, rest) = s.split_once('\n').ok_or("corrupt record")?;
                Ok(Some((v.parse().map_err(|_| "corrupt version")?, rest.to_string())))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
    fn commit_guarded(&self, key: &str, value: &str, guard: &CommitGuard) -> Result<u64, String> {
        check_key(key)?;
        let _lock = self.lock()?;
        match self.get("lease")? {
            Some((_, l)) if guard.holds(&l) => self.cas_locked(key, 0, value),
            _ => Err("stale fence".into()),
        }
    }
    fn cas(&self, key: &str, expected: u64, value: &str) -> Result<u64, String> {
        check_key(key)?;
        let _lock = self.lock()?;
        self.cas_locked(key, expected, value)
    }
}

impl FileStore {
    /// CAS body; the caller holds the lock file.
    fn cas_locked(&self, key: &str, expected: u64, value: &str) -> Result<u64, String> {
        let current = self.get(key)?.map(|(v, _)| v).unwrap_or(0);
        if current != expected {
            return Err(format!("cas conflict on {key}: have {current}, expected {expected}"));
        }
        let tmp = self.dir.join(format!("tmp_{}_{}", std::process::id(), SEQ.fetch_add(1, Ordering::SeqCst)));
        let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;
        f.write_all(format!("{}\n{}", current + 1, value).as_bytes()).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
        drop(f);
        fs::rename(&tmp, self.path(key)).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            e.to_string()
        })?;
        Ok(current + 1)
    }
}
