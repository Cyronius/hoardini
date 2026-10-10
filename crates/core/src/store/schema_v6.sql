-- Compression estimates for review candidates (ACT-COMPRESS-CANDIDATES).
CREATE TABLE compress_estimate (
  path          TEXT PRIMARY KEY COLLATE NOCASE,
  size          INTEGER NOT NULL,
  newest_ms     INTEGER,
  logical       INTEGER NOT NULL,
  saved_bytes   INTEGER NOT NULL,
  ratio         REAL NOT NULL,
  estimated_ms  INTEGER NOT NULL
);
