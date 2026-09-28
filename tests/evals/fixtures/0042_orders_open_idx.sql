-- Add orders index

-- Partial index: 97% of rows are 'settled' and never filtered by status, so indexing
-- only the open states keeps it small enough to stay in memory.
CREATE INDEX CONCURRENTLY orders_open_status_idx ON orders (status) WHERE status <> 'settled';

-- CONCURRENTLY can't run inside a transaction block: the migration runner must have
-- transactional = false for this file.

-- DROP INDEX orders_status_idx;
