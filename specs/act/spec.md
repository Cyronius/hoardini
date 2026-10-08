# Actions, destinations, journal (prefix ACT)

The executor runs approved proposals as background jobs. Canonical design: `.claude/plans/hoardini-design.md` §5.6.
Code: `crates/core/src/{actions,destinations}`. Tests: `specs/act/tests/act.rs`.

## Model

`Executor::approve(proposal_id, item_ids)` is the only call that changes the filesystem. It accepts a `pending`
proposal and a subset of its selectable item ids, revalidates them (POLICY-REVALIDATE), marks the proposal `approved`,
starts a job on its own thread, and returns `{approved, items, job_id}`. Jobs run one at a time (`queued` → `running` →
`done` | `failed` | `cancelled`); progress (bytes, items, current path) goes to `Executor::subscribe()` and the `job`
table; cancel takes effect between files and between 1 MiB copy chunks. Kinds: delete, archive (local folder,
SMB UNC path), compress, dedupe.

- **Delete**: `remove_file` (read-only cleared), `remove_dir_all` for folders (never follows links inside), and a link
  item removes the link itself.
- **Archive**: copy to `<destination>\<yyyy-mm-dd UTC>\<original path without the drive colon>` (`UNC\server\share\...`
  for network sources), hashing the source while copying into `<name>.hoardini-partial`; flush; re-read the copy; size
  and BLAKE3 must match; rename into place; then delete the source. Folders copy file by file with the same check; links
  inside are skipped and named in the item detail. A re-run reuses a final copy whose BLAKE3 still matches. User
  folders (no catalog entry, `user-data`, `downloads`) get a `<name>.hoardini.txt` stub naming the copy; caches do not
  (`meta.archive_stubs = false` turns stubs off).
- **Compress**: formats `zip`, `7z`, `tar.zst` write `<name>.<ext>` next to the original (`<name>.<ext>.hoardini-partial`
  while writing), hashing every source file while it is added; then the archive is re-read, every entry's size and
  BLAKE3 must match, the sources must be unchanged, and only then is the archive renamed into place and the originals
  deleted (undo = extract). Links, reparse points, and cloud-only files make the item fail up front. `lzx`
  (`compact /c /exe:lzx`, for data that is not written to again) and `lznt1` (`compact /c`) run `compact.exe` in place with
  no console window (undo = `compact /u`). Freed bytes = allocation before minus after.
- **Dedupe**: items are the extra copies; `keep` (from the proposal) is never listed, opened for write, or changed.
  `hardlink` (same volume only) hashes keep and extra in full at execution, creates the link under a temp name beside
  the extra, then renames it over the extra; `delete` removes the extra. Candidates come from `core::dupes`: same
  logical size, distinct `file_id` (hard links are one file), equal head+tail sample, then equal full BLAKE3, files of
  at least 100 MB by default, never cloud placeholders. Full hashes are reused while size, mtime, and `file_id` hold.
- **Journal**: one `action` row per item, written `running` before work starts and `done`/`failed`/`cancelled` with
  `freed_bytes`, `undo_json`, and `error` after; archive also writes an `archive_entry` (size, BLAKE3 of the file or of
  the folder manifest, stub path) and the action's `undo_json` before it deletes the source. The index is patched after
  each item. `vanished_total` = sum of `action.freed_bytes`.
- **Restore**: `Executor::restore(archive_entry_id)` copies the archive back to a sibling temp name, checks it against
  the recorded BLAKE3, renames it to the original path, removes the stub, sets `restored_ms`, and keeps the archived copy.
- **Destinations**: `local` (`config.path`), `smb` (`config.path` = UNC, optional `username`, password from the vault
  at `config.password` or `secret://destination/<id>/password`, else the current Windows session), `rclone` (wave 3).
  Config values under keys that look secret must be `secret://` refs. `destinations::test` checks reachability,
  writability, and free space.

### ACT-ARCHIVE-VERIFY: Archive deletes the source only after the copy verifies
**Applies to:** hoardini
**Verification:** test

An archive shall delete a source file or folder only after every copied file's size and BLAKE3 match the source as read
during the copy. A copy that fails verification shall be removed, the item shall fail, and the source shall stay
untouched. A re-run shall reuse copies that still verify.

**Acceptance criteria:**
- Archiving a 3 MiB file and a nested folder (with an empty subfolder) to a local destination removes both sources; the copy sits under the destination with the original layout (`...\work\docs\model.gguf`), byte-identical, and `archive_entry.blake3` equals BLAKE3 of the original bytes.
- When each finished copy is altered before verification, the file and the folder items fail with "verification failed", both sources are byte-identical to before, no `archive_entry` is written, and the job frees 0 bytes.
- After a run whose second file failed verification, a new archive of the same folder copies only the two files not yet verified and succeeds.

### ACT-JOURNAL: Every action is journaled with undo info before it reports success
**Applies to:** hoardini
**Verification:** test

Each executed item shall have an `action` row whose status is `done` and whose `undo_json` is set before the item's
status becomes `done` or a job event reports it. Delete undo info says there is no undo; archive undo info names the
`archive_entry`, destination, and copy path. The vanished total shall equal the sum of journaled freed bytes and survive
a restart.

**Acceptance criteria:**
- At every job event during a run of a two-file delete and a one-file archive, every item already `done` has a `done` action row with `undo_json`.
- Delete rows carry `{"kind":"none"}` and `freed_bytes` equal to the item's allocated size; the archive row carries `{"kind":"restore","archive_entry_id","dest_uri"}` matching its `archive_entry`.
- `vanished_total()` equals the sum of `freed_bytes` and is unchanged after a new `Executor` opens the same database.

### ACT-RESTORE: Restore puts back the original path and content
**Applies to:** hoardini
**Verification:** test

Restoring an archive entry from a local destination shall recreate the original path with identical content (and the
file's original mtime), remove the stub, set `restored_ms`, and keep the archived copy. Restore shall refuse when
something exists at the original path, and shall fail without leaving anything at the original path when the archived
copy no longer matches its recorded BLAKE3.

**Acceptance criteria:**
- After archiving `docs\report.pdf` (stub created) and a folder, restoring the file brings back identical bytes and mtime, removes the stub, keeps the copy, and sets `restored_ms`; restoring the folder brings back the same tree.
- A second restore of the same entry is refused with "already exists".
- Restoring an entry whose archived copy was altered fails with a BLAKE3 error, leaves the original path absent, and leaves `restored_ms` unset.

### ACT-DELETE-CONFIRM: Every delete needs hold-to-confirm
**Applies to:** hoardini
**Verification:** manual

A delete proposal card shall say the delete is permanent and cannot be undone, and shall approve only after the delete
button is held for the full hold time (`HoldButton` in `ui/src/components/ProposalCard.tsx`, 1.2 s). A click, a short
press, or releasing early shall not approve. Archive cards approve with one click.

**Verification (manual):**
1. Start the app, ask the agent to propose deleting a scratch file you created (e.g. `D:\scratch\junk.bin`).
2. The card shows the item, its size, verdict badge, and the text "Deleting is permanent and cannot be undone. Nothing goes to the Recycle Bin."
3. Click "Hold to delete" once. The fill bar starts and resets; the card stays pending; the file still exists.
4. Press and release after about half a second. Same result.
5. With keyboard focus on the button, press and release Space quickly. Same result.
6. Press and hold until the bar fills (about 1.2 s). The card switches to approved, a job appears, and the file is gone.
7. Uncheck every item on another delete card: the hold button is disabled.
8. Ask for an archive proposal to a registered destination: its approve button works with a single click.

### ACT-COMPRESS-ESTIMATE: Compression estimates track the data
**Applies to:** hoardini
**Verification:** test

`compress::estimate(path)` shall sample the file (or the largest files of a folder) through the atime-preserving
handle and report, per format (`zip`, `7z`, `tar.zst`, `lzx`, `lznt1`), the compressed/original ratio, expected result
size, savings, and time. All figures are estimates. Data identified as already compressed (media, archives, quantized
weights) shall report ratio 1.0 without being compressed.

**Acceptance criteria:**
- A 4 MiB pseudo-random file: every format's ratio is within 0.97 to 1.0 and savings are under 150 KB.
- A 4 MiB file of zeros: every format's ratio is below 0.1 and savings exceed 3 MiB.
- A `.mp4`-named random file is flagged `already_compressed`, samples 0 bytes, and every format reports ratio 1.0 and 0 saved.

### ACT-COMPRESS-VERIFY: Originals go only after the archive tests clean
**Applies to:** hoardini
**Verification:** test

An archive compress shall delete originals only after the finished archive was re-read and every entry matched the
source's size and BLAKE3. A failed test shall leave the originals untouched, no archive at the final name, and no
partial file. An in-place compress shall leave content identical and reduce allocation on compressible data.

**Acceptance criteria:**
- For each of zip, 7z, tar.zst: a text file and a nested folder (with an empty subfolder) compress to `<name>.<ext>` beside them, both originals are gone, the job frees more than 0 bytes, and the zip's entries equal the folder's file bytes.
- When each finished archive is damaged before its test, the job fails, the file and folder are byte-identical to before, no archive or `.hoardini-partial` exists, and 0 bytes are freed.
- `lzx` and `lznt1` on a 4 MiB text file: content is identical, allocation after is under half of before, `freed_bytes` equals the difference, and the journaled undo is a `compact /u` command.

### ACT-DEDUPE-VERIFY: Duplicates are removed only after a fresh full-hash match
**Applies to:** hoardini
**Verification:** test

A dedupe shall act on an extra copy only if, at execution, its full BLAKE3 and size equal the kept copy's. The kept copy
shall never be modified or deleted, and a proposal that lists the kept copy for removal shall change nothing.
`hardlink` shall leave the extra's path readable with the kept content; `delete` shall remove only verified extras.

**Acceptance criteria:**
- Keep `a\keep.bin`, extras `b	win.bin` and `c\lookalike.bin` (the latter changed after proposing, same size and mtime): in hardlink mode the twin has keep's `file_id` and bytes, the lookalike is `failed` ("differs"), unchanged, and not linked; keep is byte- and `file_id`-identical; freed bytes are at least the file size; the journaled undo says "break the link".
- In delete mode the twin is gone, the lookalike remains, keep is unchanged.
- A proposal listing keep among the items fails the job and leaves every file in place.
