use crate::{
    model::{Operation, ResultRecord},
    native,
    workflow::Pending,
};
use anyhow::Result;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;
use std::{collections::HashMap, fs, path::PathBuf, sync::Mutex};

pub fn root() -> PathBuf {
    PathBuf::from(std::env::var_os("ProgramData").unwrap()).join("ZeroSecurityWindows")
}
pub fn user_root() -> PathBuf {
    PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap()).join("ZeroSecurityWindows")
}
pub fn log_path() -> PathBuf {
    root().join("operations.jsonl")
}
pub fn append(name: &str, text: &str) -> Result<()> {
    use std::io::Write;
    static LOGS: Mutex<()> = Mutex::new(());
    let _writing = LOGS.lock().unwrap();
    fs::create_dir_all(root())?;
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root().join(name))?
        .write_all(format!("{text}\n").as_bytes())?;
    Ok(())
}
pub fn read<T: DeserializeOwned>(name: &str) -> Result<Option<T>> {
    let file = root().join(name);
    if !file.exists() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_slice(&fs::read(file)?)?))
}
pub fn write<T: Serialize>(name: &str, data: &T) -> Result<()> {
    fs::create_dir_all(root())?;
    fs::write(root().join(name), serde_json::to_vec_pretty(data)?)?;
    Ok(())
}
pub fn remove(name: &str) -> Result<()> {
    let p = root().join(name);
    if p.exists() {
        fs::remove_file(p)?
    }
    Ok(())
}
pub struct Store {
    saving: Mutex<()>,
    pub sid: String,
    pub results: Mutex<HashMap<String, ResultRecord>>,
    pub pending: Mutex<Option<Pending>>,
    pub suspended: Mutex<bool>,
}
impl Store {
    pub fn new() -> Result<Self> {
        let pending: Option<Pending> = read("rust-workflow.json")?;
        let suspended = pending.as_ref().is_some_and(|p| p.suspended);
        Ok(Self {
            saving: Mutex::new(()),
            sid: native::sid()?,
            results: Mutex::new(read("rust-results.json")?.unwrap_or_default()),
            pending: Mutex::new(pending),
            suspended: Mutex::new(suspended),
        })
    }
    pub fn initialize(&self) -> Result<()> {
        fs::create_dir_all(root())?;
        native::privilege("SeBackupPrivilege")?;
        native::privilege("SeRestorePrivilege")?;
        Ok(())
    }
    pub fn save(&self) -> Result<()> {
        let _saving = self.saving.lock().unwrap();
        write("rust-results.json", &*self.results.lock().unwrap())?;
        let pending = self.pending.lock().unwrap();
        if let Some(p) = pending.as_ref() {
            write("rust-workflow.json", p)?
        } else {
            remove("rust-workflow.json")?
        }
        Ok(())
    }
    pub fn log(&self, id: &str, op: &Operation, error: &str) -> Result<()> {
        append("operations.jsonl",&json!({"Time":chrono::Local::now().to_rfc3339(),"Id":id,"Kind":op.kind,"Path":op.path,"Name":op.name,"Error":error}).to_string())
    }
    pub fn copy_executable(&self) -> Result<PathBuf> {
        let _saving = self.saving.lock().unwrap();
        copy_to(root())
    }
}
pub fn copy_to(folder: PathBuf) -> Result<PathBuf> {
    fs::create_dir_all(&folder)?;
    let target = folder.join("zero_security_windows.exe");
    let current = std::env::current_exe()?;
    if !current
        .to_string_lossy()
        .eq_ignore_ascii_case(&target.to_string_lossy())
    {
        fs::copy(current, &target)?;
    }
    Ok(target)
}
