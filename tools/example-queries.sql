-- Exactly one read-only SELECT per line. EXPLAIN will not read data.
SELECT number FROM numbers(5) ORDER BY number DESC LIMIT 2;
SELECT count(*) FROM numbers(100);
SELECT id, region FROM events WHERE id >= 10 ORDER BY id LIMIT 20;
