//! Job store port plus a file-backed in-process implementation (std only).
use std::fs;
use std::io::Write;
use std::path::PathBuf;

pub trait JobStore {
    /// (version, value); version 0 never exists.
    fn get(&self, key: &str) -> Result<Option<(u64, String)>, String>;
    /// Compare-and-set: `expected` 0 = key must be absent. Returns the new version.
    fn cas(&self, key: &str, expected: u64, value: &str) -> Result<u64, String>;
    fn append_event(&self, line: &str) -> Result<(), String>;
    fn events(&self) -> Result<Vec<String>, String>;
}

pub struct FileStore {
    dir: PathBuf,
}

impl FileStore {
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, String> {
        let dir = dir.into();
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        Ok(Self { dir })
    }
    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("kv_{}", key.replace('/', "_")))
    }
}

impl JobStore for FileStore {
    fn get(&self, key: &str) -> Result<Option<(u64, String)>, String> {
        match fs::read_to_string(self.path(key)) {
            Ok(s) => {
                let (v, rest) = s.split_once('\n').ok_or("corrupt record")?;
                Ok(Some((v.parse().map_err(|_| "corrupt version")?, rest.to_string())))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
    fn cas(&self, key: &str, expected: u64, value: &str) -> Result<u64, String> {
        let current = self.get(key)?.map(|(v, _)| v).unwrap_or(0);
        if current != expected {
            return Err(format!("cas conflict on {key}: have {current}, expected {expected}"));
        }
        let tmp = self.dir.join("tmp_write");
        fs::write(&tmp, format!("{}\n{}", current + 1, value)).map_err(|e| e.to_string())?;
        fs::rename(&tmp, self.path(key)).map_err(|e| e.to_string())?;
        Ok(current + 1)
    }
    fn append_event(&self, line: &str) -> Result<(), String> {
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("events.log"))
            .map_err(|e| e.to_string())?;
        writeln!(f, "{line}").map_err(|e| e.to_string())
    }
    fn events(&self) -> Result<Vec<String>, String> {
        match fs::read_to_string(self.dir.join("events.log")) {
            Ok(s) => Ok(s.lines().map(String::from).collect()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
            Err(e) => Err(e.to_string()),
        }
    }
}
