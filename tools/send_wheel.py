"""给 plastic-dict 窗口发滚轮事件 (Blitz 0.2 没有滚动条, 只能靠滚轮验证滚动)

用法: python tools/send_wheel.py [滚动格数]   # 正数向下滚, 负数向上
"""
import ctypes
import sys
import time
from ctypes import wintypes

notches = int(sys.argv[1]) if len(sys.argv) > 1 else 5

user32 = ctypes.windll.user32
hwnd = user32.FindWindowW(None, "plastic-dict")
if not hwnd:
    raise SystemExit("找不到 plastic-dict 窗口")

user32.ShowWindow(hwnd, 9)
user32.SetForegroundWindow(hwnd)


class RECT(ctypes.Structure):
    _fields_ = [("left", wintypes.LONG), ("top", wintypes.LONG),
                ("right", wintypes.LONG), ("bottom", wintypes.LONG)]


r = RECT()
user32.GetWindowRect(hwnd, ctypes.byref(r))
cx, cy = (r.left + r.right) // 2, (r.top + r.bottom) // 2
user32.SetCursorPos(cx, cy)
time.sleep(0.2)

MOUSEEVENTF_WHEEL = 0x0800
STEP = 120
for _ in range(abs(notches)):
    delta = -STEP if notches > 0 else STEP
    user32.mouse_event(MOUSEEVENTF_WHEEL, 0, 0, delta, 0)
    time.sleep(0.05)
print(f"sent {notches} notches at ({cx},{cy})")
