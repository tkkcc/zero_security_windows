# GUI 恢复能力检查（0.8.5）

检查 0.8.4 的全部 147 项。普通开关使用用户首选项或 Windows 原生配置接口；清理本工具原有的强制策略，不隐藏入口，不停止开关依赖的服务。用户手动修改后只读取状态，只有再次主动执行项目 / 执行全部才应用默认值。

用户已确认保留 Defender 服务、驱动、安全中心卸载等深度禁用。安全防护的系统级项目仍属于例外。独立服务项允许管理员用 `services.msc` 恢复；普通设置不要求用户进入注册表编辑器。

初次修正 26 项，13 项额外禁用合并为“旧版界面锁定清理”。0.8.10 按用户要求关闭系统还原、休眠能力及更新驱动通道；当前清单 139 项。系统还原通过原生 WMI 关闭各盘保护，保留 GUI 重新开启；休眠用 powercfg 关闭并移除休眠文件。驱动更新排除使用专门策略，可通过本地组策略编辑器恢复。备份仍交由 Windows 管理。

传递优化使用设置页同一 WMI 类 `MSFT_DeliveryOptimizationConfig.SetDownloadMode`，删除强制 `DODownloadMode` 策略后写入普通选择。实机已验证 0 → 1 → 0，模式提供者从策略来源 7 变为普通配置来源 9。[下载模式说明](https://learn.microsoft.com/en-us/windows/deployment/do/waas-delivery-optimization-reference)。

Edge 预启动和后台运行使用可覆盖的推荐默认值，不再写强制值；用户在 Edge 设置中修改。[StartupBoostEnabled](https://learn.microsoft.com/en-us/deployedge/microsoft-edge-policies/startupboostenabled)、[BackgroundModeEnabled](https://learn.microsoft.com/en-us/deployedge/microsoft-edge-browser-policies/backgroundmodeenabled)。开始菜单 `applyOnce=true` 仅首次应用，后续固定列表由用户修改。[开始布局说明](https://learn.microsoft.com/en-us/windows/configuration/start/layout)。

每用户服务模板通过 SCM 恢复；当前实例不支持修改启动类型（Windows 返回错误 87）。持久配置即使读起来已恢复，当前实例启动仍可能返回服务已禁用（1058）。修复实例时记录登录会话，重试保留“重新登录生效”，登录重建后自然解除；重启资源管理器不会提前解除提示。默认类型遵循 [Microsoft 每用户服务说明](https://learn.microsoft.com/en-us/windows/application-management/per-user-services-in-windows)。新系统实例已正常时不要求重新登录。任务只恢复操作日志中本工具实际成功禁用的记录，保留原先禁用的任务。

本机修复通过实际 Engine 执行路径，不清空用户后来添加的开始 / 任务栏固定图标；未执行安装、卸载、深度安全项目或重启。36 项常规测试通过，另通过本机修复与传递优化切换验证。开发验证使用底层接口与配置读取，没有打开设置页做视觉验收。

## 全量检查结果

| 原项目 | 处理 | 恢复入口 / 例外 |
| --- | --- | --- |
| 安全通知与托盘图标 (`security-notifications`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| 网络安全提示 (`network-prompts`) | 已修正 | 防火墙通知与网络共享设置 |
| 防火墙 (`firewall`) | 已修正 | 控制面板 / Windows 安全中心的防火墙开关 |
| 用户账户控制（UAC） (`uac`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| 应用与下载信誉检查 (`smartscreen`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| Edge 信誉检查 (`edge-smartscreen`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| 下载文件安全提示 (`open-file-warning`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| 智能应用控制 (`smart-app-control`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| AppLocker 应用限制 (`applocker`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| 默认应用与任务栏保护 (`userchoice-protection`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| 服务与驱动 (`defender`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| 篡改防护 (`tamper`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| 实时扫描 (`realtime`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| 云保护 (`cloud-defense`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| 不需要的应用拦截 (`pua`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| 恶意网址拦截 (`network-protection`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| 受控文件夹访问 (`controlled-folders`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| 攻击面减少规则 (`asr`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| 定时扫描 (`defender-tasks`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| 微信输入法 (`install-tencent.wetype`) | 保留软件安装 | 软件卸载 / 安装界面管理 |
| UniGetUI (`install-xpfftq032ptphf`) | 保留软件安装 | 软件卸载 / 安装界面管理 |
| chezmoi (`install-twpayne.chezmoi`) | 保留软件安装 | 软件卸载 / 安装界面管理 |
| Git (`install-git.git`) | 保留软件安装 | 软件卸载 / 安装界面管理 |
| uv (`install-astral-sh.uv`) | 保留软件安装 | 软件卸载 / 安装界面管理 |
| AutoHotkey (`install-autohotkey.autohotkey`) | 保留软件安装 | 软件卸载 / 安装界面管理 |
| rustup (`install-rustlang.rustup`) | 保留软件安装 | 软件卸载 / 安装界面管理 |
| VS Code (`install-microsoft.visualstudiocode`) | 保留软件安装 | 软件卸载 / 安装界面管理 |
| GitHub CLI (`install-github.cli`) | 保留软件安装 | 软件卸载 / 安装界面管理 |
| ChatGPT (`install-9plm9xgg6vks`) | 保留软件安装 | 软件卸载 / 安装界面管理 |
| Node.js (`install-openjs.nodejs`) | 保留软件安装 | 软件卸载 / 安装界面管理 |
| Python 3.14 (`install-python.python.3.14`) | 保留软件安装 | 软件卸载 / 安装界面管理 |
| fd (`install-sharkdp.fd`) | 保留软件安装 | 软件卸载 / 安装界面管理 |
| Yazi (`install-sxyazi.yazi`) | 保留软件安装 | 软件卸载 / 安装界面管理 |
| PowerShell (`install-microsoft.powershell`) | 保留软件安装 | 软件卸载 / 安装界面管理 |
| fzf (`install-junegunn.fzf`) | 保留软件安装 | 软件卸载 / 安装界面管理 |
| zoxide (`install-ajeetdsouza.zoxide`) | 保留软件安装 | 软件卸载 / 安装界面管理 |
| Neovim (`install-neovim.neovim`) | 保留软件安装 | 软件卸载 / 安装界面管理 |
| Android SDK CLI (`install-google.androidcli`) | 保留软件安装 | 软件卸载 / 安装界面管理 |
| UU 远程 (`install-netease.uuremote`) | 保留软件安装 | 软件卸载 / 安装界面管理 |
| Windows Terminal (`install-microsoft.windowsterminal`) | 保留软件安装 | 软件卸载 / 安装界面管理 |
| 资讯 (`remove-news`) | 保留软件卸载 | 需要时从商店或安装程序重新安装 |
| 天气 (`remove-weather`) | 保留软件卸载 | 需要时从商店或安装程序重新安装 |
| Xbox 应用与游戏栏 (`remove-xbox`) | 保留软件卸载 | 需要时从商店或安装程序重新安装 |
| 安全中心应用 (`remove-security-ui`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| 小组件运行库 (`remove-widgets`) | 保留软件卸载 | 需要时从商店或安装程序重新安装 |
| 纸牌与微软游戏 (`remove-games`) | 保留软件卸载 | 需要时从商店或安装程序重新安装 |
| 额外兼容性组件 (`remove-compatibility`) | 保留软件卸载 | 需要时从商店或安装程序重新安装 |
| Clipchamp (`remove-clipchamp`) | 保留软件卸载 | 需要时从商店或安装程序重新安装 |
| 便签 (`remove-notes`) | 保留软件卸载 | 需要时从商店或安装程序重新安装 |
| 待办 (`remove-todo`) | 保留软件卸载 | 需要时从商店或安装程序重新安装 |
| 反馈中心 (`remove-feedback`) | 保留软件卸载 | 需要时从商店或安装程序重新安装 |
| 获取帮助 (`remove-help`) | 保留软件卸载 | 需要时从商店或安装程序重新安装 |
| 微软电脑管家 (`remove-pcmanager`) | 保留软件卸载 | 需要时从商店或安装程序重新安装 |
| 快速助手 (`remove-quick-assist`) | 保留软件卸载 | 需要时从商店或安装程序重新安装 |
| Power Automate (`remove-powerautomate`) | 保留软件卸载 | 需要时从商店或安装程序重新安装 |
| 手机连接与跨设备应用 (`remove-phone`) | 保留软件卸载 | 需要时从商店或安装程序重新安装 |
| 家庭安全 (`remove-family`) | 保留软件卸载 | 需要时从商店或安装程序重新安装 |
| Outlook 与邮件 (`remove-outlook`) | 保留软件卸载 | 需要时从商店或安装程序重新安装 |
| Dev Home (`remove-devhome`) | 保留软件卸载 | 需要时从商店或安装程序重新安装 |
| 微软 AI 应用 (`remove-ai`) | 保留软件卸载 | 需要时从商店或安装程序重新安装 |
| OneDrive (`remove-onedrive`) | 保留软件卸载 | 需要时从商店或安装程序重新安装 |
| 鼠标速度与滚轮 (`mouse`) | 保留普通配置 | 现有用户首选项或原生接口，未加入锁定策略；可在对应设置界面修改 |
| 键盘重复与辅助按键 (`keyboard`) | 保留普通配置 | 现有用户首选项或原生接口，未加入锁定策略；可在对应设置界面修改 |
| 黑色指针，大小 3 (`cursor`) | 保留普通配置 | 现有用户首选项或原生接口，未加入锁定策略；可在对应设置界面修改 |
| 浅色主题，无透明效果 (`appearance`) | 保留普通配置 | 现有用户首选项或原生接口，未加入锁定策略；可在对应设置界面修改 |
| 视觉效果 (`visual-effects`) | 保留普通配置 | 现有用户首选项或原生接口，未加入锁定策略；可在对应设置界面修改 |
| 全局 UTF-8 (`utf8`) | 保留普通配置 | 现有用户首选项或原生接口，未加入锁定策略；可在对应设置界面修改 |
| 仅使用微信输入法 (`input-method`) | 保留普通配置 | 现有用户首选项或原生接口，未加入锁定策略；可在对应设置界面修改 |
| 电源按钮与自动休眠 (`power`) | 保留普通配置 | 现有用户首选项或原生接口，未加入锁定策略；可在对应设置界面修改 |
| 开始菜单布局 (`start-menu`) | 已修正 | 个性化 → 开始；固定列表仅首次应用 |
| 桌面图标与文件 (`desktop-icons`) | 保留普通配置 | 现有用户首选项或原生接口，未加入锁定策略；可在对应设置界面修改 |
| 任务栏对齐与合并 (`taskbar-layout`) | 保留普通配置 | 现有用户首选项或原生接口，未加入锁定策略；可在对应设置界面修改 |
| 任务栏搜索与任务视图 (`taskbar-buttons`) | 保留普通配置 | 现有用户首选项或原生接口，未加入锁定策略；可在对应设置界面修改 |
| 任务栏固定图标 (`taskbar-pins`) | 已修正 | 应用右键 → 固定到任务栏 |
| 展开托盘图标 (`tray-icons`) | 保留普通配置 | 现有用户首选项或原生接口，未加入锁定策略；可在对应设置界面修改 |
| 蓝牙托盘图标 (`bluetooth-icon`) | 保留普通配置 | 现有用户首选项或原生接口，未加入锁定策略；可在对应设置界面修改 |
| 语言栏与输入指示器 (`language-bar`) | 保留普通配置 | 现有用户首选项或原生接口，未加入锁定策略；可在对应设置界面修改 |
| 文件扩展名与隐藏文件 (`file-visibility`) | 保留普通配置 | 现有用户首选项或原生接口，未加入锁定策略；可在对应设置界面修改 |
| 此电脑与驱动器 (`this-pc`) | 已修正 | 文件资源管理器选项 |
| 快捷方式名称 (`shortcut-name`) | 明确例外 | 自动名称规则没有普通开关；用户仍可在资源管理器重命名 |
| 右键菜单样式 (`classic-menu`) | 明确例外 | Windows 没有普通开关；本工具提供 Windows 10 / 11 双向切换 |
| 桌面图片背景 (`desktop-picture`) | 已修正 | 个性化 → 背景 |
| 锁屏聚焦 (`lockscreen-spotlight`) | 已修正 | 个性化 → 锁屏 |
| 虚拟化安全（VBS） (`vbs`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| 内存完整性（HVCI） (`hvci`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| 凭据保护 (`credential-guard`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| LSA 进程保护 (`lsa`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| 内核堆栈保护 (`kernel-cet`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| 系统启动防护 (`secure-launch`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| 易受攻击驱动拦截 (`driver-blocklist`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| 增强钓鱼防护 (`phishing-protection`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| 安全中心服务 (`security-center`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| CPU 漏洞防护 (`cpu-mitigations`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| 程序漏洞防护 (`process-mitigations`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| DEP 数据执行保护 (`dep`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| 异常处理与服务进程保护 (`sehop-kernel`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| 驱动签名检查 (`driver-signing`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| 系统更新长期暂停 (`windows-update`) | 明确例外 | 保留长期暂停，可在 Windows 更新中恢复更新 |
| 自动驱动更新 (`driver-updates`) | 按用户要求排除更新驱动 | 设备安装搜索在系统属性恢复；更新驱动排除在本地组策略 → Windows 更新 → 管理从 Windows 更新提供的更新 → Windows 更新不包括驱动程序恢复 |
| 系统还原 (`system-restore`) | 关闭各盘保护 | 系统属性 → 系统保护 → 配置，可重新开启；不禁用 VSS 服务 |
| 自动设备加密 (`automatic-encryption`) | 明确例外 | 只阻止首次自动设备加密；BitLocker / 设备加密可手动开启 |
| 诊断数据收集 (`telemetry`) | 已修正 | 隐私和安全性 → 诊断和反馈 |
| 活动历史 (`activity-history`) | 移除额外禁用 | 旧策略 / 服务 / 任务由“旧版界面锁定清理”恢复 |
| 广告与个性化推荐 (`advertising`) | 已修正 | 隐私和安全性 → 常规 / 诊断和反馈 |
| 系统推广与建议 (`windows-suggestions`) | 保留普通配置 | 现有用户首选项或原生接口，未加入锁定策略；可在对应设置界面修改 |
| 应用启动跟踪 (`app-launch-tracking`) | 保留普通配置 | 现有用户首选项或原生接口，未加入锁定策略；可在对应设置界面修改 |
| 体验反馈邀请 (`feedback-prompts`) | 已修正 | 隐私和安全性 → 诊断和反馈 |
| 位置服务 (`location`) | 保留普通配置 | 现有用户首选项或原生接口，未加入锁定策略；可在对应设置界面修改 |
| Recall 屏幕快照 (`recall`) | 移除额外禁用 | 旧策略 / 服务 / 任务由“旧版界面锁定清理”恢复 |
| 性能诊断追踪 (`svc-whesvc`) | 保留服务开关 | 管理员可通过 services.msc 恢复；本机已验证修改配置权限 |
| 使用与质量数据 (`svc-wuqisvc`) | 保留服务开关 | 管理员可通过 services.msc 恢复；本机已验证修改配置权限 |
| 智能卡移除策略 (`svc-scpolicysvc`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| 家长控制监控 (`svc-wpcmonsvc`) | 移除额外禁用 | 旧策略 / 服务 / 任务由“旧版界面锁定清理”恢复 |
| 搜索热点与联网结果 (`search-highlights`) | 已修正 | 隐私和安全性 → 搜索权限 |
| 小组件入口与后台 (`widgets`) | 已修正 | 个性化 → 任务栏 |
| 文件搜索索引 (`svc-wsearch`) | 保留服务开关 | 管理员可通过 services.msc 恢复；本机已验证修改配置权限 |
| 应用启动预读 (`svc-sysmain`) | 已修正 | services.msc → SysMain |
| 程序兼容性助手 (`svc-pcasvc`) | 保留服务开关 | 管理员可通过 services.msc 恢复；本机已验证修改配置权限 |
| 诊断策略 (`svc-dps`) | 保留服务开关 | 管理员可通过 services.msc 恢复；本机已验证修改配置权限 |
| 诊断服务主机 (`svc-wdiservicehost`) | 保留服务开关 | 管理员可通过 services.msc 恢复；本机已验证修改配置权限 |
| 诊断系统主机 (`svc-wdisystemhost`) | 保留服务开关 | 管理员可通过 services.msc 恢复；本机已验证修改配置权限 |
| 诊断执行服务 (`svc-diagsvc`) | 保留服务开关 | 管理员可通过 services.msc 恢复；本机已验证修改配置权限 |
| 清单采集与兼容性评估 (`inventory-telemetry`) | 已修正 | services.msc → InventorySvc |
| 应用预读 (`prefetch`) | 移除额外禁用 | 恢复默认预读参数，后续由 SysMain 服务开关控制 |
| 应用预启动 (`app-prelaunch`) | 移除额外禁用 | 旧策略 / 服务 / 任务由“旧版界面锁定清理”恢复 |
| 系统备份与文件历史记录 (`windows-backup`) | 已修正 | 账户 → Windows 备份；控制面板 → 文件历史记录 |
| 传递优化 (`delivery-optimization`) | 已修正 | Windows 更新 → 高级选项 → 传递优化 |
| 远程桌面 (`remote-desktop`) | 已修正 | 系统 → 远程桌面 |
| 远程协助 (`remote-assistance`) | 已修正 | 系统属性 → 远程 |
| 客户体验与设备数据收集 (`ceip`) | 移除额外禁用 | 旧策略 / 服务 / 任务由“旧版界面锁定清理”恢复 |
| 功能使用与质量数据收集 (`usage-telemetry`) | 移除额外禁用 | 旧策略 / 服务 / 任务由“旧版界面锁定清理”恢复 |
| 应用隐私权限与同步 (`privacy-access`) | 已修正 | 隐私和安全性 → 语音 / 输入个性化 / 应用诊断 |
| 钓鱼防护后台 (`phishing-services`) | 保留深度禁用 | 按用户确认保留；服务、驱动、系统策略或安全中心卸载不保证普通 GUI 恢复 |
| 跨设备连接后台 (`connected-devices`) | 已修正 | 系统 → 附近共享 / 跨设备共享 |
| OneDrive 系统集成 (`onedrive-integration`) | 移除额外禁用 | 旧策略 / 服务 / 任务由“旧版界面锁定清理”恢复 |
| 跨设备任务接续 (`phone-resume`) | 已修正 | 应用 → 接续 |
| 剪贴板历史与同步 (`clipboard-history`) | 已修正 | 系统 → 剪贴板 |
| Xbox 与游戏后台服务 (`xbox-services`) | 移除额外禁用 | 旧策略 / 服务 / 任务由“旧版界面锁定清理”恢复 |
| 游戏录制后台 (`game-recording`) | 已修正 | 游戏 → 捕获 |
| 家庭安全后台任务 (`family-tasks`) | 移除额外禁用 | 旧策略 / 服务 / 任务由“旧版界面锁定清理”恢复 |
| 系统休眠 (`hibernation`) | 按用户要求关闭能力 | 管理员运行 powercfg /hibernate on 恢复；控制面板的休眠选项需先恢复能力 |
| Recall 可选组件 (`recall-component`) | 保留普通配置 | 现有用户首选项或原生接口，未加入锁定策略；可在对应设置界面修改 |
| 快速启动 (`fast-startup`) | 保留普通配置 | 现有用户首选项或原生接口，未加入锁定策略；可在对应设置界面修改 |
| 系统保留存储 (`reserved-storage`) | 明确例外 | Windows 没有普通开关；仍使用 DISM 管理，属于保留的系统级项目 |
| Edge 预启动与后台运行 (`edge-background`) | 已修正 | Edge → 设置 → 系统和性能，允许覆盖默认值 |
| Edge 与 WebView2 更新 (`edge-updates`) | 移除额外禁用 | 旧策略 / 服务 / 任务由“旧版界面锁定清理”恢复 |
| AI 与 Copilot (`ai-features`) | 移除额外禁用 | 旧策略 / 服务 / 任务由“旧版界面锁定清理”恢复 |
