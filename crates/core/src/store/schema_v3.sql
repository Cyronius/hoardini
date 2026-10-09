-- One continuous conversation ('main'); the model sees messages from window_start_seq on (AGENT-CONTEXT-WINDOW).
ALTER TABLE conversation ADD COLUMN window_start_seq INTEGER NOT NULL DEFAULT 0;

INSERT OR IGNORE INTO conversation(id, title, provider_id, created_ms, updated_ms)
  SELECT 'main', NULL, NULL, MIN(created_ms), MAX(updated_ms) FROM conversation HAVING COUNT(*) > 0;

-- Old chats are folded in time order; only the most recent one stays inside the window.
CREATE TEMP TABLE fold AS
  SELECT m.id AS id, m.conversation_id AS conv,
         ROW_NUMBER() OVER (ORDER BY c.created_ms, c.id, m.seq) AS n
  FROM message m JOIN conversation c ON c.id = m.conversation_id
  WHERE m.conversation_id <> 'main';

UPDATE conversation SET window_start_seq = COALESCE((
    SELECT MIN(f.n) FROM fold f
    WHERE f.conv = (SELECT id FROM conversation WHERE id <> 'main' ORDER BY updated_ms DESC, id DESC LIMIT 1)
  ), 0)
  WHERE id = 'main';

UPDATE message SET seq = (SELECT n FROM fold WHERE fold.id = message.id), conversation_id = 'main'
  WHERE id IN (SELECT id FROM fold);
UPDATE proposal SET conversation_id = 'main' WHERE conversation_id IS NOT NULL;
DELETE FROM conversation WHERE id <> 'main';
DROP TABLE fold;

-- Memory limit (MEM-LIMIT): least recently used, unpinned, non-protect notes are forgotten first.
ALTER TABLE memory ADD COLUMN last_used_ms INTEGER;
ALTER TABLE memory ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;
ALTER TABLE memory ADD COLUMN forgotten_ms INTEGER;
UPDATE memory SET last_used_ms = updated_ms;
UPDATE memory SET pinned = 1 WHERE source = 'user';
CREATE INDEX memory_lru ON memory(forgotten_ms, pinned, last_used_ms);

-- Touching last_used_ms must not re-index the note text.
DROP TRIGGER memory_au;
CREATE TRIGGER memory_au AFTER UPDATE OF text, key ON memory BEGIN
  INSERT INTO memory_fts(memory_fts, rowid, text, key) VALUES ('delete', old.id, old.text, old.key);
  INSERT INTO memory_fts(rowid, text, key) VALUES (new.id, new.text, new.key);
END;

-- Refreshed follow-up cards (ACT-FOLLOW-UP) and the memory shown on each item (POLICY-ITEM-MEMORY).
ALTER TABLE proposal ADD COLUMN parent_id TEXT;
CREATE INDEX proposal_parent ON proposal(parent_id);
ALTER TABLE proposal_item ADD COLUMN memory_note TEXT;
