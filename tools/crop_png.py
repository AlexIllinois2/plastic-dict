"""裁剪 / 放大 grab_window.py 产出的 PNG（纯 zlib 解 filter 0/1/2，无第三方依赖）。

用法:
    python tools/crop_png.py 输入.png 输出.png x y w h [放大倍数]

只支持 grab_window.py 写出的 8bit RGB 非隔行 PNG（filter 0），够用。
"""
import struct
import sys
import zlib


def read_png(path):
    data = open(path, "rb").read()
    assert data[:8] == b"\x89PNG\r\n\x1a\n", "不是 PNG"
    pos = 8
    idat = b""
    w = h = 0
    while pos < len(data):
        (ln,) = struct.unpack(">I", data[pos:pos + 4])
        typ = data[pos + 4:pos + 8]
        body = data[pos + 8:pos + 8 + ln]
        pos += 12 + ln
        if typ == b"IHDR":
            w, h, depth, color = struct.unpack(">IIBB", body[:10])
            assert depth == 8 and color == 2, f"仅支持 8bit RGB, 实际 {depth}/{color}"
        elif typ == b"IDAT":
            idat += body
        elif typ == b"IEND":
            break
    raw = zlib.decompress(idat)
    stride = w * 3
    rows = []
    prev = bytearray(stride)
    p = 0
    for _ in range(h):
        f = raw[p]
        line = bytearray(raw[p + 1:p + 1 + stride])
        p += 1 + stride
        if f == 1:      # Sub
            for i in range(3, stride):
                line[i] = (line[i] + line[i - 3]) & 0xFF
        elif f == 2:    # Up
            for i in range(stride):
                line[i] = (line[i] + prev[i]) & 0xFF
        elif f != 0:
            raise SystemExit(f"不支持的 filter {f}")
        rows.append(bytes(line))
        prev = line
    return w, h, rows


def write_png(path, w, h, rows):
    raw = b"".join(b"\x00" + r for r in rows)

    def chunk(typ, payload):
        return (struct.pack(">I", len(payload)) + typ + payload
                + struct.pack(">I", zlib.crc32(typ + payload) & 0xFFFFFFFF))

    png = (b"\x89PNG\r\n\x1a\n"
           + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
           + chunk(b"IDAT", zlib.compress(raw, 9))
           + chunk(b"IEND", b""))
    open(path, "wb").write(png)


def main():
    if len(sys.argv) < 7:
        raise SystemExit(__doc__)
    src, dst = sys.argv[1], sys.argv[2]
    x, y, cw, ch = (int(v) for v in sys.argv[3:7])
    scale = int(sys.argv[7]) if len(sys.argv) > 7 else 1

    w, h, rows = read_png(src)
    cw = min(cw, w - x)
    ch = min(ch, h - y)
    out = []
    for ry in range(y, y + ch):
        line = rows[ry][x * 3:(x + cw) * 3]
        if scale > 1:
            line = b"".join(line[i:i + 3] * scale for i in range(0, len(line), 3))
        for _ in range(scale):
            out.append(line)
    write_png(dst, cw * scale, ch * scale, out)
    print(f"saved {dst} ({cw * scale}x{ch * scale}, crop {x},{y} {cw}x{ch} x{scale})")


main()
