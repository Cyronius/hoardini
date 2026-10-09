# UX revisions WP-SET: settings shrink, presence, idle zero (srv)

Source: `.claude/plans/ux-revisions.md` §6, §8, Decisions (first scan runs Fast).

## Modified requirements

- **SRV-SETTINGS** (test): the keys become `index_speed` (`lean` | `fast`, default `lean`) and `builtin_idle_minutes`, plus read-only `index_size_bytes`. `threshold_bytes`, `redact_paths`, `web_search_enabled`, `indexer_workers`, and `indexer_memory_cap_mb` are gone: not returned, and a `PUT` of them changes nothing.
- **SRV-ONE-REFRESH** (test): `POST /api/index/refresh` takes no body; its mode comes from `index_speed` (`lean` → `background`, `fast` → `fast`), except that while no refresh has ever completed it runs `fast`. "Scan fast takes over" becomes "setting `index_speed` to `fast` during a `background` refresh takes it over"; setting `lean` during a `fast` refresh lets it finish.
- **SRV-JOURNAL-REFRESH** (test): the timer runs only while someone is looking (SRV-PRESENCE), and in the mode `index_speed` names. The acceptance test opens a status stream and posts `looking: true`.

## New requirements

- **SRV-PRESENCE** (test): looking = the last `POST /api/presence {looking}` said true and a status stream is open; the last stream closing resets it. No journal timer exists while not looking. On arrival, an index older than the journal interval (or never completed) gets one journal refresh at once; otherwise the next one is due when the index reaches that age. A running refresh is never cancelled when presence drops. Deterministic part: `journal_wait(age, every)`.
- **SRV-IDLE-ZERO** (manual): minimized for 10 minutes, the app has no indexer, llama-server, or rclone child and no periodic wake-ups.

## Removed requirements

None.
