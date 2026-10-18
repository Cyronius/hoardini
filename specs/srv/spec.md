# SRV — loopback server

Canonical spec for `hoardini-server` and `hoardini serve`. Design: `.claude/plans/hoardini-design.md` §4, §5.1, §7; route table: `hoardini-impl.md`.

Tests: `specs/srv/tests/srv.rs` (`cargo test -p hoardini-specs --test specs srv::`). The route list the tests walk is exported by `hoardini_server::route_list()`, which is also what builds the router.

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

### SRV-ONE-REFRESH: One index refresh at a time; full walks run fast
**Applies to:** hoardini-server
**Verification:** test

The server shall run at most one indexer child at a time. `POST /api/index/refresh` (Rescan now) takes no body. Rescan, the launch refresh, and a drive just added run `fast`; so does every full walk, including the one a timer-driven journal refresh falls back to (SRV-JOURNAL-REFRESH). Only the timer-driven journal refresh itself runs `background`. A pass refreshes every watched volume in order and skips one that is not connected. It answers `{started:false}` while a refresh runs. (A full walk of C: took 194 to 850 s in background mode and 24 to 116 s in fast mode on the same machine, 2026-10; there is no setting for it.) Progress lines from the child appear in `Status.index.progress`. The child is killed on shutdown, and it joins a kill-on-close Job Object so it also dies when the server process is killed outright.

**Acceptance criteria:**
- With a fake indexer and an index that has completed once: `POST` → started, in `fast`; a second `POST` → not started; status shows `refreshing` in mode `fast` with the child's progress. A `PUT /api/settings {index_speed: "lean"}` during it spawns nothing.

**Verification (manual, shutdown):**
1. `hoardini --db %TEMP%\h-kill.db serve --port 7044 --token dev` (it indexes C: at launch, in fast mode because it is the first scan).
2. Press Ctrl+C; in Task Manager confirm no `hoardini.exe index` child remains.
3. Start it again; while the refresh runs, end the server process outright (Task Manager → End task on the `serve` process, or `taskkill /F /PID <pid>`). Within a second the `hoardini.exe index` child is gone too. (2026-10-08: child gone 2 s after a hard kill.)
4. Delete `%TEMP%\h-kill.db*`.

### SRV-JOURNAL-REFRESH: The index stays current while the app is open
**Applies to:** hoardini-server
**Verification:** test

Launch (`refresh_on_start`) and every user refresh shall spawn the indexer with `--kind auto`. While the server runs with `refresh_on_start`, someone is looking (SRV-PRESENCE), and no refresh is running, it shall spawn `--kind journal` every `meta.journal_every_s` seconds (default 180), in `background` mode. When that child reports `needs_full` (or exits 3), the server runs `--kind full` in `fast` mode for the same volume; this is not an error. A timer-driven run shows as `idle` in `Status.index` unless it has run for more than 4 s; a user refresh asked for during one waits for it and starts next (`{started:true}`).

**Acceptance criteria:**
- With a completed index, `journal_every_s = 1`, a status stream open with `looking: true` posted, and a fake indexer that answers `--kind journal` with `needs_full` and exit code 3: the spawns are `fast auto`, `background journal`, `fast full`, and `/api/status` never reports `refreshing` after the launch refresh has finished.

**Verification (manual, live journal):**
1. Point meta `indexed_volumes` at a test folder, set meta `journal_every_s` to 15, start `hoardini serve`, and keep the UI open and focused (or hold a status stream open and post `{"looking": true}` to `/api/presence`).
2. Add a 3 MB file to the folder. Within 15 s the log shows `index refresh done kind="journal" ... dirty_dirs=Some(2)` and `hoardini top --files --under <folder>` lists the file; the header never leaves "Index just now".
3. Clear `volume.usn_journal_id`/`usn_next` in the database: the next timer run logs `journal refresh not possible; running a full refresh ... reason="no_position"`, then a `kind="full"` done line, and the run after that is a journal one again.

### SRV-PRESENCE: Timed index work only while someone is looking
**Applies to:** hoardini-server, hoardini (UI)
**Verification:** test

The server shall treat the UI as looking only while the last `POST /api/presence {looking}` said `true` and at least one status stream is open. When the last status stream closes, looking drops to `false` until the next `looking: true` (this covers a crashed or hidden page). While nobody is looking there is no journal timer at all. When looking starts and the index is older than the journal interval (`meta.journal_every_s`, default 3 minutes) or has never completed, one quiet journal refresh starts at once; otherwise the next one is due when the index reaches that age, then every interval. Presence never cancels anything: a refresh already running (including a first full scan), approved jobs, and agent runs all continue. The UI posts presence on focus, blur, and `visibilitychange` (looking = visible and focused), posts it again whenever its status stream reconnects, and closes the stream while the page is hidden. A visible but unfocused window keeps its stream (job progress) but runs no refreshes.

**Acceptance criteria:**
- `journal_wait(age, every)`: no age (never completed) → 0; 4 min or 3 min old with a 3 min interval → 0; 1 min old → 2 min; 0 old → 3 min.
- `journal_every_s = 1`, a launch refresh that runs 3 s: a stream opened with `looking: true`, then closed with `looking: false`, leaves it `refreshing`, and it runs to its end. Then no journal refresh spawns in 2.5 s with nobody looking, nor in 1.5 s with `looking: true` and no stream; opening a stream spawns one within 3 s; after the stream closes nothing more spawns in 2.5 s.
- `journal_every_s = 60`, an index refreshed 10 s ago: a stream plus `looking: true` spawns nothing in 1.5 s. With the index 5 minutes old, `looking: false` then `true` spawns exactly one `background journal`, and no second one in the next 1.5 s.

### SRV-IDLE-ZERO: An unwatched app does nothing
**Applies to:** hoardini-app, hoardini-server
**Verification:** manual

With the window minimized and no job or agent run in progress, the app shall have no indexer, llama-server, or rclone child and no periodic wake-ups of its own: no journal timer (SRV-PRESENCE), no status stream (the hidden page closes it), no built-in watchdog (PROV-BUILTIN-IDLE), and no rclone reaper (it exits with the last rclone daemon). WebView2 is told the window is hidden and set to its low memory-usage target level while minimized, and back to normal on restore.

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

`GET /api/settings` returns exactly `builtin_idle_minutes` (default 10), read-only `index_size_bytes` (database plus WAL size), and `online_checks` (`{asked, services: {huggingface, winget}}`; with no saved answer all are true, so online checks are on until the user turns them off; EVID-COPY-CONSENT). `PUT` stores `builtin_idle_minutes` and a partial `online_checks` (only the given fields change), ignores `index_size_bytes` and any other key, and rejects a wrong value (including an `online_checks` that is not an object, a non-boolean, or an unknown service or field) with 400 without storing anything from that request. Saving `online_checks` starts a copy-check pass (EVID-COPY-BACKGROUND, whose manual step 4 checks it; this requirement's test does not). The old keys (`index_speed`, `threshold_bytes`, `redact_paths`, `web_search_enabled`, `indexer_workers`, `indexer_memory_cap_mb`) are gone: the refresh mode is fixed (SRV-ONE-REFRESH), the threshold is fixed (INDEX-THRESHOLD), redaction is always on (AGENT-REDACT), web search follows its provider (AGENT-WEB-SEARCH), and workers and the memory cap follow the mode (INDEX-POLITE, INDEX-MEMORY-CAP).

**Acceptance criteria:**
- A fresh database returns exactly the three keys with 10, a size above 0, and `online_checks` `{asked: true, services: {huggingface: true, winget: true}}`.
- After `PUT {index_speed: "fast", builtin_idle_minutes: 0, index_size_bytes: 1, threshold_bytes: 5}` a `GET` shows 0, `index_size_bytes` is not 1, and there is no `index_speed` or `threshold_bytes`.
- `PUT {online_checks: {asked: true, services: {winget: false}}}` returns and stores `asked` true, `huggingface` true, `winget` false (only the given field changes), and `copycheck::consent::load`, what the copy checks read (EVID-COPY-CONSENT), says the same.
- `PUT {builtin_idle_minutes: "x"}`, `{online_checks: 3}`, `{online_checks: {services: {huggingface: "yes"}}}`, `{online_checks: {services: {bing: true}}}`, and `{builtin_idle_minutes: 5, online_checks: {asked: "no"}}` → 400, and `builtin_idle_minutes` and `online_checks` are unchanged.

### SRV-INDEX-CHILDREN: One folder level for the inline tree
**Applies to:** hoardini-server
**Verification:** test

`GET /api/index/children?path=<folder>&limit=<n>` (default 50, at most 1000) shall return `{path, size, files, children, small, more}` for an indexed folder. `children` are its direct children from the index, biggest first (ties by name), each `{path, name, kind ("dir" | "file" | "link"), size, files, mtime_ms, denied, has_children}`; `has_children` is true for a folder with any child row or folded small files. A file that is a hard-link duplicate (INDEX-HARDLINK-ONCE) lists, and ranks, with `size` and `files` 0, so the rows add up to the folder's size. `small` is `{count, bytes}` of the files folded into this folder by the row threshold (INDEX-SMALL-FOLD), and `more` is `{count, bytes}` of the children past the limit. `path` may start with `~`, matches the index without regard to case, and every returned path shows the profile as `~`. A missing or non-absolute path is 400, a file is 400, and a path not in the index is 404.

**Acceptance criteria:**
- `C:\data` holding `models\` (300 + 100 MiB), `big.iso` (200 MiB), `cache\` (50 MiB), an empty folder, and two small files (4 KiB and 8 KiB allocated): `limit=2` returns `models` (`dir`, 400 MiB, 2 files, `has_children`) then `big.iso` (`file`, 1 file, no children), `small` `{count: 2, bytes: 12288}`, `more` `{count: 2, bytes: 50 MiB}`, `size` 650 MiB + 12288 and `files` 6.
- `path=c:\DATA` with no limit lists `models`, `big.iso`, `cache`, `empty` in that order; `empty` has no children and `more` is zero.
- `path=C:\data\big.iso` is 400, `C:\nope` is 404, and no `path` is 400.
- With `C:\a\copy.gguf` owning a 200 MiB file that `C:\b\model.gguf` hard-links, `C:\b` (with a 5 MiB `other.bin`) lists `other.bin` (5 MiB) then `model.gguf` (0 bytes, 0 files), summing to the folder's 5 MiB; with `limit=1`, `more` is `{count: 1, bytes: 0}` (`index::child_rows`, `specs/index/tests/index.rs`).

Test: `specs/srv/tests/srv.rs`.

### SRV-INDEX-TYPES: What a folder holds, by file type
**Applies to:** hoardini-server
**Verification:** test

`GET /api/index/types?path=<folder>` shall return the folder's bytes by extension from INDEX-EXT-MIX, biggest first: `[{ext, bytes, files, label?, app?, use?, category?}]`. A type with a file-type row (MEM-FILETYPE) carries the row's text as `label` and its `app`, `use` and `category`; without a row, a photo or video, document or model-weight extension of the location classifier's lists (MEM-LOCATION) still gets that `category` (`photos`, `documents`, `model-weights`). Files under the row threshold have no type, so they are one entry `{ext: null, small: true, bytes, files}` placed by its size. At most 200 types are listed; the rest are one last entry `{ext: null, other: <number of types>, bytes, files}`. A path that is not an absolute indexed folder is rejected like SRV-INDEX-CHILDREN's.

**Acceptance criteria:**
- On an indexed `C:\data` (models, an iso, a few bins, two jpgs and two text files), with a `user` file-type row for `gguf` (llama.cpp, model weights): `[gguf 400 MiB 2 files with label, app, use and category model-weights; iso 200 MiB; bin 82 MiB 3 files; jpg 2 MiB with category photos; {ext: null, small: true, bytes: 12288, files: 2}]`, exactly; the whole drive `C:\` adds `dll` 50 MiB from `C:\Windows` between bin and jpg, read in one scan of the drive's rows rather than a walk.
- A file is 400 and a path not in the index 404.

Test: `specs/srv/tests/srv.rs`.

### SRV-THREAD: Item threads
**Applies to:** hoardini-server
**Verification:** test

There is no app-wide conversation: every question is about one item (AGENT-ASK). The conversation routes are:
- `POST /api/agent/run` runs only on an item thread (`thread::is_item_thread`); any other `threadId` answers 400 `threadId must be an item thread` and starts nothing. The run's events carry its own `threadId`.
- `GET /api/thread?id=<item thread>` returns `{id, messages}` with every stored message of that item's conversation (AG-UI messages, profile shown as `~`); no `id`, or any other `id`, answers 400.

`GET /api/threads`, `GET /api/threads/{id}`, `DELETE /api/threads/{id}`, `POST /api/thread/fresh` and `DELETE /api/thread` do not exist.

**Acceptance criteria:**
- `route_list()` has no `/api/threads` path, no `POST /api/thread/fresh` and no `DELETE /api/thread`.
- With a question and an answer stored on `item:abc`, `GET /api/thread?id=item:abc` answers `id` `item:abc` and both messages, the question's profile path shown as `~`; `GET /api/thread`, `?id=main` and `?id=elsewhere` answer 400. On a new database `?id=item:abc` answers no messages.
- A run posted with `threadId` `t-other` answers 400; a run on `item:teardown` streams `threadId: "item:teardown"`.

Test: `specs/srv/tests/srv.rs`.

### SRV-MEMORY-CRUD: Memory routes
**Applies to:** hoardini-server
**Verification:** test

`/api/memory` lists, creates (source `user`), searches (`?q=`, full-text), updates, and deletes memories; unknown ids answer 404. Each row carries `detail` (MEM model): null for notes, the object for location and file-type rows.

**Acceptance criteria:**
- Create → list shows it → `?q=` finds it, an unrelated query finds nothing → `PUT` changes text and kind → `DELETE` → list empty; `PUT`/`DELETE` of a missing id → 404.
- The created note's `detail` is null; a file-type row written for `q4nx` lists with `detail.category` `model-weights` and `detail.app` `FastFlowLM`.

### SRV-PROPOSAL-SELECTION: The server keeps a pending card's ticks
**Applies to:** hoardini-server
**Verification:** test

`PUT /api/proposals/{id}/selection {item_ids}` shall tick exactly the listed items of a pending proposal and untick its
other pending items (`proposals::select`), mark the proposal `touched` (REVIEW-SUPERSEDE), and answer the proposal's
display JSON, so a review's Ready to free (REVIEW-TRIGGER) follows the user's ticks. An id that is not a pending item of
the proposal, or a proposal that is not pending, answers 409 and changes nothing; an unknown proposal answers 404.
Approving or declining a proposal also marks it `touched`.

**Acceptance criteria:**
- Of two unticked items, `PUT {item_ids: [a]}` answers `touched: true` with `a` ticked and `b` not, and `GET /api/proposals/{id}` shows the same items.
- An unknown item id answers 409, an item blocked with the reason "Not found" answers 409 (not 404), an unknown proposal 404, and after a decline the route answers 409.
- An approved proposal shows `touched: true`.

Test: `specs/srv/tests/srv.rs`.

### SRV-LOCATION-ANSWER: What is this? answers
**Applies to:** hoardini-server
**Verification:** test

`POST /api/locations/answer {path, answer}`, with `answer` one of `keep`, `disposable`, `move-elsewhere` and
`never-touch` and `path` absolute or under `~`, shall record the answer with `location::answer` (MEM-LOCATION) and answer
the stored memory row. Every stored review area at that path then shows the answer's label, category, verdict, actions
and text as `why_not`, is no longer `unknown`, and is marked `user` (REVIEW-AREAS). A missing path or an unknown answer answers 400, and so does a path `location::answer`
refuses (MEM-LOCATION: not absolute, a volume root, the profile, a container, a catalog `container` entry, or a system
area); a refused answer writes nothing.

**Acceptance criteria:**
- Keep for the unknown `C:\code\covenant-agent` area answers a pinned `user` row keyed `C:\code\covenant-agent` with label `yours`, verdict `keep` and actions `[archive, compress]`; `GET /api/review` then shows that area with label `yours`, `unknown: false`, verdict `keep` and why_not `You keep this folder; it may still be moved or compressed.`
- `maybe` as the answer, or no path, answers 400; Never touch on `C:\code\covenant-agent\data` leaves a `user` protect note on it.
- `code\covenant-agent`, `C:\code\..\Windows`, `C:\` (keep or disposable), `~`, `C:\Users`, `~\AppData\Local`, `~\.lmstudio`, `C:\Windows\Temp` and `C:\Program Files\Contoso` answer 400, and the memory list is unchanged.

Test: `specs/srv/tests/srv.rs`.

### SRV-ACT: An action on Home runs at once
**Applies to:** hoardini-server
**Verification:** test

`POST /api/act {kind, paths, destination_id?}`, with `kind` one of `delete`, `compress`, `online_only` and `archive` and `paths` absolute or under `~`, shall make one proposal of those paths (conversation `home`, compress as `lzx`) through the policy gate and approve at once every item the gate left pending (UI-DETAIL: the answer is the action). It answers `{proposal_id, job_id, blocked}`, where `blocked` lists each path the gate held back or refused with its reason, paths shown with the profile as `~`. When the gate leaves nothing to run, no job starts, `job_id` is null, and a proposal that was made is declined so nothing is left pending. Any other `kind`, `archive` without a `destination_id`, or no usable path answers 400.

**Acceptance criteria:**
- Deleting `go.bin` and a protected `kept.bin` together starts a job that deletes `go.bin` without a second step; `blocked` names `kept.bin` with a reason, and it stays.
- Deleting only `kept.bin` answers `job_id` null with one blocked entry, and its proposal, if made, is `declined`.
- `dedupe`, `archive` without a destination, and a blank path answer 400.

Test: `specs/srv/tests/srv.rs`.

### SRV-LOCATION-FORGET: Bring back
**Applies to:** hoardini-server
**Verification:** test

`POST /api/locations/forget {path}` shall delete the user's own answer for that folder: its `user` location row (MEM-LOCATION) and any `user` protect note keyed to the same path, and answer `{forgot: n}`. Rows Hoardini wrote stay. A blank path answers 400. Home's Bring back calls it, then builds a new review so the folder is judged again (UI-HOME).

**Acceptance criteria:**
- After a Never touch answer for a folder (a verdict row and a protect note), forget answers `forgot` 2 and the memory list holds no `user` path row for it; forgetting again answers 0.
- A blank path answers 400.

Test: `specs/srv/tests/srv.rs`.

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
**Verification:** test

Aborting the `POST /api/agent/run` request (the Ask box's Stop, UI-ASK) or shutting the server down shall abort the agent task, which drops its model request, and decrement `running_runs`. There is no cancel route.

**Acceptance criteria:**
- Against a model that keeps streaming text, `running_runs` is 1 after the first `TEXT_MESSAGE_CONTENT`; once the client drops the stream, `running_runs` is 0 within 5 s.

Test: `specs/srv/tests/srv.rs`.

### SRV-APPS: Rarely used apps and their uninstall route
**Applies to:** hoardini-server
**Verification:** test

The review answer (REVIEW-TRIGGER) carries `apps`: the apps of ACT-APPS-RARELY-USED whose folder lies in the review's scope, each `{id, name, publisher, source, path, size, last_run_ms, last_run_from}` (paths shown with the profile as `~`), read fresh from the registry, UserAssist, and Steam for each answer. `POST /api/apps/{id}/uninstall` starts the uninstaller registered for that id (ACT-UNINSTALL) and answers `{started: true, id, name}`; an id no installed app has answers 404 and starts nothing. The request body is ignored, so a client can never name the command. The app list and the launcher come from `apps::source()`, which tests replace through `apps::set_source`.

**Acceptance criteria:**
- With a fake app source (Big Tool, 3 GiB, run 200 days ago; Busy, 3 GiB, run yesterday) over an indexed fake `D:\` selected as the drive: its review lists exactly one app, Big Tool, with its `reg-` id, publisher, `source: "registry"`, path `D:\Apps\Big`, size, last run, and `last run of d:\apps\big\big.exe`; with the fake `C:\` selected instead, the review lists none.
- POSTing its id with a body naming `calc.exe` answers `{started: true, id, name: "Big Tool"}` and launches exactly `D:\Apps\Big\unins000.exe` with `/S`, its registered uninstall string.
- POSTing `reg-0000000000000000` answers 404 and launches nothing more.

Test: `specs/srv/tests/srv.rs`.

## Retired

### SRV-SHOW-IN-EXPLORER: Show in Explorer
**Applies to:** hoardini-server
**Verification:** test

`POST /api/reveal {path}` shall run `explorer.exe /select,"<path>"` (the path quoted after the comma) and answer `{ok: true}` without waiting for Explorer. `path` may start with `~`. It starts nothing and refuses: a missing or empty path, a path that is not an absolute drive path, contains `"`, or has a `.` or `..` component (400); a path that is not a watched drive (INDEX-VOLUMES) or under one (403); a path that does not exist (404). The refusals are tested; opening Explorer is checked by hand.

**Acceptance criteria:**
- With `indexed_volumes` set to one scratch folder: `{}`, `{path: ""}` and `{path: "relative\x.txt"}` are 400; an existing file in another folder is 403; `<root>\..\<other>\x.txt` is 400; `<root>\missing.txt` is 404; every refusal is a JSON `error`.

**Verification (manual, Explorer):**
1. Start `hoardini serve --port 72xx --token dev --no-refresh` with `HOARDINI_DATA_DIR` and `--db` in a scratch folder, and watch a scratch folder holding `a b\c.txt` (`PUT /api/volumes`).
2. `POST /api/reveal?token=dev` with `{"path": "<folder>\a b\c.txt"}`: Explorer opens `a b` with `c.txt` selected, and the answer is `{"ok":true}`.

These IDs are not reused. Retired 2026-10-16 with the chat and the old cards (simpler-model plan): SRV-COPY-CHECKS, SRV-HISTORY-RANGE, SRV-REVEAL, SRV-LEARNED, SRV-RUN-CANCEL. Retired earlier the same day with Explore: SRV-INDEX-MAP, SRV-EXPLORE-FOCUS.
