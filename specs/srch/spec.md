# SRCH: Web search

Canonical spec for `crates/agent/src/search/`. Design reference: `hoardini-design.md` section 5.10.

### SRCH-PROVIDERS: Five search providers
**Applies to:** hoardini-agent, hoardini-server
**Verification:** manual

Tavily (default), Brave, Exa, Serper and SearXNG shall each return normalized results (`title`, `url`, `snippet`; at most 6 results, snippets at most 300 characters). Results are untrusted web content and reach the model only inside the `<<<UNTRUSTED_WEB_CONTENT` delimiters. SearXNG shall surface a clear error when its server does not answer JSON.

**Verification (manual):**
1. Open the drawer, Search tab. For each of Tavily, Brave, Exa and Serper paste a key and press Test: the row shows OK with a latency. For SearXNG enter the base URL and press Test.
2. Point SearXNG at an instance without `json` in `search.formats`: Test shows the message naming `settings.yml`.
3. Press Test with a wrong key: the row shows the provider's HTTP error and the key does not appear in it.
4. Activate a provider, enable web search in Settings, and ask the agent about an unfamiliar folder name. The chat shows a search tool chip and an answer citing a source URL.

Parsing of each provider's response shape is covered by the recorded fixtures in `specs/srch/fixtures/` (see `specs/srch/tests/srch.rs`); that is a regression net, not the verification of this requirement.

### SRCH-QUERY-HYGIENE: Queries leave the machine without personal details
**Applies to:** hoardini-agent
**Verification:** test

Before a query is sent or cached, `sanitize_query` shall remove the profile path and the user name, reduce every absolute path to its last two components, and cap the query at 200 characters.

**Acceptance criteria:**
- With profile `C:\Users\tester`, `"unsloth_compiled_cache" folder safe to delete` is unchanged.
- `is C:\Users\tester\AppData\Local\unsloth_compiled_cache safe to delete for tester` becomes `is Local/unsloth_compiled_cache safe to delete for`.
- `c:/users/TESTER/.cache/huggingface/hub` (any case, forward slashes) becomes `huggingface/hub`; the bare profile path becomes empty.
- `"D:\Program Files\Vendor\cache\blobs"` becomes `"cache/blobs"`; `\\nas\share\media\old.iso` becomes `media/old.iso`.
- A 1,000-character query is capped at 200 characters.
- The URL of the request built for a sanitized query contains neither `tester` nor `Users`.

### SRCH-CACHE: Thirty-day result cache
**Applies to:** hoardini-agent
**Verification:** test

Results shall be cached in `search_cache` keyed by provider and sanitized, case-folded query, for 30 days.

**Acceptance criteria:**
- Against a local SearXNG stand-in, the first query hits the server once; the same query (different case, padded with spaces) 29 days later is served from the cache with identical results and no second request.
- The same query 31 days after the first hits the server again (the clock is injected through `Ctx.now_ms`).
