#!/usr/bin/env bash
# dic (bun 版) 系统依赖安装脚本
# 用法: sudo ./install-deps.sh   (或按提示手动执行对应命令)
set -euo pipefail

if command -v apt-get >/dev/null 2>&1; then
  # Debian 13 / Ubuntu 24.04 及以上 (需要 GTK4 的 WebKitGTK 6.0)
  echo "==> apt 安装: libwebkitgtk-6.0-4 + CJK 字体 + 音频播放器"
  apt-get install -y libwebkitgtk-6.0-4 fonts-noto-cjk mpv || {
    echo "!! 部分包安装失败, 请检查发行版是否提供 libwebkitgtk-6.0-4"
    echo "!! Ubuntu 22.04 / Debian 12 没有 GTK4 版 WebKitGTK, 无法使用预编译 webview 库"
    exit 1
  }
elif command -v dnf >/dev/null 2>&1; then
  echo "==> dnf 安装: webkitgtk6.0 + CJK 字体 + 音频播放器"
  dnf install -y webkitgtk6.0 google-noto-sans-cjk-ttc-fonts mpv || {
    echo "!! 部分包安装失败, 请检查包名 (老版本 Fedora 无 webkitgtk6.0)"
    exit 1
  }
else
  echo "!! 未识别的包管理器, 请手动安装 GTK4 WebKit 运行时:"
  echo "   Debian/Ubuntu: sudo apt install libwebkitgtk-6.0-4 fonts-noto-cjk mpv"
  echo "   Fedora/RHEL:   sudo dnf install webkitgtk6.0 google-noto-sans-cjk-ttc-fonts mpv"
  exit 1
fi

echo "==> 完成。接下来在本目录执行: bun install"
