# Evidence engine (prefix EVID)

`inspect(path)` builds one deterministic `Evidence` record (no LLM). Canonical design: `.claude/plans/hoardini-design.md` §5.3.
Code: `crates/core/src/{catalog,evidence}`, data: `catalog/*.toml`.

### EVID-CATALOG: Known-location catalog
**Applies to:** hoardini
**Verification:** test

For an absolute path the catalog shall return the most specific matching entry (deepest matching ancestor-or-self wins;
ties go to the longer literal pattern), supplying `app`, `category`, `regenerable`, `verdict_hint`, an optional policy `cap`,
`shared_blobs`, `safe_unit`, and `notes`. Patterns support `~`, `%VAR%` expansion, `*`, `?`, `**`, and "any directory
named X" (`**\X`); matching ignores case. Shared-blob stores shall name their safe unit and carry a `review` cap; hiberfil.sys
and pagefile.sys shall carry a `keep` cap.

**Acceptance criteria:**
- `~\.lmstudio\models\TheBloke\x\x.Q4_K_M.gguf` matches app `LM Studio`, category `model-store`, hint `redownload`, notes mention "whole model folder".
- `~\.cache\huggingface\hub\models--a--b\blobs\0123` matches `hf-hub-blobs` with `shared_blobs`, cap `review`, safe unit "a whole models--org--name folder"; a snapshot path under the same repo matches the shallower `hf-hub-cache`.
- `C:\hiberfil.sys` and `C:\pagefile.sys` carry cap `keep`.
- `C:\code\app\node_modules\react\index.js` matches `node-modules`, regenerable.
- A path with no entry (`D:\photos\2024\a.jpg`) matches nothing.
- The seed list resolves: Ollama blobs, Triton, MIOpen, NuGet, cargo registry, `.venvs`, llama.cpp build, Rust `target`, `$WINDOWS.~BT`, `SoftwareDistribution\Download`, `Windows\Installer`, `ESD`, Docker `.vhdx`, Steam libraries, Downloads installers, `C:\models`, `unsloth_compiled_cache`.

### EVID-GGUF: GGUF header identity
**Applies to:** hoardini
**Verification:** test

`identify` shall read a GGUF file's header and tensor-info region only (never tensor data) and report the model name,
architecture, quantization name (from `general.file_type`, else the dominant tensor type), size label, context length,
and parameter count (sum of tensor element counts).

**Acceptance criteria:**
- A synthetic GGUF v3 with `general.architecture=llama`, `general.name="Tiny Test 7B"`, `general.file_type=15`, `llama.context_length=32768`, a string-array KV, and two tensors yields name, architecture `llama`, quant `Q4_K_M`, context 32768, parameters equal to the tensors' element sum.
- Without `general.file_type` the quant is the tensor type holding most elements (F16 here).
- For a file with 5 MB of trailing payload, the reader does not read beyond the first buffered block (64 KB) of the file.

### EVID-ATIME-PRESERVE: Reads leave last-access time unchanged
**Applies to:** hoardini
**Verification:** test

Header reads and hashes performed by the evidence engine shall not change a file's last-access time. The engine shall open
the handle with `FILE_WRITE_ATTRIBUTES` and set last-access to `0xFFFFFFFF` before the first read; if that open fails it
skips the read and reports identity "unknown (no access)".

**Acceptance criteria:**
- After `inspect` reads a file's header and hashes it, the file's last-access time equals its value before the call.
- When the preserving open fails, identity is `unknown (no access)` and no read occurs.
- A cloud placeholder (offline / recall-on-access attributes, or flagged in the index) is never opened; its identity is `cloud placeholder`.
- The same holds on a second inspect, whether served from `evidence_cache` or recomputed.

### EVID-IN-USE: Open-handle detection
**Applies to:** hoardini
**Verification:** test

`inspect` shall list the processes that hold a file open (Restart Manager). For a directory it shall check the files inside,
largest first, up to a cap of 64 (`evidence::inuse::DIR_FILE_CAP`). Evidence is advisory; the policy gate checks up to 500
files of a folder at proposal and execution time (POLICY-IN-USE).

Measured 2026-10-08 on this PC (heavily loaded, debug build, C: index of 936k dirs, 5.9M files): one Restart Manager
session over the 500 largest files of a cold folder took 10–76 s, over 64 files 1–8 s, warm 0.1–0.4 s. Asking the kernel
per file (`FileProcessIdsUsingFileInformation`) was slower still (≈ 0.18 s per file), so the cap is the lever. With the cap
and the subtree-query fix, `inspect` of `~\.lmstudio` took 0.7–1.9 s per call (Restart Manager ≈ 1.3 s of it; one 12 s
outlier under load), against 11.4 s before. Folders of 6k+ files elsewhere on C: inspected in 0.6–1.4 s.

**Acceptance criteria:**
- A file held open is reported with the holder's process name and pid; once the handle is closed it is not.
- For an indexed directory containing that file, the file's holder is reported (one Restart Manager session over the largest files).

### EVID-GGUF-SOURCE: GGUF headers name where the weights came from
**Applies to:** hoardini
**Verification:** test

`identify` shall also read `general.basename`, `general.source.huggingface.repository`, `general.source.url`,
`general.url`, and every `general.base_model.N.repo_url` from a GGUF header. `GgufInfo::hf_repos()` lists the Hugging
Face repos they name, most direct first (the source repository, then the source URL, the model URL, and the base
models), without duplicates; URLs that are not `huggingface.co/<org>/<name>` model pages are left out. The copy checks
use these only as places to look (EVID-COPY-MODELHUB).

**Acceptance criteria:**
- A synthetic GGUF with `general.basename = "SmolLM2"`, `general.source.huggingface.repository = "HuggingFaceTB/SmolLM2-135M-Instruct"`, `general.source.url` pointing at the same repo, `general.url = "https://huggingface.co/lmstudio-community/SmolLM2-135M-Instruct-GGUF"`, and two base models (one a `datasets/` URL) yields those fields and `hf_repos()` = the source repository, the `lmstudio-community` repo, then the model base model.
- A GGUF without them has no basename, no source fields, and an empty `hf_repos()`; the EVID-GGUF fields are unchanged.

## Copy checks (prefix EVID-COPY)

A copy check answers, for one file or folder: where is another copy, and how sure are we? Code:
`crates/core/src/copycheck` (the `CopyCheck` trait with `applies`, `find`, `verify`, and `covers_dir` for folder items;
`Registry::standard()`), and `crates/core/src/cloud.rs` (sync roots and placeholder state behind the `Cloud` trait).
Tests: `specs/evid/tests/copy.rs`, run with the `evid` target.

### EVID-COPY-CHECK: One result shape, stored per file state, with confidence deciding the effect
**Applies to:** hoardini
**Verification:** test

Every check returns `{check, locator, confidence, checked_ms, detail}`, with `confidence` one of `identical`,
`same-name-and-size`, `claimed`, `none`. `detail.note` is the one line that item notes and evidence show ("identical to
org/repo on Hugging Face", "in winget as Balena.Etcher 2.1.7", "synced to OneDrive"); `detail.pending` marks an answer
that waits on the background hash (`hash`), the sync engine (`sync`), or a folder's own files (`files`). Results are
stored in `copy_check`, keyed by volume, path, and check, with the file id, size (a folder's indexed logical total), and
mtime they were made for. `results_for(path)` returns only rows that still match the path's current id, size, and mtime.
`check_subject` reuses a matching row instead of running the check again, unless the row is pending and what it waits on
has arrived (a row waiting on the sync engine is always re-run), or the check says it went stale through something other
than the path (`CopyCheck::still_valid`: a duplicate's twin changed, git state, a sync folder's files, a moved archive
copy). A check that finds the path is not its business (`Find::Skip`) drops its old row. The confidence decides the effect (`copycheck::effect`, `copycheck::verdict_for`):

| Confidence | Effect | Verdict it allows |
|---|---|---|
| `identical` | may start ticked | `redownload`; `synced` when the check is `sync` |
| `same-name-and-size` | review, with the locator in the note | `review` |
| `claimed` | follows the catalog | none |
| `none` | only copy we know of | none |

`only_copy(path)` is true when at least one check answered, every answer is `none`, none is pending, and no online check
that applies is waiting on consent (EVID-COPY-CONSENT). `note_for(path)` is the best answer's note, or "only copy we know
of". `candidates(cx, registry, volume_root)` lists the volume's indexed files of at least 100 MiB (not placeholders,
denied, or hard-link duplicates) and its folders of at least 1 GiB that some check covers (a git work tree, a Hugging
Face `models--*` or LM Studio repo folder, a folder under a sync root, a catalog launcher or package unit, an archived
folder), biggest first, at most 2,000.

A knowledge row (memory) whose `detail_json.copy_hint` names a check (`"huggingface"`, `"huggingface:<org>"`,
`"huggingface:<org>/<repo>"`, `"winget:<name>"`, or `{"check", "where"}`) makes that check apply to the paths the row
covers and tells it where to look first. A hint never sets or raises a confidence.

`run_volume(db, env, registry, root, go)` is one background pass: every candidate is checked, then each one whose answer
waits on a hash gets a SHA-256, biggest first, and is checked again. Hashes are read through the atime-preserving handle
(EVID-ATIME-PRESERVE), stored in `file_hash` keyed by volume and file id, and reused while size and mtime hold; a file
that changes while it is hashed gets no row. Hugging Face blobs (`models--*\blobs\<sha256>`) and Ollama blobs
(`sha256-<hex>`) are named by their hash and are never read. `go` is polled between files and between 1 MiB chunks.
`RunReport.online_waiting` counts the candidates that wait on an unanswered online consent; the server stores the total
over all watched volumes in meta `copy_online_waiting` after each full pass (`set_online_waiting`).

**Acceptance criteria:**
- `effect` maps identical, same-name-and-size, claimed, none to may-start-ticked, review, follow-catalog, only-copy; `verdict_for` gives `synced` for an identical `sync` answer, `redownload` for an identical `huggingface` answer, `review` for same-name-and-size, and nothing for claimed and none.
- A counting check runs once for two `check_subject` calls on an unchanged file, and again after the file's mtime changes; `results_for` returns its row before the change and nothing after it.
- `only_copy` is false with no answers, true when the only answer is `none`, false when that answer is pending, and false when another check answered `identical`.
- On an indexed folder holding files of 120 MiB and 101 MiB (two of each, the same bytes within each pair) and 99 MiB, a 1.1 GiB folder with `.git`, and a 1.1 GiB plain folder, `candidates` lists the git folder and the four big files, biggest first, and not the 99 MiB file or the plain folder.
- `run_volume` hashes the four big files, the 120 MiB pair first; each `file_hash` row equals the SHA-256 of the file's bytes, and afterwards each file's `dup` answer is `identical` with its twin as the locator. A second pass hashes nothing; after one file is appended to, the next pass hashes only that file.
- When `go` turns false inside the first hash, `run_volume` reports `stopped` with nothing hashed and no `file_hash` row; the next pass hashes both files.
- A file named like a hub blob whose bytes do not match its name keeps the name as its hash, and no `file_hash` row is written for it.
- With a glob row `**\*.q4nx` carrying `{"copy_hint": "huggingface:acme"}`, the Hugging Face check applies to `x.q4nx` and its search term starts with `acme`; without the row it does not apply.

### EVID-COPY-CONSENT: Online checks wait for consent; offline checks always run
**Applies to:** hoardini
**Verification:** test

Online checks (Hugging Face, winget) declare a service, and `check_subject` runs one only while
`online_checks.services.<service>` is on (meta `online_checks`, SRV-SETTINGS). Until the user has answered (`asked`
false), an online check that applies is reported as waiting (`Outcome.waiting`, counted by `run_volume`) and `only_copy`
stays false for its paths; once asked with the service off, the check simply does not run. Offline checks (duplicate,
archive, sync, git, Ollama, catalog) never look at consent. Requests that go out carry repo ids, search terms made from
file names, and package names, never a local hash, a file size, a local path, or file contents: the remote hash is
fetched and compared on this machine.

**Acceptance criteria:**
- With consent unasked, checking an LM Studio GGUF sends no request to the Hugging Face stand-in, reports `huggingface` as waiting, still runs the offline `dup` check, and `only_copy` is false.
- With `asked` true and `huggingface` off, nothing is sent and nothing waits.
- With `huggingface` on, the stand-in receives requests whose paths and queries contain neither the file's SHA-256, nor its size, nor the profile path.

### EVID-COPY-DUP: Another copy on the indexed drives
**Applies to:** hoardini
**Verification:** test

`dup` applies to files of at least 100 MiB that are not placeholders. It looks for other indexed files of the same
logical size and a different file id (hard links are one file). A persisted duplicate group (`dupes::find`, full BLAKE3)
counts while both members still have the mtime and file id they were hashed with; otherwise two current SHA-256 rows in
`file_hash` that are equal count. Either gives `identical`, with the other path as the locator, and goes stale when that
twin's mtime, size, or file id changes. With no same-size file
the answer is `none`; with same-size files not all hashed yet, `none` pending `hash`; when all are hashed and differ,
`none`.

**Acceptance criteria:**
- Two 101 MiB files in a persisted group with matching mtimes and file ids: each is `identical` to the other, with `detail.via` "duplicate group".
- After the twin is rewritten (new mtime), the group no longer counts and the answer is `none` pending `hash`.
- A 101 MiB file with no other file of that size is `none` with the note "no other file of this size on the indexed drives".

### EVID-COPY-ARCHIVE: Hoardini's own archive copies
**Applies to:** hoardini
**Verification:** test

`archive` applies to a path that has an `archive_entry` (its own, or a folder's above it; the newest wins). When the entry
was restored and the archived copy is still reachable (a local or SMB path that exists; an rclone copy is taken as
recorded; the answer goes stale when a local copy disappears), the copy was verified when it was archived, so the path is `identical` while its size matches the copy and
nothing in it was modified after `archived_ms` (2 s slack), and `same-name-and-size` when something was modified since. A
different size is `none`. When the entry was never restored, the archive removed the original, so a file at that path
now is a newer one: `same-name-and-size` when its size equals the recorded size, otherwise `none`.

**Acceptance criteria:**
- A restored file with the recorded size and an mtime before `archived_ms`, whose archived copy exists, is `identical` with the copy's path as the locator and a note containing "verified when archived".
- The same file with its mtime set after the archive is `same-name-and-size`; grown by a byte, it is `none`.
- An entry never restored, with a new file of the recorded size at the original path: `same-name-and-size`; with another size: `none`.
- A restored entry whose archived copy is gone: `none`, "no longer reachable".

### EVID-COPY-SYNC: Files the cloud sync engine holds
**Applies to:** hoardini
**Verification:** test

`sync` applies under a registered Cloud Files sync root (OneDrive, Dropbox, iCloud Drive, Google Drive when it registers
one, and other providers), read from `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\SyncRootManager\<id>\UserSyncRoots`
with the provider named from the registration id. A file is `identical` when the sync engine marks its placeholder in
sync (`CfGetPlaceholderInfo`, opened for attributes with no recall), with the provider as the locator and the note
"synced to <provider>". A folder is `identical` when every file in it (up to 20,000) is in sync, and is walked again on every pass, since
a file inside can fall out of sync without the folder changing. Shell metadata the engines never upload (`desktop.ini`,
`thumbs.db`, `.DS_Store`) is not counted. Anything else is `none`
pending `sync`, so the next pass looks again. The Win32 calls sit behind the `Cloud` trait, and the decisions are tested
against a fake one.

**Acceptance criteria:**
- With a fake root `<tmp>\OneDrive` (provider OneDrive): an in-sync file is `identical`, locator `OneDrive`, note "synced to OneDrive"; a file the fake reports as not in sync, and a plain file that is no placeholder, are `none` pending `sync`; a file outside the root gets no `sync` answer.
- A folder whose two files are in sync is `identical` although it also holds a `desktop.ini` that is no placeholder; with one of the two not in sync it is `none` with "1 of 2 files not in sync".
- Provider names: `OneDrive!S-1-...!Personal|x` is OneDrive, `Dropbox!...` Dropbox, `iCloudDrive!...` iCloud Drive (`hoardini_win::provider_display_name`, in-crate test); an ordinary file is no placeholder (`placeholder_info` gives `None`).

**Verification (manual, live provider, read-only):**
1. On a PC with Dropbox or OneDrive, `cloud::system().roots()` lists the sync root with its provider name.
2. For a synced file and a synced folder under it, `hoardini_win::placeholder_info` reports in sync, and `copycheck::sync_result` answers `identical` "synced to <provider>". Only metadata is opened (no recall), so nothing downloads and nothing is written.
(2026-10-09: pass against `C:\Users\josha\SVI Dropbox` (provider Dropbox) for two documents and two folders. The first folder run counted the folder's `desktop.ini` as not in sync, which is why shell metadata files are now skipped.)

### EVID-COPY-GIT: Files a git remote holds
**Applies to:** hoardini
**Verification:** test

`git` applies inside a git work tree (a folder at or above the path holding `.git`). It runs the `git` CLI read-only
(`status`, `rev-parse`, `rev-list`, `diff --no-ext-diff --no-textconv`, `config --get`) with `core.fsmonitor` off, so a
repository's config cannot start a program. A file is `identical` when it is tracked, unchanged since the last commit,
and the branch is not ahead of its upstream as of the last fetch, or is ahead only by commits that leave the file as the
upstream has it; the locator is the upstream (`origin/main`) and the note names the remote URL with any credentials
removed. A folder is `identical` when nothing under it is untracked, ignored, or changed and the same push rule holds; its
note reads "everything tracked is pushed to ...". Untracked: `none` ("not tracked by git"); changed: `none`; no upstream:
`none`. Ignored files are not this check's business (no answer). A push, commit, or fetch changes the answer without
touching the file, so git answers are asked again on every pass.

**Acceptance criteria (against real temporary repositories; the test fails if `git` is not on PATH):**
- After a commit pushed to a bare remote, a tracked file is `identical` with the clone's upstream as the locator, and the work tree folder is `identical`.
- An untracked file is `none` "not tracked by git" and makes the folder `none`; an ignored file has no answer; a modified tracked file is `none` "changed since the last commit".
- After a local commit that changes the file and is not pushed, the file is `none` naming the unpushed commit, while another tracked file the commit did not touch stays `identical`.
- `https://user:token@host/org/repo.git` shows as `host/org/repo` (in-crate test).

### EVID-COPY-MODELHUB: Model weights on Hugging Face, and Ollama blobs
**Applies to:** hoardini
**Verification:** test

`huggingface` (online, service `huggingface`) applies to model weights (`.gguf`, `.safetensors`, `.onnx`, `.pt`, `.pth`,
`.ckpt`, and `.bin` next to a `config.json`), to anything in a Hugging Face cache repo (`models--org--name`), to LM
Studio's `models\<publisher>\<repo>\...`, to model folders (a `config.json` beside weights), and to paths a knowledge
hint names it for. It looks for the repo in this order, each source only a place to look: the folder layout (the cache
repo and revision; LM Studio's publisher and repo), the GGUF header (EVID-GGUF-SOURCE), a sibling `config.json`
`_name_or_path` or README `base_model`, the hint, and, when none of those names the file's exact path,
`GET /api/models?search=<file name without its quant tag>&limit=10` (`&filter=gguf` for GGUF), taking up to 8 repos. For
each candidate it reads `GET /api/models/{repo}/tree/{rev}?recursive=1` (following `Link: rel="next"`; a missing revision
falls back to `main`; trees are cached for an hour) and takes the entry with the same path (or the same file name, or
for a hub blob the same LFS oid) and the same size: `same-name-and-size` pending `hash` until the local SHA-256 is known,
then `identical` when it equals the entry's `lfs.oid` and `none` when it differs. No match anywhere: `none` "not found on
Hugging Face". A hub cache repo folder is `identical` when every blob name is an oid in the trees of its snapshot
revisions. An LM Studio or model folder is `identical` when every file matches by path and size and every file of at
least 100 MiB has its own `identical` answer (small files are hashed inline); until then it is `same-name-and-size`
pending `files`.

`ollama` (offline) applies to blobs in the Ollama models folder (`OLLAMA_MODELS`, else `~\.ollama\models`): a manifest
under `manifests\` naming the blob's digest makes it `claimed` ("in Ollama as llama3:8b (ollama pull brings it back)");
with no manifest it is `none`.

**Acceptance criteria (against a local stand-in serving the recorded responses in `specs/evid/fixtures/`):**
- `<profile>\.lmstudio\models\lmstudio-community\SmolLM2-135M-Instruct-GGUF\SmolLM2-135M-Instruct-Q4_K_M.gguf` of 105,454,144 bytes: one tree request for that repo at `main` and no search; the answer is `same-name-and-size` pending `hash` with locator `lmstudio-community/SmolLM2-135M-Instruct-GGUF`. Once its SHA-256 is the recorded `lfs.oid` it is `identical`, note "identical to lmstudio-community/SmolLM2-135M-Instruct-GGUF on Hugging Face"; with another hash it is `none`.
- The same file outside any layout: the search asks for `SmolLM2-135M-Instruct` with `filter=gguf`, bartowski's same-named file of another size does not match, and the lmstudio-community entry does.
- A file of another size outside any layout matches nothing: `none` "not found on Hugging Face", with `detail.searched` listing the repos tried.
- A cache repo folder `models--o--m` whose blob names are the tree's oids is `identical` without hashing; with an extra unknown blob it is `none`.
- An Ollama blob named by a `registry.ollama.ai\library\llama3\8b` manifest is `claimed` "in Ollama as llama3:8b ..."; an orphan blob is `none`.

### EVID-COPY-WINGET: Installers winget can fetch again
**Applies to:** hoardini
**Verification:** test

`winget` (online, service `winget`) applies to `.exe`, `.msi`, `.msix`, `.msixbundle`, `.appx`, and `.appxbundle` files
that are not DLLs. It identifies the installer by its PE version info (the product name when it is short, the product
version), the Authenticode signer of its embedded signature (`evidence::pe::signer`, read through the atime-preserving
handle), and its file name (`balenaEtcher-2.1.7.Setup.exe` gives "balenaEtcher" and 2.1.7). It runs the local `winget`
CLI (`search --name <term> --source winget`, then `show --id <id> --exact --source winget [--version <v>]`, always with
`--accept-source-agreements --disable-interactivity` and no console window) and parses the fixed-width table and the
`show` fields (`Version`, `Publisher`, `Installer Url`, `Installer SHA256`). With the same version listed: `identical`
when the local SHA-256 equals `Installer SHA256` ("in winget as <Id> <version>"), `same-name-and-size` pending `hash`
before the hash is known, and `claimed` when the hash differs. With only another version: `claimed` ("winget can install
<Id> <version>"). Not found: `none`.

**Acceptance criteria (a fake CLI replays the recorded output in `specs/evid/fixtures/winget-*.txt`):**
- `parse_search` reads the recorded `balenaEtcher` table as one row (`balenaEtcher`, `Balena.Etcher`, `2.1.7`), the recorded Firefox table's rows with their ids and versions, and the "No package found" output as no rows; `parse_show` reads `Balena.Etcher`, version 2.1.7, the publisher, the installer URL, and the SHA-256.
- `balenaEtcher-2.1.7.Setup.exe`: the fake receives `search --name balenaEtcher --source winget`, then `show --id Balena.Etcher --exact --source winget --version 2.1.7`; the answer is `same-name-and-size` pending `hash`, then `identical` "in winget as Balena.Etcher 2.1.7" once the file's SHA-256 is the recorded one, and `claimed` with another hash.
- A name winget does not know is `none` "not found in winget".

### EVID-COPY-CLAIMED: Launchers and package managers claim what they installed
**Applies to:** hoardini
**Verification:** test

`catalog` applies where the catalog entry is a game library, package cache, toolchain (`dev-tool`), installer cache, or
build output that is regenerable or re-downloadable. The answer is `claimed`, with the entry's app as the locator ("pip
can reinstall or rebuild it"). Nothing is compared, so it never changes a verdict; the item follows the catalog as
before.

**Acceptance criteria:**
- With a test catalog environment, a wheel under `<profile>\AppData\Local\pip\cache` is `claimed` with locator `pip`, and a game file under `D:\SteamLibrary\steamapps\common\Game` is `claimed` with Steam as the locator.
- A file in an ordinary folder gets no `catalog` answer.

### EVID-COPY-BACKGROUND: Copy checks run in the background while someone looks
**Applies to:** hoardini-server
**Verification:** manual

The server's copy worker (`crates/server/src/copywork.rs`) runs `run_volume` over every watched volume on one thread in
thread background mode (low CPU and very low I/O priority, EcoQoS), one pass at a time. A pass starts when an index
refresh pass ends (unless the last full pass finished less than 15 minutes ago, since journal refreshes come every few
minutes), when `online_checks` is saved, and when someone starts looking (SRV-PRESENCE) while a pass is owed (at
launch, or after one stopped early). It runs only while someone is looking and stops between files and between hash
chunks when nobody is, so a minimized app does no copy-check work (SRV-IDLE-ZERO). Each volume's pass logs `copy checks
done` with its candidate, checked, and hashed counts and how many wait on consent.

**Verification (manual):**
1. Use a scratch data dir and database, and point meta `indexed_volumes` at two folders that are only read: `~\.lmstudio\models\lmstudio-community\SmolLM2-135M-Instruct-GGUF` and `~\Downloads`.
2. Run `hoardini --db <scratch>\h.db serve --port 72xx --token dev`, hold a status stream open (`curl -N "http://127.0.0.1:72xx/api/status/stream?token=dev"`), and post `{"looking": true}` to `/api/presence`.
3. After the refresh, the log shows `copy checks done` for each folder with `online_waiting` above 0, and `GET /api/copy-checks/waiting` reports it.
4. `PUT /api/settings {"online_checks": {"asked": true, "services": {"huggingface": true, "winget": true}}}`. A new pass hashes the waiting files. `GET /api/copy-checks?path=~\.lmstudio\models\lmstudio-community\SmolLM2-135M-Instruct-GGUF\SmolLM2-135M-Instruct-Q4_K_M.gguf` then shows `huggingface` `identical` "identical to lmstudio-community/SmolLM2-135M-Instruct-GGUF on Hugging Face", and `~\Downloads\balenaEtcher-2.1.7.Setup.exe` shows `winget` `identical` "in winget as Balena.Etcher 2.1.7".
5. While it hashes, Process Explorer shows the `hoardini-copycheck` thread at background priority. Closing the status stream stops the pass within a second (the log shows `stopped=true`), and opening it again with `looking: true` resumes.
(2026-10-09: steps 1-4 pass, debug build, live Hugging Face and winget. The first pass logged `online_waiting=1` for each folder; `/api/copy-checks/waiting` then said 1 because each volume overwrote the count, which was fixed to the total and re-run: 2. After consent the pass hashed 105,454,144 + 200,093,336 bytes; the GGUF was `identical` to `lmstudio-community/SmolLM2-135M-Instruct-GGUF` (`lfs.oid` bda48499...), the installer `identical` "in winget as Balena.Etcher 2.1.7" with signer "Balena Ltd", and `~\Downloads\omarchy-4.0.4.iso` "only copy we know of" (no disk-image check yet). Step 5 was not run: the files hashed in under 3 s.)
