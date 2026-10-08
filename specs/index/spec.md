# INDEX — persistent disk index

Canonical spec for the indexer (`hoardini-core::index`, `hoardini index`). Design: `.claude/plans/hoardini-design.md` §5.1.
INDEX-JOURNAL-EQUIV (M5, USN journal refresh) lives here too once M5 adds it.

Tests: `specs/index/tests/index.rs` (`cargo test -p hoardini-specs --test index`). Most run against `index::FakeFs`, an
in-memory volume; the `real_*` tests run the real lister (`index::WinLister`) on a temp directory.

## Terms

- **Row**: a `node` table entry. Every directory and every link has one; a file has one when
  `max(allocated, logical) >= threshold`.
- **Counted size** of a file: its allocated bytes, or 0 when it is a hard-link duplicate (see INDEX-HARDLINK-ONCE).
- **Subtree totals** of a directory (`sub_alloc`, `sub_logical`, `sub_count`): its folded small files plus the counted
  size of every child file row plus the subtree totals of every child directory. Links count 0.
- **Refresh**: one full walk of a volume (`index::refresh`). A first build is a refresh of an empty index.

## Requirements

### INDEX-ALLOC-SIZE: Allocated-size accounting
**Applies to:** hoardini
**Verification:** test

Sizes shall be allocated bytes. A file whose listing reports 0 allocated bytes, a logical size above 0, no sparse
attribute, and no cloud-placeholder attribute (the measured signature of a WOF-compressed file) shall be sized by one
open that reads its true allocation (`alloc_size_by_open`); when that open fails the logical size is used. A sparse
file keeps its listed allocation. A cloud placeholder counts its listed allocation (0 when cloud-only) and is never
opened. Logical size is kept for display.

**Acceptance criteria:**
- File listed alloc 8 MiB / logical 5 MiB → row size 8 MiB, logical 5 MiB.
- WOF file listed alloc 0 / logical 4 MiB, true allocation 20 KiB → size 20 KiB, logical 4 MiB.
- Sparse file listed alloc 1 MiB / logical 10 GiB → size 1 MiB; sparse file listed alloc 0 → size 0, not opened.
- Cloud placeholder listed alloc 0 / logical 5 GiB → size 0, logical 5 GiB, `placeholder` flag set, never opened.
- Exactly the WOF file is opened; the parent's subtree size is the sum of the above plus folded small files.
- Real lister on a temp dir: a 2 MiB + 1 B file has logical 2 MiB + 1 B and allocated ≥ logical and < logical + 1 MiB;
  file counts and logical totals of the dir and the root are exact.

### INDEX-HARDLINK-ONCE: Hard links count once per volume
**Applies to:** hoardini
**Verification:** test

A file with several hard links shall count once per volume, keyed by its 128-bit file ID. Every link at or above the
threshold keeps a row; exactly one is counted and the others carry the `hardlink_dup` flag with counted size 0. The
counted link is the one with the smallest path (case-insensitive), independent of walk order or worker count. Links
below the threshold fold once, into the directory of the first link the walk meets. A file ID of all zeros means
"unknown" (the portable lister) and is never merged. On NTFS the dedupe set is a bitset over MFT record numbers;
other file systems use an exact hash set.

**Acceptance criteria:**
- One 200 MiB file with links `C:\b\model.gguf`, `C:\a\copy.gguf`, `C:\c\third.gguf`: the volume counts 200 MiB once,
  `C:\a\copy.gguf` is counted, the other two are flagged, `C:\a` = 200 MiB, `C:\b` = 0 — on every run with 4 workers,
  for both the NTFS (bitset) and the non-NTFS (hash set) path.
- A 100 KiB file linked from two directories adds 100 KiB in total to their small totals.
- Two files with all-zero IDs both count.
- Real lister: a 2 MiB file plus a hard link in another dir counts once when the lister reports file IDs, twice when
  it reports zero IDs.

### INDEX-NO-FOLLOW: Junctions and symlinks are never traversed
**Applies to:** hoardini
**Verification:** test

The indexer shall not list the contents of a junction or symbolic link. Any reparse point whose tag is a name
surrogate (junction, symlink, WSL symlink), and any reparse directory whose tag the lister cannot read (tag 0), is
recorded as a `link` row with size 0. A reparse directory with a non-surrogate tag (for example a cloud-files folder)
is an ordinary directory and is traversed.

**Acceptance criteria:**
- A junction, a directory symlink, and a tagless reparse directory, each "containing" a 5 GiB file, are never listed;
  each has a `link` row of size 0; no row exists beneath them; the root total excludes the 5 GiB files.
- A file symlink is a `link` row of size 0.
- A directory with tag `0x9000601A` (cloud files) is listed and its 2 MiB file counts.
- Real lister: a `mklink /J` junction inside a temp dir is a `link` row; its target's file is counted once, via the
  target only.

### INDEX-SMALL-FOLD: Small files fold into their parent
**Applies to:** hoardini
**Verification:** test

Files with `max(allocated, logical)` below the threshold shall get no row; their count, allocated bytes, and logical
bytes add to the parent directory's `small_count`, `small_bytes`, `small_logical`. For every directory, subtree totals
shall equal its small totals plus its counted children (`index::verify_totals` returns no problems).

**Acceptance criteria:**
- For a generated tree of 144 files around the 1 MiB threshold: every file ≥ threshold has a row with its allocated
  size, every smaller file has none, and each directory's `small_count`/`small_bytes` equal the sum of its small files.
- The root's allocated, logical, and file-count totals equal the sums over all files.
- `verify_totals` is empty.

### INDEX-THRESHOLD: File-row threshold is a setting
**Applies to:** hoardini
**Verification:** test

The file-row threshold shall be the `meta.threshold_bytes` setting, default 1 MiB (1,048,576 bytes); `hoardini index
--threshold` overrides it for one run. The threshold used by the last completed refresh is stored on the volume.
After a change, the next full refresh shall produce the same index as a fresh build at the new threshold.

**Acceptance criteria:**
- With no setting, the completed refresh records threshold 1,048,576; a file of exactly 1 MiB has a row, one of
  1 MiB − 1 does not.
- After setting 64 KiB, then 4 MiB, then 1 MiB, each following refresh's snapshot equals a fresh build at that
  threshold, and totals verify.

### INDEX-REFRESH-EQUIV: A refresh equals a fresh build
**Applies to:** hoardini
**Verification:** test

After any mix of changes on the volume, a completed refresh of an existing index shall produce the same index as a
fresh build of the same tree: same rows (by path and kind), sizes, small totals, subtree totals, times, attributes,
reparse tags, file IDs, and flags (`index::snapshot`). Hotspot-first ordering and the worker count do not change the
result. Exception: for a hard-linked file below the threshold whose links sit in different directories, which
directory's small totals hold its bytes may differ (their common ancestors' totals do not).

**Acceptance criteria:**
- Starting from a sample tree, two rounds of changes — added, deleted, grown, shrunk, and renamed files and dirs; a
  file crossing the threshold both ways; a file replaced by a same-named dir; a dir replaced by a junction and a
  junction by a dir; a denied dir becoming readable and a new denied dir; a hard-link owner removed and a new link
  added; a WOF file's allocation changed; an mtime change — each refresh with 3 workers yields a snapshot equal to a
  fresh build. A further refresh with 1 worker does too.

### INDEX-ACTION-PATCH: Executed actions patch the index at once
**Applies to:** hoardini
**Verification:** test

The executor shall update the index right after an action, without a refresh: `index::patch_removed(conn, path)` for a
deleted or archived path that had a row, and `index::patch_rescan(conn, lister, path)` for anything else (a removed
small file, a compressed file, a replaced or hard-linked file, a new path). Both update the row(s) and every ancestor's
subtree totals in one transaction, and re-resolve hard-link ownership.

**Acceptance criteria:**
- Deleting a 300 MiB file + `patch_removed`: the row is gone and its parent, grandparent, … root each shrink by exactly
  its size.
- Deleting a dir subtree + `patch_removed` removes every row beneath it.
- Compressing a file (alloc 500 → 100 MiB) + `patch_rescan` returns the node with size 100 MiB.
- Deleting a small file + `patch_rescan` lowers the parent's `small_count` by 1.
- Replacing a copy with a hard link + `patch_rescan` counts the pair once (the smallest path owns it); removing the
  owner + `patch_removed` promotes the remaining link.
- `patch_rescan` of a removed directory returns `None`; of a new directory indexes its contents.
- After every step the snapshot equals a fresh build and totals verify.

### INDEX-INTERRUPT-SAFE: An interrupted refresh leaves a valid index
**Applies to:** hoardini
**Verification:** test

A refresh that stops early (cancelled, killed, or failed) shall delete no rows and leave every committed batch
consistent: rows it reached are new, the rest are old, and subtree totals equal the sum of their parts. Rows not seen
in a generation are deleted only when that refresh completes. An interrupted refresh records no history entry. The next
refresh starts over.

**Acceptance criteria:**
- After a completed build and a round of changes, refreshes cancelled after 1, 4, and 9 listings each return
  `completed = false` and no scan id; every path present before is still present; `verify_totals` is empty; the
  scan count is unchanged.
- The next completed refresh equals a fresh build.

### INDEX-NO-HYDRATE: Cloud-only files are never read
**Applies to:** hoardini
**Verification:** manual

No component shall open or read the data of a file with `RECALL_ON_DATA_ACCESS`, `RECALL_ON_OPEN`, or `OFFLINE`
attributes. Listing a directory and reading attributes is allowed. (M0 covers the indexer; the evidence engine, hashing,
compression, and archive add their own checks against this requirement.)

**Verification (manual):**
1. In OneDrive (or SVI Dropbox), pick a folder with at least one file over 100 MB and choose "Free up space" so every
   file shows the cloud-only icon.
2. Run `hoardini index --db %TEMP%\h-hydrate.db --volume C:\ --mode fast`.
3. Confirm the files still show the cloud-only icon and the sync client shows no download for them.
4. `hoardini --db %TEMP%\h-hydrate.db top --files --under "<folder>"` lists them at 0 B with "(cloud only)".
5. Delete `%TEMP%\h-hydrate.db*`.

### INDEX-UNACCOUNTED: Each refresh reports the unaccounted gap
**Applies to:** hoardini
**Verification:** manual

Each completed refresh of a volume root shall report `unaccounted = (total − free) − indexed total` in its `done` line,
its `scan` row, `index_status`, and the prompt summary. It is omitted when the indexed root is not a volume root or
volume space is unknown.

**Verification (manual):**
1. Run `hoardini index --db %TEMP%\h-gap.db --volume C:\ --mode fast` and note `indexed_bytes` and `unaccounted_bytes`
   from the `done` line.
2. Run `fsutil volume diskfree C:` at about the same time; used = total − free.
3. Check `unaccounted_bytes` ≈ used − `indexed_bytes` (within what changed between the two readings).
4. `vssadmin list shadowstorage` shows the shadow-copy share of the gap.
5. Delete `%TEMP%\h-gap.db*`.

### INDEX-POLITE: Background mode is visibly polite
**Applies to:** hoardini
**Verification:** manual

In background mode the indexer shall run with process background mode (low CPU, very low I/O and memory priority) and
EcoQoS, with 2 workers by default (`meta.indexer_workers`). Fast mode uses all cores at normal priority. If the OS calls
fail, the refresh logs a warning and continues.

**Verification (manual):**
1. Start `hoardini index --db %TEMP%\h-polite.db --volume C:\ --mode background`.
2. Task Manager → Processes: `hoardini.exe` shows the Efficiency mode leaf.
3. Resource Monitor → Disk → Processes with Disk Activity: `hoardini.exe` I/O priority is "Very Low".
4. Repeat with `--mode fast`: no leaf, I/O priority "Normal".
5. Delete `%TEMP%\h-polite.db*`.

### INDEX-MEMORY-CAP: Background refresh stays small
**Applies to:** hoardini
**Verification:** manual

A background refresh of C: shall keep the indexer's working set under 100 MB. A Job Object caps the process at
`meta.indexer_memory_cap_mb` (default 256 MB, `--memory-cap-mb`) as a backstop. Bounded channels and a depth-first
work stack keep the pending tree small; the SQLite page cache is 16 MB.

**Verification (manual):**
1. Start `hoardini index --db %TEMP%\h-mem.db --volume C:\ --mode background`.
2. In PowerShell, sample every 2 s until it exits:
   `while ($p = Get-Process hoardini -ErrorAction SilentlyContinue) { $p.WorkingSet64 / 1MB; Start-Sleep 2 }`.
3. The peak stays under 100 MB. Record the peak, the duration, `dirs`/`files` from the `done` line, and the DB size.
4. Delete `%TEMP%\h-mem.db*`.

### INDEX-STALE-AGE: Answers from the stored index state its age
**Applies to:** hoardini
**Verification:** manual

Every answer drawn from the stored index shall state how old it is: `index_status` returns `age_ms` and
`refresh_incomplete`; `summary_for_prompt` starts with "Index of <volume> as of <age>"; `hoardini top` prints the same
header and notes a refresh that has not finished.

**Verification (manual):**
1. Index C: with a temp `--db`, wait at least two minutes.
2. Start a second refresh; while it runs, run `hoardini --db <db> top`.
3. The header reads "Index of C:\ as of N minutes ago (refresh 1), a newer refresh has not finished".
4. After it completes, `top` reads "as of just now (refresh 2)" with no unfinished note.
