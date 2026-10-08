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

`PUT /api/secrets` shall store the value in the vault and return only `{"ok":true}`. Provider `api_key` must be a `secret://` reference: a raw key is rejected (400) and never written to `provider.config_json`. `has_key` is derived from the vault. Error responses for `/api/secrets` and provider writes never quote the submitted value, and request bodies and query strings are never logged.

**Acceptance criteria:**
- After storing `sk-…` and creating a provider that references it, `GET /api/providers` has `has_key: true` and the response text does not contain the value; `provider.config_json` contains the ref and not the value.
- `POST /api/providers` with `api_key: "sk-…"` → 400 and the response does not contain the value.
- A malformed `PUT /api/secrets` body → 4xx and the response does not contain the submitted text.

### SRV-ONE-REFRESH: One index refresh at a time; "Scan fast" takes over
**Applies to:** hoardini-server
**Verification:** test

The server shall run at most one indexer child at a time. `POST /api/index/refresh` answers `{started:false}` while a refresh runs, except that `fast` during a `background` refresh stops the child and restarts the same volume in fast mode (`{started:true}`). Progress lines from the child appear in `Status.index.progress`. The child is killed on shutdown.

**Acceptance criteria:**
- With a fake indexer: `background` → started; a second `background` → not started; status shows `refreshing` with the child's progress.
- `fast` during that refresh → started; the indexer was spawned exactly twice, in modes `background` then `fast`; a second `fast` → not started.

**Verification (manual, shutdown):**
1. `hoardini serve --port 7043 --token dev` and POST a `background` refresh.
2. Press Ctrl+C; in Task Manager confirm no `hoardini.exe index` child remains.

### SRV-SETTINGS: Settings round trip
**Applies to:** hoardini-server
**Verification:** test

`GET /api/settings` returns the meta settings with their defaults (threshold 1048576, redact_paths true, web_search_enabled false, indexer_workers 2, indexer_memory_cap_mb 256, builtin_idle_minutes 10) plus read-only `index_size_bytes` (database plus WAL size). `PUT` stores known keys, ignores `index_size_bytes`, and rejects wrongly typed values with 400.

**Acceptance criteria:**
- Defaults as listed; after `PUT {threshold_bytes: 2097152, redact_paths: false, indexer_workers: 4, index_size_bytes: 1}` a `GET` shows the new values and `index_size_bytes` is not 1.
- `PUT {redact_paths: "yes"}` → 400.

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
