use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Feature {
    pub id: String,
    pub zh: String,
    pub en: String,
    pub purpose_zh: String,
    pub purpose_en: String,
    pub impact_zh: String,
    pub impact_en: String,
    pub group_zh: String,
    pub group_en: String,
    pub probe: String,
    pub page: String,
    pub intent: String,
    pub refresh: String,
    pub safe: bool,
    pub restart: bool,
    pub manual: bool,
    pub ops: Vec<Operation>,
}
impl Default for Feature {
    fn default() -> Self {
        Self {
            id: String::new(),
            zh: String::new(),
            en: String::new(),
            purpose_zh: String::new(),
            purpose_en: String::new(),
            impact_zh: String::new(),
            impact_en: String::new(),
            group_zh: String::new(),
            group_en: String::new(),
            probe: String::new(),
            page: String::new(),
            intent: "Disable".into(),
            refresh: String::new(),
            safe: false,
            restart: false,
            manual: false,
            ops: vec![],
        }
    }
}
impl Feature {
    pub fn name(&self, zh: bool) -> &str {
        if zh { &self.zh } else { &self.en }
    }
    pub fn purpose(&self, zh: bool) -> &str {
        if zh {
            &self.purpose_zh
        } else {
            &self.purpose_en
        }
    }
    pub fn category(&self, zh: bool) -> &str {
        match self.page.as_str() {
            "Install" => choose(zh, "安装", "Install"),
            "Remove" => choose(zh, "卸载", "Remove"),
            "Desktop" => choose(zh, "桌面", "Desktop"),
            _ => {
                if zh {
                    &self.group_zh
                } else {
                    &self.group_en
                }
            }
        }
    }
    pub fn impact(&self, zh: bool) -> &str {
        choose(zh, &self.impact_zh, &self.impact_en)
    }
    pub fn security(&self) -> bool {
        self.page == "System"
            && matches!(
                self.group_zh.as_str(),
                "核心隔离" | "Defender" | "安全" | "缓解"
            )
    }
    pub fn toggle(&self) -> bool {
        self.id == "classic-menu"
    }
    pub fn can_run_safe(&self) -> bool {
        self.ops
            .iter()
            .any(|op| !matches!(op.kind.as_str(), "Task" | "TaskGroup"))
            && self.ops.iter().all(|op| {
                matches!(
                    op.kind.as_str(),
                    "Registry"
                        | "RegistryDelete"
                        | "RegistryKey"
                        | "Service"
                        | "UserServices"
                        | "Asr"
                        | "Bcd"
                        | "UpdatePause"
                        | "ProcessBlock"
                        | "ResumeAccess"
                        | "Task"
                        | "TaskGroup"
                )
            })
    }
}
#[derive(Clone, Default, Debug, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Operation {
    pub kind: String,
    pub path: String,
    pub name: String,
    #[serde(rename = "Type")]
    pub value_type: String,
    pub pattern: String,
    pub property: String,
    pub source: String,
    pub package_family: String,
    pub uninstall_key: String,
    pub group: String,
    pub scheme: String,
    pub service: String,
    pub names: Vec<String>,
    pub patterns: Vec<String>,
    pub system_app: bool,
    pub value: Value,
}
impl Operation {
    pub fn reg(path: impl Into<String>, name: impl Into<String>, value: impl Into<Value>) -> Self {
        Self {
            kind: "Registry".into(),
            path: path.into(),
            name: name.into(),
            value: value.into(),
            value_type: "DWord".into(),
            ..Self::default()
        }
    }
}
pub fn bool_value(v: &Value) -> bool {
    v.as_bool()
        .unwrap_or_else(|| v.as_str().is_some_and(|s| s.eq_ignore_ascii_case("true")))
}
pub fn text_value(v: &Value) -> String {
    if let Some(s) = v.as_str() {
        s.into()
    } else {
        v.to_string()
    }
}
pub fn same(a: &Value, b: &Value) -> bool {
    if a.is_null() || b.is_null() {
        return a == b;
    }
    if a.is_array() || b.is_array() {
        return a == b;
    }
    text_value(a).eq_ignore_ascii_case(&text_value(b))
}
pub fn choose<'a>(zh: bool, a: &'a str, b: &'a str) -> &'a str {
    if zh { a } else { b }
}
pub fn catalog() -> anyhow::Result<Vec<Feature>> {
    Ok(serde_json::from_str(include_str!("../catalog.json"))?)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    PendingCheck,
    Checking,
    Ready,
    Done,
    Unknown,
    Restricted,
    Absent,
    Running,
    Queued,
    SafeQueued,
    Restart,
    SignIn,
    Failed,
    Deferred,
}
#[derive(Clone, Debug, Serialize)]
pub struct Check {
    pub state: Status,
    pub detail: String,
}
impl Check {
    pub fn new(state: Status) -> Self {
        Self {
            state,
            detail: String::new(),
        }
    }
    pub fn active(active: bool) -> Self {
        Self::new(if active { Status::Ready } else { Status::Done })
    }
    pub fn visible(&self, f: &Feature) -> bool {
        !f.manual
    }
    pub fn actionable(&self) -> bool {
        matches!(
            self.state,
            Status::Ready | Status::Failed | Status::PendingCheck | Status::Checking
        )
    }
    pub fn recheckable(&self) -> bool {
        self.state == Status::Unknown
    }
    pub fn summary(&self, f: &Feature, zh: bool) -> &str {
        match self.state {
            Status::PendingCheck => choose(
                zh,
                "尚未读取当前设置。",
                "Current settings have not been checked yet.",
            ),
            Status::Checking => choose(zh, "正在读取当前设置。", "Reading current settings."),
            Status::Ready if f.toggle() => choose(
                zh,
                "当前配置为 Windows 11 右键菜单。",
                "The Windows 11 context menu is selected.",
            ),
            Status::Done if f.toggle() => choose(
                zh,
                "当前配置为 Windows 10 经典右键菜单。",
                "The Windows 10 classic context menu is selected.",
            ),
            Status::Ready => match f.intent.as_str() {
                "Install" => choose(zh, "尚未安装此应用。", "This application is not installed."),
                "Remove" => choose(
                    zh,
                    "检测到可卸载的应用或组件。",
                    "An application or component is available to uninstall.",
                ),
                _ => choose(
                    zh,
                    "当前设置尚未完全符合此项目的目标。",
                    "Current settings do not fully match this item's target.",
                ),
            },
            Status::Done if f.security() && !self.detail.is_empty() => "",
            Status::Done if f.security() => choose(
                zh,
                "可配置的设置已达到此项目的优化目标。",
                "Configurable settings meet this item's optimization target.",
            ),
            Status::Done => match f.intent.as_str() {
                "Install" => choose(zh, "此应用已安装。", "This application is installed."),
                "Remove" => choose(
                    zh,
                    "未检测到此应用或组件。",
                    "This application or component is not installed.",
                ),
                _ => choose(
                    zh,
                    "当前设置已符合此项目的目标。",
                    "Current settings match this item's target.",
                ),
            },
            Status::Unknown => choose(
                zh,
                "Windows 暂未提供可用的状态信息。",
                "Windows has not provided usable status information.",
            ),
            Status::Restricted => choose(
                zh,
                "Windows 限制了此设置的访问，此项目已跳过。",
                "Windows restricts access to this setting; this item has been skipped.",
            ),
            Status::Absent => choose(
                zh,
                "当前系统未提供此功能，或没有需要处理的对象。",
                "This feature is unavailable on this system, or there is nothing to change.",
            ),
            Status::Deferred => choose(
                zh,
                "此项目需要在正常模式下执行。",
                "This item requires normal mode.",
            ),
            Status::Running => match f.intent.as_str() {
                "Install" => choose(zh, "正在安装此应用。", "Installing this application."),
                "Remove" => choose(
                    zh,
                    "正在卸载应用或组件。",
                    "Uninstalling the application or component.",
                ),
                _ => choose(
                    zh,
                    "正在应用此项目的设置。",
                    "Applying this item's settings.",
                ),
            },
            Status::Queued => choose(
                zh,
                "已加入执行队列，尚未开始执行。",
                "Queued and waiting to run.",
            ),
            Status::SafeQueued => choose(
                zh,
                "已加入安全模式执行队列。",
                "Queued to run in Safe Mode.",
            ),
            Status::Restart => choose(
                zh,
                "配置已保存，等待重启生效。",
                "Settings are saved and will take effect after restart.",
            ),
            Status::SignIn => choose(
                zh,
                "配置已保存，等待重新登录后更新界面。",
                "Settings are saved; the interface will update after signing in again.",
            ),
            Status::Failed => choose(
                zh,
                "上次执行未完成，部分设置可能已应用。",
                "The last run was incomplete; some settings may have been applied.",
            ),
        }
    }
    pub fn instruction(&self, f: &Feature, zh: bool) -> Option<&str> {
        Some(match self.state {
            Status::Ready | Status::Done if f.toggle() => choose(
                zh,
                "按 Space 切换菜单样式。",
                "Press Space to switch the menu style.",
            ),
            Status::Ready | Status::PendingCheck | Status::Checking => {
                if f.safe {
                    choose(
                        zh,
                        "按 Space 执行；必要时进入安全模式。",
                        "Press Space to run; Safe Mode will be used if needed.",
                    )
                } else if f.restart {
                    choose(
                        zh,
                        "按 Space 执行；更改需要重启生效。",
                        "Press Space to run; changes require a restart.",
                    )
                } else if matches!(f.refresh.as_str(), "Shell" | "Wallpaper") {
                    choose(
                        zh,
                        "按 Space 执行；更改需要重新登录生效。",
                        "Press Space to run; changes require signing in again.",
                    )
                } else {
                    choose(zh, "按 Space 执行此项目。", "Press Space to run this item.")
                }
            }
            Status::Failed => choose(zh, "按 Space 重试。", "Press Space to retry."),
            Status::Unknown => choose(
                zh,
                "按 Space 重新检测；执行全部时暂时跳过此项目。",
                "Press Space to check again; Run all skips this item for now.",
            ),
            Status::SafeQueued => choose(
                zh,
                "按 R 重启，进入安全模式后自动继续。",
                "Press R to restart; execution continues in Safe Mode.",
            ),
            Status::Restart => choose(
                zh,
                "重启电脑以应用更改。",
                "Restart the computer to apply changes.",
            ),
            Status::SignIn => choose(
                zh,
                "注销并重新登录以应用更改。",
                "Sign out and sign in again to apply changes.",
            ),
            Status::Deferred => choose(
                zh,
                "返回正常模式后执行此项目。",
                "Run this item after returning to normal mode.",
            ),
            _ => return None,
        })
    }
    pub fn label(&self, f: &Feature, zh: bool) -> String {
        match self.state {
            Status::PendingCheck => choose(zh, "待检测", "Awaiting check"),
            Status::Checking => choose(zh, "检测中", "Checking"),
            Status::Ready if f.id == "taskbar-pins" => choose(zh, "可清理", "Can clean up"),
            Status::Done if f.id == "taskbar-pins" => choose(zh, "无固定项", "No pins"),
            Status::Ready if f.id == "location" => choose(zh, "可关闭", "Can turn off"),
            Status::Done if f.id == "location" => choose(zh, "已关闭", "Off"),
            Status::Ready if f.toggle() => "Windows 11",
            Status::Done if f.toggle() => "Windows 10",
            Status::Done if f.security() => choose(zh, "已优化", "Optimized"),
            Status::Ready => match f.intent.as_str() {
                "Install" => choose(zh, "可安装", "Can install"),
                "Remove" => choose(zh, "可卸载", "Can uninstall"),
                "Apply" => choose(zh, "可设置", "Can configure"),
                _ => choose(zh, "可禁用", "Can disable"),
            },
            Status::Done => match f.intent.as_str() {
                "Install" => choose(zh, "已安装", "Installed"),
                "Remove" => choose(zh, "未安装", "Not installed"),
                "Apply" => choose(zh, "已设置", "Configured"),
                _ => choose(zh, "已禁用", "Disabled"),
            },
            Status::Running => choose(zh, "执行中", "Running"),
            Status::Queued => choose(zh, "排队中", "Queued"),
            Status::SafeQueued => choose(zh, "待安全模式", "Needs Safe Mode"),
            Status::Restart => choose(zh, "待重启", "Needs restart"),
            Status::SignIn => choose(zh, "待重新登录", "Needs sign-in"),
            Status::Failed => choose(zh, "执行未完成", "Incomplete"),
            Status::Deferred => choose(zh, "待正常模式", "Needs normal mode"),
            Status::Unknown => choose(zh, "状态待确认", "Unconfirmed"),
            Status::Restricted => choose(zh, "已跳过", "Skipped"),
            Status::Absent => choose(zh, "无需处理", "Not applicable"),
        }
        .into()
    }
}

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct ResultRecord {
    pub errors: Vec<String>,
    pub restart: bool,
    pub changed: bool,
    pub boot: u64,
    pub shell: u64,
    #[serde(default)]
    pub logon: u64,
}
