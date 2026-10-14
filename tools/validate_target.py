"""在独立目标机批量验证；重启由测试脚本控制，每项保存独立结果。"""
import argparse
import atexit
import base64
from datetime import datetime
import json
import os
from pathlib import Path
import re
import subprocess
import time


class Target:
    def __init__(self, configuration):
        self.folder = configuration.resolve().parent
        self.config = json.loads(configuration.read_text(encoding='utf-8'))
        self.connect()
        atexit.register(self.close)

    def connect(self):
        self.tunnel_log = (self.folder / 'tunnel.stderr').open('w', encoding='utf-8')
        self.tunnel = subprocess.Popen(
            [self.config['client_binary'], '--key=' + self.config['client_key'], 'forward',
             '--bind=127.0.0.1', self.config['address'], '0:22'],
            stdout=subprocess.DEVNULL, stderr=self.tunnel_log, text=True,
            encoding='utf-8', creationflags=subprocess.CREATE_NO_WINDOW,
        )
        endpoint = None
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline and self.tunnel.poll() is None:
            endpoint = re.search(r'127\.0\.0\.1:(\d+)', (self.folder / 'tunnel.stderr').read_text(encoding='utf-8'))
            if endpoint:
                break
            time.sleep(.1)
        if not endpoint:
            self.close()
            raise RuntimeError('The persistent tailcat tunnel did not start')
        self.flags = [
            '-o', 'BatchMode=yes', '-o', 'StrictHostKeyChecking=accept-new',
            '-o', 'ConnectTimeout=20', '-o', 'ServerAliveInterval=15', '-o', 'ServerAliveCountMax=2',
            '-o', 'UserKnownHostsFile=' + (self.folder / 'target_known_hosts').as_posix(),
            '-o', 'Hostname=127.0.0.1', '-o', 'HostKeyAlias=zsw-target', '-o', 'Port=' + endpoint[1],
        ]

    def close(self):
        if self.tunnel.poll() is None:
            self.tunnel.terminate()
            self.tunnel.wait(timeout=5)
        self.tunnel_log.close()

    def run(self, script, timeout=120):
        script = "[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); $OutputEncoding=[Console]::OutputEncoding; $ErrorActionPreference='Stop'; $ProgressPreference='SilentlyContinue'; " + script
        encoded = base64.b64encode(script.encode('utf-16-le')).decode()
        result = subprocess.run(
            ['ssh', '-n', '-T', *self.flags, 'zsw-target', 'powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand ' + encoded],
            capture_output=True, encoding='utf-8', errors='replace', timeout=timeout,
            creationflags=subprocess.CREATE_NO_WINDOW,
        )
        if result.returncode:
            detail=(result.stderr + result.stdout).replace(self.config['address'], '[target]')
            raise RuntimeError(f'SSH command failed ({result.returncode}): {detail}')
        return result.stdout.strip()

    def status(self):
        return json.loads(base64.b64decode(self.run("""
$os=Get-CimInstance Win32_OperatingSystem;
$pending=if(Test-Path 'C:/ProgramData/ZeroSecurityWindows/rust-workflow.json'){[IO.File]::ReadAllText('C:/ProgramData/ZeroSecurityWindows/rust-workflow.json') | ConvertFrom-Json}else{$null};
$processes=@(Get-CimInstance Win32_Process -Filter "Name='zero_security_windows.exe' OR Name='zsw-validation-control.exe'");
$resume=@($processes | Where-Object {$_.Name -eq 'zero_security_windows.exe' -and $_.CommandLine -match '--(safe-resume|resume)'});
$agents=@($processes | Where-Object {$_.ExecutablePath -eq 'C:\\ProgramData\\ZeroSecurityWindowsValidation\\zsw-validation-control.exe'});
$json=@{Host=$env:COMPUTERNAME; Boot=$os.LastBootUpTime.ToUniversalTime().ToString('o'); Phase=[string]$pending.phase; Resume=$resume.Count; Control=(Get-ScheduledTask -TaskName 'ZeroSecurityWindowsValidationControl').State.ToString(); ControlInstances=$agents.Count} | ConvertTo-Json -Compress;
[Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($json))
""", timeout=60)))

    def report(self, arguments, path, timeout=2400):
        quoted = ','.join("'" + value.replace("'", "''") + "'" for value in [*arguments, path])
        return json.loads(base64.b64decode(self.run(f"""
$report='{path}'; if(Test-Path -LiteralPath $report){{Remove-Item -LiteralPath $report}}
$process=Start-Process -FilePath 'C:/ProgramData/ZeroSecurityWindowsValidation/zero_security_windows.exe' -ArgumentList @({quoted}) -WindowStyle Hidden -PassThru;
$process.WaitForExit();
if(-not (Test-Path -LiteralPath $report)){{throw ('No report, exit='+$process.ExitCode)}}
[Convert]::ToBase64String([IO.File]::ReadAllBytes($report))
""", timeout)))

    def selection(self, ids):
        path = 'C:/ProgramData/ZeroSecurityWindowsValidation/selection.json'
        data = base64.b64encode(json.dumps(ids).encode()).decode()
        self.run(f"[IO.File]::WriteAllBytes('{path}', [Convert]::FromBase64String('{data}')); 'Selection saved'")
        return path

    def check(self, selection):
        path='C:/ProgramData/ZeroSecurityWindowsValidation/current-check.json'
        return json.loads(base64.b64decode(self.run(f"""
$p=Start-Process 'C:/ProgramData/ZeroSecurityWindowsValidation/zero_security_windows.exe' -ArgumentList @('--check','{path}','{selection}') -WindowStyle Hidden -PassThru;
$p.WaitForExit();
if($p.ExitCode -ne 0){{throw ('Check failed: '+$p.ExitCode)}}
[Convert]::ToBase64String([IO.File]::ReadAllBytes('{path}'))
""", timeout=900)))

    def reboot(self):
        before=self.status()['Boot']
        # 正常模式由测试脚本接续；工具界面的重启保持手动。
        self.run("Unregister-ScheduledTask -TaskName 'ZeroSecurityWindows.Return' -Confirm:$false -ErrorAction SilentlyContinue; Remove-ItemProperty 'HKLM:\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\RunOnce' -Name 'ZeroSecurityWindows.Return' -ErrorAction SilentlyContinue; shutdown.exe /r /t 5 /f; 'Restart scheduled'")
        print('  batch reboot scheduled', flush=True)
        deadline=time.monotonic()+900
        while time.monotonic()<deadline:
            time.sleep(15)
            try:
                state=self.status()
            except (RuntimeError,subprocess.TimeoutExpired):
                self.close()
                self.connect()
                continue
            if state['Boot']!=before and state['Resume']==0:
                if state['Phase']=='AwaitNormal':
                    continued=self.report(['--continue'],'C:/ProgramData/ZeroSecurityWindowsValidation/continuation.json')
                    return continued['restart']
                if not state['Phase']:
                    return False
                if state['Phase']=='AwaitSafe':
                    raise RuntimeError('Normal mode returned without completing the Safe Mode workflow')
        raise RuntimeError('Target did not return to normal mode within 15 minutes')


def save(path,data):
    path.write_text(json.dumps(data,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--connection',type=Path,required=True)
    parser.add_argument('--hostname',required=True)
    parser.add_argument('--results',type=Path,required=True)
    parser.add_argument('--finish-all',action='store_true')
    parser.add_argument('--only',help='仅复测指定批次')
    args=parser.parse_args()
    target=Target(args.connection)
    state=target.status()
    if state['Host']!=args.hostname or state['Host'].lower()==os.environ['COMPUTERNAME'].lower():
        raise RuntimeError('Wrong independent target')
    if state['Phase'] or state['Resume']:
        raise RuntimeError('Existing boot workflow must finish first')
    args.results.mkdir(parents=True,exist_ok=True)
    checkpoint=args.results/'batch-checkpoint.json'
    progress=json.loads(checkpoint.read_text(encoding='utf-8')) if checkpoint.exists() else {'host':args.hostname,'passed':{}}
    catalog=json.loads((Path(__file__).resolve().parents[1]/'resources/catalog.json').read_text(encoding='utf-8'))
    final_ids={'input-method','desktop-icons','start-menu','taskbar-pins'}
    standalone=[f['Id'] for f in catalog if f.get('DefaultOff')]
    remote_id='install-netease.uuremote'
    installers=[f['Id'] for f in catalog if f['Page']=='Install' and f['Id']!=remote_id]
    removals=[f['Id'] for f in catalog if f['Page']=='Remove']
    final=[f['Id'] for f in catalog if f['Id'] in final_ids]
    settings=[f['Id'] for f in catalog if f['Page'] not in ('Install','Remove') and f['Id'] not in final_ids and not f.get('DefaultOff')]
    defender=[f['Id'] for f in catalog if f['GroupZh']=='Defender']
    restore_settings=[i for i in settings if i not in defender and i!='smart-app-control']+final
    restore_apps=removals+installers
    normal_restore=restore_settings+restore_apps
    batches=[
        ('prepare-installers','restore',installers),
        ('memory-apply','apply',standalone),('memory-restore','restore',standalone),
        ('settings-apply','apply',settings),('apps-apply','apply',removals),
        ('installers-apply','apply',installers),('desktop-apply','apply',final),
        ('normal-restore','restore',normal_restore),('defender-restore','restore',defender),
        ('sac-restore','restore',['smart-app-control']),
        ('remote-restore','restore',[remote_id]),('remote-apply','apply',[remote_id]),
    ]
    for name,stage,ids in batches:
        if args.only and args.only!=name or not args.only and name in progress['passed']:
            continue
        if name=='remote-restore':
            control=target.status()
            if control['Control']!='Running' or control['ControlInstances']!=1:
                raise RuntimeError('The independent controller must be running alone before testing UU removal')
        print(f'[{name}] {stage} {len(ids)} items',flush=True)
        retry=ids
        previous=args.results/(name+'-check.json')
        if previous.exists() and not args.only:
            observed='defaults' if stage=='restore' else 'check'
            # A disconnected installer may finish after the SSH command exits.
            prior=target.check(target.selection(ids))
            save(previous,prior)
            retry=[row['id'] for row in prior['items'] if row['id'] in ids and
                   (row[observed]['state'] not in ('Done','Absent') or row['check']['state']=='Failed')]
            print(f'  retrying {len(retry)} items',flush=True)
        phases=[retry]
        if name=='normal-restore':
            # Load security and service defaults before invoking Store installations.
            phases=[[i for i in retry if i in group] for group in [restore_settings,restore_apps]]
        for phase,selected in enumerate(phases):
            if not selected:continue
            selection=target.selection(selected)
            report=target.report(['--'+stage+'-items',selection],'C:/ProgramData/ZeroSecurityWindowsValidation/current-report.json',timeout=7200)
            suffix=f'-phase-{phase}' if len(phases)>1 else ''
            save(args.results/(name+suffix+'-execute.json'),report)
            needs_boot=report['restart'] or any(row['check']['state'] in ('SafeQueued','Restart','SignIn') for row in report['items'])
            for _ in range(3):
                if not needs_boot: break
                needs_boot=target.reboot()
            if needs_boot:
                raise RuntimeError('Batch exceeded its boot stages')
        checked=target.check(target.selection(ids))
        save(args.results/(name+'-check.json'),checked)
        failures=[]
        for row in checked['items']:
            observed=row['defaults'] if stage=='restore' else row['check']
            save(args.results/(row['id']+'-'+stage+'.json'),row)
            if observed['state'] not in ('Done','Absent') or row['check']['state']=='Failed':
                failures.append(row)
        if failures:
            raise RuntimeError(name+': '+json.dumps(failures,ensure_ascii=False))
        if name=='defender-restore':
            runtime=json.loads(base64.b64decode(target.run("""
$mp=Get-MpComputerStatus;
$json=@{Service=(Get-Service WinDefend).Status.ToString(); AMServiceEnabled=[bool]$mp.AMServiceEnabled; AntivirusEnabled=[bool]$mp.AntivirusEnabled; RealTimeProtectionEnabled=[bool]$mp.RealTimeProtectionEnabled; IsTamperProtected=[bool]$mp.IsTamperProtected} | ConvertTo-Json -Compress;
[Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($json))
""")))
            save(args.results/'defender-restore-runtime.json',runtime)
            if runtime['Service']!='Running' or not all(v for k,v in runtime.items() if k!='Service'):
                raise RuntimeError('Defender runtime defaults failed: '+str(runtime))
        progress['passed'][name]={'time':datetime.now().isoformat(timespec='seconds'),'version':checked['version'],'items':ids}
        save(checkpoint,progress)
        print('  all item targets verified',flush=True)
    if args.only: return
    print(f'All {len(catalog)} item capabilities verified',flush=True)
    if args.finish_all:
        report=target.report(['--apply-all'],'C:/ProgramData/ZeroSecurityWindowsValidation/final-apply.json',timeout=7200)
        save(args.results/'apply-all.json',report)
        needs_boot=report['restart'] or any(row['check']['state'] in ('SafeQueued','Restart','SignIn') for row in report['items'])
        for _ in range(3):
            if not needs_boot: break
            needs_boot=target.reboot()
        final=target.check(target.selection([f['Id'] for f in catalog]))
        save(args.results/'final-check.json',final)
        failures=[row for row in final['items'] if row['check'] != {'state':'Done','detail':''}]
        if failures: raise RuntimeError('Final state failed: '+json.dumps(failures,ensure_ascii=False))
        print('All final states verified',flush=True)


if __name__=='__main__':
    main()
