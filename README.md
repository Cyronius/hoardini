# Hoardini

**The world's smartest trashcan.**

A Windows disk agent. It keeps an index of your drive, explains what the big files are, remembers what it learned, and frees space only after you approve.

- **The agent proposes; you approve.** No tool deletes, archives or compresses anything on its own. Destructive actions come back as a proposal card with exact paths and sizes, and only your click runs it. Rules in code can veto any proposal.
- **Local evidence first, LLM second, web last.** A known-locations catalog, file headers, the installed-programs registry and open-handle checks answer most "what is this?" questions. The LLM explains and recommends.
- **No Recycle Bin.** Delete is permanent. Archive copies to a destination, verifies, then deletes the source.
- **Compression only where it pays.** Weights, video, images and archives are skipped; the rest is sampled before anything is offered.
- **It remembers.** What you decide about a location or file type is stored as path-keyed rows in SQLite and applied next time.
- **A persistent index, refreshed politely.** The indexer runs in a separate Efficiency-mode process and patches the index from the NTFS change journal when it can.

## Build and run

Requires Windows, Rust 1.92+ and [Bun](https://bun.sh).

```powershell
cd ui; bun run build; cd ..
cargo run -p hoardini-app
```

The same thing is in [start.ps1](start.ps1) if you have it locally. `cargo build` builds the default members (`win`, `core`, `agent`, `server`, `cli`) without the desktop shell.

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
