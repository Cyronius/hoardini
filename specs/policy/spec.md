# Policy gate (prefix POLICY)

The gate is code, not prompt. It runs when a proposal is created (`proposals::create`) and again before each item
executes (`Executor::approve` and the job thread). Canonical design: `.claude/plans/hoardini-design.md` §5.5.
Code: `crates/core/src/policy`, `crates/core/src/proposals`. Tests: `specs/policy/tests/policy.rs`.

## Model

Every requested path is resolved first: `~\...` expands to the profile, `.`/`..` collapse, and short names and junctions
in the parent chain resolve to the real location (a final link stays the link itself). A final link or cloud
placeholder, which is not opened, keeps its on-disk name, so a trailing dot or an 8.3 alias resolves to the real name.
Then one of three outcomes:

- **Denied**: the path never becomes an item. It is listed in the proposal's `dropped` with a reason. A request whose
  every path is denied fails with an error that names each path and reason.
- **Blocked**: the item is stored and shown with `status = "blocked"`, a `block_reason`, and `selected = false`. It
  cannot be approved.
- **Selectable**: `status = "pending"`; `total_alloc` sums these items only. It starts ticked or unticked per
  POLICY-DEFAULT-TICK.

Each item stores a `cap` (`keep` / `review` / none) and the shown `verdict`, which is the LLM's verdict raised to at
least the cap (`Verdict::capped`). `policy::cap_for(conn, path, catalog_cap, in_use)` exposes the same cap to evidence.
The locations below come from `PolicyEnv` (`system()` reads the real environment; `with_paths(profile, data_dir)` lets
tests point the profile and data folder at temp directories).

`system_cleanup` proposals (ACT-WINDOWS-CLEANUP) hold operations that Windows' own tools carry out, not paths, so they do
not go through the gate at creation or execution; the elevated helper runs only its fixed list of operations
(ACT-WINDOWS-CLEANUP-RUN).

### POLICY-DENY: Deny-listed locations are never proposed
**Applies to:** hoardini
**Verification:** test

The gate shall refuse, with no override, any path that is, is inside, or contains: `%SystemRoot%`, a `Program Files*`
folder at a volume root (and the `%ProgramFiles*%` locations), `ProgramData\Microsoft`, `System Volume Information`,
`$Recycle.Bin`, and Hoardini's data folder. It shall refuse `pagefile.sys`, `hiberfil.sys`, and `swapfile.sys`. It shall
refuse any volume root (`C:\`, `\\server\share`), the profile root, `AppData`, and `AppData\Local`, `Roaming`, and
`LocalLow` themselves, and any ancestor of them (so `C:\Users` is refused because it holds the profile). It shall refuse
any network path that leads back to a fixed volume of this PC (`\\localhost\C$\...`, a share of a local folder; the
share reports the local volume's serial), since the rules above are written for local names. When the active
provider points at `localhost`/`127.0.0.1`/`::1` and names a model, files of that model under the LM Studio, Ollama, and
Hugging Face stores (path contains the model id, case-insensitive; Ollama blobs via the model's manifest) and every
ancestor holding them shall be refused (best effort).

**Acceptance criteria:**
- Each of `C:\`, `D:`, `\\nas\share`, `%SystemRoot%` and a file under it, `C:\Program Files\Vendor\app.exe`, `C:\Program Files (x86)`, `E:\Program Files\Steam`, `ProgramData\Microsoft\Windows\WER`, `ProgramData`, a file under `System Volume Information`, a file under `$Recycle.Bin`, `pagefile.sys`, `hiberfil.sys`, `swapfile.sys`, the profile root, `AppData`, `AppData\Local|Roaming|LocalLow`, the profile's parent, the data folder, and a file in it has a deny reason.
- `~\Downloads\big.iso`, `~\AppData\Local\pip\cache\wheel.whl`, and a file in an ordinary folder have none.
- A request made only of denied paths creates no proposal and returns an error naming the reasons ("volume root", "profile").
- A mixed request keeps the allowed items; `profile\Downloads\..\..` and `~\AppData\Local` are among the `dropped` entries.
- With an active `http://localhost:1234/v1` provider and model `qwen3.8-27b`, `~\.lmstudio\models\...\Qwen3.8-27B-GGUF\Qwen3.8-27B-Q8_0.gguf`, its folder, `models\lmstudio-community`, `models`, and `~\.lmstudio` are refused with the model id in the reason; a sibling Llama model is not. With a non-local base URL nothing is refused.
- Where the administrative share is reachable, the profile root and a file in it, written as `\\localhost\C$\...`, are refused with "this PC" in the reason, and a proposal for the profile that way creates nothing.

### POLICY-PROTECT: Protect memories block their paths
**Applies to:** hoardini
**Verification:** test

A path shall be refused when a `protect` memory covers it (the path itself, an ancestor, or a matching glob, per
MEM-RECALL-ANCESTRY) and when it contains a protected path (a `path` key below it, or a glob whose literal prefix is at
or below it, or a wildcard glob that matches something deeper inside it, found by walking the folder's index nodes up to a
bound; a folder the index cannot answer for is refused with a "review it first" reason). A protect memory added after a
proposal shall stop that item at execution.

**Acceptance criteria:**
- With protect `work\keep` ("family photos"): `work\keep\2024\a.jpg`, `work\keep`, and `work` are refused with the memory text; `work\other\b.bin` is not.
- With protect glob `work\secrets\*.kdbx`: `work\secrets\vault.kdbx` and `work\secrets` are refused.
- A proposal for `work\other\b.bin`, approved after a protect on `work\other` is added, skips the item with a "Denied: ... late protect" detail and the file stays.
- With protect glob `work\**\secrets\**`: `work\proj` (holding `app\secrets\key.pem`) and `work\proj\app` are refused naming the memory; `work\proj\app\src` and an unrelated `work\clean` are not. Before the index is built, `work\proj` is refused with "the index cannot confirm".
- Deleting the memory lifts the refusal.
- A protected offline (cloud placeholder) file requested with a trailing dot (`Tax Return 2024.pdf.`) is refused with the memory text.

### POLICY-IN-USE: Open files block delete and archive
**Applies to:** hoardini
**Verification:** test

A file held open by any process (Restart Manager) shall be blocked with `Open in <app> (pid N)`. A folder shall be
checked through its largest files (up to 500): from the index when the folder is indexed, else by listing it. The item
cap is `keep`. Archive and compress of a cloud placeholder (or a folder holding one) shall be blocked because it would
download; a missing path shall be blocked as `Not found`. A file opened after the proposal shall be skipped at execution
with the same reason.

**Acceptance criteria:**
- A file this test process holds open, an indexed folder holding it, and an unindexed folder holding another held file are each `blocked`, unselected, cap `keep`, with a reason starting `Open in ` and containing `(pid <this pid>)`; `total_alloc` is 0.
- An archive proposal for a held file is blocked the same way; approving a blocked item is rejected and the proposal stays `pending`.
- A file opened between proposal and approval is `skipped` with the `Open in` detail and still exists.
- A file with the offline attribute is blocked for archive (reason mentions the download) and selectable for delete; a missing path is blocked `Not found`.
- A folder whose open file is too small to have an index row (so the check cannot see it) is not deleted in part: the item does not finish `done`, every file in the folder still exists, and 0 bytes are freed (ACT delete renames the folder first).

### POLICY-REVALIDATE: Changed items are skipped at execution
**Applies to:** hoardini
**Verification:** test

Before an approved item runs, the gate shall compare the path with what was proposed: kind, size (files), mtime, and
file ID. Any difference, a deny reason, an open handle, or a missing path shall mark the item `skipped` with a detail;
it shall not execute. The proposal ends `partial` when some approved items ran and others did not. The open-handle
check is the slow one, so it runs first; the deny list and the kind, size, mtime, file-ID, and link comparisons run
after it, immediately before the item executes, so a path swapped for a junction during the slow check is caught.

**Acceptance criteria:**
- A folder replaced by a junction while the open-handle check runs is skipped with a detail starting `Changed since the proposal` (verified by a test-only hook between the two phases in `crates/core/src/policy/gate.rs`).
- Of four proposed files, one grown, one with only its mtime changed, and one deleted and re-created with the same size and mtime are `skipped` with a detail starting `Changed since the proposal` and still exist; the untouched one is deleted.
- `approve` returns only the untouched item's id; the proposal status is `partial`.

### POLICY-VERDICT-CAP: The shown verdict never exceeds the cap
**Applies to:** hoardini
**Verification:** test

Each item's cap shall be the strongest of: the catalog cap; `review` for a path no catalog entry knows; `review` for a
single file inside a shared-blob store (the whole model folder is the safe unit); `keep` when the file is in use or the
path is denied. The stored verdict is the LLM verdict raised to the cap; a verdict above the cap stays.

**Acceptance criteria:**
- LLM `safe` on an HF hub blob file → verdict `review`, cap `review`; on `disk.vhdx` → `review`/`review`; on an unknown file → `review`/`review`.
- LLM `redownload` on a whole `models--org--name` folder → `redownload`, no cap; `safe` on `node_modules` → `safe`, no cap.
- LLM `keep` on an unknown file stays `keep` and the item starts unselected.
- `cap_for_with(.., in_use = true)` and the cap of the profile root are `keep`; every item satisfies verdict ≥ cap.

### POLICY-DEFAULT-TICK: Only worthwhile safe or redownload items start ticked
**Applies to:** hoardini
**Verification:** test

A new proposal's item shall start ticked (`selected = true`) only when all three hold: it is not blocked; its stored
verdict, which is already capped (POLICY-VERDICT-CAP), is `safe` or `redownload` (an item with no verdict counts as
`review`); and it is not small. An item is small when its `alloc` is under 50 MiB **and** under 2% of the proposal's
`total_alloc`, so a card made only of small files still ticks its biggest ones. Every item's JSON carries
`small: boolean`. The rule is `proposals::is_small(alloc, total)` and `proposals::default_tick(blocked, verdict, small)`.
A follow-up proposal (ACT-FOLLOW-UP) keeps the approved tick state instead. Tests: `specs/policy/tests/policy.rs`;
the item JSON is also checked in `specs/srv/tests/srv.rs`.

**Acceptance criteria:**
- `is_small`: 50 MiB − 1 byte of a huge total is small; exactly 50 MiB and 50 MiB + 1 are not. 1,999 of 100,000 is small; exactly 2,000 (2%) and 2,001 are not. 3 MiB of a 3 MiB + 40 KiB card is not small while 4 KiB of it is. 0 of 0 is not small.
- `default_tick` is true for unblocked, not-small `safe` and `redownload`; false when blocked, small, `review`, `keep`, or no verdict.
- In one proposal: a `node_modules` folder the model calls `safe` and an HF `models--org--m` folder it calls `redownload` start ticked; a `node_modules` called `review`, one with no verdict, and an unknown file called `safe` (shown as `review`) start unticked; a 4 KiB `node_modules` called `safe` is `small` and unticked; no other item is `small`.

### POLICY-ITEM-MEMORY: Each item shows the note that covers it
**Applies to:** hoardini
**Verification:** test

When a proposal is created, each item shall store `memory_note`: the text of the first non-protect, non-forgotten
memory covering its path in MEM-RECALL-ANCESTRY order (the path itself, then each ancestor, then globs), or null.
Attaching a note marks that note used (MEM-USAGE); other covering notes are not touched. A protect memory still refuses
the path (POLICY-PROTECT). The server redacts the note like the other item text.

**Acceptance criteria:**
- With notes on `work` ("work folder"), `work\scratch` ("scratch is disposable"), and `work\scratch\old` ("old builds, fine to drop"): `work\scratch\old\a.bin` carries "old builds, fine to drop", `work\scratch\b.bin` carries "scratch is disposable", and a file outside `work` carries null.
- `work\keep\d.bin`, under both a fact and a protect ("family photos") on `work\keep`, is in `dropped` with the protect text.
- The "scratch is disposable" note's `last_used_ms` moves to the proposal time; the "work folder" note's stays at its old value.
