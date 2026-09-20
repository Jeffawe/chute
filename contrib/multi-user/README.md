# Sharing received files between local users

A machine has exactly **one** Taildrop inbox. It belongs to `tailscaled`, not
to any user, and incoming files carry no indication of which local account
they are meant for. So exactly one user may run a receiver: two receivers race
for the same queue and each file lands wherever the winner happens to put it.

Sending has no such limit. Any local user can send at any time, because
`tailscale file cp` is just a client call to the daemon.

These scripts give you the practical equivalent of "everyone receives":

```
Taildrop inbox
      │
      ▼  one receiver only (Chute, on one account)
/home/taildrop                       group-writable shared folder
      │
      ├──▶ ~jeff/Downloads           each opted-in user pulls their own copy
      └──▶ ~serveruser/Downloads
```

Nothing here modifies Chute, and nothing runs as root after setup.

## Setup

**1. Shared folder, once, as root**

```sh
sudo ./setup-shared-drop.sh jefferyawagu serveruser
```

Creates a `taildrop` group, adds those users, and makes `/home/taildrop`
group-writable with a default ACL so files created by the receiver stay
writable by everyone else. Log out and back in afterwards — group membership
only applies to new logins.

**2. Point the one receiver at it**

In Chute, on the account that receives:

- *Save received files to* → `/home/taildrop`
- *Receive files automatically* → **on**

On every other account, *Receive files automatically* → **off**. This is the
part that matters; leaving it on in two places is the race described above.

**3. Opt each user in to their own copies**

Run as each user who wants one (no root):

```sh
./install-user.sh
```

Toggle per user at any time:

```sh
systemctl --user disable --now chute-fanout.path chute-fanout.timer   # off
systemctl --user enable  --now chute-fanout.path chute-fanout.timer   # on
```

## How the copying behaves

- Triggered by a systemd **path unit** watching the shared folder, so copies
  appear within seconds, plus a 5-minute **timer** as a backstop.
- A file whose size is still changing is left alone and picked up on the next
  pass, so a partially written file is never copied.
- Copies use `cp --reflink=auto`. On btrfs (where `/home` usually lives) that
  is instant and costs no extra disk, however many users are opted in.
- Each copy is recorded, so **deleting your copy is respected** — it will not
  reappear. Clear `~/.local/state/chute-fanout/seen` to re-copy everything.
- Nothing is ever removed from the shared folder. Prune it yourself when it
  gets big.

## Configuration

`~/.config/chute-fanout.conf`, written by `install-user.sh`:

```sh
CHUTE_SHARED_DIR=/home/taildrop
CHUTE_USER_DIR=/home/you/Downloads
```

Changing `CHUTE_SHARED_DIR` also means editing `PathChanged=` in
`~/.config/systemd/user/chute-fanout.path`, because a path unit cannot read
variables. Then `systemctl --user daemon-reload`.

## Removing it

```sh
./uninstall-user.sh              # per user; leaves files alone
sudo gpasswd -d <user> taildrop  # revoke shared folder access
```
