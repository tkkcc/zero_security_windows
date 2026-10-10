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

Items skipped because of the current environment show “Not running”, “Nothing to change”, or “Run in normal mode”, with an explanation below the table. For restricted or unconfirmed states, **Space** checks again without executing changes. Execution failures remain red and retryable. Original check diagnostics are saved in `%ProgramData%\ZeroSecurityWindows\checks.jsonl`.

Location access and location request notifications are turned off while their Windows Settings switches remain available. Applying this item removes the forced-off policy left by older versions and restores the Geolocation Service to manual trigger start.

Windows Update uses a long-term pause while its settings page and manual controls remain available. The update item restores services, processes and tasks blocked by older versions. Delivery Optimization only disables peer sharing, preserving normal downloads.

“App launch preloading” disables application prefetching, prelaunch and operation recording. SysMain starts automatically; memory compression and page combining retain Windows settings, with their current states shown below the table.

General notifications are controlled by Windows and individual apps. The tool no longer changes banners, sounds, notification center, taskbar badges or lock-screen notifications.

LSA checks the actual process protection type; an audit flag does not count as protection. DEP checks both the live boot policy and process state. Windows mandates DEP for 64-bit programs even when the configurable boot policy is off, so retained protection is reported without repeatedly requesting a restart.

Ordinary privacy, desktop and connection controls remain editable in their GUI. Checks only read user changes. Relevant items retain necessary service configurations and manual controls; there is no separate legacy recovery item. Deep disabling of Defender services and drivers remains intentional. See the [GUI recovery audit](docs/GUI恢复检查.md) for the full scope.

“Pinned taskbar apps” checks pins saved by Windows, showing “Can clear” or “No pins” and listing their count and names below the table. Older hiding settings may temporarily keep saved pins invisible. Running windows also appear on the taskbar without necessarily being pinned.
