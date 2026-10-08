# Memory (prefix MEM)

Path-keyed notes the agent and the user leave about the disk. Stored in the `memory` table (SQLite, FTS5 for free text).
Canonical design: `.claude/plans/hoardini-design.md` §5.7. Code: `crates/core/src/memory`.

## Model

A row has `scope` (`path` | `glob` | `global`), `key`, `kind` (`fact` | `preference` | `protect` | `verdict`), `text`,
`source` (`user` | `agent` | `web:<url>` | `catalog`), optional `confidence`, `created_ms`, `updated_ms`, and an optional
`review_after_ms`. Keys are stored profile-relative (`~\...`), compare case-insensitively, and use `\`.
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

### MEM-EDIT: View, edit, and delete memories
**Applies to:** hoardini
**Verification:** manual

The Memory drawer shall list every memory row, let the user change its text, kind, and key, and delete it. Rows whose
source is `web:<url>` shall show that URL.

**Verification (manual):**
1. Ask the agent to remember a fact about a folder ("remember that D:\scratch is disposable"). A chip appears in the chat with an undo.
2. Open the Memory drawer. The row appears with scope `path`, key `D:\scratch`, source `agent`.
3. Edit its text and kind to `protect`, save, reload the app. The edited row persists.
4. Ask the agent to propose deleting a file under `D:\scratch`. The proposal item is blocked with the protect note.
5. Delete the row in the drawer. It disappears and the same proposal is no longer blocked.
6. For a row created from a web search, confirm the drawer shows its source URL.
