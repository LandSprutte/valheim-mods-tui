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

## Updating

The binary can replace itself, which matters on a VPS where there is no package
manager or Rust toolchain involved:

```sh
valheim-mods-tui --update
```

Tell it once where builds come from, and it remembers:

```sh
# GitHub releases — picks the asset matching this machine's target triple
valheim-mods-tui --update-source landsprutte/valheim-mods-tui

# or any static host serving a binary for this platform
valheim-mods-tui --update-source https://files.example/valheim-mods-tui
```

`VALHEIM_MODS_TUI_UPDATE_SOURCE` overrides it for one run, and `--force`
reinstalls even when the version already matches. `--version` prints the build
and its target triple.

Updating is deliberately cautious. The download is rejected unless it is a
native executable for this platform, so a rate-limit page or a 404 body can
never land on top of a working binary. The replacement is then staged beside the
target, **run once with `--version` to prove it works**, and only then renamed
into place — an atomic swap, and the only way to overwrite a running executable
on Linux. If any step fails, the staged file is removed and the current binary
is untouched. Installing into a system directory needs the usual privileges;
the error says so rather than failing obscurely.

If you publish via GitHub, `.github/workflows/release.yml` builds all three
targets and uploads them under the names `--update` looks for. Push a tag to
trigger it:

```sh
git tag v0.2.0 && git push origin v0.2.0
```

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
| `d` | *(in the confirm dialog)* install into a different folder |
| `e` | export the selection as a `MODS=` list |
| `w` | *(in the export dialog)* write `MODS` into your compose or `.env` file |
| `i` | show only mods already installed here |
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

The destination defaults to the configured BepInEx directory, and the confirm
dialog shows exactly where files will land. Press **`d`** there to install
somewhere else — a server root, a `BepInEx` directory, or a `plugins` directory
are all understood, and the choice is saved for next time. An unusable path is
reported and the previous one kept.

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

## Seeing what is already installed

When an install directory is configured, the TUI reads it on start and marks
each mod's state:

| mark | meaning |
| --- | --- |
| `●` | installed, at the latest published version |
| `▲` | installed at a different version — the row shows `have <version>` |
| *(blank)* | not installed here |

The details pane names the version and which BepInEx directory it sits in, and
`i` narrows the list to just what is installed. That view deliberately ignores
the 1.0 filter: a mod on disk is worth seeing whether or not it signals 1.0
support — libraries pulled in as dependencies often predate the release.

Installed state is detected from the per-mod folders under `BepInEx/plugins`,
`patchers` and `monomod`, with the version read from each `manifest.json`;
BepInEx itself is recognised by its core assembly. Mods placed by another
manager may use different folder names and will not be recognised. The list
refreshes itself after an install, so the marks are never stale.

## Docker servers that install mods themselves

Several Valheim server images do not want plugin files at all — they take a list
of Thunderstore mods and install them on start. Copying files into a host folder
does nothing for those, because the server's filesystem is inside the container.

Press **`e`** to export your selection as Thunderstore dependency strings. It
resolves dependencies the same way installing does, writes `valheim-mods.env`,
and shows the list on screen:

```
BEPINEXPACK_VERSION=5.4.2350
MODS=ValheimModding-JsonDotNET-13.0.4,ValheimModding-Jotunn-2.30.0,RandyKnapp-EpicLoot-0.14.7
```

Press **`w`** and it writes those straight into your `docker-compose.yml` or
`.env`. A `docker-compose.yml`, `compose.yml` or `.env` in the working directory
is found automatically; `--compose <file>` points at one elsewhere, and
`--compose-service <name>` picks the service when the file has several.

The edit is line-targeted rather than a YAML rewrite, so comments, indentation,
quoting style and key order all survive — a typical result is a two-line diff:

```diff
       BEPINEX_ENABLED: "true"
+      MODS: "ValheimModding-JsonDotNET-13.0.4,RandyKnapp-EpicLoot-0.14.7"
+      BEPINEXPACK_VERSION: "5.4.2350"
```

Both `KEY: value` and `- KEY=value` styles are supported, and whichever the file
already uses is kept. An existing `MODS` is replaced rather than duplicated, and
the original is copied to `.bak` first. Nothing is written until you press `w`.
If the service has no `environment:` block the edit is refused rather than
guessed at — add one first.

Then recreate the container; a plain restart will not pick up changed
environment variables:

```sh
docker compose up -d --force-recreate
```

For `indifferentbroccoli/valheim-server-docker`, `MODS` also turns BepInEx on by
itself; `BEPINEX_ENABLED=true` is the explicit switch, and `BEPINEXPACK_VERSION`
pins the loader (its default trails the current release).

The loader is deliberately kept out of `MODS` and reported separately — those
images manage BepInEx through their own version setting, so listing it as a mod
invites a version clash.

Use `--install-dir` (writing real files) when the server reads a directory you
control; use `e` when the image manages mods for you.

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
