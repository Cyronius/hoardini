# SRV — loopback server

Canonical spec for `hoardini-server` and `hoardini serve`. Design: `.claude/plans/hoardini-design.md` §4, §5.1, §7; route table: `hoardini-impl.md`.

Tests: `specs/srv/tests/srv.rs` (`cargo test -p hoardini-specs --test srv`). The route list the tests walk is exported by `hoardini_server::route_list()`, which is also what builds the router.

## Requirements

### SRV-TOKEN: Every API route requires the per-launch token
**Applies to:** hoardini-server
**Verification:** test

Every path under `/api` shall answer 401 `{"error":"unauthorized"}` unless the request carries the launch token as `Authorization: Bearer <t>` or `?token=<t>`. This includes unknown `/api` paths and every method on every registered route. The token is compared in constant time. When no token is given the server generates 32 random bytes, hex encoded.

**Acceptance criteria:**
- For every `(method, path)` in `route_list()`: no token → 401; wrong bearer → 401; wrong `?token=` → 401; right bearer → not 401; right `?token=` → not 401.
- `/api`, `/api/` and `/api/no-such-route` without a token → 401; with the token → 404 JSON.

### SRV-STATIC-OPEN: The UI is served without a token and under a strict CSP
**Applies to:** hoardini-server
**Verification:** test

Non-`/api` paths shall serve the UI (from `--ui-dir`, else the embedded build, else a placeholder page) without a token, falling back to `index.html` for extensionless paths and answering 404 for missing assets. Every response carries `Content-Security-Policy: default-src 'self'; connect-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; font-src 'self' data:; script-src 'self'` and no CORS headers. A request whose `Host` is not `127.0.0.1` or `localhost` is refused (403), which blocks DNS rebinding.

**Acceptance criteria:**
- `GET /`, `/assets/app.js`, `/threads/abc` succeed without a token; `/assets/missing.js` is 404.
- The CSP contains `default-src 'self'` and `script-src 'self'`; `Access-Control-Allow-Origin` is absent.
- `Host: evil.example` → 403.

### SRV-SECRETS-WRITE-ONLY: Secret values are never returned, stored in config, or echoed
**Applies to:** hoardini-server
**Verification:** test

`PUT /api/secrets` shall store the value in the vault and return only `{"ok":true}`. Provider `api_key` must be a `secret://` reference: a raw key is rejected (400) and never written to `provider.config_json`. So must any provider header whose name contains `auth`, `key`, `token`, `secret`, `password`, or `cookie`. `has_key` is derived from the vault. Error responses for `/api/secrets` and provider writes never quote the submitted value, and request bodies and query strings are never logged.

**Acceptance criteria:**
- After storing `sk-…` and creating a provider that references it, `GET /api/providers` has `has_key: true` and the response text does not contain the value; `provider.config_json` contains the ref and not the value.
- `POST /api/providers` with `api_key: "sk-…"` → 400 and the response does not contain the value.
- `POST /api/providers` with a raw `api-key`, `Authorization`, or `X-Goog-Api-Key` header → 400 naming `secret://`, without the value; the same provider with `api-key: secret://…` and a plain `HTTP-Referer` is accepted, and no `provider.config_json` holds the value.
- A malformed `PUT /api/secrets` body → 4xx and the response does not contain the submitted text.

### SRV-ONE-REFRESH: One index refresh at a time; Fast takes over a Lean refresh
**Applies to:** hoardini-server
**Verification:** test

The server shall run at most one indexer child at a time. `POST /api/index/refresh` (Rescan now) takes no body and runs in the mode `index_speed` names (SRV-SETTINGS: `lean` → `background`, `fast` → `fast`), except that a volume whose index has never completed (its first full scan: a new install, or a drive just added, INDEX-VOLUMES) is refreshed `fast`; the other watched volumes of the same pass keep the switch's mode. A pass refreshes every watched volume in order and skips one that is not connected. The launch refresh and timer-driven journal refreshes pick their mode the same way. It answers `{started:false}` while a refresh runs. Setting `index_speed` to `fast` during a `background` refresh stops the child and restarts the same volume in fast mode; setting `lean` during a `fast` refresh lets it finish. A short timer-driven journal refresh is never stopped this way. Progress lines from the child appear in `Status.index.progress`. The child is killed on shutdown, and it joins a kill-on-close Job Object so it also dies when the server process is killed outright.

**Acceptance criteria:**
- With a fake indexer and an index that has completed once: `POST` → started, in `background`; a second `POST` → not started; status shows `refreshing` with the child's progress.
- `PUT /api/settings {index_speed: "fast"}` during that refresh: the indexer was spawned exactly twice, in modes `background` then `fast`, status mode is `fast`, and a `POST` → not started. A following `PUT {index_speed: "lean"}` spawns nothing and the fast refresh keeps running.
- On a database with no completed refresh and `index_speed` `lean`, `POST` spawns `fast`.

**Verification (manual, shutdown):**
1. `hoardini --db %TEMP%\h-kill.db serve --port 7044 --token dev` (it indexes C: at launch, in fast mode because it is the first scan).
2. Press Ctrl+C; in Task Manager confirm no `hoardini.exe index` child remains.
3. Start it again; while the refresh runs, end the server process outright (Task Manager → End task on the `serve` process, or `taskkill /F /PID <pid>`). Within a second the `hoardini.exe index` child is gone too. (2026-10-08: child gone 2 s after a hard kill.)
4. Delete `%TEMP%\h-kill.db*`.

### SRV-JOURNAL-REFRESH: The index stays current while the app is open
**Applies to:** hoardini-server
**Verification:** test

Launch (`refresh_on_start`) and every user refresh shall spawn the indexer with `--kind auto`. While the server runs with `refresh_on_start`, someone is looking (SRV-PRESENCE), and no refresh is running, it shall spawn `--kind journal` every `meta.journal_every_s` seconds (default 180), in the mode SRV-ONE-REFRESH picks. When that child reports `needs_full` (or exits 3), the server runs `--kind full` in the same mode for the same volume; this is not an error. A timer-driven run shows as `idle` in `Status.index` unless it has run for more than 4 s; a user refresh asked for during one waits for it and starts next (`{started:true}`).

**Acceptance criteria:**
- With a completed index, `index_speed` `lean`, `journal_every_s = 1`, a status stream open with `looking: true` posted, and a fake indexer that answers `--kind journal` with `needs_full` and exit code 3: the spawns are `background auto`, `background journal`, `background full`, and `/api/status` never reports `refreshing` after the launch refresh has finished.

**Verification (manual, live journal):**
1. Point meta `indexed_volumes` at a test folder, set meta `journal_every_s` to 15, start `hoardini serve`, and keep the UI open and focused (or hold a status stream open and post `{"looking": true}` to `/api/presence`).
2. Add a 3 MB file to the folder. Within 15 s the log shows `index refresh done kind="journal" ... dirty_dirs=Some(2)` and `hoardini top --files --under <folder>` lists the file; the header never leaves "Index just now".
3. Clear `volume.usn_journal_id`/`usn_next` in the database: the next timer run logs `journal refresh not possible; running a full refresh ... reason="no_position"`, then a `kind="full"` done line, and the run after that is a journal one again.

### SRV-PRESENCE: Timed index work only while someone is looking
**Applies to:** hoardini-server, hoardini (UI)
**Verification:** test

The server shall treat the UI as looking only while the last `POST /api/presence {looking}` said `true` and at least one status stream is open. When the last status stream closes, looking drops to `false` until the next `looking: true` (this covers a crashed or hidden page). While nobody is looking there is no journal timer at all. When looking starts and the index is older than the journal interval (`meta.journal_every_s`, default 3 minutes) or has never completed, one quiet journal refresh starts at once; otherwise the next one is due when the index reaches that age, then every interval. Presence never cancels anything: a refresh already running (including a first full scan), approved jobs, and chat runs all continue. The UI posts presence on focus, blur, and `visibilitychange` (looking = visible and focused), posts it again whenever its status stream reconnects, and closes the stream while the page is hidden. A visible but unfocused window keeps its stream (job progress) but runs no refreshes.

**Acceptance criteria:**
- `journal_wait(age, every)`: no age (never completed) → 0; 4 min or 3 min old with a 3 min interval → 0; 1 min old → 2 min; 0 old → 3 min.
- `journal_every_s = 1`, a launch refresh that runs 3 s: a stream opened with `looking: true`, then closed with `looking: false`, leaves it `refreshing`, and it runs to its end. Then no journal refresh spawns in 2.5 s with nobody looking, nor in 1.5 s with `looking: true` and no stream; opening a stream spawns one within 3 s; after the stream closes nothing more spawns in 2.5 s.
- `journal_every_s = 60`, an index refreshed 10 s ago: a stream plus `looking: true` spawns nothing in 1.5 s. With the index 5 minutes old, `looking: false` then `true` spawns exactly one `background journal`, and no second one in the next 1.5 s.

### SRV-IDLE-ZERO: An unwatched app does nothing
**Applies to:** hoardini-app, hoardini-server
**Verification:** manual

With the window minimized and no job or chat run in progress, the app shall have no indexer, llama-server, or rclone child and no periodic wake-ups of its own: no journal timer (SRV-PRESENCE), no status stream (the hidden page closes it), no built-in watchdog (PROV-BUILTIN-IDLE), and no rclone reaper (it exits with the last rclone daemon). WebView2 is told the window is hidden and set to its low memory-usage target level while minimized, and back to normal on restore.

**Verification (manual):**
1. `cargo build -p hoardini-app`, set `HOARDINI_DATA_DIR` to a scratch folder, run `target\debug\hoardini-app.exe`, and wait for the first scan to finish ("Index just now").
2. Minimize the window and leave it for 10 minutes.
3. Task Manager → Details: no `hoardini-app.exe` child with `index` on its command line, no `llama-server.exe`, no `rclone.exe`.
4. Process Explorer → `hoardini-app.exe` → Threads: over 60 s, the `tokio-runtime-worker` threads show no context-switch delta. The log (`%HOARDINI_DATA_DIR%\logs\hoardini.log.*`) has no `index refresh done` line from the minimized period.
5. The `msedgewebview2.exe` processes under the app drop in private working set within a minute of minimizing (record before and after).
6. Restore and click into the window: the page reconnects at once, and because the index is now older than 3 minutes the log shows one `kind="journal"` refresh within a few seconds.
(2026-10-09, partial: debug build, meta `indexed_volumes` on a 30-file folder, `journal_every_s = 20`, minimized for 45 s rather than 10 minutes. No refresh ran while minimized and one journal refresh ran 1 s after restore; no child processes. WebView2 working set 403 MB visible, 62 MB after 45 s minimized, 149 MB after restore; private 228 → 190 → 201 MB. Over raw CDP (`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=…`, not Playwright, whose focus emulation keeps the page visible), `document.hidden` is true while minimized and the status stream's connection closes. Step 4's context-switch check was not run.)

### SRV-SETTINGS: Settings round trip
**Applies to:** hoardini-server
**Verification:** test

`GET /api/settings` returns exactly `index_speed` (`"lean"` | `"fast"`, default `"lean"`), `builtin_idle_minutes` (default 10), and read-only `index_size_bytes` (database plus WAL size). `PUT` stores `index_speed` and `builtin_idle_minutes`, ignores `index_size_bytes` and any other key, and rejects a wrong value with 400 without storing anything from that request. The old keys (`threshold_bytes`, `redact_paths`, `web_search_enabled`, `indexer_workers`, `indexer_memory_cap_mb`) are gone: the threshold is fixed (INDEX-THRESHOLD), redaction is always on (AGENT-REDACT), web search follows its provider (AGENT-WEB-SEARCH), and workers and the memory cap follow `index_speed` (INDEX-POLITE, INDEX-MEMORY-CAP).

**Acceptance criteria:**
- A fresh database returns exactly the three keys with `lean`, 10, and a size above 0.
- After `PUT {index_speed: "fast", builtin_idle_minutes: 0, index_size_bytes: 1, threshold_bytes: 5}` a `GET` shows `fast` and 0, `index_size_bytes` is not 1, and there is no `threshold_bytes`.
- `PUT {index_speed: "turbo"}`, `{index_speed: true}`, and `{builtin_idle_minutes: "x"}` → 400, and `index_speed` is unchanged.

### SRV-HISTORY-RANGE: History points can be limited to a time range
**Applies to:** hoardini-server
**Verification:** test

`GET /api/history` returns one point per completed refresh of one volume, oldest first: the `volume` query parameter's, else the selected drive's once it has an index (INDEX-VOLUMES), else the most recently refreshed one. The optional
`since_ms` and `until_ms` query parameters keep only points whose `finished_ms` is inside the range, both ends
inclusive; with neither, every point is returned. With no indexed volume the result is `[]`.

**Acceptance criteria:**
- Scans finished at 1000, 2000, 3000, and 4000 ms: `since_ms=2000&until_ms=3000` returns exactly those at 2000 and
  3000, with their `free_bytes`; `since_ms=3000` starts at 3000; `until_ms=1000` ends at 1000.
- No parameters returns every point in ascending order; an empty database returns `[]`.

### SRV-INDEX-CHILDREN: One folder level for the inline tree
**Applies to:** hoardini-server
**Verification:** test

`GET /api/index/children?path=<folder>&limit=<n>` (default 50, at most 1000) shall return `{path, size, files, children, small, more}` for an indexed folder. `children` are its direct children from the index, biggest first (ties by name), each `{path, name, kind ("dir" | "file" | "link"), size, files, mtime_ms, denied, has_children}`; `has_children` is true for a folder with any child row or folded small files. `small` is `{count, bytes}` of the files folded into this folder by the row threshold (INDEX-SMALL-FOLD), and `more` is `{count, bytes}` of the children past the limit. `path` may start with `~`, matches the index without regard to case, and every returned path shows the profile as `~`. A missing or non-absolute path is 400, a file is 400, and a path not in the index is 404.

**Acceptance criteria:**
- `C:\data` holding `models\` (300 + 100 MiB), `big.iso` (200 MiB), `cache\` (50 MiB), an empty folder, and two small files (4 KiB and 8 KiB allocated): `limit=2` returns `models` (`dir`, 400 MiB, 2 files, `has_children`) then `big.iso` (`file`, 1 file, no children), `small` `{count: 2, bytes: 12288}`, `more` `{count: 2, bytes: 50 MiB}`, `size` 650 MiB + 12288 and `files` 6.
- `path=c:\DATA` with no limit lists `models`, `big.iso`, `cache`, `empty` in that order; `empty` has no children and `more` is zero.
- `path=C:\data\big.iso` is 400, `C:\nope` is 404, and no `path` is 400.

Test: `specs/srv/tests/srv.rs`.

### SRV-REVEAL: Show in Explorer
**Applies to:** hoardini-server
**Verification:** test

`POST /api/reveal {path}` shall run `explorer.exe /select,"<path>"` (the path quoted after the comma) and answer `{ok: true}` without waiting for Explorer. `path` may start with `~`. It starts nothing and refuses: a missing or empty path, a path that is not an absolute drive path, contains `"`, or has a `.` or `..` component (400); a path that is not a watched drive (INDEX-VOLUMES) or under one (403); a path that does not exist (404). The refusals are tested; opening Explorer is checked by hand.

**Acceptance criteria:**
- With `indexed_volumes` set to one scratch folder: `{}`, `{path: ""}` and `{path: "relative\x.txt"}` are 400; an existing file in another folder is 403; `<root>\..\<other>\x.txt` is 400; `<root>\missing.txt` is 404; every refusal is a JSON `error`.

**Verification (manual, Explorer):**
1. Start `hoardini serve --port 72xx --token dev --no-refresh` with `HOARDINI_DATA_DIR` and `--db` in a scratch folder, and watch a scratch folder holding `a b\c.txt` (`PUT /api/volumes`).
2. `POST /api/reveal?token=dev` with `{"path": "<folder>\\a b\\c.txt"}`: Explorer opens `a b` with `c.txt` selected, and the answer is `{"ok":true}`.
3. The same for `<folder>\\a b`: Explorer opens the scratch folder with `a b` selected.

Test: `specs/srv/tests/srv.rs`.

### SRV-INDEX-MAP: A folder's treemap, pruned to the canvas
**Applies to:** hoardini-server
**Verification:** test

`GET /api/index/map?path=<folder>&w=<px>&h=<px>` (`w` and `h` default 1200 and 800, clamped to 16–8192) shall return `{tree, ancestors, min_bytes, nodes, capped}` for an indexed folder, for the Explore treemap (UI-EXPLORE). A block of `size` bytes covers `size / root size` of the `w × h` canvas, so `min_bytes` is the smallest size that still gets 9 px² (rounded up). Each node is `{path, name, kind ("dir" | "file" | "link" | "small"), size, files, ext (files only, lower case, "" when none), children?, more?, tag}`. A folder is opened biggest first: its child rows and one `small` block for its folded small files (INDEX-SMALL-FOLD; `path` is the folder's, `name` "small files", `files` their count) are kept, biggest first (ties by name), while they are at least `min_bytes`, and the rest are merged into the folder's `more: {count, bytes}` (`count` is the rows plus the folded files it stands for). An opened folder always has `children` and `more`, and its `size` is the sum of its children's sizes plus `more.bytes`, so every level adds up to the root's size; a gap in the index is counted in `more.bytes`. The response holds at most 5,000 nodes: folders open largest first until the cap, then nothing more is opened, merged blocks go to `more`, and `capped` is true. `nodes` is the number of nodes returned. `ancestors` lists `{path, name}` from the drive root down to the folder's parent, for breadcrumbs. Paths follow SRV-INDEX-CHILDREN (`~` accepted and shown; 400 for a missing or non-absolute path or a file; 404 for a path not in the index).

`tag` is `{state, label?, verdict?}`, decided by the nearest knowledge at or above the node (above the indexed folder too, when a watched folder is not a whole drive), with no model call:
1. **proposed:** the node is, or is under, a pending item of a pending proposal (a chat or review card); the label is the item's reason (else the card's summary) and the verdict the item's.
2. Otherwise the deepest of: a protect note (`keep`, its text as label); a location row (MEM-LOCATION): category `system` or `protected` → `system`; `keep` in `photos`, `documents`, `user` or `user-data` → `user`; other `keep` → `keep`; `review`, `safe` or `redownload` → `review`; no verdict → `unknown` (its label kept); a catalog entry matching the node itself (EVID-CATALOG): `system` category → `system`, `keep` user data → `user`, `keep` → `keep`, else `review`, labelled with the entry's name. At one path a protect note beats a location row, which beats the catalog. Container rows and container entries (and POLICY-DENY's container areas such as the drive root) only label the node itself.
3. Otherwise a Windows system area of POLICY-DENY → `system` (its label, verdict `keep`), else `unknown`.
A `small` block has its folder's tag.

**Acceptance criteria:**
- `C:\data` holding `models\` (300 + 100 MiB `.gguf`), `big.iso` (200 MiB), `proj\node_modules\dep.bin` (60 MiB), `cache\c.bin` (20 MiB), `tiny\` (2 MiB `.bin` + 2 MiB `.jpg`) and two small files (12 KiB): at 16 × 16 px `min_bytes` is 25,215,408, `nodes` 8, the children are `models`, `big.iso`, `proj`, `more` is `{count: 4, bytes: 24 MiB + 12288}`, `a.gguf` has `ext` `gguf`, `ancestors` is `[{path: "C:\", name: "C:"}]`, and every level adds up to 684 MiB + 12288.
- At 1000 × 1000 px nothing is merged and a `small` block of 12288 bytes and 2 files with path `C:\data` is listed.
- With a pending card item `C:\data\cache` (reason "Rebuilds itself", `safe`), a declined one for `big.iso`, a `catalog` location row for `models` (LM Studio models, `redownload`) and the user's Keep for `tiny`: `cache` and `c.bin` are `proposed` with that label and verdict; `models` and `a.gguf` are `review` / LM Studio models / `redownload`; `node_modules` is `review` / node_modules / `redownload` while `proj` is `unknown`; `tiny` is `user` / yours / `keep`; `big.iso` is `unknown`; the small block has `C:\data`'s tag. At `C:\` the root is `unknown` labelled "the drive root" and `Windows` is `system` / the Windows folder / `keep`. A watched folder `C:\x\node_modules\lib` indexed on its own is `review` / node_modules / `redownload`, from the entry above it.
- 5,100 files of 2 MiB in one folder at 8192 × 8192: `nodes` 5,000, `capped` true, 4,999 children from `f0000.bin`, `more` `{count: 101, bytes: 202 MiB}`, and the size still adds up.
- A file is 400, a path not in the index 404, no path 400.

Test: `specs/srv/tests/srv.rs`.

### SRV-INDEX-TYPES: What a folder holds, by file type
**Applies to:** hoardini-server
**Verification:** test

`GET /api/index/types?path=<folder>` shall return the folder's bytes by extension from INDEX-EXT-MIX, biggest first: `[{ext, bytes, files, label?, app?, use?, category?}]`. A type with a file-type row (MEM-FILETYPE) carries the row's text as `label` and its `app`, `use` and `category`; without a row, a photo or video, document or model-weight extension of the location classifier's lists (MEM-LOCATION) still gets that `category` (`photos`, `documents`, `model-weights`). Files under the row threshold have no type, so they are one entry `{ext: null, small: true, bytes, files}` placed by its size. At most 200 types are listed; the rest are one last entry `{ext: null, other: <number of types>, bytes, files}`. Paths are checked as in SRV-INDEX-MAP.

**Acceptance criteria:**
- On SRV-INDEX-MAP's `C:\data`, with a `user` file-type row for `gguf` (llama.cpp, model weights): `[gguf 400 MiB 2 files with label, app, use and category model-weights; iso 200 MiB; bin 82 MiB 3 files; jpg 2 MiB with category photos; {ext: null, small: true, bytes: 12288, files: 2}]`, exactly; the whole drive `C:\` adds `dll` 50 MiB from `C:\Windows` between bin and jpg, read in one scan of the drive's rows rather than a walk.
- A file is 400 and a path not in the index 404.

Test: `specs/srv/tests/srv.rs`.

### SRV-EXPLORE-FOCUS: What the user has selected in Explore
**Applies to:** hoardini-server
**Verification:** test

`PUT /api/explore/focus {path}` shall store the path the user selected in Explore (meta `explore_focus`, normalized, `~` expanded) and answer `{path}` with the profile shown as `~`; `{path: null}`, an empty path or no path clears it. A non-absolute path is 400 and changes nothing. A server start clears it, since a selection belongs to the window that made it. The system prompt names it (AGENT-PROMPT-CONTEXT).

**Acceptance criteria:**
- A focus left in the database is gone after the server starts.
- `{path: "C:\data\models\"}` → `{path: "C:\data\models"}` and `explore::focus` returns it; `{path: null}` → `{path: null}` and clears it; `{path: "relative\x"}` → 400 and the focus stays cleared.

Test: `specs/srv/tests/srv.rs`.

### SRV-THREAD: One conversation and its window
**Applies to:** hoardini-server
**Verification:** test

The conversation routes are:
- `GET /api/thread` returns `{id: "main", messages, proposals, window_start_id, has_hidden, fresh, earlier, showing_all}`. `messages` is the scrollback from the last Clear chat on (`meta.chat_view_start_seq`), or all of it with `?all=1` (AG-UI messages, profile shown as `~`); `earlier` counts the stored messages before that mark and `showing_all` echoes `?all=1` and `proposals` maps every proposal of the conversation by ID. `window_start_id` is the first message the model still sees, or null when its window is empty. `has_hidden` is true when some returned message lies before the window start. `fresh` is true when there are no messages, the last message is more than 6 hours old, or the window is empty (Start fresh was pressed).
- `POST /api/thread/fresh` (Clear chat) moves both the window start and the screen mark past the last message and returns `{ok: true}`; nothing is deleted. `DELETE /api/thread` also resets the screen mark.
- `DELETE /api/thread` (Clear chat history) deletes the messages only, resets the window start, and returns `{ok: true}`. Proposals, memories, jobs and the archive journal stay. While a chat run is in progress it answers 409 and deletes nothing.
- `POST /api/agent/run` always runs on `main`, whatever `threadId` the client sends; the run's events carry `threadId: "main"`.

`GET /api/threads`, `GET /api/threads/{id}` and `DELETE /api/threads/{id}` do not exist.

**Acceptance criteria:**
- `route_list()` has no `/api/threads` path. On a new database `GET /api/thread` is `id: "main"`, no messages, `window_start_id` null, `has_hidden` false, `fresh` true.
- With `u1` and `a1` stored just now: `window_start_id` `u1`, `has_hidden` false, `fresh` false. After `POST /api/thread/fresh`: both messages still listed, `window_start_id` null, `has_hidden` true, `fresh` true. After a new `u2`: `window_start_id` `u2`, `fresh` false. With every message 7 hours old: `fresh` true.
- `DELETE /api/thread` leaves no message rows, keeps a stored proposal and the memory count, and reports `has_hidden` false; the next stored message is `window_start_id`.
- A run posted with `threadId` `t-other` streams `threadId: "main"`, its user message is listed by `GET /api/thread`, and `DELETE /api/thread` during the run answers 409.

### SRV-MEMORY-CRUD: Memory routes
**Applies to:** hoardini-server
**Verification:** test

`/api/memory` lists, creates (source `user`), searches (`?q=`, full-text), updates, and deletes memories; unknown ids answer 404. Each row carries `detail` (MEM model): null for notes, the object for location and file-type rows.

**Acceptance criteria:**
- Create → list shows it → `?q=` finds it, an unrelated query finds nothing → `PUT` changes text and kind → `DELETE` → list empty; `PUT`/`DELETE` of a missing id → 404.
- The created note's `detail` is null; a file-type row written for `q4nx` lists with `detail.category` `model-weights` and `detail.app` `FastFlowLM`.

### SRV-STATUS-STREAM: Status stream
**Applies to:** hoardini-server
**Verification:** test

`GET /api/status/stream` shall be `text/event-stream`, send a `status` frame immediately, then `status` frames on change (at most two per second), coalesced `job` frames from the executor, and a `: ping` comment every 15 s. Closing the connection ends the server-side task.

**Acceptance criteria:**
- The first frame is `data: {"type":"status",…}` with `index.state` and `running_runs`.

**Verification (manual, teardown):**
1. Start a server, open `/api/status/stream` with `curl -N`, press Ctrl+C in curl.
2. `GET /api/status` still answers and the process shows no growth in handles after repeating 50 times.

### SRV-RUN-TEARDOWN: Closing the run stream stops the run
**Applies to:** hoardini-server
**Verification:** manual

Aborting the `POST /api/agent/run` request, `POST /api/agent/runs/{id}/cancel`, or server shutdown shall abort the agent task and decrement `running_runs`. A cancel ends the stream with `RUN_FINISHED`.

**Verification (manual):**
1. Start `hoardini serve` against a slow local model and send a run with `curl -N`.
2. `GET /api/status` shows `running_runs: 1`; press Ctrl+C in curl.
3. Within a second `running_runs` is 0 and the model server shows the request cancelled.
4. Repeat, ending with `POST /api/agent/runs/{runId}/cancel`: the stream ends with `RUN_FINISHED`.
5. In the UI, ask something that streams for a while and press Stop: the stream closes at once, the question stays on screen (it is stored in the thread), `running_runs` returns to 0, the model server logs a client disconnect, and a new question in the same chat answers normally. (2026-10-08, scripted mock model: all as described.)

### SRV-RUN-CANCEL: A cancelled run ends with a valid event sequence
**Applies to:** hoardini-server
**Verification:** test

`POST /api/agent/runs/{runId}/cancel` shall end that run's stream with `RUN_FINISHED`, after closing whatever the aborted agent task left open (`TEXT_MESSAGE_END` for an open text message, the thinking end events, `TOOL_CALL_END` for an open call), so a client that checks AG-UI ordering accepts it. `running_runs` drops back.

**Acceptance criteria:**
- Against a model that streams one text chunk and then stalls: `running_runs` is 1 after the first `TEXT_MESSAGE_CONTENT`; after the cancel the stream ends with `TEXT_MESSAGE_END`, `RUN_FINISHED`, and `running_runs` is 0.
