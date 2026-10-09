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

`/api/memory` lists, creates (source `user`), searches (`?q=`, full-text), updates, and deletes memories; unknown ids answer 404.

**Acceptance criteria:**
- Create → list shows it → `?q=` finds it, an unrelated query finds nothing → `PUT` changes text and kind → `DELETE` → list empty; `PUT`/`DELETE` of a missing id → 404.

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

### SRV-APPS: Rarely used apps and their uninstall route
**Applies to:** hoardini-server
**Verification:** test

`GET /api/apps/rarely-used` returns the list from ACT-APPS-RARELY-USED as a JSON array of `{id, name, publisher,
source, path, size, last_run_ms, last_run_from}` (paths shown with the profile as `~`), read fresh from the registry,
UserAssist, and Steam on each call. `POST /api/apps/{id}/uninstall` starts the uninstaller registered for that id
(ACT-UNINSTALL) and answers `{started: true, id, name}`; an id no installed app has answers 404 and starts nothing. The
request body is ignored, so a client can never name the command. The app list and the launcher come from
`apps::source()`, which tests replace through `apps::set_source`.

**Acceptance criteria:**
- With a fake app source (Big Tool, 3 GiB, run 200 days ago; Busy, 3 GiB, run yesterday) over an indexed fake `D:\`: the GET returns exactly one entry, Big Tool, with its `reg-` id, publisher, `source: "registry"`, path `D:\Apps\Big`, size, last run, and `last run of d:\apps\big\big.exe`.
- POSTing its id with a body naming `calc.exe` answers `{started: true, id, name: "Big Tool"}` and launches exactly `D:\Apps\Big\unins000.exe` with `/S`, its registered uninstall string.
- POSTing `reg-0000000000000000` answers 404 and launches nothing more.

### SRV-RUN-CANCEL: A cancelled run ends with a valid event sequence
**Applies to:** hoardini-server
**Verification:** test

`POST /api/agent/runs/{runId}/cancel` shall end that run's stream with `RUN_FINISHED`, after closing whatever the aborted agent task left open (`TEXT_MESSAGE_END` for an open text message, the thinking end events, `TOOL_CALL_END` for an open call), so a client that checks AG-UI ordering accepts it. `running_runs` drops back.

**Acceptance criteria:**
- Against a model that streams one text chunk and then stalls: `running_runs` is 1 after the first `TEXT_MESSAGE_CONTENT`; after the cancel the stream ends with `TEXT_MESSAGE_END`, `RUN_FINISHED`, and `running_runs` is 0.
