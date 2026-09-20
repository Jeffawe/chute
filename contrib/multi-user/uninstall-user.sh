#!/usr/bin/env bash
# Stop copying shared arrivals for the current user. Leaves files alone.
set -euo pipefail
UNITS="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
systemctl --user disable --now chute-fanout.path chute-fanout.timer 2>/dev/null || true
rm -f "$UNITS"/chute-fanout.{path,service,timer} "$HOME/.local/bin/chute-fanout"
systemctl --user daemon-reload
echo "removed for $USER (files and state left in place)"
