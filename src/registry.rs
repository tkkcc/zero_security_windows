use crate::{
    model::{Operation, text_value},
    native,
};
use anyhow::Result;
use serde_json::{Value, json};
use winreg::{RegKey, enums::*};
pub struct Watch {
    key: RegKey,
    event: windows::core::Owned<windows::Win32::Foundation::HANDLE>,
}
impl Watch {
    pub fn new(path: &str) -> Result<Self> {
        let (hive, path) = parse(path);
        let watch = Self {
            key: hive.open_subkey_with_flags(path, KEY_NOTIFY)?,
            event: unsafe {
                windows::core::Owned::new(windows::Win32::System::Threading::CreateEventW(
                    None, false, false, None,
                )?)
            },
        };
        watch.arm()?;
        Ok(watch)
    }
    fn arm(&self) -> Result<()> {
        use windows::Win32::System::Registry::*;
        unsafe {
            RegNotifyChangeKeyValue(
                HKEY(self.key.raw_handle()),
                false,
                REG_NOTIFY_CHANGE_LAST_SET,
                Some(*self.event),
                true,
            )
            .ok()?;
        }
        Ok(())
    }
    pub fn changed(&self) -> Result<bool> {
        use windows::Win32::{Foundation::*, System::Threading::WaitForSingleObject};
        match unsafe { WaitForSingleObject(*self.event, 0) } {
            WAIT_OBJECT_0 => {
                self.arm()?;
                Ok(true)
            }
            WAIT_TIMEOUT => Ok(false),
            _ => Err(std::io::Error::last_os_error().into()),
        }
    }
}
pub fn parse(path: &str) -> (RegKey, &str) {
    let (hive, path) = path.split_once('\\').unwrap_or((path, ""));
    (
        RegKey::predef(if hive == "HKLM:" {
            HKEY_LOCAL_MACHINE
        } else {
            HKEY_CURRENT_USER
        }),
        path,
    )
}
pub fn read(path: &str, name: &str) -> Result<Value> {
    let (hive, sub) = parse(path);
    let key = match hive.open_subkey(sub) {
        Ok(k) => k,
        Err(e) if e.raw_os_error() == Some(2) => return Ok(Value::Null),
        Err(e) => return Err(e.into()),
    };
    let v = match key.get_raw_value(name) {
        Ok(v) => v,
        Err(e) if e.raw_os_error() == Some(2) => return Ok(Value::Null),
        Err(e) => return Err(e.into()),
    };
    Ok(match v.vtype {
        REG_DWORD => json!(u32::from_le_bytes(v.bytes[..4].try_into()?)),
        REG_QWORD => json!(u64::from_le_bytes(v.bytes[..8].try_into()?)),
        REG_BINARY => json!(v.bytes),
        REG_MULTI_SZ => json!(
            String::from_utf16_lossy(
                &v.bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|p| u16::from_le_bytes([p[0], p[1]]))
                    .collect::<Vec<_>>()
            )
            .trim_end_matches('\0')
            .split('\0')
            .collect::<Vec<_>>()
        ),
        _ => json!(
            String::from_utf16_lossy(
                &v.bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|p| u16::from_le_bytes([p[0], p[1]]))
                    .collect::<Vec<_>>()
            )
            .trim_end_matches('\0')
        ),
    })
}
pub fn exists(path: &str) -> Result<bool> {
    let (h, p) = parse(path);
    match h.open_subkey(p) {
        Ok(_) => Ok(true),
        Err(e) if e.raw_os_error() == Some(2) => Ok(false),
        Err(e) => Err(e.into()),
    }
}
pub fn children(path: &str) -> Result<Vec<String>> {
    let (h, p) = parse(path);
    match h.open_subkey(p) {
        Ok(k) => Ok(k.enum_keys().collect::<std::io::Result<Vec<_>>>()?),
        Err(e) if e.raw_os_error() == Some(2) => Ok(vec![]),
        Err(e) => Err(e.into()),
    }
}
pub fn names(path: &str) -> Result<Vec<String>> {
    let (h, p) = parse(path);
    match h.open_subkey(p) {
        Ok(k) => Ok(k
            .enum_values()
            .map(|r| r.map(|(n, _)| n))
            .collect::<std::io::Result<Vec<_>>>()?),
        Err(e) if e.raw_os_error() == Some(2) => Ok(vec![]),
        Err(e) => Err(e.into()),
    }
}
pub fn desired(op: &Operation) -> Value {
    if let Some(s) = op.value.as_str() {
        json!(native::expand(s))
    } else {
        op.value.clone()
    }
}
pub fn write(op: &Operation) -> Result<()> {
    if op.kind == "RegistryDelete" {
        return delete(&op.path, &op.name);
    }
    let (_, sub) = parse(&op.path);
    let v = desired(op);
    let (kind, bytes) = match op.value_type.as_str() {
        "String" | "ExpandString" => (
            if op.value_type == "String" { 1 } else { 2 },
            native::wide(&text_value(&v))
                .iter()
                .flat_map(|n| n.to_le_bytes())
                .collect::<Vec<_>>(),
        ),
        "Binary" => (
            3,
            v.as_array()
                .ok_or_else(|| anyhow::anyhow!("Binary requires an array"))?
                .iter()
                .map(|n| {
                    n.as_u64()
                        .map(|n| n as u8)
                        .ok_or_else(|| anyhow::anyhow!("Invalid binary"))
                })
                .collect::<Result<Vec<_>>>()?,
        ),
        "MultiString" => (
            7,
            native::wide(
                &(v.as_array()
                    .ok_or_else(|| anyhow::anyhow!("MultiString requires an array"))?
                    .iter()
                    .map(text_value)
                    .collect::<Vec<_>>()
                    .join("\0")
                    + "\0"),
            )
            .iter()
            .flat_map(|n| n.to_le_bytes())
            .collect(),
        ),
        "QWord" => (
            11,
            v.as_u64()
                .ok_or_else(|| anyhow::anyhow!("Invalid QWord"))?
                .to_le_bytes()
                .to_vec(),
        ),
        _ => (
            4,
            v.as_u64()
                .ok_or_else(|| anyhow::anyhow!("Invalid DWord"))?
                .to_le_bytes()[..4]
                .to_vec(),
        ),
    };
    native::reg_write(
        if op.path.starts_with("HKLM:") {
            0x80000002
        } else {
            0x80000001
        },
        sub,
        &op.name,
        kind,
        &bytes,
        false,
    )
}
pub fn set(path: &str, name: &str, value: impl Into<Value>, kind: &str) -> Result<()> {
    write(&Operation {
        value_type: kind.into(),
        ..Operation::reg(path, name, value)
    })
}
pub fn delete(path: &str, name: &str) -> Result<()> {
    let (_, sub) = parse(path);
    native::reg_write(
        if path.starts_with("HKLM:") {
            0x80000002
        } else {
            0x80000001
        },
        sub,
        name,
        0,
        &[],
        true,
    )
}
pub fn key(path: &str, enabled: bool) -> Result<()> {
    let (h, p) = parse(path);
    if enabled {
        set(path, "__zsw_init", 0, "DWord")?;
        delete(path, "__zsw_init")
    } else {
        match h.delete_subkey_all(p) {
            Ok(()) => Ok(()),
            Err(e) if e.raw_os_error() == Some(2) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}
pub fn number(path: &str, name: &str, default: u64) -> Result<u64> {
    let v = read(path, name)?;
    Ok(v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(default))
}
