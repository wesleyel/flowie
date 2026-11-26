---
layout: page
title: Downloads
description: Install Flowie on macOS, Windows or Linux, verify your download, and keep your notes when upgrading from a release candidate.
permalink: /download.html
---

<section lang="en" markdown="1">

# Download Flowie

[Latest stable release]({{ site.github_url }}/releases/latest) · [All releases and candidates]({{ site.github_url }}/releases)

## macOS

Requires macOS 13 or later. The universal app runs on Apple silicon and Intel.

- **[Mac App Store]({{ site.app_store_url }})**: signed, sandboxed and updated automatically.
- **Homebrew**: `brew install --cask wesleyel/tap/flowie`. Use `brew upgrade --cask wesleyel/tap/flowie` to update. This cask follows the newest published version, including release candidates.
- **GitHub**: download `Flowie-<version>-macos.zip`, unpack it and move Flowie to Applications. The GitHub and Homebrew builds aren't notarized. On macOS 15 or later, try opening the app once, then choose **System Settings → Privacy & Security → Open Anyway**. On earlier versions, right-click the app and choose **Open**.

## Windows (preview)

On Windows 10 or 11 x64, download `Flowie-<version>-windows-x64.zip`, extract it and run `Flowie.exe`. The zip is portable and doesn't need a separate Visual C++ runtime. It is not code-signed. There is no native Windows ARM64 package.

## Linux (preview)

For x86_64 with glibc 2.35 or later (Ubuntu 22.04 or newer). Download the `.AppImage`, mark it executable and run it. If your system doesn't support AppImage mounting, use the `.tar.gz` instead:

```sh
mkdir -p ~/.local
tar -xzf Flowie-<version>-linux-x86_64.tar.gz -C ~/.local --strip-components=1
~/.local/bin/flowie
```

Replace `<version>` with the downloaded version. File dialogs need an XDG desktop portal; for Chinese, Japanese or Korean text, install a font such as Noto CJK. See the [Linux requirements]({{ site.github_url }}/blob/main/docs/building.md#linux) if it doesn't start.

## Verify and upgrade

- The GitHub downloads each have a `.sha256` file. Run `shasum -a 256 -c <file>.sha256` on macOS or `sha256sum -c <file>.sha256` on Linux, from the downloads folder. On Windows, use `Get-FileHash <file> -Algorithm SHA256` and compare the result with the checksum file.
- **Upgrading from a 1.0 release candidate:** before updating, go to Home, clear the selection and use **File → Export as OPML**. For branches with images, select those branches and export them as a **Flowie share package** instead. Import the result into 1.0. The old database is kept separately as `flowie.sqlite.old-<timestamp>`; do not replace the new database with it.
- Notes are stored locally on each device. Cross-device sync is not included in 1.0.

</section>

<section lang="zh-CN" markdown="1">

# 下载 Flowie

[最新正式版]({{ site.github_url }}/releases/latest) · [全部版本和候选版]({{ site.github_url }}/releases)

## macOS

需要 macOS 13 或更高版本。通用版支持 Apple 芯片和 Intel。

- **[Mac App Store]({{ site.app_store_url }})**：已签名、在沙盒中运行，可自动更新。
- **Homebrew**：运行 `brew install --cask wesleyel/tap/flowie` 安装，使用 `brew upgrade --cask wesleyel/tap/flowie` 更新。这个 cask 跟随最新发布版本，包括候选版。
- **GitHub**：下载 `Flowie-<版本号>-macos.zip`，解压后把 Flowie 移到「应用程序」。GitHub 和 Homebrew 提供的版本未经公证。macOS 15 及更新版本上，先尝试打开一次，再到「系统设置 → 隐私与安全性」点「仍要打开」；更早的 macOS 上，右键点按应用并选择「打开」。

## Windows（预览）

在 Windows 10 或 11 x64 上下载 `Flowie-<版本号>-windows-x64.zip`，解压后运行 `Flowie.exe`。无需安装，也不需要单独安装 Visual C++ 运行库。该包未做代码签名，暂不提供原生 Windows ARM64 包。

## Linux（预览）

适用于 x86_64、glibc 2.35 及更新版本（Ubuntu 22.04 或更新版本）。下载 `.AppImage`，赋予可执行权限后运行。如果系统不支持挂载 AppImage，改用 `.tar.gz`：

```sh
mkdir -p ~/.local
tar -xzf Flowie-<版本号>-linux-x86_64.tar.gz -C ~/.local --strip-components=1
~/.local/bin/flowie
```

将 `<版本号>` 替换为下载的版本。文件对话框需要 XDG 桌面门户，中日韩文字需要 Noto CJK 等字体。如果无法启动，参阅 [Linux 运行要求]({{ site.github_url }}/blob/main/docs/building.md#linux)。

## 校验与升级

- GitHub 下载附带 `.sha256` 文件。在下载目录中，macOS 用 `shasum -a 256 -c <文件>.sha256`，Linux 用 `sha256sum -c <文件>.sha256` 校验；Windows 用 `Get-FileHash <文件> -Algorithm SHA256`，再与校验文件里的值比较。
- **从 1.0 候选版升级：**更新前回到首页，取消选区，使用「文件 → 导出为 OPML」。需要保留图片时，选中相应分支并导出为 Flowie 分享包，再导入 1.0。旧数据库会单独保留为 `flowie.sqlite.old-<时间戳>`；不要用它覆盖新版数据库。
- 笔记保存在各自设备上。1.0 不包含跨设备同步。

</section>
