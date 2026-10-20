"""截取 plastic-dict 窗口画面 (纯 ctypes GDI + 手写 PNG, 无第三方依赖)"""
import ctypes
import struct
import sys
import time
import zlib
from ctypes import wintypes

OUT = sys.argv[1] if len(sys.argv) > 1 else r"D:\weiheng\proj\dict\plastic-dict\target\shot.png"
# 可选: 第 2/3 个参数指定窗口尺寸 (像素, 含边框), 用于验证布局随宽度变化的行为
RESIZE = (int(sys.argv[2]), int(sys.argv[3])) if len(sys.argv) > 3 else None

user32 = ctypes.windll.user32
gdi32 = ctypes.windll.gdi32

hwnd = user32.FindWindowW(None, "plastic-dict")
if not hwnd:
    raise SystemExit("找不到 plastic-dict 窗口")
# 先最小化再还原, 然后临时置顶: 只调 SetForegroundWindow 在别的窗口刚激活时
# 会被系统拒绝, BitBlt 抓到的就是被遮挡的其他窗口 (踩过: 抓到浏览器)
user32.ShowWindow(hwnd, 6)          # SW_MINIMIZE
time.sleep(0.25)
user32.ShowWindow(hwnd, 9)          # SW_RESTORE
user32.SetForegroundWindow(hwnd)
HWND_TOPMOST, HWND_NOTOPMOST = -1, -2
SWP_NOMOVE, SWP_NOSIZE = 0x0002, 0x0001
user32.SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE)
time.sleep(0.45)
if RESIZE:
    user32.MoveWindow(hwnd, 26, 26, RESIZE[0], RESIZE[1], True)
    # 给布局 + 重绘留时间
    time.sleep(1.2)


class RECT(ctypes.Structure):
    _fields_ = [("left", wintypes.LONG), ("top", wintypes.LONG),
                ("right", wintypes.LONG), ("bottom", wintypes.LONG)]


r = RECT()
user32.GetWindowRect(hwnd, ctypes.byref(r))
w, h = r.right - r.left, r.bottom - r.top
if w <= 0 or h <= 0:
    raise SystemExit(f"窗口尺寸异常: {w}x{h}")

SRCCOPY = 0x00CC0020
hdc_screen = user32.GetDC(0)
hdc_mem = gdi32.CreateCompatibleDC(hdc_screen)
hbmp = gdi32.CreateCompatibleBitmap(hdc_screen, w, h)
gdi32.SelectObject(hdc_mem, hbmp)

# 优先 PrintWindow(PW_RENDERFULLCONTENT): 让窗口自己重绘到目标 DC,
# 完全不受其他窗口遮挡影响. 置顶/置前那套在别的全屏窗口抢焦点时并不可靠
# (踩过: 抓到小智浏览器的 mp3 播放页). 失败再退回 BitBlt 抓屏幕.
PW_RENDERFULLCONTENT = 0x00000002
ok = False
try:
    ok = bool(user32.PrintWindow(hwnd, hdc_mem, PW_RENDERFULLCONTENT))
except Exception:
    ok = False
if not ok:
    ok = bool(gdi32.BitBlt(hdc_mem, 0, 0, w, h, hdc_screen, r.left, r.top, SRCCOPY))
    print("PrintWindow 不可用, 退回 BitBlt(可能被遮挡)")
else:
    print("PrintWindow OK")


class BITMAPINFOHEADER(ctypes.Structure):
    _fields_ = [("biSize", wintypes.DWORD), ("biWidth", wintypes.LONG),
                ("biHeight", wintypes.LONG), ("biPlanes", wintypes.WORD),
                ("biBitCount", wintypes.WORD), ("biCompression", wintypes.DWORD),
                ("biSizeImage", wintypes.DWORD), ("biXPelsPerMeter", wintypes.LONG),
                ("biYPelsPerMeter", wintypes.LONG), ("biClrUsed", wintypes.DWORD),
                ("biClrImportant", wintypes.DWORD)]


bi = BITMAPINFOHEADER()
bi.biSize = ctypes.sizeof(BITMAPINFOHEADER)
bi.biWidth = w
bi.biHeight = -h            # 负数 = top-down
bi.biPlanes = 1
bi.biBitCount = 32
bi.biCompression = 0        # BI_RGB
buf = ctypes.create_string_buffer(w * h * 4)
gdi32.GetDIBits(hdc_mem, hbmp, 0, h, buf, ctypes.byref(bi), 0)

# BGRA → RGB, 每行前加 filter 0
raw_px = buf.raw
rows = []
for y in range(h):
    line = raw_px[y * w * 4:(y + 1) * w * 4]
    rgb = bytearray(w * 3)
    rgb[0::3] = line[2::4]
    rgb[1::3] = line[1::4]
    rgb[2::3] = line[0::4]
    rows.append(b"\x00" + bytes(rgb))
raw = b"".join(rows)


def chunk(typ, data):
    return (struct.pack(">I", len(data)) + typ + data
            + struct.pack(">I", zlib.crc32(typ + data) & 0xFFFFFFFF))


png = (b"\x89PNG\r\n\x1a\n"
       + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
       + chunk(b"IDAT", zlib.compress(raw, 6))
       + chunk(b"IEND", b""))
with open(OUT, "wb") as f:
    f.write(png)

gdi32.DeleteObject(hbmp)
gdi32.DeleteDC(hdc_mem)
user32.ReleaseDC(0, hdc_screen)
print(f"saved {OUT} ({w}x{h}, at {r.left},{r.top})")
