CREATE TABLE meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE volume (
  id               INTEGER PRIMARY KEY,
  root             TEXT NOT NULL UNIQUE,
  serial           INTEGER,
  fs_name          TEXT,
  label            TEXT,
  total_bytes      INTEGER,
  free_bytes       INTEGER,
  root_node_id     INTEGER,
  gen              INTEGER NOT NULL DEFAULT 0,
  threshold        INTEGER,
  last_refresh_ms  INTEGER,
  usn_journal_id   INTEGER,
  usn_next         INTEGER
);

-- kind: 0 dir, 1 file, 2 link (junction/symlink, never followed)
-- flags: bit0 access denied, bit1 cloud placeholder, bit2 hard-link duplicate (bytes counted elsewhere)
CREATE TABLE node (
  id             INTEGER PRIMARY KEY,
  parent_id      INTEGER,
  volume_id      INTEGER NOT NULL,
  name           TEXT NOT NULL,
  kind           INTEGER NOT NULL,
  alloc          INTEGER NOT NULL DEFAULT 0,
  logical        INTEGER NOT NULL DEFAULT 0,
  small_count    INTEGER NOT NULL DEFAULT 0,
  small_bytes    INTEGER NOT NULL DEFAULT 0,
  small_logical  INTEGER NOT NULL DEFAULT 0,
  sub_alloc      INTEGER NOT NULL DEFAULT 0,
  sub_logical    INTEGER NOT NULL DEFAULT 0,
  sub_count      INTEGER NOT NULL DEFAULT 0,
  mtime          INTEGER,
  atime          INTEGER,
  ctime          INTEGER,
  attrs          INTEGER NOT NULL DEFAULT 0,
  reparse_tag    INTEGER,
  file_id        BLOB,
  flags          INTEGER NOT NULL DEFAULT 0,
  seen_gen       INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX node_parent_name ON node(parent_id, name COLLATE NOCASE);
CREATE INDEX node_file_alloc ON node(kind, alloc DESC);
CREATE INDEX node_sub_alloc ON node(kind, sub_alloc DESC);
CREATE INDEX node_file_id ON node(volume_id, file_id);

CREATE TABLE scan (
  id                 INTEGER PRIMARY KEY,
  volume_id          INTEGER NOT NULL,
  gen                INTEGER NOT NULL,
  kind               TEXT NOT NULL,
  mode               TEXT NOT NULL,
  started_ms         INTEGER NOT NULL,
  finished_ms        INTEGER,
  total_bytes        INTEGER,
  used_bytes         INTEGER,
  free_bytes         INTEGER,
  indexed_bytes      INTEGER,
  unaccounted_bytes  INTEGER,
  file_count         INTEGER,
  dir_count          INTEGER,
  denied_count       INTEGER,
  keep_monthly       INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX scan_volume ON scan(volume_id, finished_ms);

CREATE TABLE scan_dir (
  scan_id  INTEGER NOT NULL,
  path     TEXT NOT NULL,
  alloc    INTEGER NOT NULL,
  count    INTEGER NOT NULL,
  PRIMARY KEY (scan_id, path)
) WITHOUT ROWID;

CREATE TABLE scan_file (
  scan_id  INTEGER NOT NULL,
  path     TEXT NOT NULL,
  alloc    INTEGER NOT NULL,
  mtime    INTEGER,
  PRIMARY KEY (scan_id, path)
) WITHOUT ROWID;

CREATE TABLE evidence_cache (
  node_id     INTEGER PRIMARY KEY,
  file_id     BLOB,
  mtime       INTEGER,
  json        TEXT NOT NULL,
  created_ms  INTEGER NOT NULL
);

CREATE TABLE dup_group (
  id          INTEGER PRIMARY KEY,
  size        INTEGER NOT NULL,
  hash        TEXT NOT NULL,
  created_ms  INTEGER NOT NULL
);

CREATE TABLE dup_member (
  group_id  INTEGER NOT NULL,
  path      TEXT NOT NULL,
  node_id   INTEGER,
  file_id   BLOB,
  mtime     INTEGER,
  PRIMARY KEY (group_id, path)
) WITHOUT ROWID;

-- scope: path | glob | global; kind: fact | preference | protect | verdict
-- key: profile-relative path or glob (~\...), or a short name for global rows
CREATE TABLE memory (
  id               INTEGER PRIMARY KEY,
  scope            TEXT NOT NULL CHECK (scope IN ('path', 'glob', 'global')),
  key              TEXT NOT NULL,
  kind             TEXT NOT NULL CHECK (kind IN ('fact', 'preference', 'protect', 'verdict')),
  text             TEXT NOT NULL,
  source           TEXT NOT NULL,
  confidence       REAL,
  created_ms       INTEGER NOT NULL,
  updated_ms       INTEGER NOT NULL,
  review_after_ms  INTEGER
);
CREATE INDEX memory_key ON memory(scope, key COLLATE NOCASE);

CREATE VIRTUAL TABLE memory_fts USING fts5(text, key, content='memory', content_rowid='id');
CREATE TRIGGER memory_ai AFTER INSERT ON memory BEGIN
  INSERT INTO memory_fts(rowid, text, key) VALUES (new.id, new.text, new.key);
END;
CREATE TRIGGER memory_ad AFTER DELETE ON memory BEGIN
  INSERT INTO memory_fts(memory_fts, rowid, text, key) VALUES ('delete', old.id, old.text, old.key);
END;
CREATE TRIGGER memory_au AFTER UPDATE ON memory BEGIN
  INSERT INTO memory_fts(memory_fts, rowid, text, key) VALUES ('delete', old.id, old.text, old.key);
  INSERT INTO memory_fts(rowid, text, key) VALUES (new.id, new.text, new.key);
END;

CREATE TABLE conversation (
  id           TEXT PRIMARY KEY,
  title        TEXT,
  provider_id  TEXT,
  created_ms   INTEGER NOT NULL,
  updated_ms   INTEGER NOT NULL
);

-- json: the full message as sent to/received from the LLM, tool results already compacted
CREATE TABLE message (
  id               TEXT PRIMARY KEY,
  conversation_id  TEXT NOT NULL,
  seq              INTEGER NOT NULL,
  role             TEXT NOT NULL,
  json             TEXT NOT NULL,
  created_ms       INTEGER NOT NULL
);
CREATE INDEX message_conv ON message(conversation_id, seq);

-- kind: delete | archive | compress | dedupe
-- status: pending | approved | declined | running | done | partial | failed
CREATE TABLE proposal (
  id               TEXT PRIMARY KEY,
  conversation_id  TEXT,
  tool_call_id     TEXT,
  kind             TEXT NOT NULL,
  params_json      TEXT NOT NULL,
  status           TEXT NOT NULL,
  total_alloc      INTEGER NOT NULL DEFAULT 0,
  created_ms       INTEGER NOT NULL,
  decided_ms       INTEGER,
  job_id           TEXT
);
CREATE INDEX proposal_conv ON proposal(conversation_id);

-- verdict: safe | redownload | review | keep (LLM value after the cap); cap: policy cap or NULL
-- status: pending | blocked | skipped | done | failed
CREATE TABLE proposal_item (
  id             INTEGER PRIMARY KEY,
  proposal_id    TEXT NOT NULL,
  seq            INTEGER NOT NULL,
  path           TEXT NOT NULL,
  node_id        INTEGER,
  kind           INTEGER NOT NULL,
  alloc          INTEGER NOT NULL,
  logical        INTEGER NOT NULL,
  mtime          INTEGER,
  file_id        BLOB,
  verdict        TEXT,
  cap            TEXT,
  reason         TEXT,
  block_reason   TEXT,
  selected       INTEGER NOT NULL DEFAULT 1,
  status         TEXT NOT NULL DEFAULT 'pending',
  detail         TEXT
);
CREATE INDEX proposal_item_proposal ON proposal_item(proposal_id, seq);

CREATE TABLE job (
  id             TEXT PRIMARY KEY,
  kind           TEXT NOT NULL,
  proposal_id    TEXT,
  status         TEXT NOT NULL,
  progress_json  TEXT,
  created_ms     INTEGER NOT NULL,
  finished_ms    INTEGER,
  error          TEXT
);

-- the journal; undo_json says how to reverse the action (archive: destination + uri; compress: format + archive path)
CREATE TABLE action (
  id            INTEGER PRIMARY KEY,
  job_id        TEXT,
  proposal_id   TEXT,
  item_id       INTEGER,
  kind          TEXT NOT NULL,
  path          TEXT NOT NULL,
  status        TEXT NOT NULL,
  freed_bytes   INTEGER NOT NULL DEFAULT 0,
  undo_json     TEXT,
  error         TEXT,
  started_ms    INTEGER NOT NULL,
  finished_ms   INTEGER
);
CREATE INDEX action_proposal ON action(proposal_id);

CREATE TABLE archive_entry (
  id              INTEGER PRIMARY KEY,
  action_id       INTEGER,
  original_path   TEXT NOT NULL,
  destination_id  TEXT NOT NULL,
  dest_uri        TEXT NOT NULL,
  kind            INTEGER NOT NULL,
  size            INTEGER NOT NULL,
  blake3          TEXT,
  stub_path       TEXT,
  archived_ms     INTEGER NOT NULL,
  restored_ms     INTEGER
);
CREATE INDEX archive_entry_path ON archive_entry(original_path COLLATE NOCASE);

-- config_json holds secret://refs only, never secret values
CREATE TABLE destination (
  id           TEXT PRIMARY KEY,
  name         TEXT NOT NULL,
  kind         TEXT NOT NULL CHECK (kind IN ('local', 'smb', 'rclone')),
  config_json  TEXT NOT NULL,
  created_ms   INTEGER NOT NULL
);

CREATE TABLE provider (
  id           TEXT PRIMARY KEY,
  name         TEXT NOT NULL,
  config_json  TEXT NOT NULL,
  caps_json    TEXT,
  is_active    INTEGER NOT NULL DEFAULT 0,
  created_ms   INTEGER NOT NULL
);

CREATE TABLE search_provider (
  id           TEXT PRIMARY KEY,
  kind         TEXT NOT NULL CHECK (kind IN ('tavily', 'brave', 'exa', 'serper', 'searxng')),
  config_json  TEXT NOT NULL,
  is_active    INTEGER NOT NULL DEFAULT 0,
  created_ms   INTEGER NOT NULL
);

CREATE TABLE search_cache (
  provider    TEXT NOT NULL,
  query       TEXT NOT NULL,
  json        TEXT NOT NULL,
  created_ms  INTEGER NOT NULL,
  PRIMARY KEY (provider, query)
) WITHOUT ROWID;
