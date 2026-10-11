# 开发文档

本文只记录功能约束和开发流程。具体功能、状态规则和实现以源码与测试为准。

## 功能约束

- 产品应自解释，操作目标、影响、结果与下一步直接在界面呈现，不依赖说明书。README 只介绍产品。
- 文档文件名使用英文，内容默认使用中文。
- 检测只读取设置，更改由用户主动执行。普通设置保留系统界面的手动控制；深度禁用明确说明影响范围。不提供独立恢复条目。
- 有 Windows 设置开关的项目只更改偏好或对应功能开关，保留依赖服务与可编辑条件；没有系统开关的项目可以直接优化。开始菜单关闭常用应用显示时保留应用启动跟踪；诊断、附近共享与网络发现不得通过禁用依赖服务阻断用户重新开启。
- 普通功能服务优先设为手动启动，保留现有触发器，不因减少常驻而禁用搜索、打印、联系人数据、故障排查、流量统计、链接跟踪、文件共享、IPsec 或离线地图能力。手动启动允许系统和应用请求启动，不保证服务始终停止。SysMain 与 W32Time 因内存管理和自动校时保持自动启动；Spooler 因打印接口无法按需启动它而保持自动，打印工作流设为手动。通知与硬件功耗、温度控制服务保留正常运行配置。
- 重试保持列表顺序和选中项，失败只阻断有依赖的操作。合并重启与重新登录的生效步骤，避免不必要的等待。
- 完成以可配置目标为准，系统强制限制如实说明，不制造重复待办；实际执行失败保留重试入口。
- 后台优化保留当前用户程序运行，需要重启或重新登录生效的更改按相应边界处理。
- 不得禁用全局通知弹窗、声音、通知中心、应用角标或推送服务；安全通知项目仅处理安全中心提醒，不接管其他应用的通知设置。
- 界面截图直接捕获程序真实渲染，不另行重绘字体或组件。

## 构建与验证

需要 Rust stable、Visual Studio C++ Build Tools 的 VCTools 与 Windows SDK。`tools/build.py` 自动导入 MSVC / SDK 环境，隐藏运行 Cargo。

```text
python tools/build.py test --locked -- --test-threads=1
python tools/build.py clippy --locked --all-targets -- -D warnings
python tools/build.py build --release --locked
```

按修改范围运行必要检查，以上命令顺序执行。修改版本后先用 `python tools/build.py check --offline` 更新 Cargo.lock；文档修改检查内容和链接即可。

界面修改可使用隐藏终端验证首屏、导航、背景、滚动条及退出恢复：

```text
python tools/measure_ui.py target/release/zero_security_windows.exe
python tools/check_console.py target/release/zero_security_windows.exe
```

测试默认使用隔离数据或只读检测。修改设备的测试默认忽略，仅在授权对应操作时运行。开发验证不弹出前台窗口，不自行重启设备。

## 提交与发布

保持实现简单，优先使用成熟库的默认能力；测试验证行为，避免文案关键词匹配和重复检查。在 master 上提交并推送，不创建分支或 CI。

发布 `target/release/zero_security_windows.exe`，GitHub Release 的 `--target` 指向完整提交 SHA。下载完成后核验附件与本地文件的 SHA256，再分发同一附件。

临时脚本与验证记录放在忽略的 `artifacts/`，构建产物位于 `target/`，均不提交 Git。文档和目录整理不单独升级程序版本。
