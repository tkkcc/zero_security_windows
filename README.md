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

无需安装或额外运行库。仅右键菜单样式支持 Windows 10 / 11 切换。
