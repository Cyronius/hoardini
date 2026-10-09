# UX revisions, WP-DRIVES: drive selector (agent)

Design: `.claude/plans/ux-revisions.md` §11.

## Modified requirements

- **AGENT-PROMPT-CONTEXT** (test): the index part of the system prompt starts with `The user is looking at <selected root>` and carries one summary per watched drive: the selected drive's in full (age, free space, biggest folders, changes since the refresh before), each other watched drive's compact (age, free space, five biggest folders). Small profiles get the selected drive only. A watched drive with no completed index says so. When no index lies on any watched drive (a test fixture elsewhere) the most recently refreshed index is summarized as before.

## New requirements

None.

## Removed requirements

None.
