#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""生成一个最小但格式合法的 MDX v2.0 / MDD 测试夹具 (牛津 9 风格词条 HTML)

用途: 在本地没有真实牛津词典的情况下, 端到端验证 Rust 版
      MDX 解析 → 索引缓存 → 查词 → CLI 纯文本渲染 / GUI 预处理 整条链路。

格式依据 writemdict/fileformat.md (v2.0), 并与 js-mdict 7.0.0 的读取行为一致:
  header_sect:  [4] length(BE) + [N] UTF-16LE XML + [4] adler32(小端)
  keyword_sect: [40] 5×u64(BE) + [4] adler32(大端, v2.0 特有)
               + key_index   (comp_type+checksum+数据 封装, 通常 zlib)
               + key_blocks  (每块同封装; 解压后每词条 offset(8,BE)+key+NUL)
  record_sect:  [32] 4×u64(BE) + comp/decomp 对 (每块 16 字节, 无校验和!)
               + record_blocks (每块同封装; 解压后所有记录依次相连,
                                MDX 词条以 NUL 结尾, MDD 二进制资源不加)
  压缩块封装:   comp_type(4, 小端 uint32: 0=raw/1=LZO/2=zlib)
               + adler32(4, 大端) + 数据

夹具故意拆成 2 个 key block + 2 个 record block, 以覆盖多块的
累加偏移 (pack_acc/unpack_acc) 与跨块查词路径 (真实词典都是多块)。
"""
import struct
import zlib
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "testdata"
OUT.mkdir(exist_ok=True)

# 词条 HTML (贴近牛津 9 的标签风格: top-g / h / phon / pos-g / sn-g / x-g / chn / vpform)
ENTRIES = {
    "apple": (
        '<top-g><h>apple</h><brelabel>BrE</brelabel>'
        '<phon data-src-mp3="sound_us_apple.spx">/ˈæp.əl/</phon>'
        '<namelabel>NAmE</namelabel><phon>/ˈæp.əl/</phon></top-g>'
        '<pos-g><pos>noun</pos></pos-g>'
        '<sn-g><x-g><x>a round fruit with firm white flesh and a green, red, or yellow skin</x>'
        '<chn>苹果</chn></x-g></sn-g>'
        '<v-g><vpform>past simple</vpform><h>appled</h><phon>/ˈæp.əld/</phon></v-g>'
    ),
    "banana": (
        '<top-g><h>banana</h><brelabel>BrE</brelabel><phon>/bəˈnɑː.nə/</phon></top-g>'
        '<pos-g><pos>noun</pos></pos-g>'
        '<sn-g><x-g><x>a long curved fruit with a yellow skin</x><chn>香蕉</chn></x-g></sn-g>'
    ),
    "run": (
        '<top-g><h>run</h><brelabel>BrE</brelabel><phon>/rʌn/</phon></top-g>'
        '<pos-g><pos>verb</pos></pos-g>'
        '<sn-g><x-g><x>to move quickly using your legs</x><chn>跑</chn></x-g></sn-g>'
        '<pos-g><pos>noun</pos></pos-g>'
        '<sn-g><x-g><x>a period of time spent doing a particular activity</x><chn>一段时期</chn></x-g></sn-g>'
    ),
}

# MDD: 一张 1x1 红色 PNG (程序化生成, 保证 CRC/块结构合法) + 一个音频占位文件
def make_png_1x1_red() -> bytes:
    def chunk(typ: bytes, data: bytes) -> bytes:
        return (struct.pack(">I", len(data)) + typ + data
                + struct.pack(">I", zlib.crc32(typ + data) & 0xFFFFFFFF))

    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(b"\x00\xff\x00\x00", 9))
            + chunk(b"IEND", b""))


PNG_1X1_RED = make_png_1x1_red()

MDD_ENTRIES = {
    "\\apple.png": PNG_1X1_RED,
    "\\sound_us_apple.spx": b"fake-spx-audio-bytes",
}


def comp_block(data: bytes) -> bytes:
    """v2.0 压缩块封装: comp_type(小端 2=zlib) + adler32(大端) + zlib 数据

    comp_type 的 4 字节是小端 uint32 (字节序 02 00 00 00 = 2 = zlib),
    与 js-mdict 对 Buffer.toString('hex') == '02000000' 的判断一致;
    adler32 按 fileformat.md 是大端。
    """
    return (
        struct.pack("<I", 2)
        + struct.pack(">I", zlib.adler32(data) & 0xFFFFFFFF)
        + zlib.compress(data, 9)
    )


def build(path: Path, attrs: str, entries: list, key_split: int, is_mdd: bool) -> None:
    """entries: [(key, record_bytes)] 已按 key 排序; key_split: 第一个 key block 的词条数
    (0 < key_split < len(entries), 即固定拆成 2 个 key block / 2 个 record block)"""
    assert 0 < key_split < len(entries), "key_split 必须在 (0, n) 内"
    enc = "utf-16-le" if is_mdd else "utf-8"
    unit = 2 if is_mdd else 1          # first/last_word_size 的计数单位
    term = b"\x00\x00" if is_mdd else b"\x00"
    n = len(entries)

    # ---- record 数据: 所有记录依次相连 (key 里的 offset 指向这个拼接数组) ----
    record_data = bytearray()
    offsets: list[int] = []
    for _, rec in entries:
        offsets.append(len(record_data))
        record_data.extend(rec)
        if not is_mdd:
            record_data.append(0)      # MDX 词条以 NUL 结尾 (读取端会剥掉)
    total = len(record_data)

    # ---- 切 record blocks (在词条边界切, 与 key block 边界一致) ----
    rec_boundary = offsets[key_split]
    rec_chunks = [bytes(record_data[:rec_boundary]), bytes(record_data[rec_boundary:])]
    rec_blocks = [comp_block(c) for c in rec_chunks]
    rec_info = b"".join(
        struct.pack(">QQ", len(b), len(c)) for b, c in zip(rec_blocks, rec_chunks)
    )

    # ---- key blocks: 解压后每块 = offset(8,BE) + key + NUL 序列 ----
    ranges = [(0, key_split), (key_split, n)]
    key_datas: list[bytes] = []
    for lo, hi in ranges:
        kd = bytearray()
        for i in range(lo, hi):
            kd.extend(struct.pack(">Q", offsets[i]))
            kd.extend(entries[i][0].encode(enc))
            kd.extend(term)
        key_datas.append(bytes(kd))
    key_blocks = [comp_block(kd) for kd in key_datas]

    # ---- key_index (block info), 每块一条:
    #      num_entries(8) first_size(2)+word+NUL last_size(2)+word+NUL pack(8) unpack(8) ----
    info = bytearray()
    for bi, (lo, hi) in enumerate(ranges):
        first, last = entries[lo][0], entries[hi - 1][0]
        fw, lw = first.encode(enc), last.encode(enc)
        info.extend(struct.pack(">Q", hi - lo))
        info.extend(struct.pack(">H", len(fw) // unit))
        info.extend(fw + term)
        info.extend(struct.pack(">H", len(lw) // unit))
        info.extend(lw + term)
        info.extend(struct.pack(">Q", len(key_blocks[bi])))   # comp_size
        info.extend(struct.pack(">Q", len(key_datas[bi])))   # decomp_size
    key_info = bytes(info)
    key_info_block = comp_block(key_info)   # key_index 自身也是压缩块封装

    # ---- headers ----
    header = (
        '<Dictionary GeneratedByEngineVersion="2.0" RequiredEngineVersion="2.0" '
        f'{attrs} Encrypted="No" KeyCaseSensitive="No" '
        f'Title="plastic-dict 测试{"资源库" if is_mdd else "词典"}" Description="fixture"/>'
        "\x00\x00"
    ).encode("utf-16-le")

    key_header = struct.pack(
        ">QQQQQ",
        len(ranges),                 # keyword_blocks_num
        n,                           # keyword_num
        len(key_info),               # key_info_unpack_size
        len(key_info_block),         # key_info_packed_size (含 comp_type+checksum)
        sum(len(b) for b in key_blocks),  # keyword_block_packed_size
    )
    rec_header = struct.pack(
        ">QQQQ",
        len(rec_blocks),             # record_blocks_num
        n,                           # entries_num
        len(rec_info),               # record_info_comp_size
        sum(len(b) for b in rec_blocks),  # record_block_comp_size
    )

    blob = bytearray()
    blob.extend(struct.pack(">I", len(header)))
    blob.extend(header)
    blob.extend(struct.pack("<I", zlib.adler32(header) & 0xFFFFFFFF))  # header 校验和 (小端)
    blob.extend(key_header)
    blob.extend(struct.pack(">I", zlib.adler32(key_header) & 0xFFFFFFFF))  # key header 校验和 (大端)
    blob.extend(key_info_block)
    for b in key_blocks:
        blob.extend(b)
    blob.extend(rec_header)          # record header 后无校验和, 直接跟 comp/decomp 对
    blob.extend(rec_info)
    for b in rec_blocks:
        blob.extend(b)
    path.write_bytes(bytes(blob))
    kind = "MDD" if is_mdd else "MDX"
    print(f"{kind} -> {path} ({len(blob)} bytes, {n} 词条, "
          f"{len(ranges)} key blocks, {len(rec_blocks)} record blocks)")


if __name__ == "__main__":
    mdx_entries = [(w, ENTRIES[w].encode("utf-8")) for w in sorted(ENTRIES)]
    mdd_entries = [(k, MDD_ENTRIES[k]) for k in sorted(MDD_ENTRIES)]
    build(OUT / "Oxford9.mdx", 'Encoding="UTF-8"', mdx_entries, key_split=2, is_mdd=False)
    build(OUT / "Oxford9.mdd", 'Encoding="UTF-16"', mdd_entries, key_split=1, is_mdd=True)
