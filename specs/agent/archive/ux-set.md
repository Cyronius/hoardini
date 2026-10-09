# UX revisions WP-SET: redaction always on, web search follows its provider (agent)

Source: `.claude/plans/ux-revisions.md` §6, §7.

## Modified requirements

- **AGENT-REDACT** (test): always on. The provider profile loses `redact_paths`; a stored profile that still carries `redact_paths: false` loads and is redacted anyway. The "redaction off" acceptance case becomes that stored-profile case.
- **AGENT-WEB-SEARCH** (test): offered whenever a `search_provider` row is active; `meta.web_search_enabled` is gone. The tool's off-state error points at the Search tab. Acceptance adds a direct call with no active provider.

## New requirements

None.

## Removed requirements

None.
