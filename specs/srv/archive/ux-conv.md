# UX revisions, WP-CONV: conversation routes (srv)

Design: `.claude/plans/ux-revisions.md` §1, §2; contract: `.claude/plans/ux-revisions-impl.md` (Conversation block).

## New requirements

- **SRV-THREAD** (test): the route list's conversation routes. `GET /api/thread` returns `main` with `window_start_id`, `has_hidden` and `fresh`; `POST /api/thread/fresh` moves the window past the last message; `DELETE /api/thread` deletes messages only (proposals, memories, jobs and the journal stay) and is refused while a run is in progress; `POST /api/agent/run` always uses `main`. `GET /api/threads`, `GET /api/threads/{id}` and `DELETE /api/threads/{id}` are gone.

## Modified requirements

None. (`SRV-TOKEN` walks `route_list()`, which now carries the new routes.)

## Removed requirements

None.
