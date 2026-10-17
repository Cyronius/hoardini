# PROV: LLM providers

Canonical spec for the OpenAI-compatible provider client in `crates/agent/src/provider/`. Design reference: `hoardini-design.md` section 5.8.

### PROV-COMPAT: Lenient streaming across server types
**Applies to:** hoardini-agent
**Verification:** test

The client shall assemble a turn (text, reasoning, tool calls, finish reason, usage) from the recorded or faithful SSE streams of OpenAI, OpenRouter, Ollama `/v1`, LM Studio, llama.cpp and DeepSeek/Cerebras-style servers, with the stream split at arbitrary byte boundaries. An HTTP 200 response (JSON body or SSE data) that carries an `error` object shall surface as an error, never as an empty turn.

**Acceptance criteria:**
- Every fixture in `specs/prov/fixtures/*.sse` assembles to the same result when fed in 1-byte, 7-byte and whole-file chunks.
- OpenAI fixture: two tool calls accumulated by `index` (`call_aaa` args `{"drive":"C"}`, `call_bbb` args `{"drive":"D"}`), usage 71/38/109 taken from the trailing chunk with empty `choices`.
- OpenRouter fixture: `: OPENROUTER PROCESSING` comments are ignored; the `reasoning` field fills `reasoning`; text is `Your disk is fine.`.
- DeepSeek fixture: `reasoning_content` fills `reasoning`, null `content` deltas are ignored.
- Ollama fixture: a whole call in one chunk yields one call with id `call_k1x9m2ab`.
- llama.cpp fixtures: a call without `index` or `id` gets a generated `call_<uuid>` id; a following call with a new id and no index is a second call; `arguments` sent as a JSON object is re-serialized to a string.
- LM Studio recordings: the text stream yields non-empty text; the tool stream yields one valid-JSON call; the parallel stream yields two calls with distinct ids; the unknown-model recording still assembles (LM Studio answers from a different loaded model with HTTP 200).
- `error-200.sse`, and a mock server that answers HTTP 200 with a JSON `{"error": ...}` body, make `chat_stream` and `chat` return `ProviderError::Api` carrying the server's message.

### PROV-EXTRA-BODY: Raw request-body merge
**Applies to:** hoardini-agent
**Verification:** test

`extra_body` from the provider config shall be deep-merged into the request JSON after every other field is built. Objects merge recursively; scalars and arrays replace.

**Acceptance criteria:**
- `extra_body {"reasoning_effort": "low"}` adds `reasoning_effort` to the body.
- `extra_body {"provider": {"order": ["A"]}}` merged over an existing `provider: {"sort": "price"}` yields `{"sort": "price", "order": ["A"]}`.
- `extra_body {"temperature": 0.9}` overrides `params.temperature` 0.2.
- `extra_body {"stop": ["x"]}` replaces an existing array value rather than appending.

### PROV-MODEL-CHECK: Test connection catches unknown model ids
**Applies to:** hoardini-agent
**Verification:** test

Test connection shall compare the configured model id with `GET {base}/models` and report an unlisted id as a failure with up to three near-match suggestions, because servers such as LM Studio answer an unknown id with HTTP 200 from a different loaded model. When the id is listed it shall probe tool support and fill `caps`.

**Acceptance criteria:**
- Against a mock server listing `qwen3-8b-instruct`, `llama3.2:3b`, `qwen3-4b`, a configured model `qwen3-8b-instrcut` yields `ok == false`, `model_found == Some(false)`, `suggestions[0] == "qwen3-8b-instruct"`, at most 3 suggestions, and `caps == None` (no chat request is made).
- A configured model that is listed yields `ok == true`, `model_found == Some(true)`, and `caps` with `tools`, `streaming`, `streamed_tool_calls` and `tool_choice` all true against a mock that supports them.
- A `/models` endpoint answering 404 yields `model_found == None` and does not fail the test by itself.
- `suggest_models` ranks a case-differing id, a substring match and a token-overlap match above unrelated ids.

### PROV-SANITIZE: Request sanitizer
**Applies to:** hoardini-agent
**Verification:** test

Outgoing requests shall omit a blank or whitespace-only `model`, and shall strip `reasoning_content` and `reasoning` from assistant messages in the history (Cerebras and DeepSeek reject them on the second turn).

**Acceptance criteria:**
- `model: None`, `model: ""` and `model: "  "` produce a body with no `model` key; `model: "m"` keeps it.
- An assistant message carrying `reasoning_content` is serialized without it; its `content` and `tool_calls` are unchanged.
- User and tool messages are unchanged.
- `stream_options.include_usage` appears only when the request opts in and `stream` is true.

## Built-in model (design 5.12)

The "Built-in" provider runs a pinned GGUF, or a GGUF the user points at (model id `file:<path>`, PROV-BUILTIN-FILE), in a `llama-server` child. Code: `crates/server/src/builtin.rs` (supervisor, download job, routes `/api/builtin*`), UI `ui/src/drawer/BuiltinCard.tsx`. Requirement tests: `specs/prov/tests/builtin.rs` (part of `--test specs prov::`), against a fake launcher that stands in for llama-server. Download and hash checks are RUNTIME-PINNED-HASH. The choice of the two pinned models is AGENT-EVAL-GATE.

### PROV-BUILTIN-LAZY: The child starts on the first chat turn
**Applies to:** hoardini-server
**Verification:** test

Nothing shall start until a run (or a connection test) uses the Built-in profile. The run then verifies the runtime and model against their pins, starts the child, waits for `/health`, and sends the request to the child's loopback address, which the provider row does not store. Later runs reuse the running child. When the runtime or model is not installed the run ends with a `RUN_ERROR` that says so, and nothing starts.

**Acceptance criteria:**
- A supervisor with a fake launcher has launched nothing before `acquire`; the first `acquire` launches once and returns `http://127.0.0.1:<port>/v1`; a second `acquire` returns the same address without launching.
- A server whose active provider is the Built-in row (runtime and model present) reports `running: false` and no launch before the first run; `POST /api/agent/run` then streams the child's answer and `RUN_FINISHED`, the launcher ran once, and `GET /api/builtin` reports `running: true`.
- With the Built-in row active but no files installed, the run streams `RUN_ERROR` containing "not installed" and nothing is launched.

### PROV-BUILTIN-IDLE: The child exits after the idle timeout
**Applies to:** hoardini-server
**Verification:** test

The child shall stop once no run has used it for `builtin_idle_minutes` (default 10, read at every check; 0 stops it as soon as it is idle), and never while a run holds it. A watchdog checks every 15 seconds, and it exists only while a child does: it starts when a child is launched and exits once the child is gone (stopped as idle, crashed, or removed), so an app with no child has no watchdog timer (SRV-IDLE-ZERO).

**Acceptance criteria:**
- While a run holds the child, an idle check one hour later does not stop it.
- After the run ends, a check at 9 minutes leaves it running and a check at 10 minutes 1 second kills it and reports `running: false`.
- With the setting at 0, the first check after the run ends stops it.
- On a paused clock: no watchdog before the first acquire, even after 45 s; one after it, which leaves a held child alone; with the setting at 0, the first tick after the run ends stops the child and the watchdog is gone. A new child brings it back. With the setting at 10, a child that crashes ends the watchdog at the next tick.

### PROV-BUILTIN-RESTART: A crashed child is restarted when next needed
**Applies to:** hoardini-server
**Verification:** test

When the child has exited on its own, the next run shall start a fresh one rather than fail against a dead address. Switching the chosen model stops the old child before starting the new one.

**Acceptance criteria:**
- After the fake child exits, the next `acquire` launches a second child at a different address.
- Acquiring model `a` then model `b` launches twice and kills the first child.

### PROV-BUILTIN-CHILD: Loopback child, CPU or GPU per model, sized to the machine
**Applies to:** hoardini-server
**Verification:** test

The child shall run as `llama-server.exe -m <gguf> --host 127.0.0.1 --port <random> --jinja -t <physical cores - 2, at least 1> -c 8192 --no-webui -np 1 --cache-ram 0 <backend flags> -a <model id>`, with `CREATE_NO_WINDOW` and normal priority, inside a kill-on-close job object so it dies with the app. A CPU model (Granite 4.1 3B) runs on the CPU build with `--cache-type-k q8_0 --cache-type-v q8_0 -ngl 0`. A GPU model (Qwen3.5 9B) and a user's own file run on the Vulkan build with `-ngl 99` and the default KV cache; that build carries the CPU backends, so it falls back to the CPU when there is no GPU. The Built-in profile is `small: true` (paths are redacted for every provider, AGENT-REDACT).

**Acceptance criteria:**
- `builtin::args` puts `127.0.0.1` after `--host`, the port after `--port`, the GGUF after `-m`, `0` after `-ngl`, 8192 after `-c`, `physical_cores() - 2` (at least 1) after `-t`, and includes `--jinja` and `--no-webui`, for a CPU spec.
- For a GPU spec `-ngl` is `99` and there is no `--cache-type-k`.
- `builtin::runtime_for` is the Vulkan pin for `qwen3.5-9b` and for any `file:` id, and the CPU pin for `granite-4.1-3b`.
- `builtin::profile(id)` is small.

### PROV-BUILTIN-STATUS: The download card's facts
**Applies to:** hoardini-server
**Verification:** test

`GET /api/builtin` shall return `{installed, running, runtime, custom_runtime, model, candidates, free_bytes, recommended, install_job, provider_id, threads, context}`. A runtime is `{id, version, size, installed, license, license_url}`; `runtime` is the one the chosen (else the recommended) model runs on, `custom_runtime` is the Vulkan one a user's own file uses. A candidate is `{id, name, size, license, license_url, installed, recommended, needs_gpu, runtime}`; `model` is the chosen candidate or `{..., custom: true, path}` for a user's file. Sizes come from the pins and `free_bytes` from the data folder's volume. `POST /api/builtin/install {model_id}` rejects an unknown id, and a `file:` path that is not a file, with 400.

**Acceptance criteria:**
- On an empty data folder: `installed` false, `model` null, runtime size and version equal `LLAMA_SERVER` (the recommended Granite's), one candidate per `runtime::MODELS` entry in order with its size, an `https://` license URL, a named license, `needs_gpu` and its runtime id, `custom_runtime.id` equal to the Vulkan pin, `free_bytes` > 0, and `recommended` equal to `builtin::RECOMMENDED`.
- `POST /api/builtin/install {"model_id": "not-a-model"}` returns 400, and so does `{"model_id": "file:C:\\no\\such\\model.gguf"}`.

### PROV-BUILTIN-REMOVE: Remove gives the space back
**Applies to:** hoardini-server
**Verification:** test

`DELETE /api/builtin` shall stop the child, delete the runtime folder and every installed model with its install record, delete the Built-in provider row (clearing it as the active provider), and return the bytes freed. It is refused with 409 while a download runs.

**Acceptance criteria:**
- After a run started the child, `DELETE /api/builtin` returns `ok` and `freed_bytes` at least the size of the stand-in files, the child is dead, the runtime and model files are gone, `GET /api/providers` has no `builtin` row, and `GET /api/builtin` reports `installed: false` and `provider_id: null`.

### PROV-BUILTIN-DOWNLOAD: Two models to download, on demand
**Applies to:** hoardini
**Verification:** manual

Nothing shall be downloaded until the user clicks a download button. First-run (UI-FIRST-RUN) and Settings, AI list the two models, Qwen3.5 9B ("needs a graphics card") and Granite 4.1 3B ("runs on the CPU"), each with its download size (runtime plus model, less what is already installed), a one-line description and a Download button that is disabled without enough free space. While the job runs the list gives way to progress from the status stream and a Cancel button; when it is done the Built-in row exists, is made active, and the first chat turn starts the child. Once a model is installed Settings shows the "Built-in model" card with its state, size and license link, and Remove.

**Verification (manual):**
1. Open Settings, AI with nothing installed: "Run a model on this PC" lists both models with sizes (Qwen3.5 9B 5.6 GB plus the 33 MB Vulkan runtime, Granite 4.1 3B 2.0 GB plus the 19 MB CPU runtime) and the file box.
2. Click Download on Granite: a progress bar advances (`builtin_install` job frames on `/api/status/stream`); Cancel stops it and leaves no `.part` file in the data folder.
3. Download again to the end: the card changes to "Built-in model", Granite 4.1 3B, "idle", the provider list gains "Built-in", and the first chat turn on it answers after the model loads.
4. Click "Remove built-in model" and confirm: the card reports the freed size and offers the list again, and `%LOCALAPPDATA%\hoardini\runtime\` and `models\` no longer hold the files.
5. With a GPU, download Qwen3.5 9B: it runs on the GPU (Task Manager shows GPU memory use by `llama-server.exe`) and answers a tool question.

### PROV-BUILTIN-FILE: A model file the user points at
**Applies to:** hoardini-server, hoardini-ui
**Verification:** manual

First-run and Settings, AI also offer "A model file on this PC": a path box with Browse (a native file dialog, `POST /api/dialog/pick-model`, filtered to `.gguf`, 501 in headless mode) and "Use this file". The file runs through the same child as a model id `file:<path>`, on the Vulkan build (GPU when there is one, else CPU), with its own chat template and the 8192 context. Only the runtime is downloaded; the file is not hashed, copied or checked for tool support or memory, and Remove never deletes it. A start failure shows llama-server's last log lines.

**Verification (manual):**
1. Choose "A model file on this PC", Browse, pick a `.gguf` (for example `~\.lmstudio\models\...`), and Use this file: the Vulkan runtime downloads if it is not there, the Built-in row appears and is active, and the card reads the file's name with "your file" as the license.
2. A chat turn starts the child with `-ngl 99` (see `logs\llama-server.log`) and answers.
3. Point at a path that is not a file: Use this file reports an error and installs nothing.
4. Remove built-in model: the file is still on disk.

### PROV-BUILTIN-OFFLINE: No outbound traffic with web search off
**Applies to:** hoardini
**Verification:** manual

With the Built-in profile active and web search off, a chat session shall make no outbound network connection: the app talks to the child on 127.0.0.1 only.

**Verification (manual):**
1. Install the built-in model, activate the Built-in profile, and confirm Settings shows web search off.
2. Open Resource Monitor, Network, TCP Connections (or run `netstat -ano` in a loop) filtered to the PIDs of `hoardini-app.exe` / `hoardini.exe` and `llama-server.exe`.
3. Ask three questions that use tools ("what's taking space?", "what's in LM Studio and what can go?", "what grew since last week?").
4. Expect only connections between 127.0.0.1 ports (the UI to the server, the server to llama-server) and no remote address for either process. Optionally disconnect the network adapter and repeat: the Ask box still answers.

### PROV-BUILTIN-EXIT: The child never outlives the app
**Applies to:** hoardini
**Verification:** manual

Closing the app, or the app crashing, shall end the child.

**Verification (manual):**
1. Chat once on the Built-in profile so `llama-server.exe` runs (Task Manager, Details).
2. Close the app normally: `llama-server.exe` disappears within a few seconds.
3. Start it again, chat once, then end `hoardini-app.exe` from Task Manager: `llama-server.exe` disappears too (kill-on-close job object).

### PROV-RATE-LIMIT: HTTP 429 waits and retries; a daily quota ends the run
**Applies to:** hoardini-agent
**Verification:** test

On HTTP 429 the client shall wait and retry the same request, at most 3 attempts in all. The wait is `Retry-After` (seconds or an HTTP date) when present, else 2 s then 4 s, and never more than 60 s. Before each wait it calls the registered notice with the seconds, and the run loop turns that into `CUSTOM rate_limit_wait {seconds}`. A 429 whose body names a daily limit (`per day`, `per-day`, `PerDay`, `daily`) shall not wait: it fails at once with `ProviderError::DailyQuota`, which the run reports as "the free tier is used up for today" and suggests another provider. Three 429s in a row fail with `ProviderError::RateLimited`. Sleeping is injectable (`Client::with_sleeper`).

**Acceptance criteria:**
- A mock answering 429 with `Retry-After: 7` and then 200: `chat` returns the turn, 2 requests were made, the sleeper saw exactly 7 s and the notice saw 7.
- `Retry-After` as an HTTP date 30 s ahead: the sleeper sees 27 to 30 s (streaming path).
- `Retry-After: 3600` sleeps 60 s.
- Three 429s without `Retry-After` give `RateLimited` after 3 requests with sleeps of 2 s and 4 s.
- Bodies mentioning OpenRouter `free-models-per-day`, Gemini `RESOURCE_EXHAUSTED` with `...PerDay...`, and Cerebras "Tokens per day limit exceeded" each give `DailyQuota` after 1 request and no sleep.

### PROV-ZERO-DEFAULTS: A blank timeout, step limit, or context means the default
**Applies to:** hoardini-agent, hoardini-server
**Verification:** test

A provider saved with `timeout_s`, `max_tool_steps`, or `context_window` equal to 0 (what the setup pop-up and the AI tab's form send for a blank field) shall be stored and used with the defaults: 120 s, 20 steps, and 32,768 tokens. A row stored with zeros before this rule loads with the defaults too.

**Acceptance criteria:**
- `POST /api/providers` with all three at 0 answers with 120, 20, and 32768.
- A stored row whose config is set to 0 for all three is listed by `GET /api/providers` with 120, 20, and 32768.

Found 2026-10-09: a first-run "A model on this PC" provider had a 1 s timeout, so every chat ended with "provider timed out ... operation timed out".

### PROV-PRESETS: Free-tier presets, and no implicit provider
**Applies to:** hoardini-agent, hoardini-server
**Verification:** test

`presets()` shall include the six free providers that first run offers (UI-FIRST-RUN), each with a default model and its key-page URL: Google Gemini (`https://generativelanguage.googleapis.com/v1beta/openai/`, a Flash model; Flash-Lite, since Flash allows only about 20 free requests a day), OpenRouter (a `:free` model that no NVIDIA trial terms cover, so it works for people who block hosts that train), Mistral, Z.ai, Ollama Cloud and Cloudflare Workers AI (whose base URL carries an `{account_id}` placeholder that the setup form fills in). Cerebras (`https://api.cerebras.ai/v1`, default model `gpt-oss-120b`) stays as a preset, but first run no longer offers it as free: its free offer is now a $5 trial that needs a card. The server shall never create a provider by itself: a run with no provider ends with `RUN_ERROR` and leaves `GET /api/providers` empty and no active provider. `GET /api/providers/local` lists running local servers (LM Studio, Ollama, llama.cpp, vLLM) by probing each preset's `/models` with a 1 s timeout. Local presets use `127.0.0.1`, not `localhost`: on Windows `localhost` tries `::1` first, which these servers don't listen on, and that refusal alone takes about 2 s. `GET /api/setup` reports `meta.ai_setup_done` (an install that already has a provider counts as done); `POST /api/setup/done` sets it.

**Acceptance criteria:**
- The preset list contains `cerebras` and `gemini` with the base URLs above, a default model for each, and `needs_key`.
- `gemini`, `openrouter`, `mistral`, `zai`, `ollama-cloud` and `cloudflare` each have a default model, a key-page URL and `needs_key`; Cloudflare's base URL contains `{account_id}`.
- OpenRouter's default model ends in `:free`; Ollama has no default model.
- On a fresh data dir `GET /api/setup` is `{done:false}`; `POST /api/agent/run` streams `RUN_ERROR`, `/api/providers` stays `[]`, `/api/status` has no active provider; after `POST /api/setup/done` it is `{done:true}`.
- After creating a provider, `GET /api/setup` is `{done:true}` on an otherwise fresh data dir.
