# UX revisions, WP-DRIVES: drive selector (srv)

Design: `.claude/plans/ux-revisions.md` §11.

## Modified requirements

- **SRV-ONE-REFRESH** (test): the first-scan rule is per volume. A volume whose index has never completed (a new install, or a drive just added) is refreshed `fast`, whatever `index_speed` says; the other volumes of the same run keep the switch's mode. Unplugged watched volumes are skipped (INDEX-VOLUMES).

## New requirements

None here. The volume routes are specified by INDEX-VOLUMES (`specs/index/spec.md`); their server-side tests live in `specs/srv/tests/srv.rs`.

## Removed requirements

None.
