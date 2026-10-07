# Zero Security Windows

[English](README.en.md)

一键精简 Windows 的安全限制、后台活动与干扰提示。

- 关闭防护、遥测和不需要的服务，暂停系统更新 7000 天。
- 清理内置应用，调整桌面、输入与电源设置，安装常用工具。

## 使用

[下载 EXE](https://github.com/tkkcc/zero_security_windows/releases/latest/download/zero_security_windows.exe)，双击打开，按 **A** 执行全部。需要重启时按界面提示操作，重启后自动继续。

也可在命令提示符（cmd）中下载并运行：

```cmd
curl.exe -fL https://github.com/tkkcc/zero_security_windows/releases/latest/download/zero_security_windows.exe -o "%TEMP%\zero_security_windows.exe" && start "" /wait "%TEMP%\zero_security_windows.exe"
```

快捷键不区分大小写：**Space** 执行或重试当前项，**A** 执行全部，**R** 重启，**Q** 退出。列表顺序固定，安装放在本地优化之后；本地设置和安装各最多六路并行。安全模式往返前合并需要重启或重新登录的本地设置，一般只需两次重启；安装器明确要求重启时仍按实际状态提示。

正常跳过的项目显示“未运行”“无需处理”或“返回正常模式执行”，选中后在底部说明原因。状态读取受限或尚未确认时，按 **Space** 只重新检测；执行失败仍显示红色“未完成 · 可重试”。原始检测信息保存在 `%ProgramData%\ZeroSecurityWindows\checks.jsonl`。

位置访问与应用请求位置的通知默认关闭，但保留 Windows 设置里的手动开关；执行此项会解除旧版本留下的强制关闭策略，并恢复定位服务的手动触发启动。

系统更新使用长期暂停，保留更新页面和手动恢复更新的能力；修复旧版对更新服务、进程及任务的拦截。“传递优化”只关闭设备间共享，保留正常下载。

普通隐私、桌面和连接设置保留 GUI 手动控制，用户修改后只检测状态。旧版界面锁定会由对应项目或“旧版界面锁定清理”解除；每用户服务的旧实例需要重新登录后恢复。Defender 服务、驱动等深度禁用按原需求保留，完整范围见 [GUI 恢复检查](docs/GUI恢复检查.md)。

“任务栏固定图标”检测 Windows 保存的固定项，显示“可清理”或“无固定项”，选中后列出数量与名称。旧版隐藏设置可能暂时让保存的固定项不可见；打开的窗口也会在任务栏显示，但不一定已固定。

无需安装或额外运行库。仅右键菜单样式支持 Windows 10 / 11 切换。
