# 开发文档

本文只记录功能约束和开发流程。具体功能、状态规则和实现以源码与测试为准。

## 功能约束

- 产品应自解释，操作目标、影响、结果与下一步直接在界面呈现，不依赖说明书。README 只介绍产品。
- 检测只读取设置。每项通过 `Ops` 定义优化，通过 `RestoreOps` 定义 Windows 默认可用配置；`R` 恢复当前项，`F10` 重启。恢复记录和启动流程保存执行方向，返回正常模式后只接续该方向。
- 禁用项目直接禁用服务、任务与配套程序，使用策略锁定功能；系统入口可能隐藏或无法切换。恢复解除策略并恢复服务的默认启动方式，需要重启时在界面说明。
- SysMain 总服务默认禁用。预读、内存压缩与页面合并分别定义，`DefaultOff` 子项不参与类别或全部执行；单独执行子项启用总服务，再修改指定能力。W32Time 保留独立自动校时项目；通知与硬件功耗、温度控制服务保留正常运行配置。
- 应用包卸载所有用户注册并清除预配，禁用状态也计入待卸载对象。`PackageSources` 按具体包名指定可靠 winget 商店源，有源的包只保存卸载清单，不备份应用文件，恢复直接重装。无源包在卸载前保存一份当前最新签名布局及必要依赖，恢复原路径并重新注册；缺少材料时报告失败。恢复曾有的预配及系统不可卸载策略，安全中心和小组件同步处理关联入口；界面缓存按重新登录刷新。
- 桌面文件和任务栏快捷方式移动到用户对应的恢复目录，保存移动清单；恢复不覆盖同名现有内容。固定项保存必要的数据文件，恢复前解除清空策略。系统还原通过策略与任务关闭，保留现有还原点；恢复重新启用未来监测。
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

测试默认使用隔离数据或只读检测。修改设备的测试默认忽略，仅在授权对应操作时运行。已优化设备不执行实际系统恢复，使用定义、隔离数据和只读检测验证。

## 提交与发布

发布 `target/release/zero_security_windows.exe`，GitHub Release 的 `--target` 指向完整提交 SHA。下载完成后核验附件与本地文件的 SHA256，再分发同一附件。

临时脚本与验证记录放在忽略的 `artifacts/`，构建产物位于 `target/`，均不提交 Git。文档和目录整理不单独升级程序版本。
