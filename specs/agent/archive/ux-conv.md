# UX revisions, WP-CONV: one continuous conversation (agent)

Design: `.claude/plans/ux-revisions.md` §1, §2; contract: `.claude/plans/ux-revisions-impl.md` (Conversation block).

## New requirements

- **AGENT-CONTEXT-WINDOW** (test): before each model request the loop estimates the request at 4 characters per token (system prompt, tool definitions, window messages as JSON) against the profile's `context_window` (8,192 when 0). Over 75%, the window start moves to the first user message that brings the estimate to 40% or less (else the last user message) and is stored. At or under 75% it never moves, so consecutive requests share their message prefix.
- **AGENT-SESSION-GAP** (test): a new user message more than 6 h after the previous stored message moves the window start to itself. The clock is `AgentEnv::clock`, so tests set it.

## Modified requirements

- **AGENT-HISTORY**: one conversation (`main` on the server); requests are the system prompt plus messages from `conversation.window_start_seq` on; a proposal answer for a call before the window start reaches the model as a user note. The title rule is removed (`thread::title_from` goes away).
- **AGENT-OVERFLOW**: after tool-result shedding fails or is used up, move the window start forward one user turn, store it, and retry; the last resort `OVERFLOW_MESSAGE` says "start fresh".
- **AGENT-PROMPT-CONTEXT**: the system prompt adds one line per pending proposal of the conversation (ID, kind, target, size). The memory sentence is WP-MEM's.
- **AGENT-APPROVAL-KEYED**: the reload criterion names `GET /api/thread` instead of `GET /api/threads/{id}`.

## Removed requirements

None.
