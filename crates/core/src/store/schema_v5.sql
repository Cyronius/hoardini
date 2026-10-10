-- Review-first (specs/review): memory detail, tool-card data, review runs, copy checks.
ALTER TABLE memory ADD COLUMN detail_json TEXT;
ALTER TABLE message ADD COLUMN ui_json TEXT;
ALTER TABLE proposal ADD COLUMN anchor TEXT;
ALTER TABLE proposal ADD COLUMN review_id TEXT;
ALTER TABLE proposal ADD COLUMN card TEXT;
ALTER TABLE proposal ADD COLUMN touched INTEGER NOT NULL DEFAULT 0;
CREATE INDEX proposal_review ON proposal(review_id);

CREATE TABLE review (
  id           TEXT PRIMARY KEY,
  volume_id    INTEGER,
  scope_path   TEXT,
  scan_id      INTEGER,
  created_ms   INTEGER NOT NULL,
  areas_json   TEXT NOT NULL DEFAULT '[]'
);
CREATE INDEX review_volume ON review(volume_id, created_ms);

CREATE TABLE copy_check (
  id           INTEGER PRIMARY KEY,
  volume_id    INTEGER NOT NULL,
  path         TEXT NOT NULL,
  file_id      BLOB,
  size         INTEGER NOT NULL,
  mtime        INTEGER,
  check_name   TEXT NOT NULL,
  confidence   TEXT NOT NULL,
  locator      TEXT,
  detail_json  TEXT,
  checked_ms   INTEGER NOT NULL
);
CREATE UNIQUE INDEX copy_check_key ON copy_check(volume_id, path COLLATE NOCASE, check_name);

CREATE TABLE file_hash (
  volume_id    INTEGER NOT NULL,
  file_id      BLOB NOT NULL,
  size         INTEGER NOT NULL,
  mtime        INTEGER,
  sha256       TEXT NOT NULL,
  hashed_ms    INTEGER NOT NULL,
  PRIMARY KEY (volume_id, file_id)
);
