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
    pub restore_ops: Vec<Operation>,
    pub restore_safe: bool,
    pub default_off: bool,
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
            restore_ops: vec![],
            restore_safe: false,
            default_off: false,
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
        choose(zh, &self.group_zh, &self.group_en)
    }
    pub fn security(&self) -> bool {
        self.page == "System"
            && matches!(
                self.group_zh.as_str(),
                "核心隔离" | "Defender" | "安全" | "漏洞防护"
            )
    }
    pub fn toggle(&self) -> bool {
        self.id == "classic-menu"
    }
    pub fn restore_target(&self) -> Self {
        Self {
            ops: self.restore_ops.clone(),
            safe: self.restore_safe,
            probe: String::new(),
            intent: "Restore".into(),
            ..self.clone()
        }
    }
    pub fn can_run_safe(&self) -> bool {
        self.ops
            .iter()
            .any(|op| !matches!(op.kind.as_str(), "Task" | "TaskGroup"))
            && self.ops.iter().all(|op| {
                matches!(
                    op.kind.as_str(),
                    "Registry"
                        | "Mitigations"
                        | "ServiceStart"
                        | "UserServicesStart"
                        | "LocaleCodePages"
                        | "RegistryDelete"
                        | "RegistryKey"
                        | "Service"
                        | "UserServices"
                        | "Asr"
                        | "Bcd"
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
    pub uninstall_args: String,
    pub group: String,
    pub scheme: String,
    pub service: String,
    pub names: Vec<String>,
    pub patterns: Vec<String>,
    pub package_sources: std::collections::BTreeMap<String, String>,
    pub task_states: std::collections::BTreeMap<String, bool>,
    pub non_removable: bool,
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
    Ok(serde_json::from_str(include_str!(
        "../resources/catalog.json"
    ))?)
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
        matches!(self.state, Status::Ready | Status::Failed)
    }
    pub fn recheckable(&self) -> bool {
        self.state == Status::Unknown
    }
    pub fn label(&self, f: &Feature, zh: bool) -> String {
        match self.state {
            Status::PendingCheck => choose(zh, "待检测", "Awaiting check"),
            Status::Checking => choose(zh, "检测中", "Checking"),
            Status::Ready if f.default_off => choose(zh, "可单独关闭", "Run separately"),
            Status::Ready if f.id == "taskbar-pins" => choose(zh, "可清理", "Can clean up"),
            Status::Ready if f.toggle() => "Windows 11",
            Status::Done if f.toggle() => "Windows 10",
            Status::Ready => match f.intent.as_str() {
                "Install" => choose(zh, "可安装", "Can install"),
                "Remove" => choose(zh, "可卸载", "Can uninstall"),
                "Apply" => choose(zh, "可设置", "Can configure"),
                "Enable" => choose(zh, "可开启", "Can enable"),
                "Hide" => choose(zh, "可隐藏", "Can hide"),
                "Show" => choose(zh, "可显示", "Can show"),
                "Clean" => choose(zh, "可清理", "Can clean up"),
                "Pause" => choose(zh, "可暂停", "Can pause"),
                _ => choose(zh, "可关闭", "Can turn off"),
            },
            Status::Done => choose(zh, "已完成", "Done"),
            Status::Running => choose(zh, "执行中", "Running"),
            Status::Queued => choose(zh, "排队中", "Queued"),
            Status::SafeQueued => choose(zh, "待安全模式", "Needs Safe Mode"),
            Status::Restart => choose(zh, "待重启", "Needs restart"),
            Status::SignIn => choose(zh, "待重新登录", "Needs sign-in"),
            Status::Failed => choose(zh, "执行失败", "Failed"),
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
    #[serde(default)]
    pub restoring: bool,
}
