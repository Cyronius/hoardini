# UX revisions WP-SET: the built-in watchdog sleeps with its child (prov)

Source: `.claude/plans/ux-revisions.md` §7, §8.

## Modified requirements

- **PROV-BUILTIN-IDLE** (test): the watchdog exists only while a llama-server child does. It starts with the child and exits once the child is gone (idle stop, crash, or removal). Acceptance adds: no watchdog before the first acquire, one after it, and none after it has stopped an idle child.
- **PROV-BUILTIN-CHILD** (test): drop "and `redact_paths: true`" from the Built-in profile (the field is gone, AGENT-REDACT).

## New requirements

None.

## Removed requirements

None.
