#!/usr/bin/env bash
set -euo pipefail

packages=(pkg-config libfontconfig-dev libfreetype-dev libwayland-dev
  libxkbcommon-dev libxkbcommon-x11-dev libx11-xcb-dev)
if [[ ${1:-} == package ]]; then
  packages+=(imagemagick desktop-file-utils shared-mime-info libxml2-utils)
fi
sudo apt-get update
sudo apt-get install -y --no-install-recommends "${packages[@]}"
