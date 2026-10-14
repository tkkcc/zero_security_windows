use crate::{engine::Engine, native, registry, store};
use anyhow::{Context, Result, ensure};
use std::{fs, path::PathBuf};

pub struct Image {
    pub file: PathBuf,
    pub recovery_entry: String,
    pub disk_options: String,
}
fn folder() -> PathBuf {
    store::root().join("offline-sac")
}
fn marker(engine: &Engine) -> PathBuf {
    PathBuf::from(native::expand("%WINDIR%"))
        .join("Temp")
        .join(format!("zsw-sac-{}.marker", engine.boot))
}
pub fn prepare(engine: &Engine) -> Result<Image> {
    let windows = PathBuf::from(native::expand("%WINDIR%"));
    let document = fs::read_to_string(windows.join("System32/Recovery/ReAgent.xml"))
        .context("Read Windows recovery configuration")?;
    let xml = roxmltree::Document::parse(&document)?;
    let recovery_entry = xml
        .descendants()
        .find(|n| n.has_tag_name("WinreBCD"))
        .and_then(|n| n.attribute("id"))
        .ok_or_else(|| anyhow::anyhow!("No Windows recovery boot entry"))?
        .to_string();
    let details = native::command("reagentc.exe", &["/info"])?;
    let location = details
        .lines()
        .find_map(|line| {
            line.find(r"\\?\GLOBALROOT\")
                .map(|start| line[start..].trim())
        })
        .ok_or_else(|| anyhow::anyhow!("Windows recovery environment is unavailable"))?;
    let bcd = native::command("bcdedit.exe", &["/enum", &recovery_entry, "/v"])?;
    let disk_options = bcd
        .lines()
        .filter(|line| line.contains("ramdisk=["))
        .find_map(|line| {
            line.rsplit_once(',')
                .map(|(_, value)| value.trim().to_string())
        })
        .ok_or_else(|| anyhow::anyhow!("No Windows recovery ramdisk options"))?;
    let work = folder();
    let mount = work.join("mount");
    fs::create_dir_all(&mount).context("Create recovery staging directory")?;
    let file = work.join("restore.wim");
    fs::copy(PathBuf::from(location).join("winre.wim"), &file)
        .with_context(|| format!("Copy Windows recovery image from {location}"))?;
    native::command(
        "dism.exe",
        &[
            "/Mount-Image",
            &format!("/ImageFile:{}", file.display()),
            "/Index:1",
            &format!("/MountDir:{}", mount.display()),
        ],
    )?;
    let staged = (|| -> Result<()> {
        let system32 = mount.join("Windows/System32");
        let windows_path = windows.to_string_lossy();
        let relative = windows_path
            .get(3..)
            .ok_or_else(|| anyhow::anyhow!("Invalid Windows directory"))?;
        let control = registry::read(r"HKLM:\SYSTEM\Select", "Current")?
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("No current control set"))?;
        let script = format!(
            r#"@echo off
wpeinit
set "_zsw="
for %%D in (C D E F G H I J K L M N O P Q R S T U V W Y Z) do if exist "%%D:\{relative}\Temp\zsw-sac-{boot}.marker" set "_zsw=%%D:\{relative}"
if not defined _zsw goto finish
set "_log=%_zsw%\Temp\zsw-sac-{boot}.log"
reg load HKLM\ZSW_SAC_SYS "%_zsw%\System32\Config\SYSTEM" >"%_log%" 2>&1
if errorlevel 1 goto finish
reg add HKLM\ZSW_SAC_SYS\ControlSet{control:03}\Control\CI\Policy /v VerifiedAndReputablePolicyState /t REG_DWORD /d 2 /f >>"%_log%" 2>&1
if errorlevel 1 goto unload
reg add HKLM\ZSW_SAC_SYS\ControlSet{control:03}\Control\CI\Protected /v VerifiedAndReputablePolicyStateMinValueSeen /t REG_DWORD /d 2 /f >>"%_log%" 2>&1
if errorlevel 1 goto unload
reg load HKLM\ZSW_SAC_SOFT "%_zsw%\System32\Config\SOFTWARE" >>"%_log%" 2>&1
if errorlevel 1 goto unload
reg add "HKLM\ZSW_SAC_SOFT\Microsoft\Windows Defender" /v SacLearningModeSwitch /t REG_DWORD /d 0 /f >>"%_log%" 2>&1
:unload
reg unload HKLM\ZSW_SAC_SYS >>"%_log%" 2>&1
reg unload HKLM\ZSW_SAC_SOFT >>"%_log%" 2>&1
:finish
wpeutil reboot
"#,
            boot = engine.boot
        );
        fs::write(system32.join("zsw-sac.cmd"), script.replace('\n', "\r\n"))
            .context("Save offline restoration script")?;
        fs::write(
            system32.join("winpeshl.ini"),
            "[LaunchApps]\r\n%SYSTEMROOT%\\System32\\cmd.exe, /c %SYSTEMROOT%\\System32\\zsw-sac.cmd\r\n",
        ).context("Save recovery startup configuration")?;
        Ok(())
    })();
    native::command(
        "dism.exe",
        &[
            "/Unmount-Image",
            &format!("/MountDir:{}", mount.display()),
            if staged.is_ok() {
                "/Commit"
            } else {
                "/Discard"
            },
        ],
    )?;
    staged?;
    ensure!(file.exists(), "Missing recovery image");
    if native::service_running("WinDefend")? {
        native::command(
            &format!(
                r"{}\Windows Defender\MpCmdRun.exe",
                native::expand("%ProgramFiles%")
            ),
            &["-RemoveDefinitions", "-DynamicSignatures"],
        )?;
    }
    fs::write(marker(engine), b"Smart App Control recovery")?;
    Ok(Image {
        file,
        recovery_entry,
        disk_options,
    })
}
pub fn cleanup(boot: u64) -> Result<()> {
    let work = folder();
    let file = work.join("restore.wim");
    if file.exists() {
        fs::remove_file(file)?;
    }
    let mount = work.join("mount");
    if mount.exists() {
        fs::remove_dir(mount)?;
    }
    if work.exists() {
        fs::remove_dir(work)?;
    }
    let windows = PathBuf::from(native::expand("%WINDIR%"));
    let file = windows.join("Temp").join(format!("zsw-sac-{boot}.marker"));
    if file.exists() {
        fs::remove_file(file)?;
    }
    Ok(())
}
