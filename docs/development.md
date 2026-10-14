# 开发文档

本文只记录功能约束和开发流程。具体功能、状态规则和实现以源码与测试为准。

## 功能约束

- 描述区只说明所选项的用途；状态由右栏表达，操作集中在按钮栏。README 只介绍产品。
- 检测只读取设置。每项通过 `Ops` 定义优化，通过 `RestoreOps` 定义 Windows 默认可用配置；`R` 恢复当前项，`F10` 重启。恢复记录和启动流程保存执行方向，返回正常模式后只接续该方向。
- 正常模式不设自动重启或关闭倒计时；只有安全模式执行后倒计时重启。列表检测当前优化配置，恢复方向只用于接续和错误核对，不形成“已恢复”状态。
- 首次检测、重新检测和执行后的复查完成前，单项、类别、全部、恢复及重启均不接受键盘或鼠标操作；保留浏览、切换类别和退出。空闲状态栏留空，检测期间显示剩余项数。
- 专用后台直接禁用；商店下载、网络计费和系统故障诊断所需服务设为手动启动。系统自动更新由策略关闭，保留商店和手动安装能力。跨设备、手机连接、远程桌面和文件共享禁用对应服务及入口。恢复解除策略并恢复默认启动方式。
- SysMain 总服务默认禁用。预读、内存压缩与页面合并分别定义，`DefaultOff` 子项不参与类别或全部执行；单独执行子项启用总服务，再修改指定能力。MMAgent 调用明确传入全部开关，只设置所选能力；同一批次的内存项顺序执行，防止共享配置互相覆盖。W32Time 保留独立自动校时项目；通知与硬件功耗、温度控制服务保留正常运行配置。
- 应用包卸载所有用户注册并清除预配，禁用状态也计入待卸载对象。`PackageSources` 按具体包名指定可靠 winget 商店源，有源的包只保存卸载清单，不备份应用文件，恢复直接重装。无源包在卸载前保存一份当前最新签名主包、依赖及语言资源，恢复原路径并由 Windows 一起注册主包和依赖；缺少材料时报告失败。恢复选择主包，卸载选择包集合；安全中心及 SystemApps 中的组件移除实际文件，保留签名恢复材料；受保护文件使用 TrustedInstaller 权限处理。没有安装路径的待清理记录不计为已安装。恢复确认当前用户可用及原有预配，安全中心按默认不可卸载策略复原，并同步设置和托盘入口；界面缓存按重新登录刷新。
- 桌面项只显示此电脑，回收站可见时将此电脑放在其前；保留其他图标、文件和快捷方式。任务栏快捷方式移动到用户对应的恢复目录，保存移动清单；恢复不覆盖同名现有内容。固定项保存必要的数据文件，恢复前解除清空策略。系统还原通过策略与任务关闭，保留现有还原点；恢复解除限制并还原任务默认配置。
- 重试保持列表顺序和选中项，失败只阻断有依赖的操作。合并重启与重新登录的生效步骤，避免不必要的等待。
- 安全模式返回后，同一项未完成时停止自动接续，禁止再次自动安排安全模式；正常模式接续也不重试执行失败项，用户明确重试才启动新一轮。临时启动项已经移除时，仍能完成剩余接续清理。
- Windows 更新或组件维护等待重启时，先通过正常启动完成维护，再安排安全模式，避免更新在安全模式中回退。正常模式每次重启仍由用户触发。
- 名称与描述准确限定操作目标，读回所有执行目标才显示已完成；实际失败保留重试入口。
- 后台优化保留当前用户程序运行，需要重启或重新登录生效的更改按相应边界处理。
- 不得禁用全局通知弹窗、声音、通知中心、应用角标或推送服务；安全通知项目仅处理安全中心提醒，不接管其他应用的通知设置。
- 界面截图直接捕获程序真实渲染，不另行重绘字体或组件。

## 操作类型

`Ops` 是执行目标，`RestoreOps` 是 R 的目标。设置采用明确的可用默认值或删除覆盖策略；电源、系统代码页和输入法从当前 Windows 获取默认配置。个人文件、固定项和无安装源的组件保存恢复所需材料。恢复读回目标状态，缺失组件保持“系统未提供”，失败保留错误与重试入口。

| 类型 | 执行逻辑 | R 恢复逻辑 |
| --- | --- | --- |
| `Registry`、`RegistryDelete` | 写入指定配置或删除指定值；支持字符串、整数、二进制和多字符串 | 写默认值或删除策略覆盖；不导入优化前的整份注册表 |
| `RegistryKey` | 创建或删除定义中的键，用于经典菜单切换 | 删除菜单覆盖键，交由 Windows 提供默认菜单 |
| `Service` | 设为禁用、停止实例；同时写入服务管理器和注册表 | 转为 `ServiceRestore`，写默认启动方式，重启后由服务管理器加载；不会删除服务本体 |
| `ServiceStart`、`ServiceRestore` | 指定启动方式；后者用于受保护服务的恢复 | 普通启动项直接更新；恢复项重启生效，检测要求服务管理器与注册表一致 |
| `UserServices`、`UserServicesStart`、`UserServiceStart` | 枚举用户服务模板和实例，禁用相应启动配置 | 恢复模板及实例的默认启动方式，需要重新登录或重启后重建实例 |
| `Task`、`TaskGroup` | 禁用指定任务或匹配任务组，保留任务定义 | 按 Windows 默认启停状态还原；安全模式跳过，返回正常模式接续 |
| `Asr` | 调整已知及当前存在的攻击面规则 | 删除规则覆盖，回到未配置状态 |
| `Bcd` | 修改正常系统的启动参数，必要时安排安全模式和重启 | 按定义恢复默认参数或删除覆盖；完成后清理临时启动项和接续任务 |
| `SmartAppControl` | 关闭智能应用控制，刷新代码完整性策略；检测实际加载的策略 | 复制现有 Windows 恢复环境，在临时副本中离线恢复评估模式及受保护状态；返回系统后核对实际策略并清理副本和启动项 |
| `Apps`：可靠安装源 | 卸载所有用户注册、清除预配，只记包标识和必要状态 | winget 联网重装，确认当前用户可用，并恢复原有预配 |
| `Apps`：无可靠安装源 | 先保存最新签名包及必要依赖，再卸载、清除预配及关联入口 | 从材料恢复主包和依赖；仍注册的旧禁用包解除禁用；复原必要保护策略；材料缺失时报错 |
| `Winget` | 安装定义中的软件，验证安装状态；处理旧配置留下的下载服务禁用，结束后还原启动配置 | 静默卸载并读回实际安装状态；安装项不备份程序文件；按实际卸载器补全静默参数，覆盖用户、系统及 32 位注册表；安装和卸载均刷新应用清单 |
| `OneDrive`、`CopilotDesktop` | 使用各自卸载器移除桌面程序，OneDrive 同时清理自启 | 使用明确安装源重新安装；Appx 版 Copilot 由 `Apps` 处理 |
| `CompatibilityDatabase` | 保存额外兼容性数据库，再注销数据库 | 从保存的数据库重新注册 |
| `OptionalFeature` | DISM 禁用可选组件，保留恢复所需组件内容 | DISM 启用；系统不支持的组件保持缺失，需要重启时提示 |
| `Hibernate` | 关闭休眠并移除休眠文件 | 重新启用休眠；检测同时核对开关和休眠文件 |
| `ReservedStorage` | DISM 关闭保留存储 | DISM 重新启用 |
| `ProcessBlock` | 写 IFEO 启动拦截并停止指定进程 | 移除本工具的拦截；已有进程由正常启动流程恢复 |
| `ResumeAccess` | 拒绝指定程序的执行权限 | 移除执行拒绝项，保留其他访问控制权限 |
| `Firewall` | 调整各配置文件的防火墙或安全通知开关，配套策略限制手动切换 | 解除策略并恢复定义中的开关值 |
| `FirewallGroup` | 关闭规则组，确认没有剩余启用规则 | 从 Windows 的 SystemDefaults 存储读取每条规则的默认开关，恢复对应规则 |
| `MemoryAgent` | 单独关闭指定内存能力，调用时启用 SysMain；总服务项目最后禁用 SysMain | 重新启用预启动、内存压缩和页面合并；子项不参加批量执行，页面合并采用可用恢复目标 |
| `TimeSync` | W32Time 自动启动、运行并取消域触发限制 | 恢复手动启动与域加入/退出触发器 |
| `Power` | 修改当前方案的交流电或电池设置 | 读取当前方案类型的 Windows 默认值，仅恢复本项；不删除其他电源方案 |
| `DesktopIcons`、`DesktopOrder` | 只显示此电脑；通过 Windows 桌面接口读取位置，必要时与回收站交换位置；不移动其他图标 | 先将回收站放回两者的前方，再删除此电脑的显示覆盖，交由 Windows 默认设置决定可见性 |
| `DesktopFiles` | 桌面项不再清理文件，仅保留已保存内容的恢复入口 | 移回此前保存的桌面内容；遇到同名内容保留材料并报告冲突 |
| `TaskbarPins` | 保存固定数据和快捷方式，再清空固定项 | 恢复数据和快捷方式；新增固定项发生冲突时停止，未处理的材料阻止完成状态 |
| `StartPins` | 保存开始菜单固定数据，再应用空固定列表 | 解除清空策略并恢复数据；尚存备份时不会提前算作完成 |
| `TrayIcons` | 启动本工具的托盘管理进程并登记自启 | 停止进程、删除自启；两者均清除才算恢复 |
| `InputMethods` | 设置微信输入法为唯一配置，按需先安装 | 重建系统语言对应的 Windows 默认输入法，清除默认输入法覆盖和微信排序覆盖 |
| `LanguageBar` | 调整语言栏显示及配套注册表值 | 恢复默认显示方式 |
| `SettingsPage` | 从设置入口中隐藏指定页面 | 重新显示该页面，保留其他页面的可见性设置 |
| `LocaleCodePages` | UTF-8 项目通过注册表启用相关代码页 | 按 Windows 系统语言恢复默认代码页，重启生效 |
| `Mitigations` | 关闭系统及目标程序的漏洞缓解启动策略 | 清除覆盖，并还原新装 Windows 自带的逐程序默认配置；已运行程序需重新启动 |

底层还保留 `AppPreloading` 与 `DeliveryMode` 调用：分别调整应用预启动、传递优化下载模式，目前菜单使用 `MemoryAgent` 和服务/策略组合。

电源默认值使用 [PowerReadACDefaultIndex](https://learn.microsoft.com/en-us/windows/win32/api/powrprof/nf-powrprof-powerreadacdefaultindex) 和对应电池接口；不同方案类型和供电方式可有不同默认值。预配记录、签名材料和任务定义只覆盖仍存在或本工具保存过的内容；旧版已经删除且没有可靠来源的组件无法凭空重建。

页面合并的[微软默认关闭说明](https://learn.microsoft.com/en-us/windows-server/administration/performance-tuning/subsystem/cache-memory-management/improvements-in-windows-server)针对 Windows Server；[客户端实验资料](https://martinschwarzl.at/media/files/thesis_main.pdf)记录过默认启用的 Windows 10 版本。本工具按 R 启用该能力，不将固定关闭值宣称为所有客户端的出厂默认。

## 构建与验证

需要 Rust stable、Visual Studio C++ Build Tools 的 VCTools 与 Windows SDK。`tools/build.py` 自动导入 MSVC / SDK 环境，隐藏运行 Cargo。

```text
python tools/build.py test --locked -- --test-threads=1
python tools/build.py clippy --locked --all-targets -- -D warnings
python tools/build.py build --release --locked
```

按修改范围运行必要检查，以上命令顺序执行。修改版本后先用 `python tools/build.py check --offline` 更新 Cargo.lock；文档修改检查内容和链接即可。

Windows 会锁住运行中的 exe。需要保留现有窗口时，先将旧发布产物改名移至 `artifacts/`，再构建；指向原产物路径的符号链接无需更新。

界面修改可使用隐藏终端验证首屏、导航、背景、滚动条及退出恢复：

```text
python tools/measure_ui.py target/release/zero_security_windows.exe
python tools/check_console.py target/release/zero_security_windows.exe
```

测试默认使用隔离数据或只读检测。修改设备的测试默认忽略，仅在授权对应操作时运行。本地开发机不执行实际系统恢复，独立验证机按批次执行与恢复。

## 目标机验证

恢复默认值以新装 Windows 11 专业版 26300 为参考；逐程序防护配置保存在 `resources/windows-defaults.json`。服务、任务、注册表默认值由原始快照核对；防火墙、电源、语言与代码页使用目标系统默认提供程序。

Smart App Control 从关闭状态恢复使用微软的[离线配置方法](https://learn.microsoft.com/en-us/windows/apps/develop/smart-app-control/test-your-app-with-smart-app-control#configure-smart-app-control-by-using-the-registry)，安全模式仍会拒绝修改受保护状态。恢复环境采用原机的临时副本，不修改原有恢复环境。

`--snapshot 文件` 记录读回目标；`--apply 项目 文件`、`--restore 项目 文件` 和 `--apply-all 文件` 使用界面相同的执行流程。`--apply-items 选择文件 报告文件`、`--restore-items 选择文件 报告文件` 批量处理 JSON 项目 ID 数组。`--check 报告文件 [选择文件]` 核对当前状态与默认配置，`--continue 报告文件` 后台接续正常模式任务，均不自动重启。

`tools/validate_target.py` 按系统设置、应用精简、安装项和桌面设置批量验证，合并重启，再分别核对每项结果。恢复按普通配置、Defender 和离线配置处理；Defender 另核对服务、实时保护和防篡改的运行状态。UU 卸载与重装放到这些重启验证之后，先确认独立控制任务正常且只有一个实例。调试连接使用独立的 tailcat 0.6.0；配置中的 `client_binary` 指定该客户端路径，每次验证复用一个后台转发进程；工具安装项仍由 winget 安装最新版。重启由测试脚本控制，不依赖正常模式的界面计时器。失败只重试对应项目，再核对整个批次。固定 tailcat 身份只运行一个远程会话，连接密钥、快照、结果和检查点均放在忽略目录。示例：

```text
python tools/validate_target.py --connection artifacts/target_connection.json --hostname DESKTOP-BPDS8UM --results artifacts/target-validation --finish-all
```

已预装的安装项先卸载，再验证实际安装与卸载。测试控制程序独立于 winget 安装目录，卸载 tailcat 安装项不会中断验证。

目标机 Defender 曾删除 tailcat 0.7.0；控制程序使用校验过官方 SHA256 的 [0.6.0](https://github.com/tailscale/tailcat/releases/tag/v0.6.0)，保留固定密钥和自启任务。启动时清理同一目录中的旧实例，保持一个服务端；只等待服务端进程退出，再重试，并在验证期间阻止自动休眠。PowerShell 的 `Start-Process -Wait` 会等待整个子进程树，测试与控制脚本均使用主进程的 `WaitForExit()`，避免常驻托盘进程阻塞。

UU 远程自带命令行入口 `bin/uuyc-cli.exe`。新建终端提示协议不兼容时，先在 UU 图形界面创建会话，再用 `term 设备名 --session-id 会话ID` 接续；已附加的会话先断开原客户端。修复连接期间才使用远程桌面，完成后回到命令行验证。

调试程序使用 `zsw-validation-control.exe` 独立进程名，固定 key 与安装项分开。控制任务设为启动、登录和每分钟接续，运行中忽略重复触发；进程退出后由脚本重新启动。核对实际进程、任务运行状态和重启后的连接，不以注册成功代替运行验证。

## 提交与发布

发布 `target/release/zero_security_windows.exe`，GitHub Release 的 `--target` 指向完整提交 SHA。下载完成后核验附件与本地文件的 SHA256，再分发同一附件。

临时脚本与验证记录放在忽略的 `artifacts/`，构建产物位于 `target/`，均不提交 Git。文档和目录整理不单独升级程序版本。
