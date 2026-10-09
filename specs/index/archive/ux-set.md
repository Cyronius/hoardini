# UX revisions WP-SET: fixed threshold and Fast / Lean (index)

Source: `.claude/plans/ux-revisions.md` §6.

## Modified requirements

- **INDEX-THRESHOLD** (test): the threshold is a fixed 1 MiB constant; `meta.threshold_bytes` is no longer read. `hoardini index --threshold` stays as a one-run override for testing. Acceptance moves from "set the setting" to "refresh with an override", and adds: a leftover `meta.threshold_bytes` is ignored.
- **INDEX-POLITE** (manual): background mode always uses 2 workers (no `meta.indexer_workers`); `--workers` stays as an override. Automatic refreshes and Rescan now follow `index_speed` (Lean = background, Fast = fast).
- **INDEX-MEMORY-CAP** (manual): the Job Object cap is a fixed 256 MB (no `meta.indexer_memory_cap_mb`); `--memory-cap-mb` stays as an override.

## New requirements

None.

## Removed requirements

None.
