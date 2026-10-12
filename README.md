<p align="center">
  <img src="app/icon-src/icon.png" alt="Hoardini's sumo mascot" width="160">
</p>

<h1 align="center">Hoardini</h1>

<p align="center"><b>The world's smartest trashcan.</b></p>

Your drive is full and you don't know why. Hoardini is an AI agent for Windows that works out what every big folder and file on your disk actually is, then proposes the smartest way to get the space back: **delete** it, **compress** it, or **move it somewhere else**. You see the exact paths and sizes, you click, and only then does anything happen.

Ask it "what is this 40 GB folder?" and it answers from evidence: a catalog of known locations, file headers, the installed-programs registry and open-handle checks come first, the LLM explains, and the web is the last resort. Tell it once that a location is fine to clear and it remembers.

- **It proposes. You decide.** Nothing is deleted, archived or compressed by the agent itself. Every action is a proposal card, and rules in code can veto any of them.
- **The right fix for each item.** Caches and leftovers get deleted. Read-only data gets compressed, but only where it pays: weights, video, images and archives are skipped. Things you want to keep go to another drive or the cloud, copied and verified before the source is removed.
- **No Recycle Bin.** Delete is permanent, because the bin frees nothing until it is emptied.
- **It learns.** Your decisions about a location or file type are stored as path-keyed rows in SQLite and applied next time.
- **It stays out of the way.** A persistent index of the whole drive, refreshed from the NTFS change journal in an Efficiency-mode process.

## Build and run

Requires Windows, Rust 1.92+ and [Bun](https://bun.sh).

```powershell
cd ui; bun run build; cd ..
cargo run -p hoardini-app
```

`cargo build` builds the default members (`win`, `core`, `agent`, `server`, `cli`) without the desktop shell.

### CLI

```powershell
cargo run -p hoardini -- top          # largest folders or files in the stored index
cargo run -p hoardini -- changes      # what grew, shrank, appeared or vanished
cargo run -p hoardini -- serve        # UI and API on loopback (default port 7043)
```

The index lives in `%LOCALAPPDATA%\Hoardini`; set `HOARDINI_DATA_DIR` to move it.

### UI against the mock server

```powershell
cd ui; bun install; bun run mock
```

## Layout

| Path | What |
|---|---|
| `crates/win` | Windows-specific code |
| `crates/core` | Index, evidence, actions, memory |
| `crates/agent` | Chat agent, proposals, policy |
| `crates/server` | Loopback HTTP API |
| `crates/cli` | `hoardini` command |
| `app` | Tauri desktop shell |
| `ui` | React front end |
| `catalog` | Known-locations catalog (TOML) |
| `specs` | Requirements and their tests, one folder per feature |

Behavior is defined in `specs/<feature>/spec.md`; each requirement has an ID and says whether it is checked by a test or by a manual procedure. Run the spec tests with `cargo test -p hoardini-specs` and the vitest tests with `bun run test` in `specs/`.
