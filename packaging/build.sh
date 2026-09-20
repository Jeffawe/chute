#!/usr/bin/env bash
# Build the Flatpak. Produces a local repo and a single-file bundle that can
# be copied to another machine (the Steam Deck) and installed offline.
set -euo pipefail

APP_ID="io.github.jeffawe.Chute"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUILD="$HERE/build"
REPO="$HERE/repo"

# Flathub's shared build definitions (libappindicator and its dependencies,
# which the GNOME runtime does not provide).
if [[ ! -d "$HERE/shared-modules" ]]; then
  echo "==> fetching shared-modules"
  git clone --depth 1 https://github.com/flathub/shared-modules.git "$HERE/shared-modules"
fi

flatpak run org.flatpak.Builder \
  --force-clean \
  --user \
  --install-deps-from=flathub \
  --repo="$REPO" \
  "$BUILD" \
  "$HERE/$APP_ID.yml"

# --runtime-repo embeds where the GNOME runtime can be fetched from. Without
# it, installing on a machine that lacks org.gnome.Platform//49 fails with
# "No remote refs found" instead of just downloading it.
flatpak build-bundle \
  --runtime-repo=https://flathub.org/repo/flathub.flatpakrepo \
  "$REPO" "$HERE/$APP_ID.flatpak" "$APP_ID"
echo
echo "bundle: $HERE/$APP_ID.flatpak"
echo "install locally: flatpak install --user -y $HERE/$APP_ID.flatpak"
