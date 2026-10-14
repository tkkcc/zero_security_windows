use crate::{
    model::{Operation, bool_value},
    native, registry as reg, store,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
use windows::{
    ApplicationModel::Package, Foundation::Uri, Management::Deployment::*, core::HSTRING,
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AppEntry {
    pub name: String,
    pub full: String,
    pub family: String,
    pub path: String,
    pub version: [u16; 4],
    pub installed: bool,
    pub current_user: bool,
    pub provisioned: bool,
    pub disabled: bool,
    pub bundle: bool,
    pub resource: bool,
    pub users: Vec<String>,
}
pub fn inventory() -> Result<Vec<AppEntry>> {
    native::com()?;
    let manager = PackageManager::new()?;
    let sid = native::sid()?;
    let provisioned_packages: Vec<_> = manager.FindProvisionedPackages()?.into_iter().collect();
    let provisioned: BTreeSet<String> = provisioned_packages
        .iter()
        .map(|p| p.Id().and_then(|id| id.FamilyName()).map(|s| s.to_string()))
        .collect::<windows::core::Result<_>>()?;
    let mut out = BTreeMap::new();
    for package in manager
        .FindPackages()?
        .into_iter()
        .chain(provisioned_packages)
    {
        let id = package.Id()?;
        if package.IsBundle()? || package.IsResourcePackage()? {
            continue;
        }
        let full = id.FullName()?.to_string();
        if out.contains_key(&full) {
            continue;
        }
        let users: Vec<String> = manager
            .FindUsers(&HSTRING::from(&full))?
            .into_iter()
            .filter_map(|user| match user.InstallState() {
                Ok(PackageInstallState::Installed) => {
                    Some(user.UserSecurityId().map(|s| s.to_string()))
                }
                Ok(_) => None,
                Err(e) => Some(Err(e)),
            })
            .collect::<windows::core::Result<_>>()?;
        let is_provisioned = provisioned.contains(&id.FamilyName()?.to_string());
        if users.is_empty() && !is_provisioned {
            continue;
        }
        let installed_path = package.InstalledPath()?.to_string();
        if !is_provisioned
            && (installed_path.is_empty() || !Path::new(&installed_path).try_exists()?)
        {
            continue;
        }
        let version = id.Version()?;
        out.insert(
            full.clone(),
            AppEntry {
                name: id.Name()?.to_string(),
                full,
                family: id.FamilyName()?.to_string(),
                path: installed_path,
                version: [
                    version.Major,
                    version.Minor,
                    version.Build,
                    version.Revision,
                ],
                installed: !users.is_empty(),
                current_user: users.iter().any(|user| user == &sid),
                provisioned: is_provisioned,
                disabled: package.Status()?.Disabled()?,
                bundle: package.IsBundle()?,
                resource: package.IsResourcePackage()?,
                users,
            },
        );
    }
    Ok(out.into_values().collect())
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
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Material {
    pub full: String,
    pub original: PathBuf,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Removal {
    pub name: String,
    pub full: String,
    pub family: String,
    pub source: String,
    pub non_removable: bool,
    pub provisioned: bool,
    pub materials: Vec<Material>,
    pub registrations: Vec<(String, String)>,
}
fn source_for<'a>(op: &'a Operation, name: &str) -> &'a str {
    op.package_sources
        .iter()
        .find(|(p, _)| p.eq_ignore_ascii_case(name))
        .map_or("", |(_, id)| id)
}
fn recovery_root(op: &Operation) -> PathBuf {
    store::root().join("recovery").join("apps").join(&op.name)
}
fn records(op: &Operation) -> Result<Vec<Removal>> {
    let path = recovery_root(op).join("packages.json");
    if !path.exists() {
        return Ok(vec![]);
    }
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
// Keep a single latest layout for each package identity, even if different users had older versions.
fn latest<'a>(entries: &[&'a AppEntry]) -> Vec<&'a AppEntry> {
    let mut out: BTreeMap<&str, &AppEntry> = BTreeMap::new();
    for &a in entries.iter().filter(|a| !a.bundle && !a.resource) {
        if out
            .get(a.name.as_str())
            .is_none_or(|old| a.version > old.version)
        {
            out.insert(&a.name, a);
        }
    }
    out.into_values().collect()
}
pub fn state(entries: &[AppEntry], op: &Operation) -> Result<bool> {
    let saved = records(op)?;
    let present = state_with_records(entries, op, &saved);
    if present && bool_value(&op.value) {
        let families: BTreeSet<_> = saved
            .iter()
            .filter(|a| a.non_removable)
            .map(|a| a.family.as_str())
            .chain(
                matching(entries, op)
                    .into_iter()
                    .filter(|_| op.non_removable)
                    .map(|a| a.family.as_str()),
            )
            .collect();
        for family in families {
            if !non_removable(family)? {
                return Ok(false);
            }
        }
    }
    Ok(present)
}
fn state_with_records(entries: &[AppEntry], op: &Operation, saved: &[Removal]) -> bool {
    let targets = matching(entries, op);
    if !bool_value(&op.value) {
        // A disabled package or provisioned-only package still needs actual removal.
        return targets.iter().any(|a| a.installed || a.provisioned);
    }
    if !saved.is_empty() {
        return saved.iter().all(|p| {
            targets
                .iter()
                .any(|a| a.name.eq_ignore_ascii_case(&p.name) && a.current_user && !a.disabled)
                && (!p.provisioned
                    || targets
                        .iter()
                        .any(|a| a.family == p.family && a.provisioned))
        });
    }
    let wanted: Vec<&str> = op.package_sources.keys().map(String::as_str).collect();
    if wanted.is_empty() {
        return !targets.is_empty() && targets.iter().all(|a| a.current_user && !a.disabled);
    }
    wanted.iter().all(|name| {
        targets
            .iter()
            .any(|a| a.name.eq_ignore_ascii_case(name) && a.current_user && !a.disabled)
    })
}
fn deployment(result: DeploymentResult) -> Result<()> {
    ensure!(
        result.ExtendedErrorCode()?.is_ok(),
        "{}",
        result.ErrorText()?
    );
    Ok(())
}
fn copy_layout(from: &Path, to: &Path) -> Result<()> {
    {
        let _guard = native::system()?;
        fs::create_dir_all(to)?;
    }
    let (code, output) = native::run(
        "robocopy.exe",
        &[
            from.to_str().unwrap(),
            to.to_str().unwrap(),
            "/E",
            "/B",
            "/COPY:DATS",
            "/SECFIX",
            "/SL",
            "/SJ",
            "/R:0",
            "/W:0",
        ],
    )?;
    ensure!(
        code < 8 && to.join("AppxManifest.xml").is_file(),
        "Package recovery copy failed: {output}"
    );
    Ok(())
}
fn save_layout(
    package: &Package,
    root: &Path,
    out: &mut Vec<Material>,
    seen: &mut BTreeSet<String>,
) -> Result<()> {
    let full = package.Id()?.FullName()?.to_string();
    if !seen.insert(full.clone()) {
        return Ok(());
    }
    for dependency in package.Dependencies()? {
        save_layout(&dependency, root, out, seen)?;
    }
    let original = PathBuf::from(package.InstalledPath()?.to_string());
    copy_layout(&original, &root.join(&full)).with_context(|| format!("Copy package {full}"))?;
    out.push(Material { full, original });
    if !package.IsResourcePackage()? && !package.IsFramework()? {
        let manager = PackageManager::new()?;
        for resource in manager.FindPackagesByPackageFamilyName(&package.Id()?.FamilyName()?)? {
            if resource.IsResourcePackage()?
                && resource.Id()?.Version()? == package.Id()?.Version()?
            {
                save_layout(&resource, root, out, seen)?;
            }
        }
    }
    Ok(())
}
#[link(name = "appxalluserstore", kind = "raw-dylib")]
unsafe extern "system" {
    fn IsPackageFamilyInUninstallBlocklist(family: *const u16, present: *mut i32) -> i32;
}
fn non_removable(family: &str) -> Result<bool> {
    let mut present = 0;
    windows::core::HRESULT(unsafe {
        IsPackageFamilyInUninstallBlocklist(native::wide(family).as_ptr(), &mut present)
    })
    .ok()?;
    Ok(present != 0)
}
fn set_non_removable(family: &str, protected: bool) -> Result<()> {
    native::dism(&[
        "/Online",
        "/English",
        "/Set-NonRemovableAppPolicy",
        &format!("/PackageFamily:{family}"),
        if protected {
            "/NonRemovable:1"
        } else {
            "/NonRemovable:0"
        },
    ])?;
    Ok(())
}
fn eol(sid: &str, full: &str) -> String {
    format!(
        r"HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Appx\AppxAllUserStore\EndOfLife\{sid}\{full}"
    )
}
pub fn remove(op: &Operation) -> Result<bool> {
    native::com()?;
    let entries = inventory()?;
    let targets = matching(&entries, op);
    if targets.is_empty() {
        return Ok(false);
    }
    let manager = PackageManager::new()?;
    let root = recovery_root(op);
    fs::create_dir_all(&root).context("Create package recovery folder")?;
    let mut saved = records(op)?;
    // Finish all required recovery copies before removing registrations or provisioning.
    for a in latest(&targets) {
        let source = source_for(op, &a.name).to_string();
        let protected = op.non_removable
            || non_removable(&a.family)?
            || saved
                .iter()
                .any(|p| p.family == a.family && p.non_removable);
        let mut materials = vec![];
        if source.is_empty() {
            let package = manager
                .FindPackages()?
                .into_iter()
                .chain(manager.FindProvisionedPackages()?)
                .find(|p| {
                    p.Id()
                        .and_then(|id| id.FullName())
                        .is_ok_and(|full| full == a.full.as_str())
                })
                .ok_or_else(|| anyhow::anyhow!("Missing package layout: {}", a.full))?;
            save_layout(&package, &root, &mut materials, &mut BTreeSet::new())?;
        }
        let registrations: BTreeSet<_> = targets
            .iter()
            .filter(|p| p.family == a.family)
            .flat_map(|p| p.users.iter().map(|sid| (sid.clone(), p.full.clone())))
            .chain(
                saved
                    .iter()
                    .filter(|p| p.family == a.family)
                    .flat_map(|p| p.registrations.iter().cloned()),
            )
            .collect();
        let provisioned = targets
            .iter()
            .any(|p| p.family == a.family && p.provisioned)
            || saved.iter().any(|p| p.family == a.family && p.provisioned);
        saved.retain(|p| !p.name.eq_ignore_ascii_case(&a.name));
        saved.push(Removal {
            name: a.name.clone(),
            full: a.full.clone(),
            family: a.family.clone(),
            source,
            non_removable: protected,
            provisioned,
            materials,
            registrations: registrations.into_iter().collect(),
        });
    }
    fs::write(
        root.join("packages.json"),
        serde_json::to_vec_pretty(&saved)?,
    )
    .context("Save package recovery record")?;
    for a in &saved {
        if a.non_removable {
            set_non_removable(&a.family, false).context("Allow package removal")?;
        }
        let _guard = native::system()?;
        for (sid, full) in &a.registrations {
            reg::key(&eol(sid, full), true).context("Allow user package removal")?;
        }
    }
    let images = native::process_images()?;
    for a in &targets {
        for (_, image) in &images {
            if image
                .to_lowercase()
                .starts_with(&format!("{}\\", a.path.to_lowercase()))
            {
                native::stop_process(Path::new(image).file_name().unwrap().to_str().unwrap())?;
            }
        }
    }
    let mut families = BTreeSet::new();
    for a in &targets {
        if a.provisioned && families.insert(&a.family) {
            deployment(
                manager
                    .DeprovisionPackageForAllUsersAsync(&HSTRING::from(&a.family))?
                    .join()?,
            )?;
        }
    }
    let mut restart = false;
    let after_deprovision = inventory()?;
    let mut removed_families = BTreeSet::new();
    for a in matching(&after_deprovision, op) {
        let system_location = PathBuf::from(std::env::var("SystemRoot")?).join("SystemApps");
        if op.non_removable || Path::new(&a.path).starts_with(&system_location) {
            let original = PathBuf::from(&a.path).canonicalize()?;
            let system = PathBuf::from(std::env::var("SystemRoot")?)
                .join("SystemApps")
                .canonicalize()?;
            let applications = PathBuf::from(std::env::var("ProgramFiles")?)
                .join("WindowsApps")
                .canonicalize()?;
            ensure!(
                original.parent() == Some(system.as_path())
                    || original.parent() == Some(applications.as_path()),
                "Unexpected system package location"
            );
            let _guard = if original.starts_with(&system) {
                native::trusted_installer().context("Use TrustedInstaller for the system app")?
            } else {
                native::system()?
            };
            if !original.starts_with(&system) {
                manager.SetPackageStatus(&HSTRING::from(&a.full), PackageStatus::Disabled)?;
            }
            fs::remove_dir_all(&original).context("Remove system app payload")?;
            continue;
        }
        if !removed_families.insert(&a.family) {
            continue;
        }
        let family: Vec<_> = manager
            .FindPackagesByPackageFamilyName(&HSTRING::from(&a.family))?
            .into_iter()
            .collect();
        let mut bundles = vec![];
        let mut main = vec![];
        for package in &family {
            if package.IsBundle()? {
                bundles.push(package);
            } else if !package.IsResourcePackage()? {
                main.push(package);
            }
        }
        let roots = if bundles.is_empty() { main } else { bundles };
        for package in roots {
            let full = package.Id()?.FullName()?;
            if manager.IsPackageRemovalPending(&full)? {
                continue;
            }
            deployment(
                manager
                    .RemovePackageWithOptionsAsync(&full, RemovalOptions::RemoveForAllUsers)?
                    .join()?,
            )
            .with_context(|| format!("Remove package {full}"))?;
        }
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
    let remaining = loop {
        let remaining = inventory()?;
        let targets = matching(&remaining, op);
        if targets.is_empty() || std::time::Instant::now() >= deadline {
            break remaining;
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    };
    restart |= !matching(&remaining, op).is_empty();
    let needed: BTreeSet<_> = saved
        .iter()
        .flat_map(|a| a.materials.iter().map(|m| &m.full))
        .collect();
    let resolved_root = root.canonicalize()?;
    for item in fs::read_dir(&root)? {
        let item = item?;
        if !item.file_type()?.is_dir()
            || needed.iter().any(|full| item.file_name() == full.as_str())
        {
            continue;
        }
        let path = item.path().canonicalize()?;
        ensure!(
            path.starts_with(&resolved_root) && path != resolved_root,
            "Recovery cleanup path escapes its folder"
        );
        let _guard = native::system()?;
        fs::remove_dir_all(path)?;
    }
    crate::preferences::explorer();
    Ok(restart)
}
fn uri(path: &Path) -> Result<Uri> {
    Ok(Uri::CreateUri(&HSTRING::from(format!(
        "file:///{}",
        path.to_string_lossy().replace('\\', "/")
    )))?)
}
pub fn restore(op: &Operation) -> Result<bool> {
    native::com()?;
    let manager = PackageManager::new()?;
    let root = recovery_root(op);
    let saved = records(op)?;
    if saved.is_empty() {
        ensure!(
            !op.package_sources.is_empty()
                || matching(&inventory()?, op).iter().any(|a| a.current_user),
            "No package recovery materials were saved for {}",
            op.name
        );
    }
    for a in &saved {
        let _guard = native::system()?;
        for (sid, full) in &a.registrations {
            reg::key(&eol(sid, full), false)?;
        }
    }
    let ids: BTreeSet<&str> = if saved.is_empty() {
        op.package_sources.values().map(String::as_str).collect()
    } else {
        saved
            .iter()
            .filter(|a| !a.source.is_empty())
            .map(|a| a.source.as_str())
            .collect()
    };
    // Each public installation source is used once; these packages have no saved payload.
    let mut restart = false;
    for id in ids {
        restart |= install(
            &Operation {
                name: id.into(),
                source: "msstore".into(),
                ..Default::default()
            },
            false,
        )?;
    }
    let mut copied = BTreeSet::new();
    for a in saved.iter().filter(|a| a.source.is_empty()) {
        ensure!(
            !a.materials.is_empty(),
            "Missing recovery layout for {}",
            a.name
        );
        for material in &a.materials {
            if !copied.insert(&material.full) {
                continue;
            }
            copy_layout(&root.join(&material.full), &material.original)?;
        }
        let main = a
            .materials
            .iter()
            .find(|m| m.full == a.full)
            .ok_or_else(|| anyhow::anyhow!("Missing main recovery layout for {}", a.name))?;
        if inventory()?
            .iter()
            .any(|p| p.full == a.full && p.current_user)
        {
            continue;
        }
        let manifest = uri(&main.original.join("AppxManifest.xml"))?;
        let dependencies: windows_collections::IIterable<Uri> = a
            .materials
            .iter()
            .filter(|m| m.full != a.full)
            .map(|m| uri(&m.original.join("AppxManifest.xml")).map(Some))
            .collect::<Result<Vec<_>>>()?
            .into();
        deployment(
            manager
                .RegisterPackageAsync(
                    &manifest,
                    &dependencies,
                    DeploymentOptions::ForceApplicationShutdown,
                )?
                .join()?,
        )?;
    }
    let restored = inventory()?;
    let mut families = BTreeSet::new();
    for a in matching(&restored, op) {
        if a.disabled {
            let _guard = native::system()?;
            manager.ClearPackageStatus(&HSTRING::from(&a.full), PackageStatus::Disabled)?;
        }
        if saved.iter().any(|p| p.family == a.family && p.provisioned) && families.insert(&a.family)
        {
            deployment(
                manager
                    .ProvisionPackageForAllUsersAsync(&HSTRING::from(&a.family))?
                    .join()?,
            )?;
        }
        if op.non_removable
            || saved
                .iter()
                .any(|p| p.family == a.family && p.non_removable)
        {
            set_non_removable(&a.family, true)?;
        }
    }
    ensure!(
        state_with_records(&inventory()?, op, &saved),
        "Packages did not return to an installed state: {}",
        op.name
    );
    crate::preferences::explorer();
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
        for branch in UNINSTALL_BRANCHES {
            if reg::exists(&format!(r"{branch}\{}", op.uninstall_key))? {
                return Ok(Some(true));
            }
        }
        return Ok(Some(false));
    }
    Ok(None)
}
const UNINSTALL_BRANCHES: [&str; 3] = [
    r"HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
    r"HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
    r"HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
];
pub fn install(op: &Operation, uninstall: bool) -> Result<bool> {
    if uninstall || !(op.source == "msstore" || op.requires_delivery) {
        return install_run(op, uninstall);
    }
    let dependencies: Vec<_> = ["DoSvc", "wuauserv"]
        .into_iter()
        .map(|service| {
            let path = format!(r"HKLM:\SYSTEM\CurrentControlSet\Services\{service}");
            reg::number(&path, "Start", 3).map(|desired| (service, path, desired as u32))
        })
        .collect::<Result<_>>()?;
    let result = (|| {
        for (service, _, desired) in &dependencies {
            if native::service_start(service, None)? == 4 {
                native::service_start(service, Some(if *desired == 4 { 3 } else { *desired }))?;
            }
        }
        install_run(op, uninstall)
    })();
    let cleanup: Vec<Result<()>> = dependencies
        .into_iter()
        .map(|(service, path, desired)| {
            if desired == 4 {
                let restart = native::disable_service(service)?;
                reg::set(&path, "Start", 4, "DWord")?;
                ensure!(!restart, "{service} could not return to its disabled state");
            } else if native::service_start(service, None)? != desired {
                native::service_start(service, Some(desired))?;
            }
            Ok(())
        })
        .collect();
    for restored in cleanup {
        restored?;
    }
    result
}
fn install_run(op: &Operation, uninstall: bool) -> Result<bool> {
    if uninstall && !op.uninstall_args.is_empty() {
        for branch in UNINSTALL_BRANCHES {
            let path = format!(r"{branch}\{}", op.uninstall_key);
            if let Some(command) = reg::read(&path, "UninstallString")?.as_str() {
                reg::set(
                    &path,
                    "QuietUninstallString",
                    format!("{command} {}", op.uninstall_args),
                    "String",
                )?;
            }
        }
    }
    let mut args = vec![
        if uninstall { "uninstall" } else { "install" },
        if uninstall && !op.uninstall_key.is_empty() {
            "--product-code"
        } else {
            "--id"
        },
        if uninstall && !op.uninstall_key.is_empty() {
            &op.uninstall_key
        } else {
            &op.name
        },
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(name: &str, version: u16, installed: bool, provisioned: bool) -> AppEntry {
        AppEntry {
            name: name.into(),
            full: format!("{name}_{version}"),
            family: format!("{name}_publisher"),
            version: [version, 0, 0, 0],
            installed,
            current_user: installed,
            provisioned,
            ..Default::default()
        }
    }
    fn snapshot(name: &str, provisioned: bool) -> Removal {
        Removal {
            name: name.into(),
            full: format!("{name}_1"),
            family: format!("{name}_publisher"),
            source: "Test.Store".into(),
            non_removable: false,
            provisioned,
            materials: vec![],
            registrations: vec![],
        }
    }
    #[test]
    fn machine_32_bit_registration_is_visible_after_winget_tracking_is_removed() -> Result<()> {
        let name = format!("ZeroSecurityWindowsRegistrationTest{}", std::process::id());
        let path = format!(r"{}\{name}", UNINSTALL_BRANCHES[2]);
        ensure!(!reg::exists(&path)?, "Test registration already exists");
        let op = Operation {
            uninstall_key: name,
            ..Default::default()
        };
        reg::key(&path, true)?;
        let result = (|| -> Result<()> {
            assert_eq!(registered_tool(&op)?, Some(true));
            reg::key(&path, false)?;
            assert_eq!(registered_tool(&op)?, Some(false));
            Ok(())
        })();
        reg::key(&path, false)?;
        result
    }
    #[test]
    fn removal_includes_disabled_and_provisioned_only_packages() {
        let op = Operation {
            patterns: vec!["Test.*".into()],
            value: json!(false),
            ..Default::default()
        };
        let mut disabled = entry("Test.App", 1, true, false);
        disabled.disabled = true;
        assert!(state_with_records(&[disabled], &op, &[]));
        assert!(state_with_records(
            &[entry("Test.App", 1, false, true)],
            &op,
            &[]
        ));
        assert!(!state_with_records(
            &[entry("Test.App", 1, false, false)],
            &op,
            &[]
        ));
        assert!(!state_with_records(&[], &op, &[]));
    }
    #[test]
    fn recovery_sources_are_assigned_per_package_and_keep_one_latest_layout() {
        let op = Operation {
            package_sources: [("Test.Public".into(), "Test.Store".into())].into(),
            ..Default::default()
        };
        assert_eq!(source_for(&op, "test.public"), "Test.Store");
        assert!(source_for(&op, "Test.Private").is_empty());
        let entries = [
            entry("Test.Private", 9, true, false),
            entry("Test.Private", 10, true, false),
            entry("Test.Public", 1, true, true),
        ];
        let refs: Vec<_> = entries.iter().collect();
        let selected = latest(&refs);
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].full, "Test.Private_10");
        assert_eq!(selected[1].full, "Test.Public_1");
    }
    #[test]
    fn recovery_selects_the_main_payload_instead_of_bundle_or_resource_metadata() {
        let main = entry("Test.App", 1, true, false);
        let mut bundle = entry("Test.App", 2022, true, true);
        bundle.bundle = true;
        let mut resource = entry("Test.App", 1, true, false);
        resource.resource = true;
        let selected = latest(&[&bundle, &resource, &main]);
        assert_eq!(selected.len(), 1);
        assert!(std::ptr::eq(selected[0], &main));
    }
    #[test]
    fn recovery_status_requires_removed_packages_and_previous_provisioning() {
        let op = Operation {
            patterns: vec!["Test.*".into()],
            value: json!(true),
            ..Default::default()
        };
        let saved = [snapshot("Test.Main", true), snapshot("Test.Extra", false)];
        assert!(!state_with_records(
            &[entry("Test.Main", 1, true, true)],
            &op,
            &saved
        ));
        assert!(!state_with_records(
            &[
                entry("Test.Main", 1, true, false),
                entry("Test.Extra", 1, true, false)
            ],
            &op,
            &saved
        ));
        assert!(state_with_records(
            &[
                entry("Test.Main", 2, true, true),
                entry("Test.Extra", 1, true, false)
            ],
            &op,
            &saved
        ));
    }
    #[test]
    fn another_user_install_does_not_complete_current_user_restoration() {
        let op = Operation {
            patterns: vec!["Test.*".into()],
            value: json!(true),
            package_sources: [("Test.Main".into(), "Test.Store".into())].into(),
            ..Default::default()
        };
        let mut package = entry("Test.Main", 1, true, true);
        package.current_user = false;
        assert!(!state_with_records(&[package.clone()], &op, &[]));
        assert!(!state_with_records(
            &[package.clone()],
            &op,
            &[snapshot("Test.Main", true)]
        ));
        package.current_user = true;
        assert!(state_with_records(
            &[package],
            &op,
            &[snapshot("Test.Main", true)]
        ));
    }
}
