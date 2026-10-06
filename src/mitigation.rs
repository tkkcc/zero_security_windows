use crate::{native, registry as reg};
use anyhow::{Result, ensure};
use std::ffi::c_void;
#[link(name = "ntdll")]
unsafe extern "system" {
    fn RtlQueryImageMitigationPolicy(
        image: *const u16,
        policy: u32,
        flags: u32,
        buffer: *mut c_void,
        size: u32,
    ) -> i32;
    fn RtlSetImageMitigationPolicy(
        image: *const u16,
        policy: u32,
        flags: u32,
        buffer: *const c_void,
        size: u32,
    ) -> i32;
}
const IFEO: &str =
    r"HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options";
// RTL_IMAGE_MITIGATION_*_POLICY: one u64 per policy option; payload also has WCHAR[512].
const POLICIES: &[(u32, usize)] = &[
    (0, 1),
    (1, 3),
    (2, 1),
    (3, 1),
    (4, 2),
    (6, 1),
    (7, 2),
    (8, 2),
    (9, 1),
    (10, 3),
    (11, 134),
    (12, 1),
    (13, 1),
    (14, 1),
    (15, 3),
];
fn system_policy(id: u32) -> bool {
    matches!(id, 0 | 1 | 7 | 13 | 14)
}
fn audit_policy(id: u32) -> bool {
    matches!(id, 2 | 4 | 8 | 9 | 10 | 11 | 12 | 13 | 15)
}
fn targets() -> Result<Vec<String>> {
    let mut targets = vec![String::new()];
    for name in reg::children(IFEO)? {
        let path = format!(r"{IFEO}\{name}");
        if !reg::read(&path, "MitigationOptions")?.is_null()
            || !reg::read(&path, "MitigationAuditOptions")?.is_null()
        {
            targets.push(name.clone())
        }
        for child in reg::children(&path)? {
            let path = format!(r"{path}\{child}");
            if (!reg::read(&path, "MitigationOptions")?.is_null()
                || !reg::read(&path, "MitigationAuditOptions")?.is_null())
                && let Some(filter) = reg::read(&path, "FilterFullPath")?.as_str()
            {
                targets.push(filter.to_owned())
            }
        }
    }
    targets.sort();
    targets.dedup();
    Ok(targets)
}
fn options(id: u32, len: usize) -> usize {
    if id == 11 { 6 } else { len }
}
fn read(image: &str, id: u32, flags: u32, len: usize) -> Result<Option<Vec<u64>>> {
    let wide = native::wide(image);
    let p = if image.is_empty() {
        std::ptr::null()
    } else {
        wide.as_ptr()
    };
    let mut data = vec![0u64; len];
    let hr = unsafe {
        RtlQueryImageMitigationPolicy(p, id, flags, data.as_mut_ptr().cast(), (len * 8) as u32)
    };
    if hr as u32 == 0xc0000034 {
        return Ok(None);
    }
    ensure!(hr >= 0, "Mitigation query {image}/{id}: {hr:08X}");
    Ok(Some(data))
}
pub fn active() -> Result<bool> {
    for image in targets()? {
        for &(id, len) in POLICIES {
            if image.is_empty() && !system_policy(id) {
                continue;
            }
            for flags in [0, 8] {
                if flags == 8 && (image.is_empty() || !audit_policy(id)) {
                    continue;
                }
                if let Some(data) = read(&image, id, flags, len)?
                    && data[..options(id, len)].iter().any(|v| v & 3 != 2)
                {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}
pub fn disable() -> Result<()> {
    let _system = native::system()?;
    for image in targets()? {
        let wide = native::wide(&image);
        let p = if image.is_empty() {
            std::ptr::null()
        } else {
            wide.as_ptr()
        };
        for &(id, len) in POLICIES {
            if image.is_empty() && !system_policy(id) {
                continue;
            }
            for flags in [0, 8] {
                if flags == 8 && (image.is_empty() || !audit_policy(id)) {
                    continue;
                }
                let mut data = vec![0u64; len];
                data[..options(id, len)].fill(2);
                let hr = unsafe {
                    RtlSetImageMitigationPolicy(
                        p,
                        id,
                        flags,
                        data.as_ptr().cast(),
                        (len * 8) as u32,
                    )
                };
                ensure!(hr >= 0, "Mitigation write {image}/{id}: {hr:08X}");
            }
        }
    }
    ensure!(
        !active()?,
        "Mitigation policies did not match after writing"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn isolated_image_policy_persists_without_changing_system_defaults() -> Result<()> {
        let before = read("", 1, 0, 3)?;
        let image = format!("zero-security-test-{}.exe", std::process::id());
        let key = format!(r"{IFEO}\{image}");
        ensure!(!reg::exists(&key)?, "Test key already exists");
        let result = (|| {
            let _system = native::system()?;
            for &(id, len) in POLICIES {
                for flags in [0, 8] {
                    if flags == 8 && !audit_policy(id) {
                        continue;
                    }
                    let mut data = vec![0u64; len];
                    data[..options(id, len)].fill(2);
                    let hr = unsafe {
                        RtlSetImageMitigationPolicy(
                            native::wide(&image).as_ptr(),
                            id,
                            flags,
                            data.as_ptr().cast(),
                            (len * 8) as u32,
                        )
                    };
                    ensure!(hr >= 0, "Test policy {id}/{flags}: {hr:08X}");
                    let actual = read(&image, id, flags, len)?.unwrap();
                    assert!(
                        actual[..options(id, len)].iter().all(|v| v & 3 == 2),
                        "Policy {id}/{flags}: {actual:?}"
                    );
                }
            }
            assert_eq!(read("", 1, 0, 3)?, before);
            Ok(())
        })();
        reg::key(&key, false)?;
        result
    }
}
