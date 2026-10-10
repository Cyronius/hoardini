# Plan: Hoardini opens on a review of what can go and what's taking space

## Outcome (2026-10-09, `feat/review-first`)
Landed: units and location rows (§1, §2), the review with its cards, Large areas, What is this? and Look closer (§4), the ▸ tree and Show in Explorer, copy checks with consent and the Online-only card (§3), Compress in place (§5), Apps you rarely use (§6), learning from jobs and web searches (§2d), chat cards and pills (§8), Explore (§9), and Cleanups only Windows can do (§7), plus the review's machine-wide `leftovers` operation. The requirements are listed under Spec impact.

Not landed:
- §4c: tree rows do not show a file's identity (a GGUF's model name and quant).
- §9: a type with no identity in By type does not run the web lookup itself; What is this? asks the chat.
- §3: no disk-image check against publishers' checksum lists, so an `.iso` stays "only copy we know of".
- §3: Ollama blobs are matched against the local manifests only (`claimed`); nothing asks the Ollama registry.

Hoardini measures a drive, works out what can be deleted, compressed or sent elsewhere, and then does it. The chat is for questions along the way. The app shall open on a **review**, which has two parts:

1. **Proposals:** cards of things Hoardini thinks can go, backed by the catalog, memories, web knowledge and checks such as "this file is on Hugging Face".
2. **Large areas:** big things the user should know about but probably can't delete, each with what it is and why it isn't proposed.

Every item resolves down to something you could actually delete, rather than its parent folder. Every item opens into a folder tree with sizes, and every row can be revealed in Explorer.

## Why
In both exchanges the tool data was right and the model's prose was wrong. Asked for hotspots, the model called no tool and reported a 127 GB *gain* that was really 130 GB freed. Asked what to delete in `C:\code`, it skipped the 161 GB folder and made no card. So the review is built from data with no model call, and the model handles questions and the cases the data can't settle.

The catalog already knows about 95 locations: temp folders, the Windows Update download cache, Delivery Optimization, crash dumps including `MEMORY.DMP` and `LiveKernelReports`, CBS and DISM logs, `Windows.old`, `$WINDOWS.~BT`, browser, shader and Electron caches, package caches, build output, model stores, game libraries and VM disks. As far as I can see it is only consulted when evidence is built for a path the model asks about. Nothing ever sweeps a drive for it, and the review is that sweep.

## 1. Resolving down to deletable items
- **Containers vs. items:** a large folder is usually a container (`~\.flm`, `C:\code`, `~\AppData\Local`). The review walks down from each large folder until it reaches **deletable units**, and containers appear only as group headings.
- **The rule, applied from the top:**
  - If a catalog entry matches the folder, the entry's new `unit` field decides what an item is:
    - `self`: the folder is one item (`target`, `node_modules`, `.venv`).
    - `children`: each child is one item. This covers model stores (one item per model folder), the HF hub (one per `models--*`), Steam `common` (one per game) and Downloads (one per file).
    - `contents`: one item for everything inside, with the folder kept and open files skipped (`%TEMP%`, `Windows\Temp`, WER). For temp entries, the item covers only children not modified for 24 h, so a running session's scratch folder stays. Your `~\AppData\Local\Temp\claude` holds 92 GB, mostly model copies in old Claude Code scratchpads, and this is the item that catches it.
  - With no catalog match, the folder is checked against the model-store detector below. If it isn't a model store, the review goes down into every child of at least 1 GiB, or at least 10% of the folder, and repeats.
  - It stops at a file, at a folder with no child that large, or at a known `keep` location.
- **Model-store detector:** a folder with no catalog match, where at least 80% of the bytes are model-weight types and the weights sit in per-model subfolders, is treated as a model store with `unit = children`. The weight extensions are a fixed list (`.gguf`, `.safetensors`, `.onnx`, `.pt`, `.ckpt`, `.bin` next to a `config.json`), extended by file-type knowledge (2b) such as `.q4nx`.
  - Example: `~\.flm` (104 GB) is a container. `~\.flm\models` is detected as a model store, and the items are `Qwen3.6-35B-A3B-NPU2`, `Qwen3.8-27B-NPU2` and so on, each with its own tick and its own copy check (3).
- **Catalog gaps to fill:**
  - `~\.flm` and `~\.hipfire`. The detector covers both, but entries give them names.
  - GPU driver unpack folders (`C:\NVIDIA`, `C:\AMD`, `C:\Intel`).
  - iPhone backups (`%APPDATA%\Apple Computer\MobileSync\Backup`, as `review`).
  - Adobe media caches.
  - Spotify and Discord caches.
  - The WinGet download cache.
  - The `unit` field on every existing entry.

## 2. What Hoardini remembers
Everything is stored as memory rows. They show in the Memory drawer, reach evidence and each item's "Noted: ..." line through `recall_path`, and can be edited or undone. A new nullable `memory.detail_json` column holds the structured part (v5 migration).

### 2a. Locations
- **Which folders count as large:** on each watched drive, a folder of at least 5 GiB or at least 1% of used space, where no single child holds 90% or more of it. At most 40 per drive.
- **One row per location:** kind `verdict`, scope `path`. The detail is `{category, verdict, actions, size_at, assessed_ms, by, cleaned:[...], kept:[...]}`.
  - The verdict uses the existing scale. `actions` lists what may still be proposed: your photos are `keep` with `[archive]`, OS files are `keep` with `[]`.
- **Classification, deterministic and in order:** the deny list, then the catalog, then the content mix, then `unknown`.
  - The content mix only sees files of 1 MiB or more (INDEX-SMALL-FOLD), so small-file bytes count as unknown share.
- **From finished jobs:** done items add to `cleaned`, which shows as a reason next time ("you cleaned this on 10-09"). Items left unticked on an approved card add to `kept`.
- **Effect on proposals (POLICY-LOCATION-KEEP):** `keep` blocks delete and dedupe at or under the location, with the row as the reason. Archive and compress are allowed when listed in `actions`.

### 2b. File types
- One glob row per type, e.g. `**\*.q4nx`. The detail is `{category, app, use}`. The text is one sentence, shown on every matching item.

### 2c. Web knowledge is knowledge, not instruction
- A row learned from the web records **what app or purpose a folder or file type belongs to**: "`.q4nx`: FastFlowLM NPU model weights", or "`~\.hipfire`: hipfire's model cache". It has no verdict and no actions, and nothing that would delete anything.
- Whether something can go is decided only by the catalog, the user, a finished job, or a verification check (3).
- Precedence for identity: user > catalog > web > model. A lower source never overwrites a higher one.

### 2d. Learning from web searches
- `web_search` gains an optional `about`, which is a path or a type such as `*.q4nx`.
- If a memory already covers `about`, the tool returns it instead of searching, unless the call passes `again: true`.
- After the run, each search with `about` that wasn't followed by a `remember` on the same key gets one small structured request to the model for `app` and `use` only. The answer is stored with source `web:<url>`, and a Learned chip shows it with the link and Undo.

### 2e. Limits
Location and type rows don't count toward the 200-note limit (MEM-LIMIT). A location row with no user or job content is dropped when the folder is no longer large.

## 3. Where else does this file exist? (copy checks)
"Re-downloadable" is a claim. A **copy check** proves it, for any file type. The Hugging Face case is one instance. Each check answers one question for one file or folder: where is another copy, and how sure are we?

- **One result shape for every check:** `{check, locator, confidence, checked_ms, detail}`. It is stored in the `copy_check` table (v5) keyed by volume, path and check, and reused until the file's id, size or mtime changes. SHA-256 results go in `file_hash`, keyed by volume and file id. Evidence and item notes read both tables.
- **Confidence decides the effect:**

| Confidence | Meaning | Effect |
|---|---|---|
| `identical` | a hash or content-addressed id matches, or a sync engine says it is in sync | may become `redownload` or `synced`, so it can start ticked |
| `same-name-and-size` | a likely match whose bytes are unchecked | `review`, with the locator in its note |
| `claimed` | a launcher or package manager says it can reinstall, with no byte check | follows the catalog verdict, as today |
| `none` | every check that applies came up empty | "only copy we know of": listed in Large areas, never in a delete card |

- **Which checks apply:** each check declares what it covers, by catalog category, type identity, location or file metadata. Knowledge rows (2b, 2c) may carry a **hint** naming a check, for example "`.q4nx`: FastFlowLM weights, published on Hugging Face". That tells the model-hub check where to look. It is a place to look, not a verdict.
- **The first checks:**

| Check | Covers | Finds the copy by | Verifies by | Online |
|---|---|---|---|---|
| Local duplicate | any file ≥ 100 MiB | existing duplicate groups | full hash (ACT-DEDUPE-VERIFY) | no |
| Hoardini archive | anything Hoardini archived | `archive_entry` | verified when archived | no |
| Cloud sync | files under OneDrive, Dropbox, Google Drive, or iCloud sync roots | the Windows Cloud Files sync-root registration | the provider's in-sync state | no |
| Git remote | files in a git work tree | `git ls-files` and the upstream branch | tracked, unmodified, and the branch is not ahead of its upstream (as of the last fetch) | no |
| Model hubs | model weights: Hugging Face first, Ollama next | path layout, file metadata (GGUF `general.source.*`, `config.json`), name search | the remote's `lfs.oid` or blob digest equals the file's SHA-256 | yes |
| Installers | `.exe`, `.msi`, `.msix` | PE version info and Authenticode publisher (`evidence/pe.rs`), then the winget manifest | the manifest's `InstallerSha256` | yes |
| Launchers and package managers | game libraries, package caches, toolchains | the catalog (Steam and Epic manifests, pip, npm and cargo caches) | none (`claimed`) | no |
| Disk images (later) | `.iso`, `.img` | the publisher's checksum list | hash | yes |

- **Synced files get "online-only", not delete:**
  - A cloud-synced file is never deleted, because deleting it removes it from the cloud too.
  - Instead it goes to an **Online-only** card. Approving the card evicts the local copy (Cloud Files dehydrate) and the file stays in the folder as a placeholder. This is the main win for most users' photos and videos.
  - This needs a new action kind.
- **Git:** a project folder becomes "on GitHub: everything tracked is pushed". Only the untracked and ignored parts, such as models or build output, need their own verdicts.
- **Adding a check:** a `CopyCheck` trait with `applies`, `find` and `verify`. Each check has its own requirement and a test against recorded responses; no test touches the live network.
- **Hashing:** SHA-256 runs in the background at Lean speed (INDEX-POLITE), biggest candidates first, cached by file id, size and mtime. Content-addressed stores (HF blobs, Ollama `sha256-*`) need no hashing.
- **Privacy:** online checks send file names, sizes and package or repo ids. They never send file contents or local hashes: the remote hash is fetched and compared on this machine. Offline checks always run. Online checks need consent (D2).

## 4. The review
- **When it runs:**
  - When the app opens, if the last review is more than 6 hours old or a refresh has finished since.
  - From the **Review now** button.
  - The build runs on the server with no model call and should finish in under a second. Copy checks and compression estimates fill in afterwards.
- **Header:** the drive, used and free space, and "Ready to free: X GB" (the sum of the ticked items). There is no "what changed since last scan" here; that stays in the `changes` tool and the history chart.

### 4a. Proposals (cards, biggest first, each approved separately)
| Card | Items | Starts ticked |
|---|---|---|
| **Temp and Windows leftovers** | `%TEMP%`, `Windows\Temp`, WER, crash dumps, CBS/DISM logs, WU download, Delivery Optimization, `$WINDOWS.~BT`, `Windows.old` | per POLICY-DEFAULT-TICK; `Windows.old` unticked |
| **Caches** | browser, shader, Electron, package caches | yes when `safe` |
| **Build output** | `target`, `node_modules`, `.venv`, `obj`, … resolved per project | yes when `safe`, `redownload` per default tick |
| **Re-downloadable, verified** | model files, installers and images whose copy check is `identical` (3), with the locator shown | yes |
| **Online-only** | cloud-synced files and folders, mostly cold photos and video; frees the local copy and keeps the file in the cloud (3) | yes when not used for 90 days |
| **Installers and archives in Downloads** | `.exe`, `.msi`, `.iso`, `.zip`, … | no |
| **Compress in place** | folders where at least 1 GiB and at least 20% would be saved (5) | no |
| **Apps you rarely use** | installed apps or games of at least 2 GiB not run for 90 days (6) | n/a: per-item **Uninstall…** |
| **Send elsewhere** | cold photos, video and documents of at least 5 GiB, not used for 180 days, when a destination exists | no |

- **What each item shows:** a one-line reason that cites its knowledge: the catalog note, a memory, web identity, a copy check ("identical to `org/repo` on Hugging Face", "in winget as Contoso.App 4.2"), or "you cleaned this on 10-09".
- **Exclusions:** an item already in a pending card or declined in the last 30 days is left out. A declined card saves no verdict, because a decline doesn't say why.
- **Each new review replaces the last:** untouched pending cards from the last review collapse to "Replaced by a newer review" (new status `superseded`). Cards the user touched stay.

### 4b. Large areas (no cards; one row each, biggest first)
- **What's listed:** big locations that are not proposed. Each row gives what it is, its size, and why it isn't proposed:
  - Windows and its parts: `WinSxS`, `pagefile.sys`, `hiberfil.sys`, `System Volume Information`.
  - `Program Files`.
  - Your photos and documents.
  - Projects with recent edits.
  - VM disks.
  - Only copy we know of: big files whose every copy check came up empty, such as your own model builds.
  - Unknown folders.
- **A hint where one exists:** "pagefile: Windows manages its size", "WinSxS: Windows can trim this itself (7)", "photos: add a destination to move them off C:".
- **Unknown folders** carry **What is this?** buttons: Keep · Disposable · Move elsewhere · Look closer. Look closer is the only place the review calls the model.

### 4c. Opening an item: the folder tree
- **Every item, Large-areas row and chat card row** has ▸. It opens an inline tree from `GET /api/index/children?path=&limit=50`.
  - Children are sorted by size. Each child shows a size bar, its percentage of the parent, file count and last modified date.
  - A "N small files · X MB" row comes from `small_count` and `small_bytes`.
  - Each row has ▸ to go deeper and "+N more" past 50.
  - Each row has **Show in Explorer**, which calls `POST /api/reveal` to run `explorer /select,<path>`. It is refused unless the path is under a watched volume and exists.
  - Files show their identity when evidence has it, such as a GGUF model's name and quant.

## 5. Compression proposals
- **Candidates:** folders of at least 2 GiB that are not already WOF- or NTFS-compressed and not mostly already-compressed content (media, archives, quantized weights). These are typically game installs, SDKs, app folders, logs and text datasets.
- **Estimate:** `compress::estimate` (ACT-COMPRESS-ESTIMATE) samples the largest files. It runs in the background for the top 10 candidates after the review renders.
- **Proposal:** in-place Windows compression (`lzx`). Files stay usable, and there's no archive to manage. An item is listed when the estimate is at least 1 GiB **and** at least 20%.
- **Exclusions:** folders modified in the last 30 days, because writes undo WOF compression.

## 6. Big apps you rarely use
- **Size:** installed programs come from the uninstall registry (already read by `evidence/owner.rs`), and their size comes from the index under `InstallLocation`.
- **Last run:** taken from UserAssist (already read by `evidence/usage.rs`), or Steam's `appmanifest_*.acf` `LastPlayed` for games.
- **Listed when:** at least 2 GiB, and not run for 90 days or never run.
- **Uninstall…** runs the app's registered `UninstallString` (`steam://uninstall/<appid>` for Steam), and the uninstaller asks for its own confirmation. Hoardini never deletes a `Program Files` folder itself.

## 7. Cleanups only Windows can do (D3)
Some big chunks of space can't be freed by deleting files. Deleting them breaks something, or Windows recreates them. The fix is to have Windows' own tool do it. Hoardini would propose these as cards like any other. Approving one shows the Windows admin prompt; Hoardini then runs the tool and reports the space freed.

| What | Why deleting files doesn't work | What Windows does instead | Typical size |
|---|---|---|---|
| `WinSxS` (component store) | Its files are hard-linked into Windows; deleting them breaks updates | Component cleanup removes superseded versions (the same as Disk Cleanup's "Windows Update Cleanup") | 2–10 GB |
| System Restore points | Stored in `System Volume Information`, which is closed to file-by-file deletes | Deletes old restore points, or caps the space they may use | 5–20 GB+ |
| `hiberfil.sys` | Recreated at boot, sized at about 40% of RAM | Turns hibernation off, or switches to the reduced size (which keeps Fast Startup) | ~25 GB with 64 GB RAM |
| WSL and Docker `.vhdx` | A virtual disk grows but never shrinks when files inside it are deleted | Compacts the disk while WSL is stopped | the unused space inside; you have 134.8 GB of `.vhdx` |
| Recycle Bin | Files deleted elsewhere stay on disk | Empties it through Windows | varies |

Restore points cannot be brought back once they are deleted, so those cards say so on their approve step.

## 8. Cards in the chat
- Tool results from `top_dirs`, `top_files` and `changes` carry a `data` payload. It is stored in `message.ui_json`, streamed as `tool_data`, and never sent to the model.
- These results render as cards with size bars, the location tag from 2a, ▸ trees and Show in Explorer. The changes card leads with the free-space delta.
- **Follow-up pills:** up to 3 under the settled last answer, from two sources.
  - **The model's recommendations:** a new tool, `suggest_next(items: [{label, kind, path?}])`, called at the end of an answer.
    - `kind` is `ask`, `clean_up`, `explore` or `what_is`. An `ask` pill sends its label as a message. The others run that UI action on `path`.
    - A pill whose path isn't in the index is dropped.
    - The pills are stored with the tool result, so a reload shows them.
    - The tool is in the small toolset.
  - **Deterministic pills** from the turn's card data fill any remaining slots, e.g. "What's inside covenant-agent (161 GB)?". Duplicates of the model's pills (same kind and path) are skipped.
  - Model pills come first. A pill never approves anything. At most it opens a review, opens Explore, or asks a question.
- The prompt's index summary tags each big folder with its category and verdict. The card rule makes the model call these tools rather than retell the numbers, and end with `suggest_next`.
- New eval questions check all three.

## 9. Explore view: tree, treemap, file types
A second home tab next to Review, laid out like WinDirStat but wired to what Hoardini knows.

- **Three linked panes:** the folder tree (the ▸ tree from 4c at full width), the treemap below it, and a **By type** list beside them. Selecting in any pane selects in the others. Clicking a type highlights all of its blocks.
- **Color by what Hoardini knows** (default):
  - **Green:** proposed, in a review card.
  - **Amber:** review.
  - **Grey:** keep or system.
  - **Blue:** your own data.
  - **Striped:** unknown.
  The map then shows at a glance what the review will free and what nobody has explained yet. A toggle switches to **color by type**, as WinDirStat does.
- **Labels:** nested blocks with folder names on blocks big enough to hold them, instead of cushion shading. In this app the question is usually "what is this block?", so names matter more than texture.
- **Selection bar:** the selected block shows its path, size, identity and verdict (from memories, the catalog or the HF check), plus these actions:
  - **Clean up**: the review scoped to this folder.
  - **What is this?**
  - **Ask Hoardini**: sends "What is {path}?" to the chat.
  - **Show in Explorer**.
  - **Zoom in**.
- **By type:** extension totals for files of 1 MiB or more (smaller ones show as "small files"). Each type has its learned identity ("Q4NX: FastFlowLM NPU weights"). A type with no identity has **What is this?**, which runs the web lookup (2d) and saves a type row.
- **Data:** `GET /api/index/map?path=&px=<width×height>` returns the subtree pruned to blocks of at least 9 px² at that canvas size, with smaller siblings merged into one "N more" block. A full C: then comes to tens of thousands of rectangles. The map is drawn on a canvas and refetched on zoom. `GET /api/index/types?path=` returns the type list.
- **AI, both ways:**
  - The model gets no treemap tool. A map is for the user's eyes, and the model can't read one. When the user asks to "show" something, the model calls `top_dirs` with an optional `view: "map"`. The folders card (8) then opens on a Map tab instead of List, with **Open in Explore**.
  - In the other direction, the system prompt carries "The user is looking at {path} in Explore", the same way it carries the selected drive. "What's this big red one?" then works.

## Spec impact
Compared with `main` at 94f4e02.

- **New requirements (60):**
  - REVIEW (new feature `specs/review/`): `REVIEW-UNITS`, `REVIEW-BUILD`, `REVIEW-AREAS`, `REVIEW-TRIGGER`, `REVIEW-SUPERSEDE` (test); `REVIEW-LOOK-CLOSER` (manual).
  - EVID: `EVID-GGUF-SOURCE`, `EVID-COPY-CHECK`, `EVID-COPY-CONSENT`, `EVID-COPY-DUP`, `EVID-COPY-ARCHIVE`, `EVID-COPY-SYNC`, `EVID-COPY-GIT`, `EVID-COPY-MODELHUB`, `EVID-COPY-WINGET`, `EVID-COPY-CLAIMED` (test); `EVID-COPY-BACKGROUND` (manual).
  - ACT: `ACT-COMPRESS-CANDIDATES`, `ACT-ONLINE-ONLY`, `ACT-APPS-RARELY-USED`, `ACT-WINDOWS-CLEANUP-DETECT`, `ACT-WINDOWS-CLEANUP-RUN` (test); `ACT-UNINSTALL`, `ACT-WINDOWS-CLEANUP` (manual).
  - MEM: `MEM-LOCATION`, `MEM-FILETYPE`, `MEM-SOURCE-RANK`, `MEM-WEB-IDENTITY`, `MEM-LEARN-JOB` (test).
  - POLICY: `POLICY-LOCATION-KEEP` (test).
  - INDEX: `INDEX-EXT-MIX` (test).
  - AGENT: `AGENT-PROPOSE-ONLINE-ONLY`, `AGENT-SYSTEM-CLEANUP`, `AGENT-WEB-LEARN`, `AGENT-TOOL-DATA`, `AGENT-SUGGEST-NEXT`, `AGENT-INSPECT-COPIES` (test).
  - SRV: `SRV-COPY-CHECKS`, `SRV-INDEX-CHILDREN`, `SRV-REVEAL`, `SRV-INDEX-MAP`, `SRV-INDEX-TYPES`, `SRV-EXPLORE-FOCUS`, `SRV-LEARNED`, `SRV-PROPOSAL-SELECTION`, `SRV-LOCATION-ANSWER`, `SRV-APPS` (test).
  - UI: `UI-LAYOUT-WIDTH`, `UI-REVIEW-ORDER`, `UI-TREEMAP-LAYOUT` (test); `UI-LAYOUT`, `UI-REVIEW`, `UI-SYSTEM-CLEANUP-CARD`, `UI-ONLINE-CONSENT`, `UI-ONLINE-ONLY-CARD`, `UI-TOOL-CARDS`, `UI-ITEM-TREE`, `UI-FOLLOW-UPS`, `UI-EXPLORE`, `UI-LEARNED-CHIP` (manual).
- **Modified requirements (30):** `ACT-FOLLOW-UP`, `ACT-COMPRESS-VERIFY`, `AGENT-NO-DIRECT`, `AGENT-REDACT`, `AGENT-SMALL-PROFILE`, `AGENT-WEB-SEARCH`, `AGENT-PROMPT-CONTEXT`, `AGENT-LIVE-MODEL`, `AGENT-EVAL-SCORING`, `AGENT-EVAL-FIXTURE`, `AGENT-EVAL-GATE`, `EVID-CATALOG`, `INDEX-ACTION-PATCH`, `MEM-EDIT`, `MEM-LIMIT`, `POLICY-REVALIDATE`, `POLICY-VERDICT-CAP`, `POLICY-DEFAULT-TICK`, `POLICY-ITEM-MEMORY`, `SRV-SETTINGS`, `SRV-MEMORY-CRUD`, `UI-CONTEXT-BOUNDARY`, `UI-WELCOME-BACK`, `UI-CLEAR-HISTORY`, `UI-MASCOT-JOBS`, `UI-PROPOSAL-COLLAPSE`, `UI-SETTINGS-AUTOSAVE`, `UI-THEME-CONTRAST`, `UI-CHANGES-CHART`, `UI-DRIVE-SELECTOR`.
- **Removed requirements:** none.
- **Schema:** `schema_v5.sql` (memory detail, tool-card data, proposal review fields, `review`, `copy_check`, `file_hash`) and `schema_v6.sql` (`compress_estimate`).

## Phases (one branch and PR each)
1. **Units and locations:** the catalog `unit` field and gap entries, the model-store detector, location rows, and POLICY-LOCATION-KEEP.
2. **Review v1:** the delete cards (temp/Windows, caches, build output, downloads), Large areas with What is this?, the ▸ tree, Show in Explorer, and the Review home view.
3. **Explore:** the treemap, By type, the linked tree, and color by knowledge.
4. **Copy checks:** the framework, then the offline checks (duplicate, archive, cloud sync, git) with the Online-only card and the "only copy we know of" area, then the online checks (model hubs, winget) with the Re-downloadable card.
5. **Compress and Apps you rarely use.**
6. **Learning:** finished jobs (`cleaned`, `kept`), web `about` with identity-only rows, and the Learned chip.
7. **Chat cards (with the Map tab), pills from both sources, prompt rule and eval.**
8. **Cleanups only Windows can do** (7), if D3 is yes.

## Defaults (say if any are wrong)
| Setting | Default |
|---|---|
| Large location | ≥ 5 GiB or ≥ 1% of used space; 40 per drive |
| Descend into a child | ≥ 1 GiB or ≥ 10% of its parent |
| Model-store detector | ≥ 80% of bytes in weight types, per-model subfolders |
| Copy-check candidate | file ≥ 100 MiB, or a folder item ≥ 1 GiB |
| Online-only | synced, in sync, not used for 90 days |
| Compress candidate | ≥ 2 GiB folder; propose when saving ≥ 1 GiB and ≥ 20%; not modified for 30 days |
| Rarely used app | ≥ 2 GiB, not run for 90 days |
| Send elsewhere | ≥ 5 GiB, not used for 180 days, destination set |
| Declined card | items skipped by reviews for 30 days |
| Review reruns | 6 h after the last one, or after a refresh |
| Temp `contents` item | children not modified for 24 h |
| Treemap block | ≥ 9 px² at the current canvas size; smaller siblings merged |

## Decided
- **Layout:** Review and Explore are the home tabs, and the chat is a side panel (a tab on a narrow window).
- **Pills:** the model's `suggest_next` recommendations come first; deterministic pills from the card data fill the rest.
- **Copy checks are general:** Hugging Face is one check among many.
- **Online copy checks (D2):** one consent the first time an online check is needed, with a switch per service in Settings. Offline checks always run.
- **Cleanups only Windows can do (D3):** included, as phase 8.
- **Storage (v5 migration, `schema_v5.sql`):**
  - `memory.detail_json`, `message.ui_json`.
  - `proposal.anchor`, `review_id`, `card` (which review card the proposal belongs to) and `touched` (the user changed a tick or decided on it).
  - `review` (one row per review run, with its Large areas in `areas_json`).
  - `copy_check`, `file_hash`.
  - Workstreams that need more schema add `schema_v6.sql` and later, renumbered when they are merged.
