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
