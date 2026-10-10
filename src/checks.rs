use crate::{
    engine::{Engine, satisfied},
    mitigation,
    model::*,
    native, preferences as prefs, registry as reg, store,
    workflow::Phase,
};
use anyhow::Result;
use serde_json::json;

fn check_error(error: &anyhow::Error) -> Check {
    Check {
        state: if error.chain().any(|cause| {
            cause
                .downcast_ref::<std::io::Error>()
                .and_then(|e| e.raw_os_error())
                .is_some_and(|code| matches!(code as u32, 5 | 1314 | 0x80070005 | 0x80070522))
                || cause
                    .downcast_ref::<windows::core::Error>()
                    .is_some_and(|e| matches!(e.code().0 as u32, 0x80070005 | 0x80070522))
        }) {
            Status::Restricted
        } else {
            Status::Unknown
        },
        detail: format!("{error:#}"),
    }
}

impl Engine {
    pub fn check(&self, f: &Feature) -> Check {
        let mut check = match self.check_inner(f) {
            Ok(v) => v,
            Err(e) => {
                let check = check_error(&e);
                let _ = store::append(
                    "checks.jsonl",
                    &json!({"Time":chrono::Local::now().to_rfc3339(),"Id":f.id,"Error":check.detail}).to_string(),
                );
                check
            }
        };
        if let Some(result) = self.store.results.lock().unwrap().get_mut(&f.id) {
            if matches!(check.state, Status::Done | Status::Absent) {
                result.errors.clear();
            }
            if !result.errors.is_empty() && check.state != Status::Restricted {
                check = Check {
                    state: Status::Failed,
                    detail: result.errors.join("; "),
                }
            } else if result.restart && result.boot == self.boot {
                check.state = Status::Restart;
            } else if result.logon != 0
                && check.state != Status::Restart
                && result.boot == self.boot
                && native::logon().ok() == Some(result.logon)
            {
                check.state = Status::SignIn;
            } else if result.shell != 0 && native::shell_stamp().ok() == Some(result.shell) {
                check = Check {
                    state: Status::SignIn,
                    detail: if f.toggle() {
                        if check.state == Status::Done {
                            "Windows 10".into()
                        } else {
                            "Windows 11".into()
                        }
                    } else {
                        check.detail
                    },
                }
            }
        }
        if let Some(p) = self.store.pending.lock().unwrap().as_ref()
            && !self.safe
            && p.phase == Phase::AwaitSafe
            && p.safe_ids.contains(&f.id)
        {
            check = Check::new(Status::SafeQueued)
        }
        check
    }
    fn check_inner(&self, f: &Feature) -> Result<Check> {
        if f.manual {
            return Ok(Check::new(Status::Absent));
        }
        if self.safe && !f.can_run_safe() {
            return Ok(Check::new(Status::Deferred));
        }
        if f.group_zh == "Defender"
            && !matches!(f.id.as_str(), "defender" | "defender-tasks")
            && !reg::exists(r"HKLM:\SYSTEM\CurrentControlSet\Services\WinDefend")?
        {
            return Ok(Check::new(Status::Absent));
        }
        if let Some(check) = if self.safe && f.can_run_safe() {
            None
        } else {
            self.probe(f)?
        } {
            if check.state != Status::Done {
                return Ok(check);
            }
            if !f
                .ops
                .iter()
                .any(|op| matches!(op.kind.as_str(), "Service" | "UserServices"))
            {
                return Ok(check);
            }
            for op in self.expand(f)?.iter().filter(|op| op.kind == "Service") {
                if native::service_running(&op.service)? {
                    return Ok(Check::new(Status::Ready));
                }
            }
            return Ok(check);
        }
        let ops = self.expand(f)?;
        if ops.is_empty() {
            return Ok(Check::new(Status::Absent));
        }
        let mut done = true;
        let mut running_until_restart = false;
        for op in ops {
            let value = self.read(&op)?;
            if !satisfied(&op, &value)
                || (op.kind == "Service" && native::service_running(&op.service)?)
            {
                done = false
            }
            if satisfied(&op, &value)
                && op.value == json!(4)
                && matches!(op.kind.as_str(), "ServiceStart" | "UserServiceStart")
                && native::service_running(if op.kind == "ServiceStart" {
                    &op.name
                } else {
                    &op.service
                })?
            {
                running_until_restart = true;
            }
        }
        if done && running_until_restart {
            return Ok(Check {
                state: Status::Restart,
                detail: choose(
                    self.zh,
                    "服务已设为禁用，当前实例仍在运行。",
                    "The service is set to Disabled; its current instance is still running.",
                )
                .into(),
            });
        }
        Ok(Check::active(!done))
    }
    fn probe(&self, f: &Feature) -> Result<Option<Check>> {
        let active = match f.probe.as_str() {
            "app-preloading" => {
                let state = native::memory_agent(false)?;
                let mut active = state.app_preloading() || !native::service_running("SysMain")?;
                for op in f.ops.iter().filter(|op| op.kind != "AppPreloading") {
                    active |= !satisfied(op, &self.read(op)?);
                }
                return Ok(Some(Check {
                    state: if active { Status::Ready } else { Status::Done },
                    detail: format!(
                        "{}{}{}{}{}",
                        choose(self.zh, "当前内存压缩：", "Memory compression: "),
                        choose(
                            self.zh,
                            if state.memory_compression {
                                "开启"
                            } else {
                                "关闭"
                            },
                            if state.memory_compression {
                                "on"
                            } else {
                                "off"
                            }
                        ),
                        choose(self.zh, "；内存页合并：", "; page combining: "),
                        choose(
                            self.zh,
                            if state.page_combining {
                                "开启"
                            } else {
                                "关闭"
                            },
                            if state.page_combining { "on" } else { "off" }
                        ),
                        choose(self.zh, "。", "."),
                    ),
                }));
            }
            "dep" => {
                for op in self.expand(f)? {
                    if !satisfied(&op, &self.read(&op)?) {
                        return Ok(Some(Check::new(Status::Ready)));
                    }
                }
                let (policy, enabled) = native::dep()?;
                return Ok(Some(Check {
                    state: if policy == 0 {
                        Status::Done
                    } else {
                        Status::Restart
                    },
                    detail: if policy != 0 {
                        choose(self.zh, "启动参数已设为关闭；当前系统 DEP 策略仍开启。", "The boot option is set to off; the current system DEP policy is still enabled.")
                    } else if enabled {
                        choose(self.zh, "启动策略已关闭；当前 64 位进程的 DEP 仍开启。", "The boot policy is off; DEP remains enabled for the current 64-bit process.")
                    } else {
                        choose(self.zh, "DEP 启动策略及当前进程 DEP 已关闭。", "The DEP boot policy and current process DEP are off.")
                    }.into(),
                }));
            }
            "lsa" | "driver-signing" => {
                for op in self.expand(f)? {
                    if !satisfied(&op, &self.read(&op)?) {
                        return Ok(Some(Check::new(Status::Ready)));
                    }
                }
                let running = if f.probe == "lsa" {
                    native::lsa_protected()?
                } else {
                    native::code_integrity()?
                };
                let mut check = Check::new(Status::Done);
                if running {
                    check.detail = if f.probe == "lsa" {
                        choose(self.zh, "启动保护已设为关闭；当前登录安全进程仍以受保护模式运行。", "Startup protection is set to off; the sign-in security process is still running in protected mode.")
                    } else {
                        choose(self.zh, "启动参数已设为关闭；内核代码完整性检查仍开启。", "The boot option is set to off; kernel code integrity checks remain enabled.")
                    }.into();
                }
                return Ok(Some(check));
            }
            "taskbar-pins" => {
                let pins = prefs::taskbar_pin_names()?;
                let mut active = !pins.is_empty();
                for op in &f.ops {
                    if op.kind != "TaskbarPins" {
                        active |= !satisfied(op, &self.read(op)?);
                    }
                }
                let mut check = Check::active(active);
                check.detail = if pins.is_empty() {
                    choose(
                        self.zh,
                        "Windows 未保存任务栏固定项。",
                        "Windows has no saved taskbar pins.",
                    )
                    .into()
                } else {
                    format!(
                        "{} {}{}{}",
                        choose(self.zh, "Windows 保存了", "Windows saved"),
                        pins.len(),
                        choose(self.zh, " 个固定项：", " taskbar pins: "),
                        pins.join(choose(self.zh, "、", ", ")),
                    )
                };
                return Ok(Some(check));
            }
            "vbs" | "hvci" | "credential" | "kernel-cet" | "secure-launch" => {
                let rows = self.fact("dg", || {
                    native::wmi(
                        r"root\Microsoft\Windows\DeviceGuard",
                        "SELECT * FROM Win32_DeviceGuard",
                    )
                })?;
                let dg = rows
                    .as_array()
                    .and_then(|v| v.first())
                    .ok_or_else(|| anyhow::anyhow!("No Device Guard status"))?;
                if f.probe == "vbs" {
                    dg["VirtualizationBasedSecurityStatus"]
                        .as_u64()
                        .unwrap_or(0)
                        != 0
                } else {
                    let code = match f.probe.as_str() {
                        "hvci" => 2,
                        "credential" => 1,
                        "kernel-cet" => 5,
                        _ => 3,
                    };
                    ["SecurityServicesRunning", "SecurityServicesConfigured"]
                        .iter()
                        .any(|key| {
                            dg[*key]
                                .as_array()
                                .is_some_and(|a| a.contains(&json!(code)))
                        })
                }
            }
            "defender" | "realtime" | "tamper" => {
                if !reg::exists(r"HKLM:\SYSTEM\CurrentControlSet\Services\WinDefend")? {
                    return Ok(if f.id == "defender" {
                        None
                    } else {
                        Some(Check::new(Status::Absent))
                    });
                }
                if self.safe {
                    return Ok(None);
                }
                if !native::service_running("WinDefend")? {
                    return Ok(Some(if f.probe == "tamper" {
                        Check {
                            state: Status::Absent,
                            detail: choose(self.zh, "Defender 未运行，篡改防护无需单独处理。", "Defender is not running; tamper protection needs no separate action.").into(),
                        }
                    } else {
                        Check::new(Status::Done)
                    }));
                }
                let rows = self.fact("mp", || {
                    native::wmi(
                        r"root\Microsoft\Windows\Defender",
                        "SELECT * FROM MSFT_MpComputerStatus",
                    )
                })?;
                let mp = rows
                    .as_array()
                    .and_then(|a| a.first())
                    .ok_or_else(|| anyhow::anyhow!("No Defender status"))?;
                mp[match f.probe.as_str() {
                    "defender" => "AntivirusEnabled",
                    "realtime" => "RealTimeProtectionEnabled",
                    _ => "IsTamperProtected",
                }]
                .as_bool()
                .ok_or_else(|| anyhow::anyhow!("Invalid Defender status"))?
            }
            "uac" => {
                reg::number(
                    r"HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System",
                    "EnableLUA",
                    1,
                )? != 0
            }
            "phishing" => {
                let background = self.check_inner(self.feature("phishing-services")?)?;
                if matches!(background.state, Status::Done | Status::Absent) {
                    return Ok(Some(Check {
                        state: background.state,
                        detail: if background.state == Status::Done {
                            choose(self.zh, "钓鱼防护后台已禁用且未运行，密码安全提醒无需单独处理。", "Phishing protection background services are disabled and stopped; password warnings need no separate action.").into()
                        } else {
                            String::new()
                        },
                    }));
                }
                let op = &f.ops[0];
                return Ok(Some(Check::active(!satisfied(op, &self.read(op)?))));
            }
            "cpu" => {
                let cpu = native::query_flags(201)?;
                let kva = native::query_flags(196)?;
                (cpu[0] & (1 | 1024 | 0x2000000 | 0x80000)) != 0
                    || (cpu[1] & (8 | 32)) != 0
                    || (kva[0] & 1) != 0
            }
            "mitigations" => {
                if self
                    .fact("mitigations", || Ok(json!(mitigation::active()?)))?
                    .as_bool()
                    .unwrap()
                {
                    return Ok(Some(Check::new(Status::Ready)));
                }
                let runtime = mitigation::runtime(self.zh)?;
                return Ok(Some(Check {
                    state: Status::Done,
                    detail: if runtime.is_empty() {
                        String::new()
                    } else {
                        format!(
                            "{}{}",
                            choose(
                                self.zh,
                                "启动配置已关闭。仍启用的防护（数量为已读取的 Windows 进程数）：\n",
                                "Startup settings are off. Retained protections (counts are inspected Windows processes):\n"
                            ),
                            runtime
                                .iter()
                                .map(|(name, count)| format!(
                                    "{name}{}{count}",
                                    choose(self.zh, "：", ": ")
                                ))
                                .collect::<Vec<_>>()
                                .chunks(2)
                                .map(|pair| pair.join(choose(self.zh, "；", "; ")))
                                .collect::<Vec<_>>()
                                .join("\n")
                        )
                    },
                }));
            }
            "updates" => {
                let path = r"HKLM:\SOFTWARE\Microsoft\WindowsUpdate\UX\Settings";
                let mut ends = vec![];
                for name in [
                    "PauseUpdatesExpiryTime",
                    "PauseFeatureUpdatesEndTime",
                    "PauseQualityUpdatesEndTime",
                ] {
                    let v = reg::read(path, name)?;
                    let Some(s) = v.as_str() else {
                        return Ok(Some(Check::new(Status::Ready)));
                    };
                    ends.push(chrono::DateTime::parse_from_rfc3339(s)?)
                }
                let expiry = *ends.iter().min().unwrap();
                let mut active = expiry <= chrono::Utc::now()
                    || reg::number(path, "FlightSettingsMaxPauseDays", 0)? < 7000;
                let components = Feature {
                    ops: f
                        .ops
                        .iter()
                        .filter(|op| op.kind != "UpdatePause")
                        .cloned()
                        .collect(),
                    ..Default::default()
                };
                for op in self.expand(&components)? {
                    active |= !satisfied(&op, &self.read(&op)?);
                }
                let mut check = Check::active(active);
                check.detail = format!(
                    "{}{}",
                    choose(self.zh, "更新暂停至 ", "Updates paused until "),
                    expiry.format("%Y-%m-%d")
                );
                return Ok(Some(check));
            }
            "mouse" => prefs::mouse()? != [0, 0, 0, 20, 6],
            "keyboard" => {
                let k = prefs::keyboard()?;
                (k.flags & 125) != 1
                    || k.delay != 140
                    || k.repeat != 16
                    || k.wait != 0
                    || k.bounce != 0
                    || prefs::sticky()? & 509 != 0
            }
            "cursor" => {
                let a = r"HKCU:\SOFTWARE\Microsoft\Accessibility";
                let mut active = reg::number(a, "CursorType", 0)? != 4
                    || reg::number(a, "CursorSize", 0)? != 3
                    || reg::number(r"HKCU:\Control Panel\Cursors", "CursorBaseSize", 0)? != 64;
                for op in f.ops.iter().filter(|op| {
                    op.path == r"HKCU:\Control Panel\Cursors" && op.value_type == "ExpandString"
                }) {
                    let desired = reg::desired(op);
                    active |= !same(&reg::read(&op.path, &op.name)?, &desired)
                        || !std::path::PathBuf::from(desired.as_str().unwrap()).exists()
                }
                active
            }
            _ => return Ok(None),
        };
        Ok(Some(Check::active(active)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retained_dep_does_not_create_a_restart_request() -> Result<()> {
        let engine = Engine::new()?;
        let mut feature = engine.feature("dep")?.clone();
        feature.id = "test-dep-restart".into();
        feature.ops.clear();
        let before = engine.check(&feature);
        let (policy, _) = native::dep()?;
        assert_eq!(
            before.state,
            if policy == 0 {
                Status::Done
            } else {
                Status::Restart
            }
        );
        let mut results = engine.store.results.lock().unwrap();
        results.insert(
            feature.id.clone(),
            ResultRecord {
                boot: engine.boot,
                ..Default::default()
            },
        );
        drop(results);
        let unchanged = engine.check(&feature);
        assert_eq!(unchanged.state, before.state);
        assert_eq!(unchanged.detail, before.detail);
        engine
            .store
            .results
            .lock()
            .unwrap()
            .get_mut(&feature.id)
            .unwrap()
            .restart = true;
        assert_eq!(engine.check(&feature).state, Status::Restart);
        Ok(())
    }
    #[test]
    fn shell_refresh_keeps_the_saved_pin_explanation() -> Result<()> {
        let engine = Engine::new()?;
        let mut f = engine.feature("taskbar-pins")?.clone();
        f.id = "test-taskbar-shell-refresh".into();
        let before = engine.check(&f);
        assert!(matches!(before.state, Status::Ready | Status::Done));
        assert!(!before.detail.is_empty());
        let shell = native::shell_stamp()?;
        assert_ne!(shell, 0);
        engine.store.results.lock().unwrap().insert(
            f.id.clone(),
            ResultRecord {
                shell,
                boot: engine.boot,
                ..Default::default()
            },
        );
        let pending = engine.check(&f);
        assert_eq!(pending.state, Status::SignIn);
        assert_eq!(pending.detail, before.detail);
        Ok(())
    }

    #[test]
    fn phishing_completion_uses_background_or_policy_without_reading_internal_state() -> Result<()>
    {
        use winreg::{RegKey, enums::HKEY_CURRENT_USER};
        let root = RegKey::predef(HKEY_CURRENT_USER);
        let name = format!(
            r"Software\ZeroSecurityWindowsPhishingTest{}",
            std::process::id()
        );
        let (key, _) = root.create_subkey(&name)?;
        let path = format!(r"HKCU:\{name}");
        let mut engine = Engine::new()?;
        let background = engine
            .catalog
            .iter()
            .position(|f| f.id == "phishing-services")
            .unwrap();
        engine.catalog[background].ops = vec![Operation::reg(&path, "Background", 0)];
        let mut f = engine.feature("phishing-protection")?.clone();
        f.id = "test-phishing-completion".into();
        let result = (|| -> Result<()> {
            key.set_value("Background", &0u32)?;
            // The feature is complete without opening even an invalid policy path.
            f.ops[0].path = "HKLM:\\invalid\0path".into();
            engine.store.results.lock().unwrap().insert(
                f.id.clone(),
                ResultRecord {
                    errors: vec!["old status read failure".into()],
                    ..Default::default()
                },
            );
            let done = engine.check(&f);
            assert_eq!(done.state, Status::Done);
            assert!(!done.actionable());
            assert!(!done.recheckable());
            assert!(!done.detail.is_empty());
            assert!(
                engine.store.results.lock().unwrap()[&f.id]
                    .errors
                    .is_empty()
            );

            key.set_value("Background", &1u32)?;
            f.ops[0].path = path.clone();
            assert_eq!(engine.check(&f).state, Status::Ready);
            key.set_value("ServiceEnabled", &0u32)?;
            assert_eq!(engine.check(&f).state, Status::Done);
            key.set_value("ServiceEnabled", &1u32)?;
            assert_eq!(engine.check(&f).state, Status::Ready);

            engine.catalog[background].ops.clear();
            f.ops[0].path = "HKLM:\\invalid\0path".into();
            assert_eq!(engine.check(&f).state, Status::Absent);
            Ok(())
        })();
        drop(key);
        root.delete_subkey_all(&name)?;
        result
    }

    #[test]
    fn status_read_restrictions_are_separate_from_execution_failures() {
        for error in [
            anyhow::Error::new(std::io::Error::from_raw_os_error(5)).context("reading state"),
            anyhow::Error::new(windows::core::Error::from_hresult(windows::core::HRESULT(
                0x80070005_u32 as i32,
            ))),
        ] {
            let check = check_error(&error);
            assert_eq!(check.state, Status::Restricted);
            assert!(!check.recheckable());
            assert!(!check.actionable());
            assert!(!check.detail.is_empty());
        }
        let error = anyhow::anyhow!("invalid state returned by Windows");
        let check = check_error(&error);
        assert_eq!(check.state, Status::Unknown);
        assert!(check.recheckable());
        assert!(!check.actionable());
    }

    #[test]
    fn safe_mode_defers_desktop_checks_without_reading_missing_services() -> Result<()> {
        let mut engine = Engine::new()?;
        engine.safe = true;
        let f = Feature {
            id: "test-safe-desktop-check".into(),
            probe: "cursor".into(),
            ops: vec![Operation {
                kind: "InputMethods".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let check = engine.check(&f);
        assert_eq!(check.state, Status::Deferred);
        assert!(check.detail.is_empty());
        assert!(!check.actionable());
        Ok(())
    }
}
