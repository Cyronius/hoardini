-- Refreshed follow-up cards (ACT-FOLLOW-UP) and the memory shown on each item (POLICY-ITEM-MEMORY).
ALTER TABLE proposal ADD COLUMN parent_id TEXT;
CREATE INDEX proposal_parent ON proposal(parent_id);
ALTER TABLE proposal_item ADD COLUMN memory_note TEXT;
