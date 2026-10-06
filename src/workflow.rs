use crate::{engine::Engine, model::*, native, registry as reg, store};
use anyhow::{Result, ensure};
use rust_fsm::{StateMachine, StateMachineImpl};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    #[default]
    Idle,
    AwaitSafe,
    AwaitNormal,
}
#[derive(Clone, Copy, Debug)]
enum Trigger {
    QueueSafe,
    QueueNormal,
    Return,
    Finish,
}
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct Pending {
    pub phase: Phase,
    pub sid: String,
    pub normal_entry: String,
    pub safe_entry: String,
    pub suspended: bool,
    pub continue_all: bool,
    pub safe_ids: Vec<String>,
}
struct Flow;
impl StateMachineImpl for Flow {
    type Input = Trigger;
    type State = Phase;
    type Output = ();
    const INITIAL_STATE: Phase = Phase::Idle;
    fn transition(state: &Phase, input: &Trigger) -> Option<Phase> {
        match (state, input) {
            (Phase::Idle, Trigger::QueueSafe) => Some(Phase::AwaitSafe),
            (Phase::Idle, Trigger::QueueNormal) | (Phase::AwaitSafe, Trigger::Return) => {
                Some(Phase::AwaitNormal)
            }
            (Phase::AwaitSafe | Phase::AwaitNormal, Trigger::Finish) => Some(Phase::Idle),
            _ => None,
        }
    }
    fn output(_state: &Phase, _input: &Trigger) -> Option<()> {
        None
    }
}
fn transition(engine: &Engine, input: Trigger) -> Result<()> {
    let mut pending = engine.store.pending.lock().unwrap();
    let p = pending
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("No boot workflow"))?;
    let mut machine = StateMachine::<Flow>::from_state(p.phase);
    machine
        .consume(&input)
        .map_err(|e| anyhow::anyhow!("Invalid boot transition: {e:?}"))?;
    p.phase = *machine.state();
    Ok(())
}
fn bcd(args: &[&str]) -> Result<String> {
    native::command("bcdedit.exe", args)
}
fn entry(text: &str) -> Result<String> {
    let start = text
        .find('{')
        .ok_or_else(|| anyhow::anyhow!("Missing boot entry"))?;
    let end = text[start..]
        .find('}')
        .ok_or_else(|| anyhow::anyhow!("Invalid boot entry"))?;
    let id = &text[start..=start + end];
    ensure!(id.len() == 38, "Invalid boot entry: {id}");
    Ok(id.into())
}
fn current() -> Result<String> {
    entry(&bcd(&["/enum", "{current}", "/v"])?)
}
fn drive() -> String {
    std::env::var("SystemDrive").unwrap_or("C:".into())
}
pub fn suspend(engine: &Engine, boots: u32) -> Result<()> {
    if *engine.store.suspended.lock().unwrap() {
        return Ok(());
    }
    let connection = match wmi::WMIConnection::with_namespace_path(
        r"root\cimv2\Security\MicrosoftVolumeEncryption",
    ) {
        Ok(c) => c,
        Err(wmi::WMIError::HResultError { hres }) if hres as u32 == 0x8004100e => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let rows: Vec<std::collections::HashMap<String, wmi::Variant>> =
        connection.raw_query(format!(
            "SELECT __PATH FROM Win32_EncryptableVolume WHERE DriveLetter='{}'",
            drive()
        ))?;
    for row in rows {
        if let Some(wmi::Variant::String(path)) = row.get("__PATH") {
            let result = connection
                .exec_method(path, "GetProtectionStatus", None)?
                .ok_or_else(|| anyhow::anyhow!("No BitLocker status"))?;
            let v = serde_json::to_value(result)?;
            ensure!(
                v["ReturnValue"].as_u64() == Some(0),
                "BitLocker status failed"
            );
            if v["ProtectionStatus"].as_u64() == Some(1) {
                native::command(
                    "manage-bde.exe",
                    &[
                        "-protectors",
                        "-disable",
                        &drive(),
                        "-RebootCount",
                        &boots.to_string(),
                    ],
                )?;
                *engine.store.suspended.lock().unwrap() = true;
                if let Some(p) = engine.store.pending.lock().unwrap().as_mut() {
                    p.suspended = true
                }
                engine.store.save()?;
            }
        }
    }
    Ok(())
}
const RUN_ONCE: &str = r"HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\RunOnce";
const RETURN: &str = "ZeroSecurityWindows.Return";
pub const QUIET: &[&str] = &[
    "security-notifications",
    "network-prompts",
    "firewall",
    "uac",
    "smartscreen",
    "edge-smartscreen",
    "open-file-warning",
    "smart-app-control",
    "applocker",
    "userchoice-protection",
];
fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn register_return(engine: &Engine) -> Result<()> {
    let exe = engine.store.copy_executable()?;
    if engine.safe {
        reg::set(
            RUN_ONCE,
            RETURN,
            format!("\"{}\" --resume", exe.display()),
            "String",
        )?;
        return Ok(());
    }
    use windows::{
        Win32::System::{TaskScheduler::*, Variant::VARIANT},
        core::BSTR,
    };
    let sid = &engine.store.sid;
    let document = format!(
        r#"<Task version="1.4" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task"><Triggers><LogonTrigger><UserId>{sid}</UserId></LogonTrigger></Triggers><Principals><Principal id="User"><UserId>{sid}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>HighestAvailable</RunLevel></Principal></Principals><Settings><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><ExecutionTimeLimit>PT0S</ExecutionTimeLimit></Settings><Actions Context="User"><Exec><Command>{exe}</Command><Arguments>--resume</Arguments><WorkingDirectory>{folder}</WorkingDirectory></Exec></Actions></Task>"#,
        exe = xml(&exe.to_string_lossy()),
        folder = xml(&store::root().to_string_lossy())
    );
    unsafe {
        native::task_root()?.RegisterTask(
            &BSTR::from(RETURN),
            &BSTR::from(document),
            TASK_CREATE_OR_UPDATE.0,
            &VARIANT::from(sid.as_str()),
            &VARIANT::default(),
            TASK_LOGON_INTERACTIVE_TOKEN,
            &VARIANT::default(),
        )?;
    }
    Ok(())
}
pub fn prepare_safe(engine: &Engine, ids: &[String], all: bool) -> Result<()> {
    engine.store.initialize()?;
    let prior = engine.store.pending.lock().unwrap().clone();
    if let Some(p) = &prior
        && p.phase == Phase::AwaitSafe
    {
        engine.store.copy_executable()?;
        let mut pending = engine.store.pending.lock().unwrap();
        let p = pending.as_mut().unwrap();
        p.safe_ids.extend_from_slice(ids);
        p.safe_ids.sort();
        p.safe_ids.dedup();
        p.continue_all |= all;
        drop(pending);
        for id in ids {
            engine.execute(engine.feature(id)?, false, true, false)?;
        }
        return engine.store.save();
    }
    let all = all || prior.as_ref().is_some_and(|p| p.continue_all);
    if prior.is_some() {
        cleanup(engine)?
    }
    let normal = current()?;
    *engine.boot_entry.lock().unwrap() = normal.clone();
    *engine.store.pending.lock().unwrap() = Some(Pending {
        sid: engine.store.sid.clone(),
        normal_entry: normal.clone(),
        suspended: *engine.store.suspended.lock().unwrap(),
        continue_all: all,
        safe_ids: ids.to_vec(),
        ..Default::default()
    });
    engine.store.save()?;
    let result = (|| {
        ensure!(
            !bcd(&["/enum", "{bootmgr}"])?
                .lines()
                .any(|s| s.trim_start().starts_with("bootsequence")),
            "Another one-time boot is pending"
        );
        suspend(engine, 2)?;
        let copied = entry(&bcd(&[
            "/copy",
            &normal,
            "/d",
            "Zero Security Windows - Safe Mode",
        ])?)?;
        engine
            .store
            .pending
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .safe_entry = copied.clone();
        engine.store.save()?;
        bcd(&["/set", &copied, "safeboot", "minimal"])?;
        bcd(&["/displayorder", &copied, "/remove"])?;
        let exe = engine.store.copy_executable()?;
        reg::set(
            RUN_ONCE,
            "*ZeroSecurityWindows.SafeApply",
            format!("\"{}\" --safe-resume", exe.display()),
            "String",
        )?;
        register_return(engine)?;
        bcd(&["/bootsequence", &copied])?;
        for id in ids {
            engine.execute(engine.feature(id)?, false, true, false)?;
        }
        transition(engine, Trigger::QueueSafe)?;
        engine.store.save()
    })();
    if result.is_err() {
        cleanup(engine)?
    }
    result
}
pub fn prepare_normal(engine: &Engine, all: bool) -> Result<()> {
    if engine.store.pending.lock().unwrap().is_some() {
        engine.store.copy_executable()?;
        engine
            .store
            .pending
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .continue_all |= all;
        return engine.store.save();
    }
    engine.store.initialize()?;
    *engine.store.pending.lock().unwrap() = Some(Pending {
        sid: engine.store.sid.clone(),
        normal_entry: current()?,
        suspended: *engine.store.suspended.lock().unwrap(),
        continue_all: all,
        ..Default::default()
    });
    register_return(engine)?;
    transition(engine, Trigger::QueueNormal)?;
    engine.store.save()
}
pub fn safe_resume(engine: &Engine, mut progress: impl FnMut(usize)) -> Result<()> {
    let p = engine
        .store
        .pending
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| anyhow::anyhow!("No boot workflow"))?;
    ensure!(
        engine.safe
            && p.phase == Phase::AwaitSafe
            && p.sid == engine.store.sid
            && p.safe_entry == current()?,
        "Safe Mode workflow does not match this boot"
    );
    engine.store.initialize()?;
    *engine.boot_entry.lock().unwrap() = p.normal_entry;
    reg::delete(RUN_ONCE, "*ZeroSecurityWindows.SafeApply")?;
    for id in p.safe_ids {
        let index = engine.catalog.iter().position(|f| f.id == id).unwrap();
        progress(index);
        engine.execute(&engine.catalog[index], false, false, true)?;
    }
    transition(engine, Trigger::Return)?;
    engine.store.save()
}
pub fn normal_resume(engine: &Engine) -> Result<bool> {
    let p = engine
        .store
        .pending
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| anyhow::anyhow!("No boot workflow"))?;
    ensure!(
        !engine.safe && p.phase == Phase::AwaitNormal && p.sid == engine.store.sid,
        "Normal workflow does not match this boot"
    );
    *engine.boot_entry.lock().unwrap() = p.normal_entry;
    engine.store.initialize()?;
    cleanup(engine)?;
    if !native::service_running("WinDefend")? {
        for id in ["defender", "defender-tasks"] {
            let f = engine.feature(id)?;
            if engine.check(f).actionable() {
                engine.execute(f, false, false, false)?;
            }
        }
    }
    Ok(p.continue_all)
}
pub fn cleanup(engine: &Engine) -> Result<()> {
    let p = engine.store.pending.lock().unwrap().clone();
    reg::delete(RUN_ONCE, "*ZeroSecurityWindows.SafeApply")?;
    reg::delete(RUN_ONCE, RETURN)?;
    if !engine.safe {
        unsafe {
            let e = native::task_root()?.DeleteTask(&windows::core::BSTR::from(RETURN), 0);
            if let Err(e) = e
                && e.code().0 as u32 != 0x80070002
            {
                return Err(e.into());
            }
        }
    }
    if let Some(p) = p {
        if !p.safe_entry.is_empty() {
            bcd(&["/delete", &p.safe_entry, "/cleanup"])?;
        }
        if p.suspended {
            native::command("manage-bde.exe", &["-protectors", "-enable", &drive()])?;
        }
        if p.phase != Phase::Idle {
            transition(engine, Trigger::Finish)?
        }
    }
    *engine.store.suspended.lock().unwrap() = false;
    *engine.store.pending.lock().unwrap() = None;
    engine.store.save()
}
pub fn requires_restart(engine: &Engine) -> bool {
    engine.store.pending.lock().unwrap().is_some()
        || engine
            .store
            .results
            .lock()
            .unwrap()
            .values()
            .any(|r| r.restart && r.boot == engine.boot && r.errors.is_empty())
}
pub fn assert_quiet(engine: &Engine) -> Result<()> {
    for id in QUIET.iter().filter(|id| **id != "userchoice-protection") {
        let f = engine.feature(id)?;
        ensure!(
            matches!(
                engine.check(f).state,
                Status::Done | Status::Absent | Status::SignIn
            ),
            "{}{}",
            choose(engine.zh, "需先完成：", "Complete first: "),
            f.name(engine.zh)
        )
    }
    ensure!(
        !native::service_running("WinDefend")?,
        "Microsoft Defender is still running"
    );
    Ok(())
}
pub fn restart() -> Result<()> {
    native::command("shutdown.exe", &["/r", "/t", "0"])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn boot_flow_returns_from_safe_mode_and_cannot_skip_a_stage() {
        let mut machine = StateMachine::<Flow>::new();
        assert!(machine.consume(&Trigger::Return).is_err());
        machine.consume(&Trigger::QueueSafe).unwrap();
        assert_eq!(*machine.state(), Phase::AwaitSafe);
        assert!(machine.consume(&Trigger::QueueNormal).is_err());
        machine.consume(&Trigger::Return).unwrap();
        assert_eq!(*machine.state(), Phase::AwaitNormal);
        machine.consume(&Trigger::Finish).unwrap();
        assert_eq!(*machine.state(), Phase::Idle);
        machine.consume(&Trigger::QueueNormal).unwrap();
        machine.consume(&Trigger::Finish).unwrap();
    }
}
