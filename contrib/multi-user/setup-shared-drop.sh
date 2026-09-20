#!/usr/bin/env bash
# One-time root setup for sharing received files between local users.
#
# There is exactly one Taildrop inbox per machine, so exactly one user may run
# a receiver. This creates a group-shared folder for that receiver to drop
# into, which other users can then read. Run once, with sudo.
#
#   sudo ./setup-shared-drop.sh jefferyawagu serveruser
set -euo pipefail

SHARED_DIR="${CHUTE_SHARED_DIR:-/home/taildrop}"
GROUP="${CHUTE_GROUP:-taildrop}"

[[ $EUID -eq 0 ]] || { echo "error: run with sudo" >&2; exit 1; }
[[ $# -ge 1 ]] || { echo "usage: $0 <user> [user...]" >&2; exit 1; }

echo "==> group '$GROUP'"
groupadd -f "$GROUP"

for u in "$@"; do
  id "$u" >/dev/null 2>&1 || { echo "error: no such user: $u" >&2; exit 1; }
  usermod -aG "$GROUP" "$u"
  echo "    added $u"
done

echo "==> $SHARED_DIR"
mkdir -p "$SHARED_DIR"
chgrp "$GROUP" "$SHARED_DIR"
# setgid: files created here inherit the group instead of the creator's.
chmod 2775 "$SHARED_DIR"

# The receiver runs with a normal umask, which would leave files group-readable
# but not group-writable. A default ACL fixes that for everything created here,
# so other members can move or delete files too.
setfacl -d -m g::rwx "$SHARED_DIR"
setfacl -m g::rwx "$SHARED_DIR"

echo
echo "Done. $SHARED_DIR is group-writable by: $*"
getfacl -p "$SHARED_DIR" 2>/dev/null | grep -E "^(owner|group|default)" || true
cat <<MSG

Next:
  1. Group membership only applies to NEW logins. Log out and back in, or
     run 'newgrp $GROUP' in an existing shell.
  2. In Chute, on the ONE account that receives (and only that one):
       Settings -> Save received files to -> $SHARED_DIR
       Settings -> Receive files automatically -> ON
  3. On every OTHER account:
       Settings -> Receive files automatically -> OFF
  4. Optionally, for each user who wants their own copy of arrivals:
       ./install-user.sh
MSG
