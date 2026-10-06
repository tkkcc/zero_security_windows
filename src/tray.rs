use crate::{native, registry as reg, store};
use anyhow::{Result, ensure};
use std::{
    ffi::c_void,
    os::windows::process::CommandExt,
    time::{Duration, Instant},
};
use winreg::{RegKey, enums::*};
type Handle = *mut c_void;
const ICONS: &str = r"Control Panel\NotifyIconSettings";
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateEventW(attributes: Handle, manual: i32, initial: i32, name: *const u16) -> Handle;
    fn OpenEventW(access: u32, inherit: i32, name: *const u16) -> Handle;
    fn SetEvent(event: Handle) -> i32;
    fn WaitForMultipleObjects(count: u32, handles: *const Handle, all: i32, millis: u32) -> u32;
    fn GetLastError() -> u32;
}
#[link(name = "advapi32")]
unsafe extern "system" {
    fn RegNotifyChangeKeyValue(
        key: Handle,
        subtree: i32,
        filter: u32,
        event: Handle,
        async_: i32,
    ) -> u32;
}
fn name() -> Result<Vec<u16>> {
    Ok(native::wide(&format!(
        r"Local\ZeroSecurityWindows.Tray.{}",
        native::sid()?
    )))
}
pub fn running() -> Result<bool> {
    let h = unsafe { OpenEventW(0x100000, 0, name()?.as_ptr()) };
    if h.is_null() {
        return Ok(false);
    }
    unsafe {
        native::CloseHandle(h);
    }
    Ok(true)
}
fn promote(key: &RegKey) -> Result<()> {
    for name in key.enum_keys() {
        let name = name?;
        match key.open_subkey_with_flags(name, KEY_READ | KEY_WRITE) {
            Ok(icon) => {
                if icon.get_value::<u32, _>("IsPromoted").ok() != Some(1) {
                    icon.set_value("IsPromoted", &1u32)?
                }
            }
            Err(e) if e.raw_os_error() == Some(1018) => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
pub fn command() -> String {
    format!(
        "\"{}\" --tray",
        store::user_root()
            .join("zero_security_windows.exe")
            .display()
    )
}
pub fn enable() -> Result<()> {
    let h = unsafe { OpenEventW(2, 0, name()?.as_ptr()) };
    if !h.is_null() {
        unsafe {
            native::ok(SetEvent(h))?;
            native::CloseHandle(h);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while running()? {
            ensure!(Instant::now() < deadline, "Tray helper did not stop");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    let exe = store::copy_to(store::user_root())?;
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(ICONS)?;
    promote(&key)?;
    reg::set(
        r"HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Run",
        "ZeroSecurityWindows.Tray",
        command(),
        "String",
    )?;
    std::process::Command::new(exe)
        .arg("--tray")
        .creation_flags(0x08000000)
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while !running()? {
        ensure!(Instant::now() < deadline, "Tray helper did not start");
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}
pub fn run() -> Result<()> {
    let stop = unsafe { CreateEventW(std::ptr::null_mut(), 1, 0, name()?.as_ptr()) };
    ensure!(!stop.is_null(), "Create tray event failed");
    if unsafe { GetLastError() } == 183 {
        unsafe {
            native::CloseHandle(stop);
        }
        return Ok(());
    }
    let changed = unsafe { CreateEventW(std::ptr::null_mut(), 0, 0, std::ptr::null()) };
    ensure!(!changed.is_null(), "Create registry event failed");
    let result = (|| {
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(ICONS)?;
        loop {
            let e = unsafe {
                RegNotifyChangeKeyValue(key.raw_handle() as Handle, 1, 0x10000005, changed, 1)
            };
            ensure!(e == 0, "Tray registry watch: {e}");
            promote(&key)?;
            let wait = unsafe { WaitForMultipleObjects(2, [stop, changed].as_ptr(), 0, u32::MAX) };
            if wait == 0 {
                break;
            }
            ensure!(wait == 1, "Tray wait failed");
        }
        Ok(())
    })();
    unsafe {
        native::CloseHandle(changed);
        native::CloseHandle(stop);
    }
    result
}
