use crate::{model::Operation, native, registry as reg};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use windows::{Management::Deployment::*, core::HSTRING};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppEntry {
    pub name: String,
    pub full: String,
    #[serde(default)]
    pub disabled: bool,
    pub path: String,
}
pub fn inventory() -> Result<Vec<AppEntry>> {
    native::com()?;
    let manager = PackageManager::new()?;
    let mut out = vec![];
    for package in manager.FindPackages()? {
        // An orphaned pending-removal registration has no installed app files.
        if !std::path::PathBuf::from(package.InstalledPath()?.to_string()).try_exists()? {
            continue;
        }
        let id = package.Id()?;
        let mut users = vec![];
        for user in manager.FindUsers(&id.FullName()?)? {
            if user.InstallState()? == PackageInstallState::Installed {
                users.push(user.UserSecurityId()?.to_string())
            }
        }
        if !users.is_empty() {
            out.push(AppEntry {
                name: id.Name()?.to_string(),
                full: id.FullName()?.to_string(),
                disabled: package.Status()?.Disabled()?,
                path: package.InstalledPath()?.to_string(),
            });
        }
    }
    for package in manager.FindProvisionedPackages()? {
        let id = package.Id()?;
        out.push(AppEntry {
            name: id.Name()?.to_string(),
            full: id.FullName()?.to_string(),
            disabled: package.Status()?.Disabled()?,
            path: package.InstalledPath()?.to_string(),
        });
    }
    Ok(out)
}
pub fn matching<'a>(entries: &'a [AppEntry], op: &Operation) -> Vec<&'a AppEntry> {
    entries
        .iter()
        .filter(|a| {
            op.patterns
                .iter()
                .any(|p| crate::engine::matches(&a.name, p))
        })
        .collect()
}
pub fn set_enabled(op: &Operation) -> Result<bool> {
    native::com()?;
    let entries = inventory()?;
    let targets = matching(&entries, op);
    let manager = PackageManager::new()?;
    let _guard = native::system()?;
    let mut seen = std::collections::HashSet::new();
    for a in &targets {
        if !seen.insert(&a.full) {
            continue;
        }
        if crate::model::bool_value(&op.value) {
            manager.ClearPackageStatus(&HSTRING::from(&a.full), PackageStatus::Disabled)?;
        } else {
            manager.SetPackageStatus(&HSTRING::from(&a.full), PackageStatus::Disabled)?;
            for (_, image) in native::process_images()? {
                if image
                    .to_lowercase()
                    .starts_with(&format!("{}\\", a.path.to_lowercase()))
                {
                    native::stop_process(
                        std::path::Path::new(&image)
                            .file_name()
                            .unwrap()
                            .to_str()
                            .unwrap(),
                    )?;
                }
            }
        }
    }
    if crate::model::bool_value(&op.value) && targets.is_empty() {
        if op.source == "Optional" {
            return Ok(false);
        }
        if op.names.is_empty() {
            drop(_guard);
            return restore_local(op, &manager);
        }
        drop(_guard);
        for id in &op.names {
            install(
                &Operation {
                    name: id.clone(),
                    source: "msstore".into(),
                    ..Default::default()
                },
                false,
            )?;
        }
    }
    Ok(false)
}
fn restore_local(op: &Operation, manager: &PackageManager) -> Result<bool> {
    use std::path::PathBuf;
    let mut manifests = vec![];
    for root in [
        native::expand(r"%ProgramFiles%\WindowsApps"),
        native::expand(r"%SystemRoot%\SystemApps"),
    ] {
        let root = PathBuf::from(root);
        if !root.exists() {
            continue;
        }
        for entry in std::fs::read_dir(root)? {
            let path = entry?.path();
            let name = path.file_name().unwrap().to_string_lossy();
            if op
                .patterns
                .iter()
                .any(|p| crate::engine::matches(name.split('_').next().unwrap(), p))
            {
                let manifest = path.join("AppxManifest.xml");
                if manifest.exists() {
                    manifests.push(manifest);
                }
            }
        }
    }
    if op.name == "security-ui" {
        let root = PathBuf::from(native::expand(r"%SystemRoot%\System32\SecurityHealth"));
        if root.exists() {
            for entry in std::fs::read_dir(root)? {
                let package = entry?
                    .path()
                    .join("Microsoft.SecHealthUI_8wekyb3d8bbwe.appx");
                if package.exists() {
                    let uri = windows::Foundation::Uri::CreateUri(&HSTRING::from(format!(
                        "file:///{}",
                        package.to_string_lossy().replace('\\', "/")
                    )))?;
                    let result = manager
                        .AddPackageAsync(&uri, None, DeploymentOptions::ForceApplicationShutdown)?
                        .join()?;
                    ensure!(
                        result.ExtendedErrorCode()?.is_ok(),
                        "{}",
                        result.ErrorText()?
                    );
                    return Ok(false);
                }
            }
        }
    }
    ensure!(
        !manifests.is_empty(),
        "Windows did not retain installation files for {}; this component must be reinstalled through Windows or its publisher",
        op.name
    );
    // Register one current package, avoiding an attempt to downgrade newer files.
    manifests.sort();
    let manifest = manifests.last().unwrap();
    let uri = windows::Foundation::Uri::CreateUri(&HSTRING::from(format!(
        "file:///{}",
        manifest.to_string_lossy().replace('\\', "/")
    )))?;
    let result = manager
        .RegisterPackageAsync(&uri, None, DeploymentOptions::None)?
        .join()?;
    ensure!(
        result.ExtendedErrorCode()?.is_ok(),
        "{}",
        result.ErrorText()?
    );
    Ok(false)
}

pub fn winget() -> Result<String> {
    let path = std::path::PathBuf::from(std::env::var("LOCALAPPDATA")?)
        .join(r"Microsoft\WindowsApps\winget.exe");
    ensure!(
        path.exists(),
        "Install or update Windows App Installer first."
    );
    Ok(path.to_string_lossy().into())
}
pub fn tools() -> Result<serde_json::Value> {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let file = std::env::temp_dir().join(format!(
        "zsw-tools-{}-{}.json",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let result = (|| {
        native::command(
            &winget()?,
            &[
                "export",
                "--output",
                file.to_str().unwrap(),
                "--include-versions",
                "--disable-interactivity",
                "--accept-source-agreements",
            ],
        )?;
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&file)?)?;
        Ok(serde_json::json!(
            v["Sources"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("Invalid winget inventory"))?
                .iter()
                .flat_map(|s| s["Packages"].as_array().into_iter().flatten())
                .filter_map(|p| p["PackageIdentifier"].as_str())
                .collect::<Vec<_>>()
        ))
    })();
    let _ = std::fs::remove_file(file);
    result
}
pub fn has_tool(id: &str) -> Result<bool> {
    let (c, t) = native::run(
        &winget()?,
        &[
            "list",
            "--id",
            id,
            "--exact",
            "--disable-interactivity",
            "--accept-source-agreements",
        ],
    )?;
    match c {
        0 => Ok(true),
        -1978335212 => Ok(false),
        _ => anyhow::bail!("winget ({c}): {t}"),
    }
}
pub fn registered_tool(op: &Operation) -> Result<Option<bool>> {
    if !op.package_family.is_empty() {
        native::com()?;
        let manager = PackageManager::new()?;
        for package in manager.FindPackagesByUserSecurityIdPackageFamilyName(
            &HSTRING::new(),
            &HSTRING::from(&op.package_family),
        )? {
            if std::path::PathBuf::from(package.InstalledPath()?.to_string()).try_exists()? {
                return Ok(Some(true));
            }
        }
        return Ok(Some(false));
    }
    if !op.uninstall_key.is_empty() {
        for hive in ["HKCU:", "HKLM:"] {
            if reg::exists(&format!(
                r"{hive}\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\{}",
                op.uninstall_key
            ))? {
                return Ok(Some(true));
            }
        }
        return Ok(Some(false));
    }
    Ok(None)
}
pub fn install(op: &Operation, uninstall: bool) -> Result<bool> {
    if uninstall || op.source != "msstore" {
        return install_run(op, uninstall);
    }
    let temporary = native::service_start("DoSvc", None)? == 4;
    if temporary {
        native::service_start("DoSvc", Some(3))?;
    }
    let result = install_run(op, uninstall);
    if temporary {
        let restart = native::disable_service("DoSvc")?;
        reg::set(
            r"HKLM:\SYSTEM\CurrentControlSet\Services\DoSvc",
            "Start",
            4,
            "DWord",
        )?;
        ensure!(
            !restart,
            "Delivery Optimization could not return to its disabled state"
        );
    }
    result
}
fn install_run(op: &Operation, uninstall: bool) -> Result<bool> {
    let mut args = vec![
        if uninstall { "uninstall" } else { "install" },
        "--id",
        &op.name,
        "--exact",
        "--silent",
        "--disable-interactivity",
        "--accept-source-agreements",
    ];
    if !uninstall {
        args.extend([
            "--source",
            &op.source,
            "--accept-package-agreements",
            "--no-upgrade",
        ])
    }
    let (c, t) = native::run(&winget()?, &args)?;
    crate::store::append("install.log", &format!("[{}]\n{t}", op.name))?;
    let already_installed = !uninstall && matches!(c as u32, 0x8a15002b | 0x8a15010d);
    ensure!(
        matches!(c, 0 | 3010 | 1641) || already_installed,
        "winget ({c}); {}",
        crate::store::root().join("install.log").display()
    );
    let installed = match registered_tool(op)? {
        Some(installed) => installed,
        None => has_tool(&op.name)?,
    };
    ensure!(
        installed != uninstall,
        "Installer did not complete: {}",
        op.name
    );
    Ok(c != 0 && !already_installed)
}
pub fn copilot_commands() -> Result<Vec<String>> {
    let mut out = vec![];
    for hive in ["HKLM:", "HKCU:"] {
        for branch in ["SOFTWARE", r"SOFTWARE\WOW6432Node"] {
            if let Some(s) = reg::read(
                &format!(
                    r"{hive}\{branch}\Microsoft\Windows\CurrentVersion\Uninstall\Microsoft Copilot"
                ),
                "UninstallString",
            )?
            .as_str()
            {
                out.push(s.into())
            }
        }
    }
    Ok(out)
}
pub fn remove_copilot() -> Result<bool> {
    let mut restart = false;
    for cmd in copilot_commands()? {
        let mut args = native::uninstaller(&cmd)?;
        let exe = args.remove(0);
        args.push("--force-uninstall".into());
        let (c, t) = native::run(&exe, &args.iter().map(String::as_str).collect::<Vec<_>>())?;
        ensure!(matches!(c, 0 | 19 | 29), "{t}");
        restart |= c == 29;
    }
    ensure!(
        copilot_commands()?.is_empty(),
        "Copilot desktop is still installed"
    );
    Ok(restart)
}
