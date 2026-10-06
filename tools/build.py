# 从已安装的 Visual Studio 导入 MSVC/SDK 环境，然后运行 cargo；不创建窗口。
import os
from pathlib import Path
import subprocess
import sys

vswhere = Path(os.environ['ProgramFiles(x86)']) / 'Microsoft Visual Studio/Installer/vswhere.exe'
installation = subprocess.check_output(
    [str(vswhere), '-latest', '-products', '*', '-requires',
     'Microsoft.VisualStudio.Component.VC.Tools.x86.x64', '-property', 'installationPath'],
    text=True, creationflags=subprocess.CREATE_NO_WINDOW,
).strip()
if not installation:
    raise SystemExit('请先完成 Visual Studio C++ Build Tools 与 Windows SDK 安装。')
devcmd = Path(installation) / 'Common7/Tools/VsDevCmd.bat'
output = subprocess.check_output(
    f'cmd.exe /d /s /c "call "{devcmd}" -no_logo -arch=x64 -host_arch=x64 >nul && set"',
    text=True, creationflags=subprocess.CREATE_NO_WINDOW,
)
environment = os.environ.copy()
environment.update(line.split('=', 1) for line in output.splitlines() if '=' in line and not line.startswith('='))
process = subprocess.Popen(['cargo', *sys.argv[1:]], env=environment,
                           stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                           creationflags=subprocess.CREATE_NO_WINDOW)
for line in process.stdout:
    print(line, end='', flush=True)
raise SystemExit(process.wait())
