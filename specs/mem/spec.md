# Memory (prefix MEM)

Path-keyed notes the agent and the user leave about the disk. Stored in the `memory` table (SQLite, FTS5 for free text).
Canonical design: `.claude/plans/hoardini-design.md` §5.7. Code: `crates/core/src/memory`.

## Model

A row has `scope` (`path` | `glob` | `global`), `key`, `kind` (`fact` | `preference` | `protect` | `verdict`), `text`,
`source` (`user` | `agent` | `web:<url>` | `catalog`), optional `confidence`, `created_ms`, `updated_ms`, and an optional
`review_after_ms`, plus `pinned`, `last_used_ms`, and `forgotten_ms` (MEM-LIMIT). Keys are stored profile-relative (`~\...`), compare case-insensitively, and use `\`.
Facts get `review_after_ms = now + 90 days` unless the caller supplies one; `Memory::is_stale(now)` reports when it passed.

### MEM-RECALL-ANCESTRY: Recall by path ancestry and glob
**Applies to:** hoardini
**Verification:** test

`recall_path(path)` shall return every `path`-scope row whose key equals the path or any of its ancestors, and every
`glob`-scope row whose pattern matches the path or any ancestor, ordered most specific first (exact path, then ancestors
from nearest to farthest, then globs). Matching shall ignore case and treat `/` and `\` alike, and a path inside the user
profile shall match keys written as `~\...`. `protect_covering(path)` shall return the most specific `protect` row from
that same set, and none when only non-protect rows match.

**Acceptance criteria:**
- Rows keyed `~\.lmstudio`, `~\.lmstudio\models`, and the exact file are all returned for that file, in the order exact, parent, grandparent; a glob `~\.lmstudio\models\**` follows them.
- A sibling file and a name that merely starts with the same characters (`big.gguf.bak` for `big.gguf`, `D:\keeper` for `D:\keep`) are not returned.
- Key `~/Projects/Keep` matches `c:\USERS\Tester\projects\keep\src\main.rs` and is stored as `~\Projects\Keep`.
- Glob `**\node_modules` matches `E:\code\web\node_modules\left-pad\index.js`; glob `D:\data\*.parquet` matches `D:\data\a.parquet` but not `D:\data\sub\a.parquet`.
- `protect_covering` returns the row for `D:\keep` when asked about `D:\keep\a\b.bin`, returns a `**\*.kdbx` protect for any `.kdbx`, and returns none under a path that has only a `fact` row.

### MEM-EDIT: View, edit, pin, and delete memories
**Applies to:** hoardini
**Verification:** manual

The Memory drawer shall list every memory row sorted by last used, each with a "last used ..." label and a pin toggle, let the user change its text, kind, and key, and delete it. Forgotten rows (MEM-LIMIT) shall sit greyed at the bottom with a Restore button. A "Remember up to N notes" control shall set the limit. Rows whose source is `web:<url>` shall show that URL.

**Verification (manual):**
1. Ask the agent to remember a fact about a folder ("remember that D:\scratch is disposable"). A chip appears in the chat with an undo and a pin toggle, and reads "noted - pinned".
2. Open the Memory drawer. The row appears with scope `path`, key `D:\scratch`, source `agent`, pinned, and "last used just now".
3. Edit its text and kind to `protect`, save, reload the app. The edited row persists.
4. Ask the agent to propose deleting a file under `D:\scratch`. No card appears: policy refuses the path (POLICY-PROTECT), and the model gets an `Error: ... refused by policy` result that it explains.
5. Delete the row in the drawer. It disappears, and the same request now produces a card.
6. For a row created from a web search, confirm the drawer shows its source URL.
7. Ask the agent to remember a preference: the chat shows a "noted" chip; Undo turns it into "forgotten" and the row is gone from the drawer.
8. In the drawer's search box, a word from the note finds it; an unrelated word shows "No notes match."
9. Add a few unpinned notes, then lower the limit to its minimum below the note count. The oldest-used unpinned notes move, greyed, to the bottom with a Restore button; pinned and protect notes stay in place. Restore one: it returns to the top as "last used just now".
10. Toggle a row's pin off and on; the state persists across a reload.
(2026-10-08, scripted mock model, sandbox folder: 1-5, 7, 8 as described; 6 not run, no web-sourced row. Steps 9-10 and the pin text in 1-2 are new and unrun.)

### MEM-USAGE: Reads mark a note as used
**Applies to:** hoardini
**Verification:** test

Each place a note reaches the model shall set its `last_used_ms` to now: `protect_covering` (the one protect note returned), `recall_path` (every note returned), the `recall` tool and `recall_text` (every note returned), and `globals_for` (every global note the user's message matches in the full-text index). Creating or editing a note also sets it. The drawer's search shall not count as a use.

**Acceptance criteria:**
- A note created at time 1,000 and then returned by `recall_path` has `last_used_ms` greater than 1,000; a sibling note not returned keeps 1,000.
- `protect_covering` raises only the protect note's `last_used_ms`, not a non-protect note covering the same path.
- `recall_text` raises the `last_used_ms` of the notes it returns; `search` does not.
- `globals_for(cap, "keep the steam library")` raises a global note mentioning "steam" and leaves one that matches nothing.
- Editing a note sets `last_used_ms` to the edit time.

### MEM-LIMIT: Least-recently-used notes are forgotten
**Applies to:** hoardini
**Verification:** test

The note limit N (default 200, minimum 10, stored in `meta.memory_limit`) shall count only live notes that are neither pinned nor `protect`. When adding a note or lowering N leaves more than N, the notes with the oldest `last_used_ms` shall get `forgotten_ms = now` until N remain. Forgotten notes shall never be deleted by this, and shall be excluded from `recall_path`, `protect_covering`, `recall_text`, `globals`, and so from evidence and the prompt. `restore` shall clear `forgotten_ms` and count as a use. Forgotten notes shall be deleted 90 days after `forgotten_ms`, when the list is read or the limit is enforced.

**Acceptance criteria:**
- With N = 10 and 12 unpinned notes used at times 1..12, adding the 13th forgets the three used at 1, 2, 3 (live count 10) and leaves the rest.
- Pinned and `protect` notes older than all others are never forgotten, however many notes there are.
- A forgotten path note is not returned by `recall_path`, a forgotten protect-less global is not in `globals`, and a forgotten note is not returned by `recall_text` but is by `search`.
- `restore` returns the note to `recall_path` with `forgotten_ms` cleared and `last_used_ms` updated.
- `set_limit(1)` stores 10; lowering the limit forgets down to the new N immediately.
- A note forgotten more than 90 days ago is gone after `list`; one forgotten 89 days ago is still listed.
- `globals` orders protect, then pinned, then most recently used.

### MEM-PIN-USER: Notes the user asked for start pinned
**Applies to:** hoardini
**Verification:** test

A note whose source is `user` (typed in the drawer) shall start pinned, and so shall a `remember` call with `user_asked: true`. A `remember` call with `user_asked: false` or without it shall not pin. Pinning only exempts a note from the limit; `PUT /api/memory/{id}` with `pinned` changes it.

**Acceptance criteria:**
- `remember_with` for a `user`-source note returns `pinned: true`; for an `agent`-source note it returns `pinned: false`.
- A `NewMemory` with `pinned: true` and source `agent` is stored pinned.
- Re-remembering the same agent note with `pinned: true` pins the existing row; with `false` it leaves it pinned.
- Updating with `pinned: Some(false)` unpins.
- The `remember` tool with `user_asked: true` stores a pinned row; with `false` or absent, an unpinned one (AGENT-MEMORY-NOTED).
