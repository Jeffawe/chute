# Chute

Send and receive files across your Tailscale network, from a desktop app or
straight from your file manager.

Tailscale's file transfer works well on macOS and Windows, where the official
client quietly saves incoming files for you. On Linux there is no receiver at
all — files sit in the Tailscale inbox until you remember to run
`tailscale file get`. Chute fixes that, and makes sending a right-click instead
of a command.

## What it does

- **Receives automatically.** A supervised background receiver drops arrivals
  into a folder you choose, with a desktop notification. Click a received file
  to reveal it in your file manager.
- **Sends by right-click.** A *Send with Chute* submenu appears in Nautilus and
  Dolphin with one entry per online device, so sending is two clicks and no
  window. The entries regenerate as devices come and go.
- **Sends from a window.** Pick a device, drag files in or browse for them.
- **Stays out of the way.** Lives in the tray, starts on login if you want.

## Requirements

Tailscale must be installed and running on the host, Chute is a front end for
it, not a replacement. Everything else ships with the app.

## Install

**Auto-updating (recommended)**

```sh
flatpak remote-add --user chute https://jeffawe.github.io/chute/chute.flatpakrepo
flatpak install --user chute io.github.jeffawe.Chute
```

`flatpak update` picks up new versions from then on.

**One-off**

Download the bundle from [Releases](https://github.com/Jeffawe/chute/releases/latest)
and install it:

```sh
flatpak install --user io.github.jeffawe.Chute.flatpak
```

No updates with this route — reinstall a newer bundle to upgrade.

### Steam Deck

Nothing special: it is an ordinary Linux machine and the commands above work
unchanged. Use Desktop Mode, and make sure Tailscale is already running on the
Deck first. Everything installs under `$HOME`, so SteamOS updates leave it
alone.

`packaging/install.sh` wraps the bundle route with preflight checks that
Tailscale is reachable from inside the sandbox, on any distro.

## Building from source

Needs `flatpak` and `flatpak-builder` (or `org.flatpak.Builder` from Flathub):

```sh
./packaging/build.sh
```

This produces `packaging/io.github.jeffawe.Chute.flatpak` and an ostree repo in
`packaging/repo/`. Dependencies are fetched during the build.

For development outside Flatpak you need Rust, Node, and the Tauri system
libraries, then:

```sh
npm install
npm run tauri dev
```

Note that frontend changes only appear via `tauri dev`, a bare `cargo build`
loads the dev server URL rather than the embedded assets.

## How it works

Chute shells out to the `tailscale` CLI rather than reimplementing anything:

| Need | Command |
| --- | --- |
| List devices | `tailscale file cp --targets` |
| Send | `tailscale file cp <files> <device>:` |
| Receive | `tailscale file get --loop --conflict=rename <dir>` |

Every invocation lives in `src-tauri/src/tailscale.rs`, so a change to the
CLI's output format is a one-file fix.

The Flatpak bundles its own copy of the statically linked `tailscale` binary,
because the sandbox cannot see the host's `/usr/bin`. It talks to the host's
`tailscaled` over `/run/tailscale/tailscaled.sock` — there is only ever one
daemon and one inbox, and the bundled CLI is just another client of it.

## Licence

MIT. Bundles the Tailscale CLI, which is BSD-3-Clause.

*Not affiliated with or endorsed by Tailscale Inc. "Tailscale" and "Taildrop"
are trademarks of Tailscale Inc., used here only to describe compatibility.*
