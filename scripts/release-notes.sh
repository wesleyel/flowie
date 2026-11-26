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
  cat <<EOF

## Downloads / 下载

| Platform | Package |
|---|---|
| macOS 13+ (Apple silicon and Intel) | \`Flowie-$version-macos.zip\` or the signed [Mac App Store](https://apps.apple.com/app/id6835913837) build |
| Windows 10/11 x64 (preview) | \`Flowie-$version-windows-x64.zip\` |
| Linux x86_64 (preview; glibc 2.35+) | \`Flowie-$version-linux-x86_64.AppImage\` or \`Flowie-$version-linux-x86_64.tar.gz\` |

[Installation, checksums and upgrading from a release candidate / 安装、校验和候选版升级说明](https://wesleyel.github.io/flowie/download.html)
EOF
fi
