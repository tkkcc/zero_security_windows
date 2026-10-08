use anyhow::{Result, ensure};
use serde_json::Value;
use std::{
    ffi::c_void,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
};
use windows::{
    Win32::{
        NetworkManagement::WindowsFirewall::*,
        System::{Com::*, TaskScheduler::*, Variant::VARIANT},
    },
    core::{BSTR, GUID, PCWSTR},
};
pub type Handle = *mut c_void;
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
pub fn error(code: u32) -> anyhow::Error {
    std::io::Error::from_raw_os_error(code as i32).into()
}
pub fn ok(b: i32) -> Result<()> {
    if b == 0 {
        Err(std::io::Error::last_os_error().into())
    } else {
        Ok(())
    }
}
#[repr(C)]
struct Luid {
    low: u32,
    high: i32,
}
#[repr(C)]
struct Priv {
    count: u32,
    id: Luid,
    attributes: u32,
}
#[repr(C)]
struct ServiceStatus {
    kind: u32,
    current: u32,
    accepted: u32,
    exit: u32,
    specific: u32,
    checkpoint: u32,
    hint: u32,
}
#[link(name = "advapi32")]
unsafe extern "system" {
    fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
    fn LookupPrivilegeValueW(system: *const u16, name: *const u16, id: *mut Luid) -> i32;
    fn AdjustTokenPrivileges(
        token: Handle,
        disable: i32,
        state: *const Priv,
        size: u32,
        old: Handle,
        needed: Handle,
    ) -> i32;
    fn ImpersonateLoggedOnUser(token: Handle) -> i32;
    fn RevertToSelf() -> i32;
    fn OpenSCManagerW(machine: *const u16, database: *const u16, access: u32) -> Handle;
    fn OpenServiceW(manager: Handle, name: *const u16, access: u32) -> Handle;
    fn QueryServiceStatus(service: Handle, status: *mut ServiceStatus) -> i32;
    fn ChangeServiceConfigW(
        service: Handle,
        kind: u32,
        start: u32,
        error: u32,
        binary: *const u16,
        group: *const u16,
        tag: Handle,
        dependencies: *const u16,
        account: *const u16,
        password: *const u16,
        display: *const u16,
    ) -> i32;
    fn ControlService(service: Handle, control: u32, status: *mut ServiceStatus) -> i32;
    fn CloseServiceHandle(service: Handle) -> i32;
    fn RegCreateKeyExW(
        hive: Handle,
        path: *const u16,
        reserved: u32,
        class: *const u16,
        options: u32,
        access: u32,
        security: Handle,
        key: *mut Handle,
        disposition: *mut u32,
    ) -> u32;
    fn RegSetValueExW(
        key: Handle,
        name: *const u16,
        reserved: u32,
        kind: u32,
        bytes: *const u8,
        size: u32,
    ) -> u32;
    fn RegDeleteValueW(key: Handle, name: *const u16) -> u32;
    fn RegCloseKey(key: Handle) -> u32;
}
#[link(name = "kernel32")]
unsafe extern "system" {
    pub fn GetCurrentProcess() -> Handle;
    pub fn CloseHandle(h: Handle) -> i32;
    fn OpenProcess(access: u32, inherit: i32, id: u32) -> Handle;
    pub fn GetCurrentProcessId() -> u32;
    fn ProcessIdToSessionId(id: u32, session: *mut u32) -> i32;
    fn TerminateProcess(process: Handle, exit: u32) -> i32;
    fn WaitForSingleObject(process: Handle, millis: u32) -> u32;
    fn QueryFullProcessImageNameW(
        process: Handle,
        flags: u32,
        path: *mut u16,
        size: *mut u32,
    ) -> i32;
    fn GetProcessInformation(process: Handle, class: u32, data: Handle, size: u32) -> i32;
    fn GetUserPreferredUILanguages(
        flags: u32,
        count: *mut u32,
        buffer: *mut u16,
        length: *mut u32,
    ) -> i32;
    fn GetProcessTimes(
        process: Handle,
        creation: *mut u64,
        exit: *mut u64,
        kernel: *mut u64,
        user: *mut u64,
    ) -> i32;
}
#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtQuerySystemInformation(class: u32, data: Handle, size: u32, length: *mut u32) -> i32;
}
#[link(name = "user32")]
unsafe extern "system" {
    fn GetSystemMetrics(index: i32) -> i32;
}
pub fn safe_mode() -> bool {
    unsafe { GetSystemMetrics(67) != 0 }
}
pub fn boot() -> u64 {
    let mut data = [0u64; 6];
    let mut len = 0;
    let hr = unsafe { NtQuerySystemInformation(3, data.as_mut_ptr().cast(), 48, &mut len) };
    assert!(hr >= 0, "Cannot read boot time: {hr:08X}");
    data[0]
}
pub fn query_flags(class: u32) -> Result<[u32; 2]> {
    let mut v = [0u32; 2];
    let mut len = 0;
    let hr = unsafe { NtQuerySystemInformation(class, v.as_mut_ptr().cast(), 8, &mut len) };
    ensure!(hr >= 0, "NtQuerySystemInformation: {hr:08X}");
    Ok(v)
}
pub fn chinese() -> bool {
    let (mut count, mut len) = (0, 0);
    unsafe {
        GetUserPreferredUILanguages(8, &mut count, std::ptr::null_mut(), &mut len);
        let mut value = vec![0u16; len as usize];
        GetUserPreferredUILanguages(8, &mut count, value.as_mut_ptr(), &mut len);
        String::from_utf16_lossy(&value)
            .to_lowercase()
            .starts_with("zh")
    }
}
pub fn com() -> Result<()> {
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe { CoUninitialize() }
        }
    }
    thread_local! {static APARTMENT:std::result::Result<Apartment,String>=unsafe{CoInitializeEx(None,COINIT_MULTITHREADED).ok().map(|_|Apartment).map_err(|e|e.to_string())};}
    APARTMENT.with(|result| {
        result
            .as_ref()
            .map(|_| ())
            .map_err(|e| anyhow::anyhow!("{e}"))
    })
}
pub fn privilege(name: &str) -> Result<()> {
    unsafe {
        let mut token = std::ptr::null_mut();
        ok(OpenProcessToken(GetCurrentProcess(), 0x28, &mut token))?;
        let result = (|| {
            let mut id = Luid { low: 0, high: 0 };
            ok(LookupPrivilegeValueW(
                std::ptr::null(),
                wide(name).as_ptr(),
                &mut id,
            ))?;
            ok(AdjustTokenPrivileges(
                token,
                0,
                &Priv {
                    count: 1,
                    id,
                    attributes: 2,
                },
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            ))?;
            let e = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            ensure!(e == 0, "{name}: {e}");
            Ok(())
        })();
        CloseHandle(token);
        result
    }
}
pub struct SystemGuard;
impl Drop for SystemGuard {
    fn drop(&mut self) {
        unsafe {
            RevertToSelf();
        }
    }
}
pub fn system() -> Result<SystemGuard> {
    privilege("SeDebugPrivilege")?;
    let id = processes()?
        .into_iter()
        .find(|(_, n)| n.eq_ignore_ascii_case("winlogon.exe"))
        .ok_or_else(|| anyhow::anyhow!("winlogon not running"))?
        .0;
    unsafe {
        let p = OpenProcess(0x1000, 0, id);
        ensure!(
            !p.is_null(),
            "Open winlogon: {}",
            std::io::Error::last_os_error()
        );
        let mut t = std::ptr::null_mut();
        let opened = ok(OpenProcessToken(p, 0x0e, &mut t));
        CloseHandle(p);
        opened?;
        let result = ok(ImpersonateLoggedOnUser(t));
        CloseHandle(t);
        result?;
    }
    Ok(SystemGuard)
}
pub fn processes() -> Result<Vec<(u32, String)>> {
    use windows::Win32::{Foundation::*, System::Diagnostics::ToolHelp::*};
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)?;
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut list = vec![];
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                list.push((
                    entry.th32ProcessID,
                    String::from_utf16_lossy(
                        &entry.szExeFile[..entry
                            .szExeFile
                            .iter()
                            .position(|v| *v == 0)
                            .unwrap_or(entry.szExeFile.len())],
                    ),
                ));
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        CloseHandle(snapshot)?;
        Ok(list)
    }
}
pub fn process_running(name: &str) -> Result<bool> {
    Ok(processes()?
        .iter()
        .any(|(_, n)| n.eq_ignore_ascii_case(name)))
}
pub fn process_images() -> Result<Vec<(u32, String)>> {
    let mut images = vec![];
    for (id, _) in processes()? {
        unsafe {
            let process = OpenProcess(0x1000, 0, id);
            if process.is_null() {
                continue;
            }
            let mut path = vec![0u16; 32768];
            let mut length = path.len() as u32;
            if QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut length) != 0 {
                images.push((id, String::from_utf16_lossy(&path[..length as usize])));
            }
            CloseHandle(process);
        }
    }
    Ok(images)
}
pub fn lsa_protected() -> Result<bool> {
    let id = processes()?
        .into_iter()
        .find(|(_, name)| name.eq_ignore_ascii_case("lsass.exe"))
        .ok_or_else(|| anyhow::anyhow!("LSASS is not running"))?
        .0;
    unsafe {
        let process = OpenProcess(0x1000, 0, id);
        ensure!(!process.is_null(), "{}", std::io::Error::last_os_error());
        let mut level = 0u32;
        let result = ok(GetProcessInformation(
            process,
            7,
            (&mut level as *mut u32).cast(),
            4,
        ));
        CloseHandle(process);
        result?;
        Ok(level != 0xfffffffe)
    }
}
pub fn code_integrity() -> Result<bool> {
    let mut data = [8u32, 0];
    let status =
        unsafe { NtQuerySystemInformation(103, data.as_mut_ptr().cast(), 8, std::ptr::null_mut()) };
    ensure!(status >= 0, "Code integrity query: {status:08X}");
    Ok(data[1] & 1 != 0)
}
pub fn stop_process(name: &str) -> Result<()> {
    let _guard = system()?;
    for (id, n) in processes()? {
        if n.eq_ignore_ascii_case(name)
            && (name != "StartMenuExperienceHost.exe" || same_session(id))
        {
            unsafe {
                let handle = OpenProcess(0x100001, 0, id);
                ensure!(
                    !handle.is_null(),
                    "Stopping {name}: {}",
                    std::io::Error::last_os_error()
                );
                let result = ok(TerminateProcess(handle, 1));
                let wait = if result.is_ok() {
                    WaitForSingleObject(handle, 2000)
                } else {
                    0
                };
                CloseHandle(handle);
                result?;
                ensure!(wait == 0, "Process did not stop: {name}");
            }
        }
    }
    Ok(())
}
pub fn shell_stamp() -> Result<u64> {
    let mut latest = 0;
    for (id, n) in processes()? {
        if n.eq_ignore_ascii_case("explorer.exe") && same_session(id) {
            unsafe {
                let h = OpenProcess(0x1000, 0, id);
                if !h.is_null() {
                    let (mut c, mut e, mut k, mut u) = (0, 0, 0, 0);
                    if GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u) != 0 {
                        latest = latest.max(c)
                    }
                    CloseHandle(h);
                }
            }
        }
    }
    Ok(latest)
}
fn service_call<T>(name: &str, access: u32, f: impl FnOnce(Handle) -> Result<T>) -> Result<T> {
    unsafe {
        let manager = OpenSCManagerW(std::ptr::null(), std::ptr::null(), 1);
        ensure!(
            !manager.is_null(),
            "OpenSCManager: {}",
            std::io::Error::last_os_error()
        );
        let svc = OpenServiceW(manager, wide(name).as_ptr(), access);
        if svc.is_null() {
            let error = std::io::Error::last_os_error();
            CloseServiceHandle(manager);
            return Err(error.into());
        }
        let value = f(svc);
        CloseServiceHandle(svc);
        CloseServiceHandle(manager);
        value
    }
}
pub fn service_running(name: &str) -> Result<bool> {
    match service_call(name, 4, |h| {
        let mut status: ServiceStatus = unsafe { std::mem::zeroed() };
        unsafe { ok(QueryServiceStatus(h, &mut status))? };
        Ok(status.current != 1)
    }) {
        Err(e)
            if e.downcast_ref::<std::io::Error>()
                .and_then(|e| e.raw_os_error())
                == Some(1060) =>
        {
            Ok(false)
        }
        v => v,
    }
}
pub fn service_start(name: &str, value: Option<u32>) -> Result<u32> {
    use windows::Win32::System::Services::{QUERY_SERVICE_CONFIGW, QueryServiceConfigW, SC_HANDLE};
    let _guard = value.map(|_| system()).transpose()?;
    service_call(name, if value.is_some() { 3 } else { 1 }, |h| unsafe {
        if let Some(start) = value {
            let null = std::ptr::null();
            ok(ChangeServiceConfigW(
                h,
                u32::MAX,
                start,
                u32::MAX,
                null,
                null,
                std::ptr::null_mut(),
                null,
                null,
                null,
                null,
            ))?;
        }
        let mut size = 0;
        let _ = QueryServiceConfigW(SC_HANDLE(h), None, 0, &mut size);
        let mut data = vec![0u64; (size as usize).div_ceil(8)];
        let config = data.as_mut_ptr().cast::<QUERY_SERVICE_CONFIGW>();
        QueryServiceConfigW(SC_HANDLE(h), Some(config), size, &mut size)?;
        Ok((*config).dwStartType.0)
    })
}
pub fn time_sync(apply: bool) -> Result<bool> {
    use windows::Win32::System::Services::{
        ChangeServiceConfig2W, QueryServiceConfig2W, SC_HANDLE, SERVICE_CONFIG_TRIGGER_INFO,
        SERVICE_TRIGGER_INFO, StartServiceW,
    };
    let _guard = apply.then(system).transpose()?;
    service_call("W32Time", if apply { 0x17 } else { 5 }, |h| unsafe {
        if apply {
            let triggers = SERVICE_TRIGGER_INFO::default();
            ChangeServiceConfig2W(
                SC_HANDLE(h),
                SERVICE_CONFIG_TRIGGER_INFO,
                Some((&triggers as *const SERVICE_TRIGGER_INFO).cast()),
            )?;
        }
        let mut size = 0;
        let _ = QueryServiceConfig2W(SC_HANDLE(h), SERVICE_CONFIG_TRIGGER_INFO, None, &mut size);
        let mut data = vec![0u8; size as usize];
        QueryServiceConfig2W(
            SC_HANDLE(h),
            SERVICE_CONFIG_TRIGGER_INFO,
            Some(&mut data),
            &mut size,
        )?;
        let triggers = std::ptr::read_unaligned(data.as_ptr().cast::<SERVICE_TRIGGER_INFO>());
        let mut status: ServiceStatus = std::mem::zeroed();
        ok(QueryServiceStatus(h, &mut status))?;
        if apply && status.current != 4 {
            if status.current != 2 {
                StartServiceW(SC_HANDLE(h), None)?;
                ok(QueryServiceStatus(h, &mut status))?;
            }
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while status.current == 2 && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(40));
                ok(QueryServiceStatus(h, &mut status))?;
            }
            ensure!(
                status.current == 4,
                "W32Time did not start (state {})",
                status.current
            );
        }
        Ok(triggers.cTriggers == 0 && status.current == 4)
    })
}
pub fn disable_service(name: &str) -> Result<bool> {
    let _guard = system()?;
    let result = service_call(name, 0x26, |h| unsafe {
        let mut restart = false;
        let null = std::ptr::null();
        if ChangeServiceConfigW(
            h,
            u32::MAX,
            4,
            u32::MAX,
            null,
            null,
            std::ptr::null_mut(),
            null,
            null,
            null,
            null,
        ) == 0
        {
            restart = true
        }
        let mut status: ServiceStatus = std::mem::zeroed();
        ok(QueryServiceStatus(h, &mut status))?;
        if status.current != 1 {
            if ControlService(h, 1, &mut status) == 0 {
                restart = true
            } else {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                while status.current != 1 && std::time::Instant::now() < deadline {
                    std::thread::sleep(std::time::Duration::from_millis(40));
                    ok(QueryServiceStatus(h, &mut status))?
                }
                restart |= status.current != 1;
            }
        }
        Ok(restart)
    });
    match result {
        Err(e)
            if e.downcast_ref::<std::io::Error>()
                .and_then(|e| e.raw_os_error())
                == Some(5) =>
        {
            Ok(true)
        }
        result => result,
    }
}
fn same_session(id: u32) -> bool {
    let (mut mine, mut other) = (0, 0);
    unsafe {
        ProcessIdToSessionId(GetCurrentProcessId(), &mut mine) != 0
            && ProcessIdToSessionId(id, &mut other) != 0
            && mine == other
    }
}
pub fn reg_write(
    hive: usize,
    path: &str,
    name: &str,
    kind: u32,
    data: &[u8],
    delete: bool,
) -> Result<()> {
    unsafe {
        let mut key = std::ptr::null_mut();
        let mut disposition = 0;
        let e = RegCreateKeyExW(
            (hive as u32 as i32 as isize) as Handle,
            wide(path).as_ptr(),
            0,
            std::ptr::null(),
            4,
            0x100,
            std::ptr::null_mut(),
            &mut key,
            &mut disposition,
        );
        if e != 0 {
            return Err(error(e));
        }
        let e = if delete {
            RegDeleteValueW(key, wide(name).as_ptr())
        } else {
            RegSetValueExW(
                key,
                wide(name).as_ptr(),
                0,
                kind,
                data.as_ptr(),
                data.len() as u32,
            )
        };
        RegCloseKey(key);
        if e != 0 && !(delete && e == 2) {
            return Err(error(e));
        }
        Ok(())
    }
}
pub fn run(program: &str, args: &[&str]) -> Result<(i32, String)> {
    let out = Command::new(program)
        .args(args)
        .creation_flags(0x08000000)
        .output()?;
    let bytes = [out.stdout, out.stderr].concat();
    let text = if bytes.starts_with(&[255, 254]) {
        String::from_utf16_lossy(
            &bytes[2..]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|p| u16::from_le_bytes([p[0], p[1]]))
                .collect::<Vec<_>>(),
        )
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    Ok((out.status.code().unwrap_or(-1), text))
}
pub fn command(program: &str, args: &[&str]) -> Result<String> {
    let (code, text) = run(program, args)?;
    ensure!(code == 0, "{program} ({code}): {}", text.trim());
    Ok(text)
}
pub fn firewall(profile: &str, notify: bool, value: Option<bool>) -> Result<bool> {
    if value.is_none() && !service_running("MpsSvc")? {
        return Ok(false);
    }
    com()?;
    unsafe {
        let p: INetFwPolicy2 = CoCreateInstance(&NetFwPolicy2, None, CLSCTX_INPROC_SERVER)?;
        let profile = NET_FW_PROFILE_TYPE2(match profile {
            "Domain" => 1,
            "Private" => 2,
            _ => 4,
        });
        if let Some(v) = value {
            if notify {
                p.put_NotificationsDisabled(profile, (!v).into())?
            } else {
                p.put_FirewallEnabled(profile, v.into())?
            }
        }
        Ok(if notify {
            !p.get_NotificationsDisabled(profile)?.as_bool()
        } else {
            p.get_FirewallEnabled(profile)?.as_bool()
        })
    }
}
pub fn task_root() -> Result<ITaskFolder> {
    com()?;
    unsafe {
        let s: ITaskService = CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER)?;
        let empty = VARIANT::default();
        s.Connect(&empty, &empty, &empty, &empty)?;
        Ok(s.GetFolder(&BSTR::from("\\"))?)
    }
}
fn collect_tasks(folder: &ITaskFolder, list: &mut Vec<String>) -> Result<()> {
    unsafe {
        let tasks = folder.GetTasks(1)?;
        for i in 1..=tasks.Count()? {
            list.push(tasks.get_Item(&VARIANT::from(i))?.Path()?.to_string())
        }
        let folders = folder.GetFolders(0)?;
        for i in 1..=folders.Count()? {
            collect_tasks(&folders.get_Item(&VARIANT::from(i))?, list)?
        }
    }
    Ok(())
}
pub fn tasks() -> Result<Vec<String>> {
    let mut list = vec![];
    collect_tasks(&task_root()?, &mut list)?;
    Ok(list)
}
fn registered_task(path: &str) -> Result<Option<IRegisteredTask>> {
    unsafe {
        match task_root()?.GetTask(&BSTR::from(path)) {
            Ok(t) => Ok(Some(t)),
            Err(e) if matches!(e.code().0 as u32, 0x80070002 | 0x80070490) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}
pub fn task_enabled(path: &str) -> Result<bool> {
    match registered_task(path)? {
        Some(t) => Ok(unsafe { t.Enabled()?.as_bool() }),
        None => Ok(false),
    }
}
pub fn set_task_enabled(path: &str, enabled: bool) -> Result<()> {
    let _guard = system()?;
    let Some(t) = registered_task(path)? else {
        return Ok(());
    };
    unsafe {
        if t.Enabled()?.as_bool() != enabled
            && let Err(e) = t.SetEnabled(enabled.into())
            && task_enabled(path)? != enabled
        {
            return Err(e.into());
        }
    }
    Ok(())
}
pub fn logon() -> Result<u64> {
    use windows::Win32::{Foundation::*, Security::*};
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken_typed(GetCurrentProcess_typed(), TOKEN_QUERY, &mut token)?;
        let mut stats = TOKEN_STATISTICS::default();
        let mut size = 0;
        let result = GetTokenInformation(
            token,
            TokenStatistics,
            Some((&mut stats as *mut TOKEN_STATISTICS).cast()),
            std::mem::size_of_val(&stats) as u32,
            &mut size,
        );
        windows::Win32::Foundation::CloseHandle(token)?;
        result?;
        Ok((stats.AuthenticationId.HighPart as u32 as u64) << 32
            | stats.AuthenticationId.LowPart as u64)
    }
}
pub fn sid() -> Result<String> {
    use windows::Win32::{Foundation::*, Security::*};
    unsafe {
        let mut t = HANDLE::default();
        OpenProcessToken_typed(GetCurrentProcess_typed(), TOKEN_QUERY, &mut t)?;
        let mut size = 0;
        let _ = GetTokenInformation(t, TokenUser, None, 0, &mut size);
        let mut data = vec![0u64; (size as usize).div_ceil(8)];
        GetTokenInformation(
            t,
            TokenUser,
            Some(data.as_mut_ptr().cast()),
            size,
            &mut size,
        )?;
        let info = &*(data.as_ptr() as *const TOKEN_USER);
        let mut out = windows::core::PWSTR::null();
        windows::Win32::Security::Authorization::ConvertSidToStringSidW(info.User.Sid, &mut out)?;
        let s = out.to_string()?;
        windows::Win32::Foundation::LocalFree(Some(HLOCAL(out.0.cast())));
        windows::Win32::Foundation::CloseHandle(t)?;
        Ok(s)
    }
}
use windows::Win32::{
    System::Threading::GetCurrentProcess as GetCurrentProcess_typed,
    System::Threading::OpenProcessToken as OpenProcessToken_typed,
};
pub fn admin() -> Result<bool> {
    use windows::Win32::{Foundation::*, Security::*};
    unsafe {
        let mut t = HANDLE::default();
        OpenProcessToken_typed(GetCurrentProcess_typed(), TOKEN_QUERY, &mut t)?;
        let mut v = TOKEN_ELEVATION::default();
        let mut size = 0;
        let r = GetTokenInformation(
            t,
            TokenElevation,
            Some((&mut v as *mut TOKEN_ELEVATION).cast()),
            std::mem::size_of_val(&v) as u32,
            &mut size,
        );
        windows::Win32::Foundation::CloseHandle(t)?;
        r?;
        Ok(v.TokenIsElevated != 0)
    }
}
pub fn elevate(args: &[String]) -> Result<()> {
    use windows::Win32::UI::{Shell::*, WindowsAndMessaging::*};
    let exe = wide(&std::env::current_exe()?.to_string_lossy());
    let verb = wide("runas");
    let args = wide(
        &args
            .iter()
            .map(|s| format!("\"{}\"", s.replace('"', "")))
            .collect::<Vec<_>>()
            .join(" "),
    );
    unsafe {
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            lpVerb: PCWSTR(verb.as_ptr()),
            lpFile: PCWSTR(exe.as_ptr()),
            lpParameters: PCWSTR(args.as_ptr()),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        ShellExecuteExW(&mut info)?
    }
    Ok(())
}
pub fn open_console() -> Result<()> {
    use windows::Win32::Storage::FileSystem::*;
    use windows::Win32::System::Console::*;
    unsafe {
        if GetConsoleWindow().0.is_null() && AttachConsole(ATTACH_PARENT_PROCESS).is_err() {
            AllocConsole()?;
        }
        SetConsoleOutputCP(65001)?;
        SetConsoleCP(65001)?;
        let output = CreateFileW(
            PCWSTR(wide("CONOUT$").as_ptr()),
            0xc0000000,
            FILE_SHARE_MODE(3),
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )?;
        let input = CreateFileW(
            PCWSTR(wide("CONIN$").as_ptr()),
            0xc0000000,
            FILE_SHARE_MODE(3),
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )?;
        SetStdHandle(STD_OUTPUT_HANDLE, output)?;
        SetStdHandle(STD_ERROR_HANDLE, output)?;
        SetStdHandle(STD_INPUT_HANDLE, input)?;
        let title = format!("Zero Security Windows {}", env!("CARGO_PKG_VERSION"));
        SetConsoleTitleW(PCWSTR(wide(&title).as_ptr()))?;
    }
    Ok(())
}
pub fn hide_console_scrollbars() {
    unsafe {
        let window = windows::Win32::System::Console::GetConsoleWindow();
        if !window.0.is_null() {
            let _ = windows::Win32::UI::Controls::ShowScrollBar(
                window,
                windows::Win32::UI::WindowsAndMessaging::SB_BOTH,
                false,
            );
        }
    }
}
pub struct ConsoleBackground {
    index: usize,
    original: u32,
}
impl ConsoleBackground {
    pub fn new(r: u8, g: u8, b: u8) -> Result<Self> {
        use windows::Win32::System::Console::*;
        let mut info = CONSOLE_SCREEN_BUFFER_INFOEX {
            cbSize: std::mem::size_of::<CONSOLE_SCREEN_BUFFER_INFOEX>() as u32,
            ..Default::default()
        };
        unsafe {
            GetConsoleScreenBufferInfoEx(GetStdHandle(STD_OUTPUT_HANDLE)?, &mut info)?;
        }
        let index = ((info.wAttributes.0 >> 4) & 15) as usize;
        let background = Self {
            index,
            original: info.ColorTable[index].0,
        };
        background.set(r, g, b)?;
        Ok(background)
    }
    pub fn set(&self, r: u8, g: u8, b: u8) -> Result<()> {
        use std::io::Write;
        let mut output = std::io::stdout();
        // 经典控制台预留的滚动条区域使用默认背景调色板，不属于字符网格。
        write!(
            output,
            "\x1b]4;{};rgb:{r:02x}/{g:02x}/{b:02x}\x1b\\",
            self.index
        )?;
        output.flush()?;
        Ok(())
    }
}
impl Drop for ConsoleBackground {
    fn drop(&mut self) {
        let _ = self.set(
            self.original as u8,
            (self.original >> 8) as u8,
            (self.original >> 16) as u8,
        );
    }
}
pub fn known_folder(id: &GUID) -> Result<PathBuf> {
    unsafe {
        let s = windows::Win32::UI::Shell::SHGetKnownFolderPath(
            id,
            windows::Win32::UI::Shell::KF_FLAG_DEFAULT,
            None,
        )?;
        let p = PathBuf::from(s.to_string()?);
        CoTaskMemFree(Some(s.0.cast()));
        Ok(p)
    }
}
pub fn wmi(namespace: &str, query: &str) -> Result<Value> {
    let c = wmi::WMIConnection::with_namespace_path(namespace)?;
    let rows: Vec<std::collections::HashMap<String, wmi::Variant>> = c.raw_query(query)?;
    Ok(serde_json::to_value(rows)?)
}
pub fn disable_system_restore() -> Result<()> {
    #[derive(serde::Deserialize)]
    struct SystemRestore;
    #[derive(serde::Serialize)]
    struct Input {
        #[serde(rename = "Drive")]
        drive: String,
    }
    #[derive(serde::Deserialize)]
    struct Output {
        #[serde(rename = "ReturnValue")]
        code: u32,
    }
    let c = wmi::WMIConnection::with_namespace_path(r"root\default")?;
    // Disabling the system drive disables every volume without locking GUI controls.
    let result: Output = c.exec_class_method::<SystemRestore, _>(
        "Disable",
        Input {
            drive: expand(r"%SystemDrive%\"),
        },
    )?;
    ensure!(
        result.code == 0,
        "SystemRestore.Disable: {:08X}",
        result.code
    );
    Ok(())
}
pub fn delivery_mode(value: Option<u32>) -> Result<u32> {
    let c =
        wmi::WMIConnection::with_namespace_path(r"root\Microsoft\Windows\DeliveryOptimization")?;
    if let Some(value) = value {
        let input = c
            .get_object("MSFT_DeliveryOptimizationConfig")?
            .get_method("SetDownloadMode")?
            .unwrap()
            .spawn_instance()?;
        input.put_property("downloadMode", value)?;
        c.exec_method(
            "MSFT_DeliveryOptimizationConfig",
            "SetDownloadMode",
            Some(&input),
        )?;
    }
    #[derive(serde::Deserialize)]
    struct Config {
        #[serde(rename = "DownloadMode")]
        mode: u32,
    }
    let rows: Vec<Config> =
        c.raw_query("SELECT DownloadMode FROM MSFT_DeliveryOptimizationConfig")?;
    Ok(rows
        .first()
        .ok_or_else(|| anyhow::anyhow!("No Delivery Optimization configuration"))?
        .mode)
}
pub fn expand(text: &str) -> String {
    let mut result = text.to_owned();
    for (k, v) in std::env::vars() {
        result = result.replace(&format!("%{k}%"), &v);
    }
    result
}
pub fn uninstaller(text: &str) -> Result<Vec<String>> {
    use windows::Win32::UI::Shell::CommandLineToArgvW;
    unsafe {
        let mut count = 0;
        let p = CommandLineToArgvW(PCWSTR(wide(text).as_ptr()), &mut count);
        ensure!(!p.is_null(), "Invalid uninstaller command");
        let mut args = vec![];
        for a in std::slice::from_raw_parts(p, count as usize) {
            args.push(a.to_string()?)
        }
        windows::Win32::Foundation::LocalFree(Some(windows::Win32::Foundation::HLOCAL(p.cast())));
        Ok(args)
    }
}
pub fn dism(args: &[&str]) -> Result<bool> {
    let (code, text) = run("dism.exe", args)?;
    ensure!(matches!(code, 0 | 3010 | 1641), "DISM ({code}): {text}");
    Ok(code != 0)
}
#[cfg(test)]
pub fn block_execute(path: &Path) -> Result<()> {
    deny_file_access(path, 0x20)
}
#[cfg(test)]
fn deny_file_access(path: &Path, mask: u32) -> Result<()> {
    use anyhow::bail;
    use windows::Win32::{
        Foundation::*,
        Security::{Authorization::*, *},
    };
    unsafe {
        let path = wide(&path.to_string_lossy());
        let mut old = std::ptr::null_mut();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        let e = GetNamedSecurityInfoW(
            PCWSTR(path.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(&mut old),
            None,
            &mut descriptor,
        );
        ensure!(e.0 == 0, "Read file ACL: {}", e.0);
        let mut entry = EXPLICIT_ACCESS_W {
            grfAccessPermissions: mask,
            grfAccessMode: DENY_ACCESS,
            grfInheritance: ACE_FLAGS(0),
            Trustee: TRUSTEE_W {
                TrusteeForm: TRUSTEE_IS_NAME,
                TrusteeType: TRUSTEE_IS_WELL_KNOWN_GROUP,
                ptstrName: windows::core::PWSTR::null(),
                ..Default::default()
            },
        };
        let mut world = windows::Win32::Security::PSID::default();
        ConvertStringSidToSidW(PCWSTR(wide("S-1-1-0").as_ptr()), &mut world)?;
        entry.Trustee.TrusteeForm = TRUSTEE_IS_SID;
        entry.Trustee.ptstrName = windows::core::PWSTR(world.0.cast());
        let mut new = std::ptr::null_mut();
        let e = SetEntriesInAclW(Some(&[entry]), Some(old), &mut new);
        if e.0 == 0 {
            let file = windows::Win32::Storage::FileSystem::CreateFileW(
                PCWSTR(path.as_ptr()),
                0x60000,
                windows::Win32::Storage::FileSystem::FILE_SHARE_MODE(7),
                None,
                windows::Win32::Storage::FileSystem::OPEN_EXISTING,
                windows::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS,
                None,
            )?;
            let e = SetSecurityInfo(
                file,
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(new),
                None,
            );
            windows::Win32::Foundation::CloseHandle(file)?;
            LocalFree(Some(HLOCAL(new.cast())));
            LocalFree(Some(HLOCAL(descriptor.0)));
            LocalFree(Some(HLOCAL(world.0)));
            ensure!(e.0 == 0, "Write file ACL: {}", e.0);
        } else {
            LocalFree(Some(HLOCAL(descriptor.0)));
            LocalFree(Some(HLOCAL(world.0)));
            bail!("Update file ACL: {}", e.0);
        }
        Ok(())
    }
}
pub fn clear_execute_deny(path: &Path) -> Result<()> {
    use windows::Win32::{
        Foundation::*,
        Security::{Authorization::*, *},
    };
    unsafe {
        let mut acl = std::ptr::null_mut();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        let e = GetNamedSecurityInfoW(
            PCWSTR(wide(&path.to_string_lossy()).as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(&mut acl),
            None,
            &mut descriptor,
        );
        ensure!(e.0 == 0, "Read file ACL: {}", e.0);
        let mut world = PSID::default();
        ConvertStringSidToSidW(PCWSTR(wide("S-1-1-0").as_ptr()), &mut world)?;
        let result = (|| -> Result<()> {
            let mut changed = false;
            if !acl.is_null() {
                let mut i = 0;
                while i < (*acl).AceCount as u32 {
                    let mut entry = std::ptr::null_mut();
                    GetAce(acl, i, &mut entry)?;
                    let ace = &mut *(entry as *mut ACCESS_DENIED_ACE);
                    if ace.Header.AceType == 1
                        && ace.Header.AceFlags & 0x10 == 0
                        && ace.Mask & 0x20 != 0
                        && EqualSid(PSID((&ace.SidStart as *const u32).cast_mut().cast()), world)
                            .is_ok()
                    {
                        ace.Mask &= !0x20;
                        changed = true;
                        if ace.Mask == 0 {
                            DeleteAce(acl, i)?;
                            continue;
                        }
                    }
                    i += 1;
                }
            }
            if changed {
                let file = windows::Win32::Storage::FileSystem::CreateFileW(
                    PCWSTR(wide(&path.to_string_lossy()).as_ptr()),
                    0x60000,
                    windows::Win32::Storage::FileSystem::FILE_SHARE_MODE(7),
                    None,
                    windows::Win32::Storage::FileSystem::OPEN_EXISTING,
                    windows::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS,
                    None,
                )?;
                let e = SetSecurityInfo(
                    file,
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    None,
                    None,
                    Some(acl),
                    None,
                );
                windows::Win32::Foundation::CloseHandle(file)?;
                ensure!(e.0 == 0, "Write file ACL: {}", e.0);
            }
            Ok(())
        })();
        LocalFree(Some(HLOCAL(descriptor.0)));
        LocalFree(Some(HLOCAL(world.0)));
        result
    }
}
pub fn execute_blocked(path: &Path) -> Result<bool> {
    use windows::Win32::{
        Foundation::*,
        Security::{Authorization::*, *},
    };
    unsafe {
        let mut acl = std::ptr::null_mut();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        let e = GetNamedSecurityInfoW(
            PCWSTR(wide(&path.to_string_lossy()).as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(&mut acl),
            None,
            &mut descriptor,
        );
        ensure!(e.0 == 0, "Read file ACL: {}", e.0);
        let mut world = PSID::default();
        ConvertStringSidToSidW(PCWSTR(wide("S-1-1-0").as_ptr()), &mut world)?;
        let mut found = false;
        if !acl.is_null() {
            for i in 0..(*acl).AceCount as u32 {
                let mut p = std::ptr::null_mut();
                GetAce(acl, i, &mut p)?;
                let ace = &*(p as *const ACCESS_DENIED_ACE);
                if ace.Header.AceType == 1
                    && ace.Mask & 0x20 != 0
                    && EqualSid(PSID((&ace.SidStart as *const u32).cast_mut().cast()), world)
                        .is_ok()
                {
                    found = true;
                    break;
                }
            }
        }
        LocalFree(Some(HLOCAL(descriptor.0)));
        LocalFree(Some(HLOCAL(world.0)));
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_acl_repair_leaves_other_denied_rights_in_place() -> Result<()> {
        let file = std::env::temp_dir().join(format!("zsw-acl-repair-{}.tmp", std::process::id()));
        std::fs::write(&file, b"test")?;
        let result = (|| {
            deny_file_access(&file, 0x22)?;
            assert!(execute_blocked(&file)?);
            clear_execute_deny(&file)?;
            assert!(!execute_blocked(&file)?);
            assert_eq!(std::fs::read(&file)?, b"test");
            assert_eq!(
                std::fs::write(&file, b"changed")
                    .unwrap_err()
                    .raw_os_error(),
                Some(5)
            );
            clear_execute_deny(&file)?;
            assert_eq!(
                std::fs::write(&file, b"changed")
                    .unwrap_err()
                    .raw_os_error(),
                Some(5)
            );
            Ok(())
        })();
        std::fs::remove_file(file)?;
        result
    }
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn CreateServiceW(
            manager: Handle,
            name: *const u16,
            display: *const u16,
            access: u32,
            kind: u32,
            start: u32,
            error: u32,
            binary: *const u16,
            group: *const u16,
            tag: Handle,
            dependencies: *const u16,
            account: *const u16,
            password: *const u16,
        ) -> Handle;
        fn DeleteService(service: Handle) -> i32;
    }
    #[test]
    fn a_temporary_service_can_be_disabled_without_starting_it() -> Result<()> {
        unsafe {
            let manager = OpenSCManagerW(std::ptr::null(), std::ptr::null(), 2);
            ensure!(!manager.is_null(), "{}", std::io::Error::last_os_error());
            let name = format!("ZeroSecurityWindowsTest{}", std::process::id());
            let svc = CreateServiceW(
                manager,
                wide(&name).as_ptr(),
                wide(&name).as_ptr(),
                0x10026,
                0x10,
                3,
                1,
                wide(r"C:\Windows\System32\cmd.exe").as_ptr(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
            );
            if svc.is_null() {
                let error = std::io::Error::last_os_error();
                CloseServiceHandle(manager);
                return Err(error.into());
            }
            let result = (|| {
                assert!(!service_running(&name)?);
                assert!(!disable_service(&name)?);
                assert_eq!(
                    crate::registry::number(
                        &format!(r"HKLM:\SYSTEM\CurrentControlSet\Services\{name}"),
                        "Start",
                        0
                    )?,
                    4
                );
                assert!(!service_running(&name)?);
                Ok(())
            })();
            let deleted = ok(DeleteService(svc));
            CloseServiceHandle(svc);
            CloseServiceHandle(manager);
            deleted?;
            result
        }
    }
}
