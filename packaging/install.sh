#!/usr/bin/env bash
# Install Chute from a bundle produced by build.sh.
#
# Works on any distro with flatpak. Everything lands in $HOME via --user, so
# nothing needs root and an immutable root filesystem (SteamOS, Silverblue)
# is not a problem.
set -euo pipefail

APP_ID="io.github.jeffawe.Chute"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUNDLE="${1:-$HERE/$APP_ID.flatpak}"

fail() { echo "error: $*" >&2; exit 1; }
warn() { echo "warning: $*" >&2; }

[[ -f "$BUNDLE" ]] || fail "bundle not found: $BUNDLE (run build.sh first)"
command -v flatpak >/dev/null || fail "flatpak is not installed"

# The app is only a client: the daemon has to already be running on the host.
if ! command -v tailscale >/dev/null && [[ ! -S /run/tailscale/tailscaled.sock ]]; then
  fail "Tailscale does not appear to be installed on this machine"
fi
if [[ ! -S /run/tailscale/tailscaled.sock ]]; then
  fail "/run/tailscale/tailscaled.sock is missing - is tailscaled running?"
fi
if ! tailscale status >/dev/null 2>&1; then
  warn "tailscale status failed - you may need to run 'tailscale up' first"
fi

# SteamOS has flathub as a system remote, but --user installs resolve against
# user remotes. Adding it here avoids "No remote refs found" when the runtime
# has to be downloaded.
if ! flatpak remotes --user --columns=name 2>/dev/null | grep -qx flathub; then
  echo "==> adding flathub as a user remote"
  flatpak remote-add --user --if-not-exists flathub \
    https://dl.flathub.org/repo/flathub.flatpakrepo
fi

echo "==> installing $APP_ID (may download ~1.5GB of GNOME runtime on first run)"
flatpak install --user -y "$BUNDLE"

# Confirm the sandbox can actually reach the daemon: this is the one
# permission the app cannot work without.
echo "==> verifying sandbox can reach tailscaled"
if flatpak run --command=tailscale "$APP_ID" file cp --targets >/dev/null 2>&1; then
  echo "    ok - devices are reachable from inside the sandbox"
else
  warn "the sandboxed CLI could not reach tailscaled."
  warn "check: flatpak info --show-permissions $APP_ID | grep tailscale"
fi

cat <<MSG

Installed. Launch it from your applications menu, or:

    flatpak run $APP_ID

In Settings, set where received files should go and turn on
"Start automatically on login" so the receiver runs without the window.

Right-click entries are written on first run, once the device list
loads. You may need to restart your file manager to see them.
MSG
