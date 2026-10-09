# UX revisions, WP-DRIVES: drive selector (index)

Design: `.claude/plans/ux-revisions.md` §11 (plus §6 and §8, which apply to every watched drive).

## New requirements

- **INDEX-VOLUMES** (test): `meta.indexed_volumes` (watched roots, default the system drive) and `meta.selected_volume` (default the system drive when watched, else the first watched root). `GET /api/volumes` lists every fixed drive plus any watched root that is not one, with `root, label, fs, total_bytes, free_bytes, watched, selected, connected, has_journal`. `PUT /api/volumes {watched, selected}` validates (at least one watched root; `selected` among them; drive-letter paths only; a newly added root must exist), normalizes, stores both, and answers the new list. Adding a root starts its first scan at once, in fast mode. Removing one stops its refresh (killing its child first if it is running) and deletes its `volume` row, `node` rows, scan history, evidence cache entries, and duplicate-group members, leaving every other volume untouched. A watched root that does not exist is skipped by refreshes, keeps its index, and reports `connected: false`. `/api/status` volumes carry `selected` and `connected`; `/api/index/summary` (the welcome-back card) reports the selected drive.

Added during implementation: `subst` drives (folders on another volume) are left out of the list, and a delete cut short by a crash finishes at the next start (`meta.volumes_forgetting`).

## Modified requirements

None.

## Removed requirements

None.
