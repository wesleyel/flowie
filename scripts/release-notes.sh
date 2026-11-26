#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

marketing=$(sed -nE '/^\[workspace.package\]/,/^\[/ s/^version = "(.*)"/\1/p' Cargo.toml)
version=${1:?Expected --version, --check or a release version}

case "$version" in
  --version)
    printf '%s\n' "$marketing"
    exit
    ;;
  --check)
    version=$marketing
    if ! scripts/changelog-section.sh "$version" > /dev/null 2>&1; then
      version=$(awk -v prefix="## [${marketing}-rc." 'index($0, prefix) == 1 { split($0, parts, /[\[\]]/); print parts[2]; exit }' CHANGELOG.md)
    fi
    ;;
esac

if ! [[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+(-rc\.[0-9]+)?$ && ${version%%-*} == "$marketing" ]]; then
  echo "Expected changelog notes matching Cargo.toml ($marketing); got '$version'" >&2
  exit 1
fi
english=$(scripts/changelog-section.sh "$version")
chinese=$(scripts/changelog-section.sh "$version" CHANGELOG.zh-CN.md)
if [[ ${1:-} == --check ]]; then exit 0; fi

printf '%s\n\n---\n\n## 中文更新日志\n\n%s\n' "$english" "$chinese"
if [[ ${2:-} == --github ]]; then
  base="https://github.com/${GITHUB_REPOSITORY:-wesleyel/flowie}/releases/download/v$version"
  # One row per package: the file links to its download, and its checksum next to it.
  row() {
    local platform=$1 file="Flowie-$version-$2" extra=${3:-}
    printf '| %s | [%s](%s/%s)%s | [sha256](%s/%s.sha256) |\n' "$platform" "$file" "$base" "$file" "$extra" "$base" "$file"
  }
  printf '\n## Downloads / 下载\n\n| Platform / 平台 | Download / 下载 | Checksum / 校验 |\n|---|---|---|\n'
  row "macOS 13+ (Apple silicon and Intel)" macos.zip " · [Mac App Store](https://apps.apple.com/app/id6835913837)"
  row "Windows 10/11 x64 (preview / 预览)" windows-x64.zip
  row "Linux x86_64, AppImage (preview / 预览; glibc 2.35+)" linux-x86_64.AppImage
  row "Linux x86_64, tar.gz (preview / 预览; glibc 2.35+)" linux-x86_64.tar.gz
  printf '\n[Installation and upgrading from a release candidate / 安装与候选版升级说明](https://wesleyel.github.io/flowie/download.html) · [All versions / 全部版本](https://wesleyel.github.io/flowie/download.html#versions)\n'
fi
