"""在 plastic-dict 窗口内点击一个相对坐标 (验证按钮/发音等交互)

用法: python tools/send_click.py X Y [--client]   # X,Y 为相对窗口左上角(含边框)的像素
"""
import ctypes
import sys
import time
from ctypes import wintypes

x, y = int(sys.argv[1]), int(sys.argv[2])

user32 = ctypes.windll.user32
hwnd = user32.FindWindowW(None, "plastic-dict")
if not hwnd:
    raise SystemExit("找不到 plastic-dict 窗口")


class RECT(ctypes.Structure):
    _fields_ = [("left", wintypes.LONG), ("top", wintypes.LONG),
                ("right", wintypes.LONG), ("bottom", wintypes.LONG)]


r = RECT()
user32.GetWindowRect(hwnd, ctypes.byref(r))
user32.ShowWindow(hwnd, 9)
user32.SetForegroundWindow(hwnd)
time.sleep(0.3)

sx, sy = r.left + x, r.top + y
user32.SetCursorPos(sx, sy)
time.sleep(0.15)
user32.mouse_event(0x0002, 0, 0, 0, 0)  # LEFTDOWN
time.sleep(0.05)
user32.mouse_event(0x0004, 0, 0, 0, 0)  # LEFTUP
print(f"clicked window({x},{y}) -> screen({sx},{sy})")
