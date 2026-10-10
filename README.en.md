# Zero Security Windows

[简体中文](README.md)

One-click Windows optimization to reduce security restrictions, background activity and unwanted prompts.

- Disable protections, telemetry and unnecessary services; pause system updates for 7000 days.
- Remove unwanted built-in apps, configure desktop, input and power settings, and install everyday tools.

## Run

[Download the EXE](https://github.com/tkkcc/zero_security_windows/releases/latest/download/zero_security_windows.exe), double-click it and press **A** to run all. Restart when prompted; the app continues automatically.

Or download and run it in Command Prompt (cmd):

```cmd
curl.exe -fL https://github.com/tkkcc/zero_security_windows/releases/latest/download/zero_security_windows.exe -o "%TEMP%\zero_security_windows.exe" && start "" /wait "%TEMP%\zero_security_windows.exe"
```

No installation or separate runtime is needed. Only the context menu style switches between Windows 10 and 11.

Keys are case insensitive: **Space** runs or retries the selected item, **A** runs all, **R** restarts, and **Q** exits. Rows keep their positions during checks and retries. Local settings run before installations, with up to six concurrent jobs. Restart and sign-in settings are applied before the Safe Mode round trip; installers that explicitly require a restart still show that requirement.

The right column shows short status labels. Select an item to see separate sections below the table: Purpose describes the target, Effect explains the scope or limitations, Status gives the current check results, and Action lists the available next step. Pause dates, saved taskbar pins and protection counts appear in Status.

“Not applicable” and “Needs normal mode” describe the current environment. “Skipped” means Windows restricts access to the setting; the reason and log are shown below, with no repeated check action. “Unconfirmed” means a usable result is temporarily unavailable; press **Space** to check again. Check diagnostics are saved in `%ProgramData%\ZeroSecurityWindows\checks.jsonl`. Actual execution failures appear in red as “Incomplete”; press **Space** to retry.

Location access and location request notifications are turned off while their Windows Settings switches remain available.

Windows Update uses a long-term pause while its settings page and manual controls remain available. Delivery Optimization only disables peer sharing, preserving normal downloads.

“App launch preloading” disables application prefetching, prelaunch and operation recording. SysMain starts automatically; memory compression and page combining retain Windows settings, with their current states shown below the table.

General notifications are controlled by Windows and individual applications.

Security items show “Optimized” once their configurable settings meet the target, with no repeat action. Windows-mandated or application-enabled protections may remain; the details explain runtime results and limits. “Optimized” does not mean every process has all protections disabled. Windows mandates DEP for 64-bit applications. When Defender or phishing protection background services are disabled, dependent warnings need no separate action.

Ordinary privacy, desktop and connection controls remain editable in their GUI. Checks only read user changes. Defender services and drivers use deep disabling. See the [GUI recovery audit](docs/GUI恢复检查.md) for the full scope.

Items that have not run show available actions such as “Can install”, “Can uninstall”, “Can configure” and “Can disable”. Once added to the execution queue, they show “Queued”.

“Pinned taskbar apps” checks pins saved by Windows, showing “Can clean up” or “No pins” and listing their count and names below the table. Running windows also appear on the taskbar without necessarily being pinned.
