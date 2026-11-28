---
layout: page
title: Downloads
description: "Install Flowie on macOS, Windows or Linux (iPhone and iPad from the App Store), verify your download, and keep your notes when upgrading from a release candidate."
permalink: /download.html
---

<div class="versions" id="versions">
  <h1><span lang="en">Download Flowie</span><span lang="zh-CN">下载 Flowie</span></h1>
  <p class="lead"><span lang="en">Every version, newest first. Each link downloads that package directly.</span><span lang="zh-CN">全部版本，最新的在最前面。每个链接都直接下载对应的安装包。</span></p>
  <div id="versions-table">
    <p><a href="{{ site.github_url }}/releases"><span lang="en">All versions on GitHub Releases</span><span lang="zh-CN">在 GitHub Releases 查看全部版本</span></a></p>
  </div>
</div>

<section lang="en" markdown="1">

## macOS

Requires macOS 13 or later. The universal app runs on Apple silicon and Intel.

- **[Mac App Store]({{ site.app_store_url }})**: signed, sandboxed and updated automatically.
- **Homebrew**: `brew install --cask wesleyel/tap/flowie`. Use `brew upgrade --cask wesleyel/tap/flowie` to update. This cask follows the newest published version, including release candidates.
- **GitHub**: download the macOS zip from the table above, unpack it and move Flowie to Applications. The GitHub and Homebrew builds aren't notarized, so macOS won't open them at first: in Terminal, run `xattr -dr com.apple.quarantine /Applications/Flowie.app`, then open Flowie. Run it again after each update.

## Windows (preview)

On Windows 10 or 11 x64, download the Windows zip from the table above, extract it and run `Flowie.exe`. The zip is portable and doesn't need a separate Visual C++ runtime. It is not code-signed. There is no native Windows ARM64 package.

## Linux (preview)

For x86_64 with glibc 2.35 or later (Ubuntu 22.04 or newer). Download the AppImage from the table above, mark it executable and run it. If your system doesn't support AppImage mounting, download the tar.gz instead:

```sh
mkdir -p ~/.local
tar -xzf Flowie-*-linux-x86_64.tar.gz -C ~/.local --strip-components=1
~/.local/bin/flowie
```

File dialogs need an XDG desktop portal; for Chinese, Japanese or Korean text, install a font such as Noto CJK. See the [Linux requirements]({{ site.github_url }}/blob/main/docs/building.md#linux) if it doesn't start.

## Verify and upgrade

- The GitHub downloads each have a `.sha256` file. Run `shasum -a 256 -c <file>.sha256` on macOS or `sha256sum -c <file>.sha256` on Linux, from the downloads folder. On Windows, use `Get-FileHash <file> -Algorithm SHA256` and compare the result with the checksum file.
- **Upgrading from a 1.0 release candidate:** before updating, go to Home, clear the selection and use **File → Export as OPML**. For branches with images, select those branches and export them as a **Flowie share package** instead. Import the result into 1.0. The old database is kept separately as `flowie.sqlite.old-<timestamp>`; do not replace the new database with it.
- Notes are stored on each device. To keep devices in step, the App Store version syncs through iCloud, and every desktop version can sync through a [Flowie server you run yourself]({{ site.github_url }}/blob/main/docs/sync.md).

</section>

<section lang="zh-CN" markdown="1">

## macOS

需要 macOS 13 或更高版本。通用版支持 Apple 芯片和 Intel。

- **[Mac App Store]({{ site.app_store_url }})**：已签名、在沙盒中运行，可自动更新。
- **Homebrew**：运行 `brew install --cask wesleyel/tap/flowie` 安装，使用 `brew upgrade --cask wesleyel/tap/flowie` 更新。这个 cask 跟随最新发布版本，包括候选版。
- **GitHub**：从上面的表格下载 macOS 的 zip，解压后把 Flowie 移到「应用程序」。GitHub 和 Homebrew 提供的版本未经公证，macOS 一开始会拒绝打开：在终端运行 `xattr -dr com.apple.quarantine /Applications/Flowie.app`，再打开 Flowie。每次更新后都要再运行一次。

## Windows（预览）

在 Windows 10 或 11 x64 上，从上面的表格下载 Windows 的 zip，解压后运行 `Flowie.exe`。无需安装，也不需要单独安装 Visual C++ 运行库。该包未做代码签名，暂不提供原生 Windows ARM64 包。

## Linux（预览）

适用于 x86_64、glibc 2.35 及更新版本（Ubuntu 22.04 或更新版本）。从上面的表格下载 AppImage，赋予可执行权限后运行。如果系统不支持挂载 AppImage，改为下载 tar.gz：

```sh
mkdir -p ~/.local
tar -xzf Flowie-*-linux-x86_64.tar.gz -C ~/.local --strip-components=1
~/.local/bin/flowie
```

文件对话框需要 XDG 桌面门户，中日韩文字需要 Noto CJK 等字体。如果无法启动，参阅 [Linux 运行要求]({{ site.github_url }}/blob/main/docs/building.md#linux)。

## 校验与升级

- GitHub 下载附带 `.sha256` 文件。在下载目录中，macOS 用 `shasum -a 256 -c <文件>.sha256`，Linux 用 `sha256sum -c <文件>.sha256` 校验；Windows 用 `Get-FileHash <文件> -Algorithm SHA256`，再与校验文件里的值比较。
- **从 1.0 候选版升级：**更新前回到首页，取消选区，使用「文件 → 导出为 OPML」。需要保留图片时，选中相应分支并导出为 Flowie 分享包，再导入 1.0。旧数据库会单独保留为 `flowie.sqlite.old-<时间戳>`；不要用它覆盖新版数据库。
- 笔记保存在各自设备上。App Store 版可以通过 iCloud 同步，所有桌面版也可以通过[你自己运行的 Flowie 服务器]({{ site.github_url }}/blob/main/docs/sync.md)同步。

</section>

<script src="{{ '/downloads.js' | relative_url }}" defer></script>
