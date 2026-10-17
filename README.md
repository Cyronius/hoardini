<p align="center">
  <img src="app/icon-src/icon.png" alt="Hoardini's sumo mascot" width="160">
</p>

<h1 align="center">Hoardini</h1>

<p align="center"><b>The world's smartest trashcan.</b></p>

Your drive is full and you don't know why. Hoardini works out what every big folder and file on your disk actually is, and shows it as one short screen:

- **Junk you won't miss**: temp files, caches, Windows leftovers and the Recycle Bin. One press clears it.
- **Free space, keep the files**: compress folders in place, or keep cloud files online only. One press frees it, and nothing is lost.
- **Your call**: the big things only you can judge, biggest first. Open one and Hoardini says what it is and why he thinks so; Delete, Shrink, Move or Keep it there, or ask him about it.

Answers come from evidence first: a catalog of known locations, file types, the installed-programs registry and open-handle checks, then the AI to explain, and the web last.

- **The answer is the action.** Pressing Delete deletes, after you hold the button. Nothing happens until you press, and rules in code can still hold any path back.
- **Your word wins.** Hoardini's guesses never outrank you; only what you said to keep is held back.
- **No Recycle Bin.** Delete is permanent, because the bin frees nothing until it is emptied.
- **It remembers quietly.** Things you keep stop showing up, with one link to bring them back.
- **It stays out of the way.** A persistent index of the whole drive, refreshed from the NTFS change journal in an Efficiency-mode process.

## Choose a brain

Hoardini needs a model. The first-run screen sets one up:

- **On this PC (recommended).** Granite 4.1 3B downloads once and runs on the CPU. Nothing leaves your PC and it works offline. Qwen3.5 9B (smarter, needs a graphics card) and any `.gguf` file you already have are there too, and so are LM Studio, Ollama, llama.cpp and vLLM.
- **A free online brain.** Built-in setup for six free providers, each with its key page: Google Gemini, OpenRouter, Ollama Cloud, Mistral, Cloudflare Workers AI and Z.ai. The setup screen says, in plain words, how much each allows, whether a card is needed, and whether it may learn from your questions.
- **Your own provider.** OpenAI, Anthropic, Groq, Together and Cerebras are presets; any OpenAI-compatible endpoint works with a base URL and a key.

Online choices send your questions, and the file and folder names Hoardini looks up to answer them, to that provider.

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
| `crates/agent` | The AI agent: answers about one item, its tools, and providers |
| `crates/server` | Loopback HTTP API |
| `crates/cli` | `hoardini` command |
| `app` | Tauri desktop shell |
| `ui` | React front end |
| `catalog` | Known-locations catalog (TOML) |
| `specs` | Requirements and their tests, one folder per feature |

Behavior is defined in `specs/<feature>/spec.md`; each requirement has an ID and says whether it is checked by a test or by a manual procedure. Run the spec tests with `cargo test -p hoardini-specs` and the vitest tests with `bun run test` in `specs/`.
