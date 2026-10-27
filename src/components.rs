use crate::{apps, model::*, native, preferences, registry as reg, store};
use anyhow::{Result, ensure};
use rusqlite::{Connection, OpenFlags, OptionalExtension, functions::FunctionFlags};
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
};

const EDGE_ID: &str = "{56EB18F8-B008-4CBD-B6D2-8C97FE7E9062}";
const EDGE_UPDATE: &str = r"HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate";

fn edge_roots() -> Vec<PathBuf> {
    ["%ProgramFiles(x86)%", "%ProgramFiles%", "%LOCALAPPDATA%"]
        .map(|p| PathBuf::from(native::expand(p)).join("Microsoft/Edge"))
        .into_iter()
        .collect()
}
fn edge_core_launchers() -> Result<Vec<PathBuf>> {
    let mut files = vec![];
    for base in ["%ProgramFiles(x86)%", "%ProgramFiles%", "%LOCALAPPDATA%"] {
        let root = PathBuf::from(native::expand(base)).join("Microsoft/EdgeCore");
        if !root.try_exists()? {
            continue;
        }
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            // Browser-only executables; msedge_proxy and the engine are shared with WebView2.
            for name in [
                "msedge.exe",
                "msedge.exe.sig",
                "msedge_pwa_launcher.exe",
                "pwahelper.exe",
                "copilotapp.exe",
                "mscopilot.exe",
            ] {
                let path = entry.path().join(name);
                if path.try_exists()? {
                    files.push(path);
                }
            }
        }
    }
    Ok(files)
}
fn edge_links() -> Result<Vec<PathBuf>> {
    use windows::Win32::UI::Shell::*;
    [
        FOLDERID_CommonPrograms,
        FOLDERID_Programs,
        FOLDERID_PublicDesktop,
        FOLDERID_Desktop,
    ]
    .iter()
    .map(|id| Ok(native::known_folder(id)?.join("Microsoft Edge.lnk")))
    .collect()
}
fn edge_registrations() -> Result<Vec<String>> {
    let mut registrations = vec![];
    for branch in [
        r"HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
        r"HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
        r"HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
    ] {
        for name in reg::children(branch)? {
            let path = format!(r"{branch}\{name}");
            if reg::read(&path, "DisplayName")?.as_str() == Some("Microsoft Edge") {
                registrations.push(path);
            }
        }
    }
    Ok(registrations)
}
fn edge_payload_present() -> Result<bool> {
    for root in edge_roots() {
        if root.try_exists()? {
            return Ok(true);
        }
    }
    if !edge_core_launchers()?.is_empty() {
        return Ok(true);
    }
    if !edge_registrations()?.is_empty() {
        return Ok(true);
    }
    for link in edge_links()? {
        if link.try_exists()? {
            return Ok(true);
        }
    }
    native::process_running("msedge.exe")
}
pub fn listed(id: &str) -> Result<bool> {
    let id = id.replace('\'', "''");
    let text = native::command(
        "powershell.exe",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!(
                "[bool](@(Get-StartApps | Where-Object AppID -eq '{id}').Count) | ConvertTo-Json -Compress"
            ),
        ],
    )?;
    Ok(serde_json::from_str(&text)?)
}
pub fn edge_state(installed: bool) -> Result<bool> {
    if installed {
        return Ok(edge_roots()
            .iter()
            .any(|root| root.join("Application/msedge.exe").is_file())
            && listed("MSEdge")?);
    }
    Ok(edge_payload_present()? || listed("MSEdge")?)
}
pub fn edge(installed: bool, progress: impl Fn(&str)) -> Result<bool> {
    if installed {
        return apps::install(
            &Operation {
                name: "Microsoft.Edge".into(),
                patterns: vec!["Microsoft Edge".into()],
                source: "winget".into(),
                ..Default::default()
            },
            false,
            progress,
        );
    }
    native::stop_process("msedge.exe")?;
    native::stop_process("msedge_proxy.exe")?;
    let mut restart = false;
    for key in edge_registrations()? {
        if let Some(command) = reg::read(&key, "UninstallString")?.as_str() {
            let msi = reg::read(&key, "WindowsInstaller")? == serde_json::json!(1);
            let mut args = if msi {
                vec![
                    "msiexec.exe".into(),
                    "/x".into(),
                    key.rsplit('\\').next().unwrap().into(),
                    "/qn".into(),
                    "/norestart".into(),
                    "UPGRADINGPRODUCTCODE=1".into(),
                ]
            } else {
                let mut args = native::uninstaller(command)?;
                args.push("--force-uninstall".into());
                args
            };
            let exe = args.remove(0);
            if msi || PathBuf::from(&exe).try_exists()? {
                let (code, output) =
                    native::run(&exe, &args.iter().map(String::as_str).collect::<Vec<_>>())?;
                store::append(
                    "install.log",
                    &format!("[Microsoft.Edge uninstall {code}]\n{output}"),
                )?;
                ensure!(
                    !msi || matches!(code, 0 | 1605 | 3010),
                    "Edge MSI removal failed: {code}"
                );
                restart |= code == 29 || code == 3010;
            }
        }
    }
    if reg::exists(r"HKLM:\SYSTEM\CurrentControlSet\Services\MicrosoftEdgeElevationService")? {
        native::disable_service("MicrosoftEdgeElevationService")?;
        native::command("sc.exe", &["delete", "MicrosoftEdgeElevationService"])?;
    }
    // Keep the shared engine, WebView2 and its updater.
    let _guard = native::trusted_installer()?;
    for root in edge_roots() {
        if root.try_exists()? {
            let parent = root.parent().unwrap().canonicalize()?;
            let resolved = root.canonicalize()?;
            ensure!(
                resolved.parent() == Some(parent.as_path())
                    && resolved
                        .file_name()
                        .is_some_and(|n| n.eq_ignore_ascii_case("Edge")),
                "Unexpected Edge program location"
            );
            fs::remove_dir_all(resolved)?;
        }
    }
    for path in edge_core_launchers()? {
        fs::remove_file(path)?;
    }
    for key in edge_registrations()? {
        reg::key(&key, false)?;
    }
    for branch in ["Clients", "ClientState", "ClientStateMedium"] {
        reg::key(&format!(r"{EDGE_UPDATE}\{branch}\{EDGE_ID}"), false)?;
    }
    for key in [
        r"HKLM:\SOFTWARE\Clients\StartMenuInternet\Microsoft Edge",
        r"HKLM:\SOFTWARE\WOW6432Node\Clients\StartMenuInternet\Microsoft Edge",
        r"HKCU:\SOFTWARE\Clients\StartMenuInternet\Microsoft Edge",
        r"HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\msedge.exe",
        r"HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\msedge.exe",
        r"HKLM:\SOFTWARE\Classes\MSEdgeHTM",
        r"HKLM:\SOFTWARE\Classes\MSEdgePDF",
    ] {
        reg::key(key, false)?;
    }
    for hive in ["HKLM:", "HKCU:"] {
        reg::delete(
            &format!(r"{hive}\SOFTWARE\RegisteredApplications"),
            "Microsoft Edge",
        )?;
    }
    for link in edge_links()? {
        if link.try_exists()? {
            fs::remove_file(link)?;
        }
    }
    drop(_guard);
    preferences::explorer();
    ensure!(
        !edge_payload_present()?,
        "Edge browser removal did not finish"
    );
    restart |= listed("MSEdge")?;
    Ok(restart)
}

pub fn app_entry(op: &Operation) -> Result<bool> {
    let mut visible = vec![];
    let paths = entry_manifests(op)?;
    for path in &paths {
        visible.push(manifest_entry_visible(
            &fs::read_to_string(path)?,
            &op.name,
        )?);
    }
    let id = format!("{}!{}", op.package_family, op.name);
    visible.push(reg::exists(&launch_contract(
        &paths[1].file_stem().unwrap().to_string_lossy(),
        &op.name,
    ))?);
    visible.extend(registered_entries(
        &entry_database(false)?,
        &id,
        bool_value(&op.value),
    )?);
    visible.push(listed(&id)?);
    Ok(if bool_value(&op.value) {
        visible.into_iter().all(|v| v)
    } else {
        visible.into_iter().any(|v| v)
    })
}
fn launch_contract(full: &str, name: &str) -> String {
    format!(
        r"HKLM:\SOFTWARE\Classes\Extensions\ContractId\Windows.Launch\PackageId\{full}\ActivatableClassId\{name}"
    )
}
fn entry_manifests(op: &Operation) -> Result<Vec<PathBuf>> {
    let full = apps::inventory()?
        .into_iter()
        .find(|p| p.current_user && p.family == op.package_family)
        .ok_or_else(|| anyhow::anyhow!("Application package is not registered"))?
        .full;
    Ok(vec![
        PathBuf::from(native::expand(&op.path)),
        PathBuf::from(native::expand("%ProgramData%"))
            .join("Microsoft/Windows/AppRepository")
            .join(format!("{full}.xml")),
    ])
}
fn manifest_entry_visible(text: &str, name: &str) -> Result<bool> {
    let doc = roxmltree::Document::parse(text)?;
    let app = doc
        .descendants()
        .find(|node| node.has_tag_name("Application") && node.attribute("Id") == Some(name))
        .ok_or_else(|| anyhow::anyhow!("Application entry is missing"))?;
    let visual = app
        .children()
        .find(|node| node.tag_name().name() == "VisualElements")
        .ok_or_else(|| anyhow::anyhow!("Application visual entry is missing"))?;
    Ok(visual.attribute("AppListEntry") != Some("none"))
}
fn entry_manifest(text: &str, name: &str, visible: bool) -> Result<String> {
    let doc = roxmltree::Document::parse(text)?;
    let app = doc
        .descendants()
        .find(|node| node.has_tag_name("Application") && node.attribute("Id") == Some(name))
        .ok_or_else(|| anyhow::anyhow!("Application entry is missing"))?;
    let visual = app
        .children()
        .find(|node| node.tag_name().name() == "VisualElements")
        .ok_or_else(|| anyhow::anyhow!("Application visual entry is missing"))?;
    let mut result = text.to_owned();
    if let Some(attr) = visual
        .attributes()
        .find(|attr| attr.name() == "AppListEntry")
    {
        result.replace_range(
            attr.range(),
            if visible { "" } else { "AppListEntry=\"none\"" },
        );
    } else if !visible {
        let offset = visual.range().start + text[visual.range()].find('>').unwrap();
        let offset = if text.as_bytes()[offset - 1] == b'/' {
            offset - 1
        } else {
            offset
        };
        result.insert_str(offset, " AppListEntry=\"none\"");
    }
    Ok(result)
}
fn entry_database(write: bool) -> Result<Connection> {
    let root =
        PathBuf::from(native::expand("%ProgramData%")).join("Microsoft/Windows/AppRepository");
    let db = Connection::open_with_flags(
        root.join("StateRepository-Machine.srd"),
        if write {
            OpenFlags::SQLITE_OPEN_READ_WRITE
        } else {
            OpenFlags::SQLITE_OPEN_READ_ONLY
        },
    )?;
    db.execute(
        "ATTACH DATABASE ?1 AS deployment",
        [root
            .join("StateRepository-Deployment.srd")
            .to_string_lossy()
            .as_ref()],
    )?;
    Ok(db)
}
fn deployment_manifest(db: &Connection, id: &str) -> Result<Vec<u8>> {
    Ok(db.query_row("SELECT m.Xml FROM deployment.AppxManifest m JOIN Application a ON m.Package=a.Package WHERE a.ApplicationUserModelId=?1", [id], |row| row.get(0))?)
}
fn unpack_manifest(data: &[u8]) -> Result<String> {
    let prefix = data
        .iter()
        .position(|b| b & 128 != 0)
        .ok_or_else(|| anyhow::anyhow!("Invalid cached manifest length"))?
        + 1;
    let length = data[..prefix]
        .iter()
        .fold(0usize, |size, b| (size << 7) | (b & 127) as usize);
    let mut text = String::new();
    flate2::read::ZlibDecoder::new(&data[prefix..]).read_to_string(&mut text)?;
    ensure!(
        text.len() == length,
        "Cached manifest length does not match"
    );
    Ok(text)
}
fn pack_manifest(text: &str) -> Result<Vec<u8>> {
    let mut length = text.len();
    let mut data = vec![(length & 127) as u8 | 128];
    while {
        length >>= 7;
        length != 0
    } {
        data.push((length & 127) as u8);
    }
    data.reverse();
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(text.as_bytes())?;
    data.extend(encoder.finish()?);
    Ok(data)
}
fn registered_entries(db: &Connection, id: &str, restoring: bool) -> Result<[bool; 4]> {
    let entry: Option<i32> = db
        .query_row(
            "SELECT AppListEntry FROM Application WHERE ApplicationUserModelId=?1",
            [id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(entry) = entry else {
        return Ok([false; 4]);
    };
    let users: i64 = db.query_row("SELECT COUNT(*) FROM ApplicationUser u JOIN Application a ON u.Application=a._ApplicationID WHERE a.ApplicationUserModelId=?1", [id], |r| r.get(0))?;
    let expected: i64 = db.query_row("SELECT COUNT(*) FROM PackageUser u JOIN Application a ON u.Package=a.Package WHERE a.ApplicationUserModelId=?1",[id],|r|r.get(0))?;
    Ok([
        true,
        entry == 0,
        manifest_entry_visible(
            &unpack_manifest(&deployment_manifest(db, id)?)?,
            id.rsplit('!').next().unwrap(),
        )?,
        if restoring {
            users > 0 && users == expected
        } else {
            users > 0
        },
    ])
}
#[derive(serde::Serialize, serde::Deserialize)]
struct ApplicationRecord {
    package: String,
    package_id: i64,
    activation: i64,
    insert: String,
}
fn entry_record(id: &str) -> PathBuf {
    store::root()
        .join("recovery")
        .join("app-entries")
        .join(format!("{id}.json"))
}
const ENTRY_TABLES: &[(&str, &str)] = &[
    ("ApplicationIdentity", "ApplicationUserModelId=?1"),
    ("Application", "ApplicationUserModelId=?1"),
    (
        "MrtApplication",
        "Application IN (SELECT _ApplicationID FROM Application WHERE ApplicationUserModelId=?1)",
    ),
    (
        "ApplicationProperty",
        "Application IN (SELECT _ApplicationID FROM Application WHERE ApplicationUserModelId=?1)",
    ),
    (
        "DefaultTile",
        "Application IN (SELECT _ApplicationID FROM Application WHERE ApplicationUserModelId=?1)",
    ),
    (
        "MrtDefaultTile",
        "DefaultTile IN (SELECT _DefaultTileID FROM DefaultTile WHERE Application IN (SELECT _ApplicationID FROM Application WHERE ApplicationUserModelId=?1))",
    ),
    (
        "PrimaryTile",
        "Application IN (SELECT _ApplicationID FROM Application WHERE ApplicationUserModelId=?1)",
    ),
    (
        "PrimaryTileUser",
        "ApplicationIdentity IN (SELECT _ApplicationIdentityID FROM ApplicationIdentity WHERE ApplicationUserModelId=?1)",
    ),
];
fn save_application(db: &Connection, id: &str, full: &str) -> Result<()> {
    let mut insert = String::new();
    for (table, filter) in ENTRY_TABLES {
        let statement = db.prepare(&format!("SELECT * FROM {table} WHERE {filter}"))?;
        let columns = statement
            .column_names()
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>();
        let quoted = columns
            .iter()
            .map(|n| format!("\"{}\"", n.replace('"', "\"\"")))
            .collect::<Vec<_>>();
        let values_sql = quoted
            .iter()
            .map(|n| format!("quote({n})"))
            .collect::<Vec<_>>()
            .join(",");
        let mut statement =
            db.prepare(&format!("SELECT {values_sql} FROM {table} WHERE {filter}"))?;
        let rows = statement.query_map([id], |row| {
            (0..columns.len())
                .map(|i| row.get::<_, String>(i))
                .collect::<rusqlite::Result<Vec<_>>>()
        })?;
        for row in rows {
            let mut values = row?;
            if *table == "Application" {
                values[columns.iter().position(|n| n == "AppListEntry").unwrap()] = "0".into();
            }
            if *table == "PrimaryTileUser" {
                for name in ["State", "_Created", "_Modified"] {
                    values[columns.iter().position(|n| n == name).unwrap()] = "0".into();
                }
            }
            insert.push_str(&format!(
                "INSERT INTO {table} ({}) VALUES ({});\n",
                quoted.join(","),
                values.join(",")
            ));
        }
    }
    let (package_id, activation) = db.query_row(
        "SELECT Package,Activation FROM Application WHERE ApplicationUserModelId=?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let record = ApplicationRecord {
        package: full.into(),
        package_id,
        activation,
        insert,
    };
    let path = entry_record(id);
    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(path, serde_json::to_vec(&record)?)?;
    Ok(())
}
fn set_registered_entry(db: &mut Connection, id: &str, visible: bool, full: &str) -> Result<()> {
    // This Windows schema uses zero sentinel references with foreign keys disabled.
    db.pragma_update(None, "foreign_keys", false)?;
    // Leave the shared package, activation records and other applications intact.
    db.create_scalar_function(
        "is_srjournal_enabled",
        0,
        FunctionFlags::SQLITE_UTF8,
        |_| Ok(1),
    )?;
    db.create_scalar_function("is_triggers_enabled", 0, FunctionFlags::SQLITE_UTF8, |_| {
        Ok(1)
    })?;
    db.create_scalar_function("workid", 0, FunctionFlags::SQLITE_UTF8, |_| Ok(0))?;
    db.create_scalar_function("now", 0, FunctionFlags::SQLITE_UTF8, |_| {
        Ok((std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            / 100) as i64
            + 116444736000000000i64)
    })?;
    let tx = db.transaction()?;
    let exists = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM Application WHERE ApplicationUserModelId=?1)",
        [id],
        |r| r.get::<_, bool>(0),
    )?;
    if !exists {
        if !visible {
            return Ok(());
        }
        let record: ApplicationRecord = serde_json::from_slice(&fs::read(entry_record(id))?)?;
        ensure!(
            record.package == full,
            "The saved application registration belongs to a different package version"
        );
        ensure!(
            tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM Package WHERE _PackageID=?1)",
                [record.package_id],
                |r| r.get::<_, bool>(0)
            )? && (record.activation == 0
                || tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM Activation WHERE _ActivationID=?1)",
                    [record.activation],
                    |r| r.get::<_, bool>(0)
                )?),
            "The saved application registration no longer matches Windows"
        );
        tx.execute_batch(&record.insert)?;
    }
    if !visible {
        save_application(&tx, id, full)?;
    }
    let before = unpack_manifest(&deployment_manifest(&tx, id)?)?;
    let after = entry_manifest(&before, id.rsplit('!').next().unwrap(), visible)?;
    if after != before {
        tx.execute("UPDATE deployment.AppxManifest SET Xml=?1,_Revision=_Revision+1 WHERE Package=(SELECT Package FROM Application WHERE ApplicationUserModelId=?2)", rusqlite::params![pack_manifest(&after)?,id])?;
    }
    if visible {
        tx.execute("UPDATE Application SET AppListEntry=0, _Revision=_Revision+1 WHERE ApplicationUserModelId=?1 AND AppListEntry<>0",[id])?;
        tx.execute("INSERT INTO ApplicationUser (User,ApplicationIdentity,Application,Package,PackageUser) SELECT u.User,i._ApplicationIdentityID,a._ApplicationID,a.Package,u._PackageUserID FROM Application a JOIN ApplicationIdentity i ON i.ApplicationUserModelId=a.ApplicationUserModelId JOIN PackageUser u ON u.Package=a.Package WHERE a.ApplicationUserModelId=?1 AND NOT EXISTS(SELECT 1 FROM ApplicationUser old WHERE old.Application=a._ApplicationID AND old.User=u.User)",[id])?;
    } else {
        tx.execute("DELETE FROM ApplicationUser WHERE Application IN (SELECT _ApplicationID FROM Application WHERE ApplicationUserModelId=?1)",[id])?;
        for (table, filter) in ENTRY_TABLES.iter().rev() {
            tx.execute(&format!("DELETE FROM {table} WHERE {filter}"), [id])?;
        }
    }
    ensure!(
        registered_entries(&tx, id, visible)?
            .iter()
            .all(|v| *v == visible),
        "Application registration did not update"
    );
    tx.commit()?;
    Ok(())
}
pub fn set_app_entry(op: &Operation) -> Result<bool> {
    let visible = bool_value(&op.value);
    let paths = entry_manifests(op)?;
    let full = paths[1].file_stem().unwrap().to_string_lossy();
    let id = format!("{}!{}", op.package_family, op.name);
    let contract = launch_contract(&full, &op.name);
    let hive = entry_record(&id).with_extension("hiv");
    if !visible && reg::exists(&contract)? {
        use windows::{Win32::System::Registry::*, core::PCWSTR};
        let (root, sub) = reg::parse(&contract);
        let key = root.open_subkey(sub)?;
        fs::create_dir_all(hive.parent().unwrap())?;
        if hive.try_exists()? {
            fs::remove_file(&hive)?;
        }
        unsafe {
            RegSaveKeyW(
                HKEY(key.raw_handle()),
                PCWSTR(native::wide(&hive.to_string_lossy()).as_ptr()),
                None,
            )
            .ok()?;
        }
    }
    {
        let _guard = native::trusted_installer()?;
        for path in &paths {
            let text = fs::read_to_string(path)?;
            fs::write(path, entry_manifest(&text, &op.name, visible)?)?;
        }
    }
    {
        let _guard = native::trusted_installer()?;
        set_registered_entry(
            &mut entry_database(true)?,
            &format!("{}!{}", op.package_family, op.name),
            visible,
            &full,
        )?;
        if visible {
            if !reg::exists(&contract)? {
                let record: ApplicationRecord =
                    serde_json::from_slice(&fs::read(entry_record(&id))?)?;
                ensure!(
                    record.package == full,
                    "The saved launch registration belongs to a different package version"
                );
                let source =
                    winreg::RegKey::load_app_key_with_flags(&hive, winreg::enums::KEY_READ, 0)?;
                let (root, sub) = reg::parse(&contract);
                let (target, _) = root.create_subkey(sub)?;
                source.copy_tree("", &target)?;
            }
        } else {
            reg::key(&contract, false)?;
        }
    }
    app_shortcut(op, visible)?;
    preferences::explorer();
    Ok(app_entry(op)? != visible)
}
fn app_shortcut(op: &Operation, visible: bool) -> Result<()> {
    use windows::{
        Win32::{System::Com::*, UI::Shell::*},
        core::{Interface, PCWSTR},
    };
    let id = format!("{}!{}", op.package_family, op.name);
    let record = store::root()
        .join("recovery")
        .join("app-entry-links")
        .join(format!("{id}.json"));
    if !visible {
        if record.try_exists()? {
            let path: PathBuf = serde_json::from_slice(&fs::read(&record)?)?;
            if path.try_exists()? {
                fs::remove_file(path)?;
            }
            fs::remove_file(record)?;
        }
        return Ok(());
    }
    if op.source != "Shortcut" || listed(&id)? {
        return Ok(());
    }
    native::com()?;
    let source: String=entry_database(false)?.query_row("SELECT m.DisplayNameReference FROM MrtApplication m JOIN Application a ON m.Application=a._ApplicationID WHERE a.ApplicationUserModelId=?1",[&id],|r|r.get(0))?;
    let mut title = [0u16; 512];
    unsafe {
        SHLoadIndirectString(PCWSTR(native::wide(&source).as_ptr()), &mut title, None)?;
    }
    let title = String::from_utf16_lossy(
        &title[..title.iter().position(|v| *v == 0).unwrap_or(title.len())],
    );
    let path = native::known_folder(&FOLDERID_CommonPrograms)?.join(format!("{title}.lnk"));
    fs::create_dir_all(path.parent().unwrap())?;
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        let mut pidl = std::ptr::null_mut();
        SHParseDisplayName(
            PCWSTR(native::wide(&format!(r"shell:AppsFolder\{id}")).as_ptr()),
            None,
            &mut pidl,
            0,
            None,
        )?;
        let result = link.SetIDList(pidl);
        CoTaskMemFree(Some(pidl.cast()));
        result?;
        link.cast::<IPersistFile>()?
            .Save(PCWSTR(native::wide(&path.to_string_lossy()).as_ptr()), true)?;
    }
    fs::create_dir_all(record.parent().unwrap())?;
    fs::write(record, serde_json::to_vec(&path)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hiding_one_application_preserves_the_other_manifest_entries() -> Result<()> {
        let original = r#"<Package><Applications><Application Id="Shared"><VisualElements AppListEntry="default" /></Application><Application Id="WindowsBackup"><VisualElements DisplayName="Backup"><Tile /></VisualElements></Application></Applications></Package>"#;
        let hidden = entry_manifest(original, "WindowsBackup", false)?;
        let xml = roxmltree::Document::parse(&hidden)?;
        let apps: Vec<_> = xml
            .descendants()
            .filter(|n| n.has_tag_name("VisualElements"))
            .collect();
        assert_eq!(apps[0].attribute("AppListEntry"), Some("default"));
        assert_eq!(apps[1].attribute("AppListEntry"), Some("none"));
        let shown = entry_manifest(&hidden, "WindowsBackup", true)?;
        let shown_xml = roxmltree::Document::parse(&shown)?;
        let shown_apps: Vec<_> = shown_xml
            .descendants()
            .filter(|n| n.has_tag_name("VisualElements"))
            .collect();
        assert_eq!(shown_apps[1].attribute("AppListEntry"), None);
        assert_eq!(shown_apps[0].attribute("AppListEntry"), Some("default"));
        Ok(())
    }
    #[test]
    fn application_visibility_roundtrip_preserves_other_registrations_and_journals_changes()
    -> Result<()> {
        let mut db = Connection::open_in_memory()?;
        db.execute_batch("CREATE TABLE Application (_ApplicationID INTEGER PRIMARY KEY,ApplicationUserModelId TEXT,AppListEntry INTEGER,_Revision INTEGER,Package INTEGER,Activation INTEGER);
            CREATE TABLE Package (_PackageID INTEGER PRIMARY KEY);
            CREATE TABLE Activation (_ActivationID INTEGER PRIMARY KEY);
            INSERT INTO Package VALUES(1);
            INSERT INTO Activation VALUES(1);
            CREATE TABLE ApplicationIdentity (_ApplicationIdentityID INTEGER PRIMARY KEY,ApplicationUserModelId TEXT);
            CREATE TABLE PackageUser (_PackageUserID INTEGER PRIMARY KEY,User INTEGER,Package INTEGER);
            CREATE TABLE ApplicationUser (User INTEGER,ApplicationIdentity INTEGER,Application INTEGER,Package INTEGER,PackageUser INTEGER);
            CREATE TABLE MrtApplication (_MrtApplicationID INTEGER PRIMARY KEY,Application INTEGER,DisplayNameReference TEXT,Data BLOB);
            CREATE TABLE ApplicationProperty (_ApplicationPropertyID INTEGER PRIMARY KEY,Application INTEGER);
            CREATE TABLE DefaultTile (_DefaultTileID INTEGER PRIMARY KEY,Application INTEGER);
            CREATE TABLE MrtDefaultTile (_MrtDefaultTileID INTEGER PRIMARY KEY,DefaultTile INTEGER);
            CREATE TABLE PrimaryTile (_PrimaryTileID INTEGER PRIMARY KEY,Application INTEGER);
            CREATE TABLE PrimaryTileUser (_PrimaryTileUserID INTEGER PRIMARY KEY,ApplicationIdentity INTEGER,State INTEGER,_Created INTEGER,_Modified INTEGER);
            ATTACH DATABASE ':memory:' AS deployment;
            CREATE TABLE deployment.AppxManifest (Package INTEGER,Xml BLOB,_Revision INTEGER);
            CREATE TABLE SRJournal (WorkId INTEGER,ChangedAt INTEGER);
            INSERT INTO Application VALUES(1,'Backup',0,1,1,1),(2,'Search',0,1,1,1);
            INSERT INTO ApplicationIdentity VALUES(1,'Backup'),(2,'Search');
            INSERT INTO PackageUser VALUES(1,1,1),(2,2,1);
            INSERT INTO ApplicationUser VALUES(1,1,1,1,1),(2,1,1,1,2),(1,2,2,1,1),(2,2,2,1,2);
            INSERT INTO MrtApplication VALUES(1,1,'backup''s name',X'010200'),(2,2,'Search',NULL);
            INSERT INTO PrimaryTile VALUES(1,1),(2,2);
            INSERT INTO PrimaryTileUser VALUES(1,1,0,17,18),(2,2,0,19,20);
            CREATE TRIGGER JournalDelete BEFORE DELETE ON Application WHEN is_srjournal_enabled() BEGIN INSERT INTO SRJournal VALUES(workid(),now()); END;
            CREATE TRIGGER JournalInsert AFTER INSERT ON Application WHEN is_srjournal_enabled() BEGIN INSERT INTO SRJournal VALUES(workid(),now()); END;")?;
        db.execute("INSERT INTO deployment.AppxManifest VALUES(1,?1,1)", [pack_manifest(r#"<Package><Applications><Application Id="Backup"><VisualElements /></Application><Application Id="Search"><VisualElements /></Application></Applications></Package>"#)?])?;
        for visible in [false, true] {
            set_registered_entry(&mut db, "Backup", visible, "Package1")?;
            assert_eq!(
                registered_entries(&db, "Backup", visible)?,
                [visible, visible, visible, visible]
            );
            assert_eq!(
                registered_entries(&db, "Search", true)?,
                [true, true, true, true]
            );
        }
        assert_eq!(
            db.query_row(
                "SELECT _Revision FROM Application WHERE ApplicationUserModelId='Search'",
                [],
                |r| r.get::<_, i32>(0)
            )?,
            1
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM SRJournal WHERE WorkId=0 AND ChangedAt>0",
                [],
                |r| r.get::<_, i32>(0)
            )?,
            2
        );
        assert_eq!(
            db.query_row(
                "SELECT DisplayNameReference,Data FROM MrtApplication WHERE Application=1",
                [],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
            )?,
            ("backup's name".into(), vec![1, 2, 0])
        );
        fs::remove_file(entry_record("Backup"))?;
        Ok(())
    }
    #[test]
    fn compressed_manifest_lengths_roundtrip_at_prefix_boundaries() -> Result<()> {
        for size in [127, 128, 16383, 16384, 254675] {
            let text = " ".repeat(size);
            assert_eq!(unpack_manifest(&pack_manifest(&text)?)?, text);
        }
        assert_eq!(
            &pack_manifest(&" ".repeat(254675))?[..3],
            &[0x0f, 0x45, 0xd3]
        );
        Ok(())
    }
}
