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

右侧显示简短状态，选中后在底部查看分区详情：功能是执行目标，影响是适用范围或限制，状态是当前检测结果，操作是可用的下一步。暂停日期、固定应用列表和防护计数显示在状态区。

“未运行”“无需处理”“待正常模式”表示当前环境无需执行或需要切换模式。“读取受限”“状态待确认”表示 Windows 尚未提供可用结果，按 **Space** 重新检测。检测信息保存在 `%ProgramData%\ZeroSecurityWindows\checks.jsonl`；实际执行失败显示红色“执行未完成”，可按 **Space** 重试。

位置访问与应用请求位置的通知默认关闭，保留 Windows 设置里的手动开关。

系统更新使用长期暂停，保留更新页面和手动恢复更新的能力。“传递优化”只关闭设备间共享，保留正常下载。

“应用启动预读”只关闭应用预读、预启动和预读记录；SysMain 保持自动启动，内存压缩和内存页合并保留系统设置，底部显示其实际状态。

通用通知交由 Windows 和各应用自行控制。

安全机制同时检查可配置的设置和实际运行状态。Windows 强制或程序自行启用的防护可能保留，显示“部分防护保留”，并在底部列出具体结果。64 位程序的 DEP 由 Windows 强制启用。

普通隐私、桌面和连接设置保留 GUI 手动控制，用户修改后只检测状态。Defender 服务、驱动等采用深度禁用，完整范围见 [GUI 恢复检查](docs/GUI恢复检查.md)。

“任务栏固定图标”检测 Windows 保存的固定项，显示“待清理”或“无固定项”，选中后列出数量与名称。打开的窗口也会在任务栏显示，但不一定已固定。

无需安装或额外运行库。仅右键菜单样式支持 Windows 10 / 11 切换。
