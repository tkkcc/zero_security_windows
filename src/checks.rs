use crate::{
    engine::{Engine, satisfied},
    mitigation,
    model::*,
    native, preferences as prefs, registry as reg, store,
    workflow::Phase,
};
use anyhow::Result;
use serde_json::json;

fn check_error(f: &Feature, error: &anyhow::Error) -> Check {
    Check {
        state: if f.page == "Install" {
            Status::Failed
        } else if error.chain().any(|cause| {
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
                let check = check_error(f, &e);
                let _ = store::append(
                    "checks.jsonl",
                    &json!({"Time":chrono::Local::now().to_rfc3339(),"Id":f.id,"Error":check.detail}).to_string(),
                );
                check
            }
        };
        if let Some(result) = self.store.results.lock().unwrap().get_mut(&f.id) {
            if check.state == Status::Done {
                result.errors.clear();
            }
            if !result.errors.is_empty() {
                check = Check {
                    state: Status::Failed,
                    detail: result.errors.join("; "),
                }
            } else if result.restart && result.boot == self.boot {
                check = Check::new(Status::Restart)
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
                        String::new()
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
        if f.manual || f.probe == "driver-signing" {
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
        for op in ops {
            if !satisfied(&op, &self.read(&op)?)
                || (op.kind == "Service" && native::service_running(&op.service)?)
            {
                done = false
            }
        }
        Ok(Check::active(!done))
    }
    fn probe(&self, f: &Feature) -> Result<Option<Check>> {
        let active = match f.probe.as_str() {
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
                    return Ok(Some(Check::new(if f.probe == "tamper" {
                        Status::Inactive
                    } else {
                        Status::Done
                    })));
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
            "firewall" | "network-prompts" => {
                let mut active = false;
                for profile in ["Domain", "Private", "Public"] {
                    active |= native::firewall(profile, f.probe == "network-prompts", None)?
                }
                active
                    || (f.probe == "network-prompts"
                        && !reg::exists(
                            r"HKLM:\SYSTEM\CurrentControlSet\Control\Network\NewNetworkWindowOff",
                        )?)
            }
            "phishing" => {
                for path in [
                    r"HKLM:\SOFTWARE\Policies\Microsoft\Windows\WTDS\Components",
                    r"HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\WTDS\Components",
                ] {
                    if same(&reg::read(path, "ServiceEnabled")?, &json!(0)) {
                        return Ok(Some(Check::new(Status::Done)));
                    }
                }
                if !reg::exists(r"HKLM:\SYSTEM\CurrentControlSet\Services\webthreatdefsvc")?
                    && !reg::exists(r"HKLM:\SYSTEM\CurrentControlSet\Services\webthreatdefusersvc")?
                {
                    return Ok(Some(Check::new(Status::Absent)));
                }
                return Ok(None);
            }
            "cpu" => {
                let cpu = native::query_flags(201)?;
                let kva = native::query_flags(196)?;
                (cpu[0] & (1 | 1024 | 0x2000000 | 0x80000)) != 0
                    || (cpu[1] & (8 | 32)) != 0
                    || (kva[0] & 1) != 0
            }
            "mitigations" => self
                .fact("mitigations", || Ok(json!(mitigation::active()?)))?
                .as_bool()
                .unwrap(),
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
                let mut check = Check::active(
                    expiry <= chrono::Utc::now()
                        || reg::number(path, "FlightSettingsMaxPauseDays", 0)? < 7000,
                );
                check.detail = format!(
                    "{}{}",
                    choose(self.zh, "暂停至 ", "Paused to "),
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
            "this-pc" => return Ok(None),
            "desktop-picture" => {
                reg::number(
                    r"HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\Wallpapers",
                    "BackgroundType",
                    0,
                )? > 0
                    || reg::number(
                        r"HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\DesktopSpotlight\Settings",
                        "EnabledState",
                        0,
                    )? > 0
            }
            "lockscreen-spotlight" => {
                !same(
                    &reg::read(
                        r"HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\ContentDeliveryManager",
                        "RotatingLockScreenEnabled",
                    )?,
                    &json!(0),
                ) && !same(
                    &reg::read(
                        r"HKCU:\SOFTWARE\Policies\Microsoft\Windows\CloudContent",
                        "ConfigureWindowsSpotlight",
                    )?,
                    &json!(2),
                )
            }
            "update-background" => {
                for name in ["wuauserv", "UsoSvc", "WaaSMedicSvc"] {
                    if native::service_running(name)? {
                        return Ok(Some(Check::new(Status::Ready)));
                    }
                }
                return Ok(None);
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
    fn status_read_restrictions_are_separate_from_execution_failures() {
        let feature = Feature::default();
        for error in [
            anyhow::Error::new(std::io::Error::from_raw_os_error(5)).context("reading state"),
            anyhow::Error::new(windows::core::Error::from_hresult(windows::core::HRESULT(
                0x80070005_u32 as i32,
            ))),
        ] {
            let check = check_error(&feature, &error);
            assert_eq!(check.state, Status::Restricted);
            assert!(check.recheckable());
            assert!(!check.actionable());
            assert!(!check.detail.is_empty());
        }
        let error = anyhow::anyhow!("invalid state returned by Windows");
        let check = check_error(&feature, &error);
        assert_eq!(check.state, Status::Unknown);
        assert!(check.recheckable());
        assert!(!check.actionable());
        let installer = Feature {
            page: "Install".into(),
            ..feature
        };
        let check = check_error(&installer, &error);
        assert_eq!(check.state, Status::Failed);
        assert!(check.actionable());
        assert!(!check.recheckable());
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
