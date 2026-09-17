# valheim-mods-tui

A terminal browser for Valheim mods on [Thunderstore](https://thunderstore.io/c/valheim/),
built with [ratatui](https://ratatui.rs). Browse the mods that are current for
Valheim 1.0, walk their dependency trees, select what you want, and install it
straight into a server's BepInEx directory.

```
 Valheim 1.0 Mods  1727 mods  ·  filter 1.0 ready  ·  sort downloads  ·  2 selected
┌ mods ───────────────────────────────────┐┌ details ──────────────────────┐
│ [ ]   ▸ Jotunn 2.30.0  1.2M ↓  tagged   ││ EpicLoot                      │
│ [x]   ▾ EpicLoot 0.11.4  890k ↓  updated││ by RandyKnapp  ·  v0.11.4     │
│ [x]   ├─ Jotunn 2.30.0                  ││                               │
│ [ ]   │  └─ BepInExPack_Valheim 5.4.2350││ Adds a loot system to Valheim │
│ [ ]   └─ BepInExPack_Valheim 5.4.2350   ││                               │
│ [ ]   ▸ PlantEverything 1.21.2  760k ↓  ││ updated     2026-09-11        │
└─────────────────────────────────────────┘└───────────────────────────────┘
 status  2 selected
 j/k move  h/l collapse/expand  space select  enter install  / search  ? help  q quit
```

## Install on a VPS

Needs a Rust toolchain (`curl https://sh.rustup.rs -sSf | sh`).

```sh
git clone <this repo> && cd valheim-mods-tui
cargo install --path .
```

That puts `valheim-mods-tui` in `~/.cargo/bin`. Point it at your Valheim server
once and the path is remembered:

```sh
valheim-mods-tui --install-dir /opt/valheim/server
```

The path can be the server root, its `BepInEx` directory, or the `plugins`
directory — all three are understood. `--print-config` shows what got resolved.

## Keys

| key | action |
| --- | --- |
| `j` `k` `↓` `↑` | move through the list |
| `l` `→` `Tab` | expand a mod's dependency tree |
| `h` `←` | collapse, or jump up to the parent mod |
| `g` `G` `Home` `End` | jump to top / bottom |
| `Ctrl-d` `Ctrl-u` | half-page down / up |
| `Space` | select or deselect a mod |
| `c` | clear the selection |
| `Enter` | install the selection and its dependencies |
| `/` | search name, author and description |
| `f` | cycle the 1.0 compatibility filter |
| `s` | sort by downloads, rating, updated or name |
| `r` | re-fetch the catalogue |
| `?` | key reference |
| `q` `Esc` | quit |

With nothing selected, `Enter` installs the mod under the cursor.

## What "supports Valheim 1.0" means here

Valheim left early access as 1.0 on **9 September 2026**, alongside the Deep
North update. Thunderstore has **no authoritative compatibility field** — nothing
in its API states that a mod has been verified against a game build. So this tool
uses the two positive signals that do exist, and shows which one matched in the
`1.0 signal` column:

- **tagged** — the author applied the `Deep North Update` category.
- **updated** — the mod has a release dated on or after the 1.0 launch.

The default `1.0 ready` filter keeps a mod when it is **not deprecated** and
carries **either** signal. That is deliberately the broader reading: at the time
of writing roughly 940 mods are tagged while about 1,750 have shipped a release
since launch, and those sets overlap only partially — many actively maintained
mods simply have not been re-tagged yet. Press `f` to narrow to tagged-only or
updated-only, or to drop the filter entirely.

Neither signal is a guarantee. Nothing stops an author from tagging a mod and
never testing it. Treat the column as evidence, not a promise.

## How installing works

Selecting a mod and pressing `Enter` resolves the full transitive dependency set,
orders it so dependencies install before the mods that need them, and shows a
confirmation listing exactly what will be written and where. Nothing touches disk
until that is accepted.

Archives are unpacked following Thunderstore's layout conventions:

| in the archive | goes to |
| --- | --- |
| `BepInExPack_Valheim/…` | the server root (this is the loader itself) |
| `plugins/…`, loose `.dll` files | `BepInEx/plugins/<Owner-Mod>/` |
| `patchers/…` | `BepInEx/patchers/<Owner-Mod>/` |
| `monomod/…` | `BepInEx/monomod/<Owner-Mod>/` |
| `core/…`, `config/…` | `BepInEx/core/`, `BepInEx/config/` |

Each mod gets its own folder under `plugins/` so installs stay separable. Shell
scripts are made executable on extraction — BepInEx ships its launch scripts as
non-executable, so they would otherwise land unrunnable.
**Existing config files are never overwritten** — a reinstall leaves your tuning
alone and reports how many files it kept. Restart the server to load new mods.

Dependencies always install at their **latest** published version, not the exact
version pinned in a mod's manifest — Thunderstore dependency strings are minimum
versions, and this matches how the established mod managers resolve them. The
confirmation lists the exact versions that will be fetched.

Dependencies that are not published on Thunderstore cannot be fetched; they are
shown in the tree marked `not on Thunderstore` and listed in the confirmation so
you know to handle them yourself.

## Making the server actually load mods

Installing mods is not enough on its own. BepInEx hooks the game through
Unity Doorstop, which means the server has to be **launched with the Doorstop
environment set** — the pack drops a `start_server_bepinex.sh` into the server
root that does this:

```sh
export DOORSTOP_ENABLED=1
export DOORSTOP_TARGET_ASSEMBLY=./BepInEx/core/BepInEx.Preloader.dll
export LD_LIBRARY_PATH="./doorstop_libs:$LD_LIBRARY_PATH"
export LD_PRELOAD="libdoorstop_x64.so:$LD_PRELOAD"
```

If your systemd unit or launch script starts `valheim_server.x86_64` directly,
or uses the stock `start_server.sh`, BepInEx never loads and none of your mods
take effect — with no error to tell you so. Point your unit at
`start_server_bepinex.sh` (copy it first; Steam overwrites the original on
update) or export those four variables in your own script.

Restart the server after installing. Mods are only read at startup.

## Notes

- The catalogue is ~11,000 packages. It is fetched gzipped (~13 MB on the wire),
  reduced to each mod's latest version, and cached for 6 hours under
  `~/.cache/valheim-mods-tui/`. `r` forces a refresh.
- Fetching and installing run off the UI thread, so the interface stays
  responsive while downloads are in flight.
- Dependency cycles and self-referential chains are detected and marked rather
  than followed.

## Tests

```sh
cargo test                      # unit, tree and UI tests
cargo test -- --ignored         # live tests against Thunderstore (slower)
```

The live tests install the real BepInEx pack and a representative mod of each
archive shape into a temporary directory and assert the resulting layout.
