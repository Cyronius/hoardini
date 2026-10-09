# Memory (prefix MEM)

Path-keyed notes the agent and the user leave about the disk. Stored in the `memory` table (SQLite, FTS5 for free text).
Canonical design: `.claude/plans/hoardini-design.md` §5.7. Code: `crates/core/src/memory`.

## Model

A row has `scope` (`path` | `glob` | `global`), `key`, `kind` (`fact` | `preference` | `protect` | `verdict`), `text`,
`source`, optional `confidence`, `created_ms`, `updated_ms`, and an optional `review_after_ms`, plus `pinned`,
`last_used_ms`, and `forgotten_ms` (MEM-LIMIT), and an optional `detail` (the `memory.detail_json` column, v5). Keys are
stored profile-relative (`~\...`), compare case-insensitively, and use `\`. Facts get `review_after_ms = now + 90 days`
unless the caller supplies one; `Memory::is_stale(now)` reports when it passed.

Sources: `user` (the drawer, a "What is this?" answer), `agent` (the `remember` tool), `web:<url>`, `catalog`, `deny`,
`mix` and `index` (the location classifier, MEM-LOCATION), `job:<id>` (a finished job) and `copy:<check>` (a copy check).

Three kinds of row:

- **Notes** have no `detail`: what the `remember` tool and the drawer write.
- **Location rows** (MEM-LOCATION): scope `path`, kind `verdict`, detail `{category, label, verdict, actions, size_at,
  assessed_ms, by, cleaned, kept}`.
- **File-type rows** (MEM-FILETYPE): scope `glob`, key `**\*.<ext>`, kind `fact`, detail `{category, app, use, copy_hint}`.

Detail fields: `category` (a catalog category such as `model-store`, or `system`, `protected`, `container`, `photos`,
`documents`, `model-weights`, `unknown`, `user`); `label`, the short name the prompt tag shows; `verdict` on the existing
scale; `actions`, the proposal kinds still allowed there (`delete`, `dedupe`, `archive`, `compress`); `app` and `use`,
what the folder or type belongs to; `copy_hint`, a copy check that knows where to look (a place to look, never a
verdict); `size_at` and `assessed_ms`, the folder size and time of the last assessment; `by`, the step that decided
(`deny`, `protect:<id>`, `catalog:<entry>`, `mix`, `index`, `user`); `cleaned` and `kept`, entries owned by job learning
(MEM-LEARN-JOB). An absent field means null or empty, and fields this list does not name are kept as written. Location
and file-type rows are kept one per scope and key (`memory::knowledge::upsert`), apart from notes on the same key, get no
`review_after_ms` (locations are reassessed instead), and reach evidence, the `recall` tool and item notes through
`recall_path` like notes do.

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

The Memory drawer shall list every memory row sorted by last used, each with a "last used ..." label and a pin toggle, let the user change its text, kind, and key, and delete it. Forgotten rows (MEM-LIMIT) shall sit greyed at the bottom with a Restore button. A "Remember up to N notes" control shall set the limit. Rows whose source is `web:<url>` shall show that URL. Location and file-type rows shall also show their label (or app, or category) and their verdict as chips; `GET /api/memory` carries each row's `detail` (null for notes).

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
11. With a location row (`~\Pictures`, "your photos and videos", `keep`) and a file-type row (`**\*.q4nx`, app FastFlowLM) present, open the drawer: the location row shows the chips "your photos and videos" and "keep", the type row shows "FastFlowLM" and no verdict chip, and a plain note shows neither.
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

The note limit N (default 200, minimum 10, stored in `meta.memory_limit`) shall count only live notes that are neither pinned nor `protect`; location and file-type rows (rows with a `detail`) are never counted and never forgotten. When adding a note or lowering N leaves more than N, the notes with the oldest `last_used_ms` shall get `forgotten_ms = now` until N remain. Forgotten notes shall never be deleted by this, and shall be excluded from `recall_path`, `protect_covering`, `recall_text`, `globals`, and so from evidence and the prompt. `restore` shall clear `forgotten_ms` and count as a use. Forgotten notes shall be deleted 90 days after `forgotten_ms`, when the list is read or the limit is enforced.

**Acceptance criteria:**
- With N = 10 and 12 unpinned notes used at times 1..12, adding the 13th forgets the three used at 1, 2, 3 (live count 10) and leaves the rest.
- Pinned and `protect` notes older than all others are never forgotten, however many notes there are.
- A forgotten path note is not returned by `recall_path`, a forgotten protect-less global is not in `globals`, and a forgotten note is not returned by `recall_text` but is by `search`.
- `restore` returns the note to `recall_path` with `forgotten_ms` cleared and `last_used_ms` updated.
- `set_limit(1)` stores 10; lowering the limit forgets down to the new N immediately.
- A note forgotten more than 90 days ago is gone after `list`; one forgotten 89 days ago is still listed.
- `globals` orders protect, then pinned, then most recently used.
- With N = 10, three location rows and one file-type row written first and twelve unpinned notes after them, ten notes stay live and all four detail rows stay live.

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

### MEM-LOCATION: Large folders get a location row
**Applies to:** hoardini
**Verification:** test

After each completed index refresh of a drive (`hoardini index` calls `memory::location::refresh_volume` before it reports
`done`), each large folder of that drive shall have one location row. A folder is large when it holds at least 5 GiB or
at least 1% of the drive's used space (never less than 256 MiB) and no single child holds 90% or more of it; only the 40
biggest per drive count, and never the drive root. `location::large_folders(conn, volume)` lists them. Each is
classified, with no model call, by the first step that applies:

1. **Deny list:** a Windows system area of POLICY-DENY (the Windows folder, Program Files, `ProgramData\Microsoft`,
   `System Volume Information`, `$Recycle.Bin`, paging files, Hoardini's data) is category `system`, verdict `keep`, no
   actions; a folder a protect note covers is `protected`, `keep`, no actions. Source `deny`.
2. **Catalog:** the deepest matching entry (EVID-CATALOG), unless it is a `container` entry matched only through an
   ancestor, gives its category, its name as label, its verdict hint raised to its cap, and all four actions unless the
   verdict is `keep` (then none). Source `catalog`. For a `user-data` entry the content mix is tried first and wins when
   it finds photos or documents.
3. **Container:** a folder the deny list refuses only as itself (the drive root, the profile, AppData and its three
   folders, or a folder holding one) is category `container` with the deny list's label and no verdict. Source `deny`.
4. **Content mix:** from INDEX-EXT-MIX, so only files of 1 MiB or more have a type and small-file bytes count against
   every type. When 80% or more of the bytes are photo or video types the folder is `photos` ("your photos and videos",
   `keep`, actions `[archive]`); documents, `documents` ("your documents", `keep`, `[archive]`); model weights
   (MEM-FILETYPE's list), `model-weights` (`review`, all actions). Source `mix`.
5. **Unknown:** otherwise category and label `unknown`, no verdict, no actions. Source `index`.

The row's text is one sentence naming what decided (`Your photos and videos: 92% of its bytes are images or video.`).
A row is reassessed when the folder's size has moved by 25% or more from `size_at`, or 90 days after `assessed_ms`; a
user's answer is never reassessed. A reassessment writes only where MEM-SOURCE-RANK allows; where it may not, only
`size_at` and `assessed_ms` move. When a folder is no longer large, its row is deleted unless its source is not the
classifier's (`deny`, `catalog`, `mix`, `index`), it is pinned, or it has `cleaned` or `kept` entries.

`location::answer(conn, env, path, answer, now)` records the user's answer to "What is this?" as a `user` row, pinned,
keeping `cleaned` and `kept`: `keep` gives verdict `keep` with `[archive, compress]`; `disposable`, `safe` with all four;
`move-elsewhere`, `keep` with `[archive]`; `never-touch`, `keep` with none, plus a `protect` note on the path
(POLICY-PROTECT). `location_at` finds a folder's row, `location_covering` the nearest at or above a path, and
`location::tag` the prompt tag: the label, then the verdict when there is one (`your photos and videos, keep`, `unknown`).

**Acceptance criteria:**
- On a fake drive with 1,000 GiB used: `~\Pictures` (80 GiB, 65 GiB of it `.jpg`, `.mp4` and `.heic` in two year folders) is a `mix` row keyed `~\Pictures`, kind `verdict`, category `photos`, `keep`, actions `[archive]`, `size_at` 80 GiB, `assessed_ms` the sweep time, tag `your photos and videos, keep`.
- On the same drive `C:\code\covenant-agent` (161 GiB of `.dat` in two folders) is `unknown` with no verdict, source `index`, tag `unknown`; `C:\code`, 96% of which is that folder, has no row; `~\.lmstudio\models` is a `catalog` row by `catalog:lmstudio-models`, category `model-store`, all four actions, tag `LM Studio models, redownload`; the Windows folder is a `deny` row, `system`, `keep`, no actions; the profile is a `container` tagged `your profile folder`; a 2 GiB folder and the drive root have none; `location_covering` of a path under `~\Pictures` returns its row.
- With 45 folders of 6 GiB and up under `C:\many`, `large_folders` returns 40: `C:\many` first, then the 39 biggest of the 45.
- An 8 GiB folder grown 10% keeps its `assessed_ms`; grown to 11 GiB (37.5%) it is reassessed with `size_at` 11 GiB; 90 days later it is reassessed again; a folder the user answered Keep to is unchanged through all of it.
- When three 6 GiB folders shrink to 2 GiB, the plain one loses its row, one whose detail has a `kept` entry keeps it, and one the user called Disposable keeps it.
- Each answer is a pinned `user` row with the verdict and actions above and `size_at` the folder's size; Never touch also leaves a `user` protect note covering the folder; the folder still has one location row.

Test: `specs/mem/tests/knowledge.rs`.

### MEM-FILETYPE: One row per file type
**Applies to:** hoardini
**Verification:** test

`knowledge::remember_filetype(conn, ext, text, source, detail, now)` shall keep one row per extension: scope `glob`, key
`**\*.<ext>` (lower case, leading `*` and `.` dropped; only letters, digits, `_`, `-` and `.` allowed), kind `fact`,
detail `{category, app, use, copy_hint}`, the text one sentence. Writing the same extension again updates that row
(MEM-SOURCE-RANK). `filetype(conn, ext)` and `filetypes(conn)` read them, and `weight_extensions(conn)` is the fixed
weight list (`gguf`, `safetensors`, `onnx`, `pt`, `ckpt`) plus every type whose category is `model-weights`, which the
location classifier and the model-store detector (REVIEW-UNITS) use. Matching files see the row through `recall_path`
(MEM-RECALL-ANCESTRY), so it reaches evidence and each item's note (POLICY-ITEM-MEMORY).

**Acceptance criteria:**
- `.Q4NX` from a `web:` source with category `model-weights`, app FastFlowLM and copy hint `model-hub` is stored as `**\*.q4nx`, scope `glob`, kind `fact`, and `filetype("q4nx")` returns it.
- A `user` write for `q4nx` updates the same row; `filetypes` has one row; `weight_extensions` includes `q4nx`.
- `recall_path` of `D:\npu\Qwen3.8-27B-NPU2\model.q4nx` returns that row, and it is the file's item note.
- `a/b` and a blank extension are rejected.

Test: `specs/mem/tests/knowledge.rs`.

### MEM-SOURCE-RANK: A lower source never overwrites a higher one
**Applies to:** hoardini
**Verification:** test

Location and file-type rows shall rank their sources: `user` > `job:<id>` > `catalog` = `deny` = `copy:<check>` >
`mix` > `web:<url>` > `agent` > `index` and anything else. `knowledge::upsert` shall write over the row at the same
scope and key only when the new source ranks at least as high, or when both are the classifier's (`deny`, `catalog`,
`mix`, `index`), so a reassessment can re-decide its own row. Otherwise it returns the stored row unchanged and says it
did not write. A write keeps the stored `cleaned` and `kept` entries unless it brings its own.

**Acceptance criteria:**
- `source_rank` rises strictly through `index`, `agent`, `web:`, `mix`, `catalog`, `job:7`, `user`, and `deny` equals `catalog`.
- Writing one key from `agent`, `web:`, `mix`, `catalog`, `job:3` and `user` in that order rewrites the same row each time; after the `user` write, writes from `job:4`, `catalog`, `deny`, `mix`, `web:`, `agent` and `index` change nothing.
- An `index` (unknown) write does not replace a `web:` row; a `mix` write replaces a `catalog` row.

Test: `specs/mem/tests/knowledge.rs`.

### MEM-WEB-IDENTITY: Web knowledge is identity, not instruction
**Applies to:** hoardini
**Verification:** test

A location or file-type row whose source is `web:<url>` shall record only what app or purpose the folder or type belongs
to: on write its verdict and actions are dropped and its kind becomes `fact`, so the stored `detail_json` has no
`verdict` and no `actions`. Whether something can go is decided only by the catalog, the user, a finished job or a copy
check.

**Acceptance criteria:**
- A `web:` row for `~\.hipfire` written with verdict `safe`, actions `[delete]` and kind `verdict` is stored as kind `fact` with app `hipfire` and use `model cache`, and its `detail_json` has neither `verdict` nor `actions`.
- A `web:` file-type row written with verdict `redownload` has no verdict.

Test: `specs/mem/tests/knowledge.rs`.
