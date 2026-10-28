-- Run in a NEW local database. Scripts are not transactions.
CREATE TABLE purchases (id UInt64, region String, amount UInt64);
INSERT INTO purchases VALUES (1, 'EU', 20), (2, 'US', 50), (3, 'EU', 30), (4, 'APAC', 12);
SELECT id, region, amount FROM purchases ORDER BY id;
SELECT region, count(*) FROM purchases GROUP BY region ORDER BY region;
SELECT sum(amount) FROM purchases WHERE amount >= 20;
SELECT id, region FROM purchases WHERE region IN ('EU', 'US') ORDER BY id LIMIT 3;
