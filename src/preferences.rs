use crate::{
    native::{self, Handle, wide},
    registry as reg,
};
use anyhow::{Result, ensure};
use std::path::PathBuf;
use windows::Win32::{System::Com::*, UI::TextServices::*};
use windows::core::{GUID, HSTRING, Interface};

fn reveal_page(policy: &str, page: &str) -> Option<String> {
    let (mode, list) = policy.split_once(':')?;
    let mut pages: Vec<&str> = list.split(';').filter(|p| !p.is_empty()).collect();
    let listed = pages.iter().any(|p| p.eq_ignore_ascii_case(page));
    match mode {
        "hide" if listed => pages.retain(|p| !p.eq_ignore_ascii_case(page)),
        "showonly" if !listed => pages.push(page),
        _ => return None,
    }
    Some(if pages.is_empty() {
        String::new()
    } else {
        format!("{mode}:{}", pages.join(";"))
    })
}
pub fn settings_page(page: &str, show: bool) -> Result<bool> {
    let mut visible = true;
    for hive in ["HKLM:", "HKCU:"] {
        let path = format!(r"{hive}\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\Explorer");
        let policy = reg::read(&path, "SettingsPageVisibility")?;
        if let Some(value) = policy.as_str().and_then(|p| reveal_page(p, page)) {
            visible = false;
            if show {
                if value.is_empty() {
                    reg::delete(&path, "SettingsPageVisibility")?;
                } else {
                    reg::set(&path, "SettingsPageVisibility", value, "String")?;
                }
            }
        }
    }
    Ok(show || visible)
}

#[cfg(test)]
mod settings_tests {
    use super::reveal_page;
    #[test]
    fn reveal_location_preserves_other_settings_page_choices() {
        assert_eq!(
            reveal_page("hide:recovery;privacy-location;maps", "privacy-location"),
            Some("hide:recovery;maps".into())
        );
        assert_eq!(
            reveal_page("hide:privacy-location", "privacy-location"),
            Some(String::new())
        );
        assert_eq!(
            reveal_page("showonly:about", "privacy-location"),
            Some("showonly:about;privacy-location".into())
        );
        assert_eq!(reveal_page("hide:maps", "privacy-location"), None);
        assert_eq!(
            reveal_page("showonly:privacy-location", "privacy-location"),
            None
        );
    }
}

#[link(name = "user32")]
unsafe extern "system" {
    fn SystemParametersInfoW(action: u32, param: u32, data: Handle, flags: u32) -> i32;
    fn SendMessageTimeoutW(
        window: Handle,
        message: u32,
        wparam: usize,
        lparam: *const u16,
        flags: u32,
        timeout: u32,
        result: *mut usize,
    ) -> isize;
}
#[link(name = "powrprof")]
unsafe extern "system" {
    fn PowerGetActiveScheme(root: Handle, out: *mut *mut GUID) -> u32;
    fn PowerReadACValueIndex(
        root: Handle,
        scheme: *const GUID,
        group: *const GUID,
        name: *const GUID,
        out: *mut u32,
    ) -> u32;
    fn PowerReadDCValueIndex(
        root: Handle,
        scheme: *const GUID,
        group: *const GUID,
        name: *const GUID,
        out: *mut u32,
    ) -> u32;
    fn PowerWriteACValueIndex(
        root: Handle,
        scheme: *const GUID,
        group: *const GUID,
        name: *const GUID,
        value: u32,
    ) -> u32;
    fn PowerWriteDCValueIndex(
        root: Handle,
        scheme: *const GUID,
        group: *const GUID,
        name: *const GUID,
        value: u32,
    ) -> u32;
    fn PowerSetActiveScheme(root: Handle, scheme: *const GUID) -> u32;
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn LocalFree(h: Handle) -> Handle;
}
#[link(name = "shell32")]
unsafe extern "system" {
    fn SHChangeNotify(event: u32, flags: u32, item1: Handle, item2: Handle);
    fn SHFileOperationW(op: *mut FileOperation) -> i32;
}
#[repr(C)]
struct FileOperation {
    window: Handle,
    operation: u32,
    from: *const u16,
    to: *const u16,
    flags: u16,
    aborted: i32,
    mappings: Handle,
    title: *const u16,
}
#[repr(C)]
#[derive(Default)]
pub struct FilterKeys {
    pub size: u32,
    pub flags: u32,
    pub wait: u32,
    pub delay: u32,
    pub repeat: u32,
    pub bounce: u32,
}
#[repr(C)]
struct StickyKeys {
    size: u32,
    flags: u32,
}
#[repr(C)]
struct Animation {
    size: u32,
    enabled: i32,
}
pub fn spi(action: u32, param: u32, data: Handle, flags: u32) -> Result<()> {
    native::ok(unsafe { SystemParametersInfoW(action, param, data, flags) })
}
pub fn mouse() -> Result<[i32; 5]> {
    let mut out = [0i32; 5];
    spi(3, 0, out.as_mut_ptr().cast(), 0)?;
    spi(0x70, 0, (&mut out[3] as *mut i32).cast(), 0)?;
    spi(0x68, 0, (&mut out[4] as *mut i32).cast(), 0)?;
    Ok(out)
}
pub fn apply_mouse() -> Result<()> {
    let p = r"HKCU:\Control Panel\Mouse";
    let mut v = [
        reg::number(p, "MouseThreshold1", 0)? as i32,
        reg::number(p, "MouseThreshold2", 0)? as i32,
        reg::number(p, "MouseSpeed", 0)? as i32,
    ];
    spi(4, 0, v.as_mut_ptr().cast(), 2)?;
    spi(
        0x71,
        0,
        reg::number(p, "MouseSensitivity", 20)? as usize as Handle,
        2,
    )?;
    spi(
        0x69,
        reg::number(r"HKCU:\Control Panel\Desktop", "WheelScrollLines", 6)? as u32,
        std::ptr::null_mut(),
        2,
    )
}
pub fn keyboard() -> Result<FilterKeys> {
    let mut v = FilterKeys {
        size: 24,
        ..Default::default()
    };
    spi(0x32, 24, (&mut v as *mut FilterKeys).cast(), 0)?;
    Ok(v)
}
pub fn sticky() -> Result<u32> {
    let mut v = StickyKeys { size: 8, flags: 0 };
    spi(0x3a, 8, (&mut v as *mut StickyKeys).cast(), 0)?;
    Ok(v.flags)
}
pub fn apply_keyboard() -> Result<()> {
    let p = r"HKCU:\Control Panel\Accessibility\Keyboard Response";
    let mut v = FilterKeys {
        size: 24,
        flags: reg::number(p, "Flags", 1)? as u32,
        wait: reg::number(p, "DelayBeforeAcceptance", 0)? as u32,
        delay: reg::number(p, "AutoRepeatDelay", 140)? as u32,
        repeat: reg::number(p, "AutoRepeatRate", 16)? as u32,
        bounce: reg::number(p, "BounceTime", 0)? as u32,
    };
    spi(0x33, 24, (&mut v as *mut FilterKeys).cast(), 2)?;
    let mut s = StickyKeys {
        size: 8,
        flags: reg::number(r"HKCU:\Control Panel\Accessibility\StickyKeys", "Flags", 0)? as u32,
    };
    spi(0x3b, 8, (&mut s as *mut StickyKeys).cast(), 2)
}
pub fn visuals() -> Result<()> {
    let mut a = Animation {
        size: 8,
        enabled: reg::number(
            r"HKCU:\Control Panel\Desktop\WindowMetrics",
            "MinAnimate",
            0,
        )? as i32,
    };
    spi(0x49, 8, (&mut a as *mut Animation).cast(), 2)?;
    spi(
        0x4b,
        if reg::number(r"HKCU:\Control Panel\Desktop", "FontSmoothing", 2)? == 2 {
            1
        } else {
            0
        },
        std::ptr::null_mut(),
        2,
    )?;
    spi(
        0x25,
        reg::number(r"HKCU:\Control Panel\Desktop", "DragFullWindows", 1)? as u32,
        std::ptr::null_mut(),
        2,
    )?;
    broadcast("WindowMetrics");
    Ok(())
}
pub fn broadcast(value: &str) {
    unsafe {
        SendMessageTimeoutW(
            0xffffusize as Handle,
            0x1a,
            0,
            wide(value).as_ptr(),
            2,
            1000,
            &mut 0,
        );
    }
}
pub fn explorer() {
    broadcast("TraySettings");
    broadcast("ShellState");
    unsafe {
        SHChangeNotify(0x08000000, 0, std::ptr::null_mut(), std::ptr::null_mut());
    }
}
pub fn wallpaper() -> Result<()> {
    let mut path = vec![0u16; 32768];
    spi(0x73, path.len() as u32, path.as_mut_ptr().cast(), 0)?;
    spi(0x14, 0, path.as_mut_ptr().cast(), 2)
}
pub fn recycle(paths: &[PathBuf]) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let from = wide(
        &(paths
            .iter()
            .map(|p| p.to_string_lossy())
            .collect::<Vec<_>>()
            .join("\0")
            + "\0"),
    );
    let mut op = FileOperation {
        window: std::ptr::null_mut(),
        operation: 3,
        from: from.as_ptr(),
        to: std::ptr::null(),
        flags: 0x454,
        aborted: 0,
        mappings: std::ptr::null_mut(),
        title: std::ptr::null(),
    };
    let code = unsafe { SHFileOperationW(&mut op) };
    ensure!(code == 0 && op.aborted == 0, "Desktop cleanup: {code}");
    Ok(())
}
pub fn guid(s: &str) -> Result<GUID> {
    Ok(GUID::try_from(s.trim_matches(['{', '}']))?)
}
fn power_ok(code: u32) -> Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(native::error(code))
    }
}
pub fn scheme() -> Result<String> {
    let mut p = std::ptr::null_mut();
    power_ok(unsafe { PowerGetActiveScheme(std::ptr::null_mut(), &mut p) })?;
    let id = unsafe { *p };
    unsafe {
        LocalFree(p.cast());
    }
    Ok(format!("{id:?}"))
}
pub fn power(s: &str, g: &str, n: &str, ac: bool, value: Option<u32>) -> Result<u32> {
    let (s, g, n) = (guid(s)?, guid(g)?, guid(n)?);
    let root = std::ptr::null_mut();
    if let Some(v) = value {
        power_ok(unsafe {
            if ac {
                PowerWriteACValueIndex(root, &s, &g, &n, v)
            } else {
                PowerWriteDCValueIndex(root, &s, &g, &n, v)
            }
        })?;
    }
    let mut out = 0;
    power_ok(unsafe {
        if ac {
            PowerReadACValueIndex(root, &s, &g, &n, &mut out)
        } else {
            PowerReadDCValueIndex(root, &s, &g, &n, &mut out)
        }
    })?;
    Ok(out)
}
pub fn activate_power() -> Result<()> {
    power_ok(unsafe { PowerSetActiveScheme(std::ptr::null_mut(), &guid(&scheme()?)?) })
}
pub fn language_bar(value: Option<u32>) -> Result<u32> {
    native::com()?;
    unsafe {
        let bar: ITfLangBarMgr =
            CoCreateInstance(&CLSID_TF_LangBarMgr, None, CLSCTX_INPROC_SERVER)?;
        if let Some(v) = value {
            bar.ShowFloating(v)?;
        }
        Ok(bar.GetShowFloatingStatus()?)
    }
}
#[repr(C)]
#[derive(Clone)]
struct LayoutProfile {
    kind: u32,
    language: u16,
    clsid: GUID,
    profile: GUID,
    category: GUID,
    substitute: u32,
    flags: u32,
    id: [u16; 260],
}
#[link(name = "input", kind = "raw-dylib")]
unsafe extern "system" {
    fn EnumEnabledLayoutOrTip(
        user: *const u16,
        system: *const u16,
        software: *const u16,
        out: *mut LayoutProfile,
        count: u32,
    ) -> u32;
    fn InstallLayoutOrTip(profile: *const u16, flags: u32) -> i32;
    fn SetDefaultLayoutOrTip(profile: *const u16, flags: u32) -> i32;
}
fn layouts() -> Vec<LayoutProfile> {
    unsafe {
        let null = std::ptr::null();
        let count = EnumEnabledLayoutOrTip(null, null, null, std::ptr::null_mut(), 0);
        let mut out = vec![std::mem::zeroed(); count as usize];
        let actual = EnumEnabledLayoutOrTip(null, null, null, out.as_mut_ptr(), count);
        out.truncate(actual as usize);
        out
    }
}
const WETYPE_CLASS: GUID = GUID::from_u128(0x86598fb9_66a2_463e_b9c2_aeb906d477ad);
const WETYPE_PROFILE: GUID = GUID::from_u128(0x607fdf85_fcc8_4dbd_a365_41296f980c9c);
pub fn input_only() -> Result<bool> {
    native::com()?;
    unsafe {
        let profiles: ITfInputProcessorProfiles =
            CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER)?;
        let mut active = vec![];
        for p in layouts() {
            if p.kind == 2
                || profiles
                    .IsEnabledLanguageProfile(&p.clsid, p.language, &p.profile)?
                    .as_bool()
            {
                active.push(p)
            }
        }
        Ok(active.len() == 1
            && active[0].clsid == WETYPE_CLASS
            && active[0].profile == WETYPE_PROFILE)
    }
}
pub fn apply_input() -> Result<()> {
    native::com()?;
    let tip = "0804:{86598FB9-66A2-463E-B9C2-AEB906D477AD}{607FDF85-FCC8-4DBD-A365-41296F980C9C}";
    ensure!(
        reg::exists(r"HKLM:\SOFTWARE\Microsoft\CTF\TIP\{86598FB9-66A2-463E-B9C2-AEB906D477AD}")?,
        "WeType is not registered"
    );
    unsafe {
        let p: ITfInputProcessorProfiles =
            CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER)?;
        native::ok(InstallLayoutOrTip(wide(tip).as_ptr(), 0))?;
        for profile in layouts() {
            if profile.clsid == WETYPE_CLASS && profile.profile == WETYPE_PROFILE {
                continue;
            }
            let id = String::from_utf16_lossy(
                &profile.id[..profile.id.iter().position(|v| *v == 0).unwrap_or(260)],
            );
            if !id.is_empty() {
                native::ok(InstallLayoutOrTip(wide(&id).as_ptr(), 1))?;
            }
            if profile.kind == 1 {
                p.EnableLanguageProfile(&profile.clsid, profile.language, &profile.profile, false)?;
            }
        }
        p.EnableLanguageProfile(&WETYPE_CLASS, 0x804, &WETYPE_PROFILE, true)?;
        p.SetDefaultLanguageProfile(0x804, &WETYPE_CLASS, &WETYPE_PROFILE)?;
        native::ok(SetDefaultLayoutOrTip(wide(tip).as_ptr(), 0))?;
    }
    reg::set(
        r"HKCU:\Control Panel\International\User Profile",
        "Languages",
        serde_json::json!(["zh-CN"]),
        "MultiString",
    )?;
    reg::set(
        r"HKCU:\Software\Microsoft\CTF\SortOrder",
        "Default",
        tip,
        "String",
    )?;
    broadcast("intl");
    Ok(())
}
#[repr(C)]
struct CursorColor {
    a: u8,
    r: u8,
    g: u8,
    b: u8,
}
pub fn cursors() -> Result<()> {
    native::com()?;
    let size = reg::number(r"HKCU:\Control Panel\Cursors", "CursorBaseSize", 64)? as u32;
    let class = HSTRING::from("Windows.Internal.Accessibility.Experience.CustomCursor");
    unsafe {
        let object = windows::Win32::System::WinRT::RoActivateInstance(&class)?;
        let iid = GUID::from_u128(0x30cbcad0_37ac_56b6_8c0b_74b54a8058a5);
        let mut cursor = std::ptr::null_mut();
        object.query(&iid, &mut cursor).ok()?;
        let vtable = *(cursor as *const *const usize);
        let apply: unsafe extern "system" fn(Handle, u32, CursorColor) -> i32 =
            std::mem::transmute(*vtable.add(6));
        let result = windows::core::HRESULT(apply(
            cursor,
            size,
            CursorColor {
                a: 255,
                r: 0,
                g: 0,
                b: 0,
            },
        ))
        .ok();
        let release: unsafe extern "system" fn(Handle) -> u32 = std::mem::transmute(*vtable.add(2));
        release(cursor);
        result?;
    }
    broadcast("Cursors");
    Ok(())
}
