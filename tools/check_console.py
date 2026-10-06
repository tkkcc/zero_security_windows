# 使用隐藏的经典控制台验证背景、滚动条与退出后的恢复，只发送退出键。
import ctypes as C
from ctypes import wintypes as W
import json
import pathlib
import subprocess
import sys
import time
import winreg

k = C.WinDLL('kernel32', use_last_error=True)
u = C.WinDLL('user32', use_last_error=True)

class Coord(C.Structure):
    _fields_ = [('x', C.c_short), ('y', C.c_short)]

class Rect(C.Structure):
    _fields_ = [('left', C.c_short), ('top', C.c_short), ('right', C.c_short), ('bottom', C.c_short)]

class Info(C.Structure):
    _fields_ = [('cb', W.DWORD), ('size', Coord), ('cursor', Coord), ('attrs', W.WORD),
                ('window', Rect), ('maximum', Coord), ('popup', W.WORD), ('fullscreen', W.BOOL),
                ('colors', W.DWORD * 16)]

class Key(C.Structure):
    _fields_ = [('down', W.BOOL), ('repeat', W.WORD), ('vk', W.WORD), ('scan', W.WORD),
                ('char', W.WCHAR), ('modifiers', W.DWORD)]

class Record(C.Structure):
    _fields_ = [('kind', W.WORD), ('padding', W.WORD), ('key', Key)]

k.CreateFileW.argtypes = [W.LPCWSTR, W.DWORD, W.DWORD, C.c_void_p, W.DWORD, W.DWORD, W.HANDLE]
k.CreateFileW.restype = W.HANDLE
k.GetConsoleScreenBufferInfoEx.argtypes = [W.HANDLE, C.POINTER(Info)]
k.GetConsoleWindow.restype = W.HWND
k.WriteConsoleInputW.argtypes = [W.HANDLE, C.POINTER(Record), W.DWORD, C.POINTER(W.DWORD)]
k.CloseHandle.argtypes = [W.HANDLE]
u.IsWindowVisible.argtypes = [W.HWND]
u.GetWindowLongW.argtypes = [W.HWND, C.c_int]

def checked(value):
    if not value:
        raise C.WinError(C.get_last_error())

def info():
    handle = k.CreateFileW('CONOUT$', 0xc0000000, 3, None, 3, 0, None)
    if handle == W.HANDLE(-1).value:
        raise C.WinError(C.get_last_error())
    result = Info()
    result.cb = C.sizeof(Info)
    try:
        checked(k.GetConsoleScreenBufferInfoEx(handle, C.byref(result)))
        return result
    finally:
        k.CloseHandle(handle)

def main():
    exe = pathlib.Path(sys.argv[1]).resolve()
    si = subprocess.STARTUPINFO()
    si.dwFlags = subprocess.STARTF_USESHOWWINDOW
    si.wShowWindow = 0
    parent = subprocess.Popen(['cmd.exe', '/d', '/q', '/c', 'ping', '127.0.0.1', '-n', '60'],
                              startupinfo=si, creationflags=subprocess.CREATE_NEW_CONSOLE,
                              stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    child = None
    try:
        time.sleep(.1)
        k.FreeConsole()
        checked(k.AttachConsole(parent.pid))
        hwnd = k.GetConsoleWindow()
        assert not u.IsWindowVisible(hwnd), 'Test console must remain hidden'
        original = info()
        index = (original.attrs >> 4) & 15
        child = subprocess.Popen([str(exe)], startupinfo=si, stdin=subprocess.DEVNULL,
                                 stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, r'Software\Microsoft\Windows\CurrentVersion\Themes\Personalize') as key:
            light = winreg.QueryValueEx(key, 'AppsUseLightTheme')[0]
        expected = 0xF5F1EF if light else 0x2E1E1E
        until = time.monotonic() + 5
        while info().colors[index] != expected and time.monotonic() < until:
            time.sleep(.01)
        active = info()
        assert active.colors[index] == expected, 'Console padding has the wrong background'
        assert not u.IsWindowVisible(hwnd), 'App must not reveal the test console'
        assert not u.GetWindowLongW(hwnd, -16) & 0x00300000, 'System scrollbars remain visible'
        handle = k.CreateFileW('CONIN$', 0xc0000000, 3, None, 3, 0, None)
        records = (Record * 2)(Record(1, 0, Key(True, 1, 81, 0, 'q', 0)),
                               Record(1, 0, Key(False, 1, 81, 0, 'q', 0)))
        written = W.DWORD()
        checked(k.WriteConsoleInputW(handle, records, 2, C.byref(written)))
        k.CloseHandle(handle)
        assert child.wait(timeout=5) == 0
        assert info().colors[index] == original.colors[index], 'Original console background was not restored'
        print(json.dumps({'hidden': True, 'background': hex(expected), 'restored': True,
                          'viewport': [active.window.right + 1, active.window.bottom + 1]}))
    finally:
        if child is not None and child.poll() is None:
            child.kill()
        subprocess.run(['taskkill.exe', '/PID', str(parent.pid), '/T', '/F'],
                       stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                       creationflags=subprocess.CREATE_NO_WINDOW)
        k.FreeConsole()

if __name__ == '__main__':
    main()
