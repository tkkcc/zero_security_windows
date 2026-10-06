use crate::{
    apps, mitigation,
    model::*,
    native, preferences as prefs, registry as reg,
    store::{self, Store},
    tray, workflow,
};
use anyhow::{Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};

type Fact = Arc<OnceLock<std::result::Result<Value, String>>>;
pub struct Engine {
    pub catalog: Vec<Feature>,
    pub store: Store,
    pub boot: u64,
    pub safe: bool,
    pub zh: bool,
    pub boot_entry: Mutex<String>,
    facts: Mutex<HashMap<&'static str, Fact>>,
}
pub fn matches(text: &str, pattern: &str) -> bool {
    globset::GlobBuilder::new(pattern)
        .case_insensitive(true)
        .backslash_escape(false)
        .build()
        .is_ok_and(|g| g.compile_matcher().is_match(text))
}
impl Engine {
    pub fn new() -> Result<Self> {
        let store = Store::new()?;
        let boot_entry = store
            .pending
            .lock()
            .unwrap()
            .as_ref()
            .map(|p| p.normal_entry.clone())
            .unwrap_or("{current}".into());
        Ok(Self {
            catalog: catalog()?,
            store,
            boot: native::boot(),
            safe: native::safe_mode(),
            zh: native::chinese(),
            boot_entry: Mutex::new(boot_entry),
            facts: Mutex::new(HashMap::new()),
        })
    }
    pub fn fact(&self, key: &'static str, read: impl FnOnce() -> Result<Value>) -> Result<Value> {
        let cell = self.facts.lock().unwrap().entry(key).or_default().clone();
        match cell.get_or_init(|| read().map_err(|e| format!("{e:#}"))) {
            Ok(v) => Ok(v.clone()),
            Err(e) => bail!("{e}"),
        }
    }
    pub fn invalidate(&self, f: &Feature) {
        let mut facts = self.facts.lock().unwrap();
        for op in &f.ops {
            facts.remove(match op.kind.as_str() {
                "Apps" => "apps",
                "OneDrive" => "tools",
                "TaskGroup" => "tasks",
                "Bcd" => "bcd",
                "Mitigations" => "mitigations",
                _ => "",
            });
        }
        if f.group_zh == "Defender" {
            facts.remove("mp");
        }
        if matches!(
            f.probe.as_str(),
            "vbs" | "hvci" | "credential" | "kernel-cet" | "secure-launch"
        ) {
            facts.remove("dg");
        }
    }
    fn remember_tool(&self, name: &str, installed: bool) {
        let mut facts = self.facts.lock().unwrap();
        let Some(Ok(value)) = facts.get("tools").and_then(|cell| cell.get()) else {
            facts.remove("tools");
            return;
        };
        let mut tools = value.as_array().unwrap().clone();
        tools.retain(|v| !v.as_str().unwrap().eq_ignore_ascii_case(name));
        if installed {
            tools.push(json!(name));
        }
        facts.insert("tools", Arc::new(OnceLock::from(Ok(json!(tools)))));
    }
    pub fn feature(&self, id: &str) -> Result<&Feature> {
        self.catalog
            .iter()
            .find(|f| f.id == id)
            .ok_or_else(|| anyhow::anyhow!("Unknown item: {id}"))
    }
    pub fn begin_batch(&self) {
        self.facts.lock().unwrap().clear();
    }
    pub fn needs_safe(&self, f: &Feature) -> Result<bool> {
        if self.safe {
            return Ok(false);
        }
        if f.group_zh == "Defender" && !native::service_running("WinDefend")? {
            return Ok(false);
        }
        if matches!(
            f.id.as_str(),
            "search-highlights" | "widgets" | "taskbar-buttons" | "userchoice-protection"
        ) {
            return native::service_running("UCPD");
        }
        if f.id == "process-mitigations"
            && reg::exists(
                r"HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options\DefenderAgentScan.exe",
            )?
        {
            return native::service_running("WinDefend");
        }
        Ok(f.safe)
    }
    pub fn expand(&self, f: &Feature) -> Result<Vec<Operation>> {
        let mut out = vec![];
        for op in &f.ops {
            if self.safe && matches!(op.kind.as_str(), "Task" | "TaskGroup") {
                continue;
            }
            match op.kind.as_str() {
                "Service" => {
                    if reg::exists(&format!(
                        r"HKLM:\SYSTEM\CurrentControlSet\Services\{}",
                        op.name
                    ))? {
                        out.push(service_op(&op.name))
                    }
                }
                "UserServices" => {
                    for name in reg::children(r"HKLM:\SYSTEM\CurrentControlSet\Services")? {
                        if (name.eq_ignore_ascii_case(&op.name)
                            || name
                                .to_lowercase()
                                .starts_with(&(op.name.to_lowercase() + "_")))
                            && reg::number(
                                &format!(r"HKLM:\SYSTEM\CurrentControlSet\Services\{name}"),
                                "Type",
                                0,
                            )? & 64
                                != 0
                        {
                            out.push(service_op(&name))
                        }
                    }
                }
                "TaskGroup" => {
                    if !self.safe {
                        let tasks = self.fact("tasks", || Ok(json!(native::tasks()?)))?;
                        for path in tasks.as_array().unwrap() {
                            let path = path.as_str().unwrap();
                            if matches(path, &op.pattern) {
                                out.push(Operation {
                                    kind: "Task".into(),
                                    path: path.into(),
                                    value: json!(false),
                                    ..op.clone()
                                })
                            }
                        }
                    }
                }
                "Asr" => {
                    let mut names = op.names.clone();
                    names.extend(reg::names(&op.path)?);
                    names.sort();
                    names.dedup();
                    for name in names {
                        out.push(Operation {
                            value_type: "String".into(),
                            ..Operation::reg(&op.path, name, "0")
                        })
                    }
                }
                "UpdatePause" => out.extend(update_pause()),
                "DesktopIcons" => out.extend(desktop_icons()?),
                "Power" => out.push(Operation {
                    scheme: prefs::scheme()?,
                    ..op.clone()
                }),
                _ => {
                    let mut expanded = Operation {
                        path: native::expand(&op.path).replace("%UserSid%", &self.store.sid),
                        ..op.clone()
                    };
                    if f.id == "visual-effects"
                        && op.name == "UserPreferencesMask"
                        && let Some(old) = reg::read(&op.path, &op.name)?.as_array()
                        && old.len() == 8
                    {
                        let mut value = op.value.as_array().unwrap().clone();
                        value[5..].clone_from_slice(&old[5..]);
                        expanded.value = json!(value)
                    }
                    out.push(expanded)
                }
            }
        }
        Ok(out)
    }
    pub fn read(&self, op: &Operation) -> Result<Value> {
        Ok(match op.kind.as_str() {
            "RegistryKey" => json!(reg::exists(&op.path)?),
            "Registry" | "RegistryDelete" | "Service" => reg::read(&op.path, &op.name)?,
            "Bcd" => {
                let bcd = self.fact("bcd", || Ok(json!(native::command("bcdedit.exe", &["/enum", &self.boot_entry.lock().unwrap()])?)))?;
                bcd.as_str().unwrap().lines()
                    .filter_map(|s| s.split_once(char::is_whitespace))
                    .find(|(key, _)| key == &op.name)
                    .map(|(_, value)| json!(value.trim()))
                    .unwrap_or(Value::Null)
            }
            "Firewall" => json!(native::firewall(&op.name, op.property == "NotifyOnListen", None)?),
            "Task" => json!(native::task_enabled(&op.path)?),
            "Apps" => {
                let entries: Vec<apps::AppEntry> = serde_json::from_value(self.fact("apps", || Ok(serde_json::to_value(apps::inventory()?)?))?)?;
                json!(!apps::matching(&entries, op).is_empty())
            }
            "Winget" => {
                if let Some(installed) = apps::registered_tool(op)? {
                    return Ok(json!(installed));
                }
                let tools = self.fact("tools", apps::tools)?;
                json!(tools.as_array().unwrap().iter().any(|v| v.as_str().is_some_and(|v| v.eq_ignore_ascii_case(&op.name))))
            }
            "OneDrive" => {
                let tools = self.fact("tools", apps::tools)?;
                json!(tools.as_array().unwrap().iter().any(|v| v.as_str() == Some("Microsoft.OneDrive"))
                    || !reg::read(r"HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Run", "OneDrive")?.is_null()
                    || !reg::read(r"HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Run", "OneDrive")?.is_null())
            }
            "CopilotDesktop" => json!(!apps::copilot_commands()?.is_empty()),
            "CompatibilityDatabase" => json!(!reg::read(&format!(r"HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\InstalledSDB\{}", op.name), "DatabasePath")?.is_null()),
            "InputMethods" => json!(if prefs::input_only()? { "WeTypeOnly" } else { "Other" }),
            "LanguageBar" => json!(if prefs::language_bar(None)? & 2063 == 8 && reg::number(r"HKCU:\Software\Microsoft\CTF\LangBar", "ShowStatus", 0)? == 3 { "Hidden" } else { "Other" }),
            "SettingsPage" => json!(prefs::settings_page(&op.name, false)?),
            "Power" => json!(prefs::power(&op.scheme, &op.group, &op.name, op.source == "AC", None)?),
            "Hibernate" => json!(reg::number(r"HKLM:\SYSTEM\CurrentControlSet\Control\Power", "HibernateEnabled", 0)?),
            "OptionalFeature" => {
                let (code, text) = native::run("dism.exe", &["/Online", "/English", "/Get-FeatureInfo", &format!("/FeatureName:{}", op.name)])?;
                if code as u32 == 0x800f080c { return Ok(json!(false)); }
                ensure!(code == 0, "DISM ({code}): {text}");
                json!(text.lines().any(|line| line.trim() == "State : Enabled" || line.trim() == "State : Enable Pending"))
            }
            "ReservedStorage" => {
                let (code, text) = native::run("dism.exe", &["/Online", "/English", "/Get-ReservedStorageState"])?;
                if code != 0 {
                    return Err(native::error(code as u32).context(format!("DISM ({code}): {text}")));
                }
                json!(if text.to_ascii_lowercase().contains("disabled") { "Disabled" } else { "Enabled" })
            }
            "ProcessBlock" => json!(same(&reg::read(&block_path(&op.name), "Debugger")?, &json!(block_command())) && !native::process_running(&op.name)?),
            "ResumeAccess" => json!(!PathBuf::from(&op.path).exists() || (!native::execute_blocked(&PathBuf::from(&op.path))? && !native::process_running(&op.name)?)),
            "UpdateService" => json!(native::service_running(&op.name)? || reg::number(&format!(r"HKLM:\SYSTEM\CurrentControlSet\Services\{}", op.name), "Start", 4)? < 3),
            "DesktopFiles" => json!(desktop_items()?.is_empty()),
            "StartPins" => {
                let file = store::root().join("start-pins.json");
                json!(same(&reg::read(r"HKCU:\SOFTWARE\Policies\Microsoft\Windows\Explorer", "ConfigureStartPins")?, &json!(1))
                    && same(&reg::read(r"HKCU:\SOFTWARE\Policies\Microsoft\Windows\Explorer", "ConfigureStartPinsJSON")?, &json!(file.to_string_lossy()))
                    && file.exists() && std::fs::read_to_string(file)? == PINS)
            }
            "TrayIcons" => json!(tray::running()? && same(&reg::read(r"HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Run", "ZeroSecurityWindows.Tray")?, &json!(tray::command()))),
            "Mitigations" => Value::Null,
            _ => bail!("Unknown operation: {}", op.kind),
        })
    }
    pub fn write(&self, op: &Operation) -> Result<bool> {
        match op.kind.as_str() {
            "Registry" | "RegistryDelete" => reg::write(op)?,
            "RegistryKey" => reg::key(&op.path, bool_value(&op.value))?,
            "Bcd" => {
                native::command(
                    "bcdedit.exe",
                    &[
                        "/set",
                        &self.boot_entry.lock().unwrap(),
                        &op.name,
                        &text_value(&op.value),
                    ],
                )?;
                return Ok(true);
            }
            "Firewall" => {
                native::firewall(
                    &op.name,
                    op.property == "NotifyOnListen",
                    Some(bool_value(&op.value)),
                )?;
            }
            "Task" => native::disable_task(&op.path)?,
            "Service" | "UpdateService" => {
                let name = if op.kind == "Service" {
                    &op.service
                } else {
                    &op.name
                };
                let path = format!(r"HKLM:\SYSTEM\CurrentControlSet\Services\{name}");
                let kind = reg::number(&path, "Type", 0)?;
                let mut restart = false;
                if kind & 64 == 0 || kind & 128 != 0 {
                    restart = native::disable_service(name)?
                }
                reg::set(&path, "Start", 4, "DWord")?;
                return Ok(if op.kind == "UpdateService" {
                    native::service_running(name)? || reg::number(&path, "Start", 4)? < 3
                } else {
                    restart
                });
            }
            "Apps" => return apps::remove(op),
            "Winget" => {
                let installed = bool_value(&op.value);
                match apps::install(op, !installed) {
                    Ok(restart) => {
                        self.remember_tool(&op.name, installed);
                        return Ok(restart);
                    }
                    Err(error) => {
                        self.facts.lock().unwrap().remove("tools");
                        return Err(error);
                    }
                }
            }
            "OneDrive" => {
                let mut restart = false;
                if apps::has_tool("Microsoft.OneDrive")? {
                    restart = apps::install(
                        &Operation {
                            name: "Microsoft.OneDrive".into(),
                            ..op.clone()
                        },
                        true,
                    )?
                }
                for path in [
                    r"HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Run",
                    r"HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Run",
                ] {
                    reg::delete(path, "OneDrive")?
                }
                return Ok(restart);
            }
            "CopilotDesktop" => return apps::remove_copilot(),
            "CompatibilityDatabase" => {
                native::command("sdbinst.exe", &["-q", "-u", "-g", &op.name])?;
            }
            "InputMethods" => prefs::apply_input()?,
            "LanguageBar" => {
                reg::set(
                    r"HKCU:\Software\Microsoft\CTF\LangBar",
                    "ShowStatus",
                    3,
                    "DWord",
                )?;
                reg::set(
                    r"HKCU:\Software\Microsoft\CTF\LangBar",
                    "ExtraIconsOnMinimized",
                    0,
                    "DWord",
                )?;
                prefs::language_bar(Some(8))?;
            }
            "SettingsPage" => {
                prefs::settings_page(&op.name, true)?;
            }
            "Power" => {
                prefs::power(
                    &op.scheme,
                    &op.group,
                    &op.name,
                    op.source == "AC",
                    Some(op.value.as_u64().unwrap() as u32),
                )?;
            }
            "Hibernate" => {
                native::command("powercfg.exe", &["/hibernate", "off"])?;
            }
            "OptionalFeature" => {
                return native::dism(&[
                    "/Online",
                    "/English",
                    "/Disable-Feature",
                    &format!("/FeatureName:{}", op.name),
                    "/NoRestart",
                ]);
            }
            "ReservedStorage" => {
                return native::dism(&[
                    "/Online",
                    "/English",
                    "/Set-ReservedStorageState",
                    "/State:Disabled",
                ]);
            }
            "ProcessBlock" => {
                self.store.copy_executable()?;
                reg::set(&block_path(&op.name), "Debugger", block_command(), "String")?;
                native::stop_process(&op.name)?;
            }
            "ResumeAccess" => {
                if PathBuf::from(&op.path).exists() {
                    native::clear_execute_deny(&PathBuf::from(&op.path))?;
                    native::stop_process(&op.name)?;
                }
            }
            "DesktopFiles" => prefs::recycle(&desktop_items()?)?,
            "StartPins" => {
                let file = store::root().join("start-pins.json");
                std::fs::write(&file, PINS)?;
                reg::set(
                    r"HKCU:\SOFTWARE\Policies\Microsoft\Windows\Explorer",
                    "ConfigureStartPinsJSON",
                    file.to_string_lossy().as_ref(),
                    "ExpandString",
                )?;
                reg::set(
                    r"HKCU:\SOFTWARE\Policies\Microsoft\Windows\Explorer",
                    "ConfigureStartPins",
                    1,
                    "DWord",
                )?;
            }
            "TrayIcons" => tray::enable()?,
            "Mitigations" => mitigation::disable()?,
            _ => bail!("Unknown operation: {}", op.kind),
        }
        Ok(false)
    }
    pub fn refresh(&self, f: &Feature) -> Result<()> {
        if self.safe {
            return Ok(());
        }
        match f.refresh.as_str() {
            "Mouse" => prefs::apply_mouse()?,
            "Keyboard" => prefs::apply_keyboard()?,
            "Visuals" => prefs::visuals()?,
            "Cursors" => prefs::cursors()?,
            "Appearance" => prefs::broadcast("ImmersiveColorSet"),
            "Power" => prefs::activate_power()?,
            "Explorer" | "Shell" => prefs::explorer(),
            "Wallpaper" => prefs::wallpaper()?,
            "Start" => native::stop_process("StartMenuExperienceHost.exe")?,
            _ => {}
        }
        Ok(())
    }
    pub fn execute(
        &self,
        f: &Feature,
        win11: bool,
        tasks_only: bool,
        settings_only: bool,
    ) -> Result<ResultRecord> {
        self.store.initialize()?;
        let mut result = if settings_only {
            self.store
                .results
                .lock()
                .unwrap()
                .get(&f.id)
                .cloned()
                .unwrap_or_default()
        } else {
            ResultRecord {
                boot: self.boot,
                ..Default::default()
            }
        };
        let mut ops = self.expand(f)?;
        if f.toggle() && win11 {
            let path = &f.ops[0].path;
            ops = vec![path.clone(), path.rsplit_once('\\').unwrap().0.into()]
                .into_iter()
                .map(|path| Operation {
                    kind: "RegistryKey".into(),
                    path,
                    value: json!(false),
                    ..Default::default()
                })
                .collect();
        }
        if ops.iter().any(|op| op.kind == "Bcd") {
            workflow::suspend(self, if self.safe { 1 } else { 2 })?;
        }
        for op in ops.into_iter().filter(|op| {
            (!tasks_only || op.kind == "Task") && (!settings_only || op.kind != "Task")
        }) {
            let applied = (|| -> Result<()> {
                let before = self.read(&op)?;
                if op.kind != "Mitigations"
                    && satisfied(&op, &before)
                    && !(op.kind == "Service" && native::service_running(&op.service)?)
                {
                    return Ok(());
                }
                let restart = self.write(&op)?;
                result.restart |= restart;
                result.changed = true;
                self.invalidate(f);
                if op.kind != "Mitigations" && !restart {
                    ensure!(
                        satisfied(&op, &self.read(&op)?),
                        "Setting did not match after writing: {} {}",
                        op.kind,
                        op.name
                    )
                }
                self.store.log(&f.id, &op, "")?;
                Ok(())
            })();
            if let Err(e) = applied {
                let error = format!("{e:#}");
                self.store.log(&f.id, &op, &error)?;
                result.errors.push(error)
            }
        }
        if !tasks_only {
            if let Err(e) = self.refresh(f) {
                let error = format!("{e:#}");
                self.store.log(
                    &f.id,
                    &Operation {
                        kind: "Refresh".into(),
                        ..Default::default()
                    },
                    &error,
                )?;
                result.errors.push(error)
            }
            if result.changed {
                result.restart |= f.restart;
                if !self.safe && matches!(f.refresh.as_str(), "Shell" | "Wallpaper") {
                    result.shell = native::shell_stamp()?
                }
            }
        }
        self.invalidate(f);
        self.store
            .results
            .lock()
            .unwrap()
            .insert(f.id.clone(), result.clone());
        if !tasks_only
            && !(f.toggle() && win11)
            && !result.restart
            && result.errors.is_empty()
            && self.check(f).state == Status::Ready
        {
            let error = "The requested runtime state was not reached".to_owned();
            result.errors.push(error.clone());
            self.store.log(
                &f.id,
                &Operation {
                    kind: "Verify".into(),
                    ..Default::default()
                },
                &error,
            )?;
            self.store
                .results
                .lock()
                .unwrap()
                .insert(f.id.clone(), result.clone());
        }
        self.store.save()?;
        Ok(result)
    }
}
pub fn satisfied(op: &Operation, v: &Value) -> bool {
    if op.kind == "RegistryDelete" {
        v.is_null()
    } else {
        same(v, &reg::desired(op))
    }
}
fn service_op(name: &str) -> Operation {
    Operation {
        kind: "Service".into(),
        service: name.into(),
        ..Operation::reg(
            format!(r"HKLM:\SYSTEM\CurrentControlSet\Services\{name}"),
            "Start",
            4,
        )
    }
}
const UPDATES: &str = r"HKLM:\SOFTWARE\Microsoft\WindowsUpdate\UX\Settings";
pub fn update_pause() -> Vec<Operation> {
    let now = chrono::Utc::now();
    let mut out = vec![Operation::reg(UPDATES, "FlightSettingsMaxPauseDays", 7000)];
    for (names, date) in [
        (
            vec![
                "PauseUpdatesStartTime",
                "PauseFeatureUpdatesStartTime",
                "PauseQualityUpdatesStartTime",
            ],
            now,
        ),
        (
            vec![
                "PauseUpdatesExpiryTime",
                "PauseFeatureUpdatesEndTime",
                "PauseQualityUpdatesEndTime",
            ],
            now + chrono::Duration::days(7000),
        ),
    ] {
        for name in names {
            out.push(Operation {
                value_type: "String".into(),
                ..Operation::reg(UPDATES, name, date.format("%Y-%m-%dT%H:%M:%SZ").to_string())
            })
        }
    }
    out
}
fn desktop_icons() -> Result<Vec<Operation>> {
    let show = [
        "{20D04FE0-3AEA-1069-A2D8-08002B30309D}",
        "{645FF040-5081-101B-9F08-00AA002F954E}",
    ];
    let mut hide = vec![
        "{59031a47-3f72-44a7-89c5-5595fe6b30ee}".into(),
        "{F02C1A0D-BE21-4350-88B0-7367FC96EF3C}".into(),
        "{5399E694-6CE5-4D6C-8FCE-1D8870FDCBA0}".into(),
        "{2cc5ca98-6485-489a-920e-b3e88a6ccce3}".into(),
    ];
    for hive in ["HKLM:", "HKCU:"] {
        for name in reg::children(&format!(
            r"{hive}\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\Desktop\NameSpace"
        ))? {
            if !show.iter().any(|v| v.eq_ignore_ascii_case(&name)) && prefs::guid(&name).is_ok() {
                hide.push(name)
            }
        }
    }
    hide.sort();
    hide.dedup();
    let mut out = vec![];
    for panel in ["NewStartPanel", "ClassicStartMenu"] {
        let path = format!(
            r"HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\HideDesktopIcons\{panel}"
        );
        for name in show {
            out.push(Operation::reg(&path, name, 0))
        }
        for name in &hide {
            out.push(Operation::reg(&path, name, 1))
        }
    }
    Ok(out)
}
pub fn desktop_items() -> Result<Vec<PathBuf>> {
    use windows::Win32::UI::Shell::{FOLDERID_Desktop, FOLDERID_PublicDesktop};
    let mut out = vec![];
    for id in [FOLDERID_Desktop, FOLDERID_PublicDesktop] {
        let path = native::known_folder(&id)?;
        if path.exists() {
            for entry in std::fs::read_dir(path)? {
                let entry = entry?;
                if !entry
                    .file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case("desktop.ini")
                {
                    out.push(entry.path())
                }
            }
        }
    }
    Ok(out)
}
const PINS: &str = r#"{"pinnedList":[],"applyOnce":true}"#;
fn block_path(name: &str) -> String {
    format!(
        r"HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options\{name}"
    )
}
fn block_command() -> String {
    format!(
        "\"{}\" --blocked",
        store::root().join("zero_security_windows.exe").display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parallel_verified_installs_update_one_inventory_without_losing_entries() -> Result<()> {
        let engine = Engine::new()?;
        engine.fact("tools", || Ok(json!(["Existing.App"])))?;
        std::thread::scope(|scope| {
            for name in ["First.App", "Second.App"] {
                let engine = &engine;
                scope.spawn(move || engine.remember_tool(name, true));
            }
        });
        let tools = engine.fact("tools", || anyhow::bail!("unexpected inventory refresh"))?;
        let tools = tools.as_array().unwrap();
        assert_eq!(tools.len(), 3);
        for name in ["Existing.App", "First.App", "Second.App"] {
            assert!(tools.contains(&json!(name)));
        }
        engine.remember_tool("First.App", false);
        assert_eq!(
            engine
                .fact("tools", || anyhow::bail!("unexpected refresh"))?
                .as_array()
                .unwrap()
                .len(),
            2
        );
        Ok(())
    }
}
