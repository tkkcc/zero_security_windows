use crate::{model::Operation, native, registry as reg};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use windows::{Management::Deployment::*, core::HSTRING};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppEntry {
    pub name: String,
    pub full: String,
    pub family: String,
    pub non_removable: bool,
    pub users: Vec<String>,
    pub provisioned: bool,
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
                family: id.FamilyName()?.to_string(),
                non_removable: non_removable(&package)?,
                users,
                provisioned: false,
            });
        }
    }
    for package in manager.FindProvisionedPackages()? {
        let id = package.Id()?;
        out.push(AppEntry {
            name: id.Name()?.to_string(),
            full: id.FullName()?.to_string(),
            family: id.FamilyName()?.to_string(),
            non_removable: false,
            users: vec![],
            provisioned: true,
        });
    }
    Ok(out)
}
pub fn matching<'a>(entries: &'a [AppEntry], op: &Operation) -> Vec<&'a AppEntry> {
    entries
        .iter()
        .filter(|a| {
            (!a.non_removable || op.system_app || a.provisioned)
                && op
                    .patterns
                    .iter()
                    .any(|p| crate::engine::matches(&a.name, p))
        })
        .collect()
}
pub fn remove(op: &Operation) -> Result<bool> {
    let entries = inventory()?;
    let mut targets = matching(&entries, op);
    targets.sort_by_key(|a| !a.provisioned);
    let manager = PackageManager::new()?;
    let mut restart = false;
    let mut errors = vec![];
    for a in targets {
        let result = (|| -> Result<()> {
            if op.system_app && !a.provisioned {
                native::dism(&[
                    "/Online",
                    "/English",
                    "/Set-NonRemovableAppPolicy",
                    &format!("/PackageFamily:{}", a.family),
                    "/NonRemovable:0",
                ])?;
                for sid in &a.users {
                    reg::key(
                        &format!(
                            r"HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Appx\AppxAllUserStore\EndOfLife\{sid}\{}",
                            a.full
                        ),
                        true,
                    )?;
                }
            }
            let result = if a.provisioned {
                manager
                    .DeprovisionPackageForAllUsersAsync(&HSTRING::from(&a.family))?
                    .join()?
            } else {
                manager
                    .RemovePackageWithOptionsAsync(
                        &HSTRING::from(&a.full),
                        RemovalOptions::RemoveForAllUsers,
                    )?
                    .join()?
            };
            let hr = result.ExtendedErrorCode()?;
            ensure!(hr.is_ok(), "{}: {}", a.name, result.ErrorText()?);
            if !a.provisioned {
                restart |= manager.IsPackageRemovalPending(&HSTRING::from(&a.full))?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            errors.push(format!("{}: {e}", a.name))
        }
    }
    ensure!(errors.is_empty(), "{}", errors.join("; "));
    // Successful removal can leave another user's registration until restart.
    restart |= !matching(&inventory()?, op).is_empty();
    Ok(restart)
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

#[link(name = "appxalluserstore", kind = "raw-dylib")]
unsafe extern "system" {
    fn IsPackageFamilyInUninstallBlocklist(family: *const u16, present: *mut i32) -> i32;
}
fn non_removable(package: &windows::ApplicationModel::Package) -> Result<bool> {
    if package.SignatureKind()? == windows::ApplicationModel::PackageSignatureKind::System {
        return Ok(true);
    }
    let mut present = 0;
    let family = native::wide(&package.Id()?.FamilyName()?.to_string());
    windows::core::HRESULT(unsafe {
        IsPackageFamilyInUninstallBlocklist(family.as_ptr(), &mut present)
    })
    .ok()?;
    Ok(present != 0)
}
