# Actions, destinations, journal (prefix ACT)

The executor runs approved proposals as background jobs. Canonical design: `.claude/plans/hoardini-design.md` §5.6.
Code: `crates/core/src/{actions,destinations}`. Tests: `specs/act/tests/act.rs`.

## Model

`Executor::approve(proposal_id, item_ids)` is the only call that changes the filesystem. It accepts a `pending`
proposal and a subset of its selectable item ids, revalidates them (POLICY-REVALIDATE), marks the proposal `approved`,
starts a job on its own thread, and returns `{approved, items, job_id}`. Jobs run one at a time (`queued` → `running` →
`done` | `failed` | `cancelled`); progress (bytes, items, current path) goes to `Executor::subscribe()` and the `job`
table; cancel takes effect between files and between 1 MiB copy chunks. Kinds today: delete and archive (local folder,
SMB UNC path); compress and dedupe plug into `actions::handler` in wave 3.

- **Delete**: `remove_file` (read-only cleared), `remove_dir_all` for folders (never follows links inside), and a link
  item removes the link itself.
- **Archive**: copy to `<destination>\<yyyy-mm-dd UTC>\<original path without the drive colon>` (`UNC\server\share\...`
  for network sources), hashing the source while copying into `<name>.hoardini-partial`; flush; re-read the copy; size
  and BLAKE3 must match; rename into place; then delete the source. Folders copy file by file with the same check; links
  inside are skipped and named in the item detail. A re-run reuses a final copy whose BLAKE3 still matches. User
  folders (no catalog entry, `user-data`, `downloads`) get a `<name>.hoardini.txt` stub naming the copy; caches do not
  (`meta.archive_stubs = false` turns stubs off).
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
