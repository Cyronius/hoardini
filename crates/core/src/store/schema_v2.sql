-- Rewriting every row is far cheaper with the touched indexes dropped and rebuilt sorted afterwards.
DROP INDEX node_file_alloc;
DROP INDEX node_sub_alloc;
DROP INDEX node_file_id;

-- NTFS ids (zero high half) become their low 8 bytes as an INTEGER (index::FileId); dir atime/ctime go unread.
UPDATE node SET
  file_id = CASE WHEN typeof(file_id) = 'blob' AND length(file_id) = 16 AND substr(file_id, 9, 8) = zeroblob(8) THEN
      (((instr('0123456789ABCDEF', substr(hex(file_id), 1, 1)) - 1) * 16 + instr('0123456789ABCDEF', substr(hex(file_id), 2, 1)) - 1))
    | (((instr('0123456789ABCDEF', substr(hex(file_id), 3, 1)) - 1) * 16 + instr('0123456789ABCDEF', substr(hex(file_id), 4, 1)) - 1) << 8)
    | (((instr('0123456789ABCDEF', substr(hex(file_id), 5, 1)) - 1) * 16 + instr('0123456789ABCDEF', substr(hex(file_id), 6, 1)) - 1) << 16)
    | (((instr('0123456789ABCDEF', substr(hex(file_id), 7, 1)) - 1) * 16 + instr('0123456789ABCDEF', substr(hex(file_id), 8, 1)) - 1) << 24)
    | (((instr('0123456789ABCDEF', substr(hex(file_id), 9, 1)) - 1) * 16 + instr('0123456789ABCDEF', substr(hex(file_id), 10, 1)) - 1) << 32)
    | (((instr('0123456789ABCDEF', substr(hex(file_id), 11, 1)) - 1) * 16 + instr('0123456789ABCDEF', substr(hex(file_id), 12, 1)) - 1) << 40)
    | (((instr('0123456789ABCDEF', substr(hex(file_id), 13, 1)) - 1) * 16 + instr('0123456789ABCDEF', substr(hex(file_id), 14, 1)) - 1) << 48)
    | (((instr('0123456789ABCDEF', substr(hex(file_id), 15, 1)) - 1) * 16 + instr('0123456789ABCDEF', substr(hex(file_id), 16, 1)) - 1) << 56)
    ELSE file_id END,
  atime = CASE WHEN kind = 0 THEN NULL ELSE atime END,
  ctime = CASE WHEN kind = 0 THEN NULL ELSE ctime END
WHERE (typeof(file_id) = 'blob' AND length(file_id) = 16 AND substr(file_id, 9, 8) = zeroblob(8))
   OR (kind = 0 AND (atime IS NOT NULL OR ctime IS NOT NULL));

CREATE INDEX node_file_id ON node(volume_id, file_id);
-- Only files are ranked by size, and only dirs >= 256 MiB (history::DIR_MIN_BYTES) by subtree size.
CREATE INDEX node_file_alloc ON node(alloc DESC) WHERE kind = 1;
CREATE INDEX node_sub_alloc ON node(sub_alloc DESC) WHERE kind = 0 AND sub_alloc >= 268435456;
