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

无需安装或额外运行库。仅右键菜单样式支持 Windows 10 / 11 切换。
