"""在已授权的独立目标机顺序验证执行、恢复与重启；连接配置和结果不提交。"""
import argparse
import base64
from datetime import datetime
import json
from pathlib import Path
import subprocess
import time


class Target:
    def __init__(self, configuration):
        self.folder = configuration.resolve().parent
        self.config = json.loads(configuration.read_text(encoding='utf-8'))
        self.flags = [
            '-o', 'BatchMode=yes', '-o', 'StrictHostKeyChecking=accept-new',
            '-o', 'ConnectTimeout=20', '-o', 'ServerAliveInterval=15',
            '-o', 'ServerAliveCountMax=2',
            '-o', 'UserKnownHostsFile=' + (self.folder / 'target_known_hosts').as_posix(),
            '-o', 'ProxyCommand=tailcat --key=' + self.config['client_key'] + ' ' + self.config['address'] + ' 22',
        ]

    def run(self, script, timeout=120):
        script = "[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); $OutputEncoding=[Console]::OutputEncoding; $ErrorActionPreference='Stop'; " + script
        encoded = base64.b64encode(script.encode('utf-16-le')).decode()
        result = subprocess.run(
            ['ssh', '-T', *self.flags, 'zsw-target',
             'powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand ' + encoded],
            capture_output=True, encoding='utf-8', errors='replace', timeout=timeout,
            creationflags=subprocess.CREATE_NO_WINDOW,
        )
        if result.returncode:
            raise RuntimeError((result.stderr + result.stdout).replace(self.config['address'], '[target]'))
        return result.stdout.strip()

    def status(self):
        return json.loads(base64.b64decode(self.run("""
$os=Get-CimInstance Win32_OperatingSystem;
$processes=@(Get-CimInstance Win32_Process | Where-Object {$_.CommandLine -match 'zero_security_windows.exe.*--(safe-resume|resume)'});
$json=@{Host=$env:COMPUTERNAME; Boot=$os.LastBootUpTime.ToUniversalTime().ToString('o');
  Pending=(Test-Path 'C:/ProgramData/ZeroSecurityWindows/rust-workflow.json'); Resume=$processes.Count} | ConvertTo-Json -Compress;
[Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($json))
""", timeout=60)))

    def report(self, arguments, path, timeout=2400):
        quoted = ','.join("'" + value.replace("'", "''") + "'" for value in [*arguments, path])
        script = f"""
$report='{path}'; if(Test-Path -LiteralPath $report){{Remove-Item -LiteralPath $report}}
$process=Start-Process -FilePath 'C:/ProgramData/ZeroSecurityWindowsValidation/zero_security_windows.exe' -ArgumentList @({quoted}) -WindowStyle Hidden -PassThru -Wait;
if(-not (Test-Path -LiteralPath $report)){{throw ('No report, exit='+$process.ExitCode)}}
[Convert]::ToBase64String([IO.File]::ReadAllBytes($report))
"""
        return json.loads(base64.b64decode(self.run(script, timeout)))

    def reboot(self):
        before = self.status()['Boot']
        self.run('shutdown.exe /r /t 5 /f; "Restart scheduled"')
        print('  reboot scheduled', flush=True)
        deadline = time.monotonic() + 1200
        while time.monotonic() < deadline:
            time.sleep(15)
            try:
                state = self.status()
            except (RuntimeError, subprocess.TimeoutExpired):
                continue
            if state['Boot'] != before and not state['Pending'] and state['Resume'] == 0:
                print('  normal boot ready', flush=True)
                return
        raise RuntimeError('Target did not finish its boot workflow within 20 minutes')


def save(path, data):
    path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--connection', type=Path, required=True)
    parser.add_argument('--hostname', required=True)
    parser.add_argument('--results', type=Path, required=True)
    parser.add_argument('--finish-all', action='store_true')
    args = parser.parse_args()
    target = Target(args.connection)
    state = target.status()
    if state['Host'] != args.hostname or state['Host'].lower() == __import__('os').environ['COMPUTERNAME'].lower():
        raise RuntimeError('Refusing to mutate a host other than the authorized independent target')
    args.results.mkdir(parents=True, exist_ok=True)
    checkpoint = args.results / 'checkpoint.json'
    progress = json.loads(checkpoint.read_text(encoding='utf-8')) if checkpoint.exists() else {'host': args.hostname, 'passed': {}}
    catalog = json.loads((Path(__file__).resolve().parents[1] / 'resources/catalog.json').read_text(encoding='utf-8'))
    if state['Pending'] or state['Resume']:
        raise RuntimeError('Existing boot workflow must finish before starting another operation')
    for index, feature in enumerate(catalog, 1):
        item = feature['Id']
        for stage, expected in [('apply', 'Done'), ('restore', 'Restored')]:
            if stage in progress['passed'].get(item, {}):
                continue
            print(f'[{index}/{len(catalog)}] {item} {stage}', flush=True)
            report_path = 'C:/ProgramData/ZeroSecurityWindowsValidation/current-report.json'
            report = target.report(['--' + stage, item], report_path)
            save(args.results / f'{item}-{stage}.json', report)
            check = report['items'][0]['check']
            if check['state'] in ('Failed', 'Restricted', 'Unknown'):
                raise RuntimeError(f"{item} {stage}: {check}")
            if report['restart'] or check['state'] in ('SafeQueued', 'Restart', 'SignIn'):
                target.reboot()
                report = target.report(['--check'], report_path)
                save(args.results / f'{item}-{stage}-after-boot.json', report)
                check = next(value for value in report['items'] if value['id'] == item)
            if check['state'] not in (expected, 'Absent'):
                raise RuntimeError(f"{item} {stage}: expected {expected}, actual {check}")
            progress['passed'].setdefault(item, {})[stage] = {
                'state': check['state'], 'time': datetime.now().isoformat(timespec='seconds'),
                'version': report['version'],
            }
            save(checkpoint, progress)
            print('  verified ' + check['state'], flush=True)
    print(f'All {len(catalog)} item cycles passed', flush=True)
    if args.finish_all:
        report_path = 'C:/ProgramData/ZeroSecurityWindowsValidation/current-report.json'
        report = target.report(['--apply-all'], report_path, timeout=3600)
        save(args.results / 'apply-all.json', report)
        if report['restart'] or any(value['check']['state'] in ('SafeQueued', 'Restart', 'SignIn') for value in report['items']):
            target.reboot()
        final = target.report(['--check'], report_path)
        save(args.results / 'final-check.json', final)
        failed = [value for value in final['items'] if value['state'] not in ('Done', 'Absent')]
        if failed:
            raise RuntimeError('Final state not complete: ' + json.dumps(failed, ensure_ascii=False))
        print('All final states verified', flush=True)


if __name__ == '__main__':
    main()
