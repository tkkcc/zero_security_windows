use crate::{native, registry as reg};
use anyhow::{Context, Result, ensure};
use std::ffi::c_void;
#[cfg(test)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetProcessMitigationPolicy(
        process: native::Handle,
        policy: u32,
        data: *mut c_void,
        size: usize,
    ) -> i32;
}
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
    (16, 1),
];
fn system_policy(id: u32) -> bool {
    matches!(id, 0 | 1 | 7 | 13 | 14)
}
fn audit_policy(id: u32) -> bool {
    matches!(id, 2 | 4 | 8 | 9 | 10 | 11 | 12 | 13 | 15 | 16)
}
fn windows_image(image: &str) -> bool {
    image.to_lowercase().starts_with(&format!(
        "{}\\",
        native::expand("%SystemRoot%").to_lowercase()
    ))
}
fn targets(include_running: bool) -> Result<Vec<String>> {
    let mut targets = vec![
        String::new(),
        "dism.exe".into(),
        "DismHost.exe".into(),
        "bcdedit.exe".into(),
        "powercfg.exe".into(),
    ];
    if include_running {
        targets.extend(
            native::process_images()?
                .into_iter()
                .filter(|(_, image)| windows_image(image))
                .map(|(_, image)| {
                    std::path::Path::new(&image)
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .to_lowercase()
                }),
        );
    }
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
    for image in targets(false)? {
        for &(id, len) in POLICIES {
            if image.is_empty() && !system_policy(id) {
                continue;
            }
            for flags in [0, 8] {
                if flags == 8 && (image.is_empty() || !audit_policy(id)) {
                    continue;
                }
                if read(&image, id, flags, len)?
                    .is_none_or(|data| data[..options(id, len)].iter().any(|v| v & 3 != 2))
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
    for image in targets(true)? {
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
pub fn defaults() -> Result<bool> {
    let defaults = default_images()?;
    for path in configured_paths(&defaults)? {
        let expected = defaults
            .iter()
            .find(|image| path.eq_ignore_ascii_case(&format!(r"{IFEO}\{}", image.name)));
        if reg::read(&path, "MitigationOptions")?
            != expected.map_or(serde_json::Value::Null, |image| image.value.clone())
            || !reg::read(&path, "MitigationAuditOptions")?.is_null()
        {
            return Ok(false);
        }
    }
    Ok(true)
}
pub fn restore() -> Result<()> {
    let _system = native::trusted_installer()?;
    let images = default_images()?;
    for path in configured_paths(&images)? {
        for name in ["MitigationOptions", "MitigationAuditOptions"] {
            if !reg::read(&path, name)?.is_null() {
                reg::delete(&path, name).with_context(|| format!("Clear {path}/{name}"))?;
            }
        }
    }
    for image in images {
        reg::set(
            &format!(r"{IFEO}\{}", image.name),
            "MitigationOptions",
            image.value,
            &image.kind,
        )?;
    }
    ensure!(
        defaults()?,
        "Mitigation policies did not return to defaults"
    );
    Ok(())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DefaultImage {
    name: String,
    value: serde_json::Value,
    #[serde(rename = "Type")]
    kind: String,
}
fn default_images() -> Result<Vec<DefaultImage>> {
    #[derive(serde::Deserialize)]
    struct Defaults {
        #[serde(rename = "Mitigations")]
        mitigations: Vec<DefaultImage>,
    }
    Ok(
        serde_json::from_str::<Defaults>(include_str!("../resources/windows-defaults.json"))?
            .mitigations,
    )
}
fn configured_paths(defaults: &[DefaultImage]) -> Result<Vec<String>> {
    let mut paths = vec![r"HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\kernel".into()];
    paths.extend(
        defaults
            .iter()
            .map(|image| format!(r"{IFEO}\{}", image.name)),
    );
    for name in reg::children(IFEO)? {
        let path = format!(r"{IFEO}\{name}");
        paths.push(path.clone());
        paths.extend(
            reg::children(&path)?
                .iter()
                .map(|child| format!(r"{path}\{child}")),
        );
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::process::CommandExt;
    fn current_flags(policy: u32) -> u32 {
        let mut flags = 0u32;
        assert_ne!(
            unsafe {
                GetProcessMitigationPolicy(
                    native::GetCurrentProcess(),
                    policy,
                    (&mut flags as *mut u32).cast(),
                    4,
                )
            },
            0
        );
        flags
    }
    #[test]
    #[ignore = "仅由隔离可执行文件启动验证测试调用"]
    fn child_uses_disabled_startup_policies() {
        for (id, mask) in [(1, 7), (3, 3), (7, 1), (15, 5), (16, 1)] {
            assert_eq!(current_flags(id) & mask, 0, "Runtime policy {id}");
        }
    }
    #[test]
    fn isolated_image_policy_persists_without_changing_system_defaults() -> Result<()> {
        let before = read("", 1, 0, 3)?;
        let parent_before = [
            current_flags(1),
            current_flags(3),
            current_flags(15),
            current_flags(16),
        ];
        let image = format!("zero-security-test-{}.exe", std::process::id());
        let executable = std::env::temp_dir().join(&image);
        let key = format!(r"{IFEO}\{image}");
        ensure!(!reg::exists(&key)?, "Test key already exists");
        let result = (|| {
            std::fs::copy(std::env::current_exe()?, &executable)?;
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
            assert_eq!(
                [
                    current_flags(1),
                    current_flags(3),
                    current_flags(15),
                    current_flags(16)
                ],
                parent_before
            );
            let output = std::process::Command::new(&executable)
                .args([
                    "--exact",
                    "mitigation::tests::child_uses_disabled_startup_policies",
                    "--ignored",
                    "--test-threads=1",
                ])
                .creation_flags(0x08000000)
                .output()?;
            ensure!(
                output.status.success(),
                "Child policy verification: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            Ok(())
        })();
        reg::key(&key, false)?;
        if executable.exists() {
            std::fs::remove_file(executable)?;
        }
        result
    }
}
