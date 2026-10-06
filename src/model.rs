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
    Checking,
    Ready,
    Done,
    Unknown,
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
        !f.manual && !matches!(self.state, Status::Unknown | Status::Absent)
    }
    pub fn actionable(&self) -> bool {
        matches!(
            self.state,
            Status::Ready | Status::Failed | Status::Checking
        )
    }
    pub fn label(&self, f: &Feature, zh: bool) -> String {
        if self.state == Status::SignIn && f.toggle() {
            return format!("{} · {}", self.detail, choose(zh, "重新登录", "Sign in"));
        }
        match self.state {
            Status::Checking => choose(zh, "检测中", "Checking"),
            Status::Ready if f.toggle() => "Windows 11",
            Status::Done if f.toggle() => "Windows 10",
            Status::Ready => match f.intent.as_str() {
                "Install" => choose(zh, "可安装", "Can install"),
                "Remove" => choose(zh, "可卸载", "Can remove"),
                "Apply" => choose(zh, "可设置", "Can configure"),
                _ => choose(zh, "可禁用", "Can disable"),
            },
            Status::Done if f.id == "windows-update" => &self.detail,
            Status::Done => match f.intent.as_str() {
                "Install" => choose(zh, "已安装", "Installed"),
                "Remove" => choose(zh, "已清理", "Removed"),
                "Apply" => choose(zh, "已设置", "Configured"),
                _ => choose(zh, "已禁用", "Disabled"),
            },
            Status::Running => choose(zh, "执行中", "Running"),
            Status::Queued => choose(zh, "等待执行", "Queued"),
            Status::SafeQueued => choose(zh, "安全模式执行", "Apply in Safe Mode"),
            Status::Restart => choose(zh, "重启生效", "Restart to apply"),
            Status::SignIn => choose(zh, "重新登录生效", "Sign in to apply"),
            Status::Failed => choose(zh, "未完成 · 可重试", "Incomplete · Retry"),
            Status::Deferred => choose(zh, "重启后执行", "Run after restart"),
            _ => "",
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
}
