# 只测试首屏、方向键和退出，不执行优化；通过隐藏 ConPTY 运行。
import ctypes as C
from ctypes import wintypes as W
import json, time, threading, pathlib, sys, os, subprocess

k=C.WinDLL('kernel32', use_last_error=True)
H=W.HANDLE
class COORD(C.Structure): _fields_=[('X',C.c_short),('Y',C.c_short)]
class SI(C.Structure):
    _fields_=[('cb',W.DWORD),('reserved',W.LPWSTR),('desktop',W.LPWSTR),('title',W.LPWSTR),('x',W.DWORD),('y',W.DWORD),('width',W.DWORD),('height',W.DWORD),('cols',W.DWORD),('rows',W.DWORD),('fill',W.DWORD),('flags',W.DWORD),('show',W.WORD),('reserved_size',W.WORD),('reserved_data',C.c_void_p),('input',H),('output',H),('error',H)]
class SIX(C.Structure): _fields_=[('StartupInfo',SI),('attributes',C.c_void_p)]
class PI(C.Structure): _fields_=[('process',H),('thread',H),('pid',W.DWORD),('tid',W.DWORD)]
k.CreatePipe.argtypes=[C.POINTER(H),C.POINTER(H),C.c_void_p,W.DWORD]
k.CreatePseudoConsole.argtypes=[COORD,H,H,W.DWORD,C.POINTER(H)];k.CreatePseudoConsole.restype=C.c_long
k.InitializeProcThreadAttributeList.argtypes=[C.c_void_p,W.DWORD,W.DWORD,C.POINTER(C.c_size_t)]
k.UpdateProcThreadAttribute.argtypes=[C.c_void_p,W.DWORD,C.c_size_t,C.c_void_p,C.c_size_t,C.c_void_p,C.c_void_p]
k.CreateProcessW.argtypes=[W.LPCWSTR,W.LPWSTR,C.c_void_p,C.c_void_p,W.BOOL,W.DWORD,C.c_void_p,W.LPCWSTR,C.c_void_p,C.POINTER(PI)]
k.ReadFile.argtypes=[H,C.c_void_p,W.DWORD,C.POINTER(W.DWORD),C.c_void_p]
k.WriteFile.argtypes=[H,C.c_void_p,W.DWORD,C.POINTER(W.DWORD),C.c_void_p]
k.WaitForSingleObject.argtypes=[H,W.DWORD];k.GetExitCodeProcess.argtypes=[H,C.POINTER(W.DWORD)]
k.CloseHandle.argtypes=[H];k.ClosePseudoConsole.argtypes=[H];k.TerminateProcess.argtypes=[H,W.DWORD]
k.DeleteProcThreadAttributeList.argtypes=[C.c_void_p]
def ok(value):
    if not value: raise C.WinError(C.get_last_error())
def write(handle,data):
    n=W.DWORD();ok(k.WriteFile(handle,data,len(data),C.byref(n),None))

def probe(exe):
    # 开发工具会注入 NO_COLOR；按用户正常打开终端的环境测量。
    os.environ.pop('NO_COLOR', None)
    ir,iw,orr,ow=H(),H(),H(),H()
    ok(k.CreatePipe(C.byref(ir),C.byref(iw),None,0));ok(k.CreatePipe(C.byref(orr),C.byref(ow),None,0))
    pty=H();hr=k.CreatePseudoConsole(COORD(120,35),ir,ow,0,C.byref(pty))
    if hr: raise RuntimeError(hex(hr&0xffffffff))
    k.CloseHandle(ir);k.CloseHandle(ow)
    size=C.c_size_t();k.InitializeProcThreadAttributeList(None,1,0,C.byref(size));attributes=C.create_string_buffer(size.value)
    ok(k.InitializeProcThreadAttributeList(attributes,1,0,C.byref(size)));ok(k.UpdateProcThreadAttribute(attributes,0,0x20016,pty,C.sizeof(H),None,None))
    si=SIX();si.StartupInfo.cb=C.sizeof(SIX);si.attributes=C.cast(attributes,C.c_void_p);pi=PI()
    data=bytearray();changes=[];lock=threading.Lock();first=threading.Event();frame_time=[]
    start=time.perf_counter()
    def reader():
        while True:
            buffer=C.create_string_buffer(65536);n=W.DWORD()
            if not k.ReadFile(orr,buffer,len(buffer),C.byref(n),None) or not n.value: break
            now=time.perf_counter()
            with lock:
                data.extend(buffer.raw[:n.value]);changes.append(now)
                if b'Space' in data and not first.is_set(): frame_time.append(now);first.set()
    thread=threading.Thread(target=reader,daemon=True);thread.start()
    cmd=pathlib.Path(os.environ['SystemRoot'])/'System32/cmd.exe'
    line=f'cmd.exe /d /q /c start "" /wait "{exe}"'
    ok(k.CreateProcessW(str(cmd),C.create_unicode_buffer(line),None,None,False,0x80000,None,str(exe.parent),C.byref(si),C.byref(pi)))
    try:
        if not first.wait(15): raise RuntimeError('No TUI frame: '+data[-2000:].decode('utf-8',errors='replace'))
        first_ms=frame_time[0]-start
        # Navigation while detection is active; only arrows and exit are sent.
        delays=[]
        for _ in range(6):
            time.sleep(.13)
            before=time.perf_counter();write(iw,b'\x1b[B')
            deadline=before+1
            while time.perf_counter()<deadline:
                with lock: after=[t for t in changes if t>before]
                if after: delays.append((after[0]-before)*1000);break
                time.sleep(.001)
        write(iw,b'q')
        if k.WaitForSingleObject(pi.process,5000)!=0: raise RuntimeError('UI did not exit')
        exit_code=W.DWORD();ok(k.GetExitCodeProcess(pi.process,C.byref(exit_code)))
        return dict(first_frame_ms=round(first_ms*1000,2),navigation_ms=[round(v,2) for v in delays],exit_code=exit_code.value,output_bytes=len(data)),bytes(data)
    finally:
        if k.WaitForSingleObject(pi.process,0)!=0:
            subprocess.run(['taskkill.exe','/PID',str(pi.pid),'/T','/F'],capture_output=True,creationflags=0x08000000)
        k.CloseHandle(pi.thread);k.CloseHandle(pi.process);k.CloseHandle(iw);k.ClosePseudoConsole(pty);k.CloseHandle(orr);k.DeleteProcThreadAttributeList(attributes)

if __name__=='__main__':
    exe=pathlib.Path(sys.argv[1]).resolve();result,data=probe(exe)
    print(json.dumps(result))
