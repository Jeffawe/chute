#!/usr/bin/env bash
# Opt the CURRENT user in to receiving copies of shared Taildrop arrivals.
# No root. Undo with ./uninstall-user.sh
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SHARED_DIR="${CHUTE_SHARED_DIR:-/home/taildrop}"
USER_DIR="${CHUTE_USER_DIR:-$HOME/Downloads}"
BIN="$HOME/.local/bin"
UNITS="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"

[[ $EUID -ne 0 ]] || { echo "error: run as your own user, not root" >&2; exit 1; }

if [[ ! -r "$SHARED_DIR" ]]; then
  echo "error: cannot read $SHARED_DIR" >&2
  echo "  run setup-shared-drop.sh first, and log out and back in so the" >&2
  echo "  new group membership takes effect." >&2
  exit 1
fi

mkdir -p "$BIN" "$UNITS" "$USER_DIR"
install -m755 "$HERE/chute-fanout" "$BIN/chute-fanout"
install -m644 "$HERE/chute-fanout.service" "$UNITS/"
install -m644 "$HERE/chute-fanout.timer" "$UNITS/"
# The path unit cannot read variables, so the location is written in.
sed "s|^PathChanged=.*|PathChanged=$SHARED_DIR|" "$HERE/chute-fanout.path" \
  > "$UNITS/chute-fanout.path"

cat > "${XDG_CONFIG_HOME:-$HOME/.config}/chute-fanout.conf" <<CONF
# Where the single receiver drops files.
CHUTE_SHARED_DIR=$SHARED_DIR
# Where this user wants their own copy.
CHUTE_USER_DIR=$USER_DIR
CONF

systemctl --user daemon-reload
systemctl --user enable --now chute-fanout.path chute-fanout.timer

echo
echo "$USER: $SHARED_DIR -> $USER_DIR"
systemctl --user --no-pager --plain list-units 'chute-fanout*' 2>/dev/null | head -5
cat <<MSG

Turn it off for this user at any time:
    systemctl --user disable --now chute-fanout.path chute-fanout.timer
And back on:
    systemctl --user enable --now chute-fanout.path chute-fanout.timer

If this account is not always logged in, keep its services running with:
    sudo loginctl enable-linger $USER
MSG
