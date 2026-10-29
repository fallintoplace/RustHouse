-- TPC-H Q4 full validation SQL. Source: https://raw.githubusercontent.com/duckdb/duckdb/6287a4e595d1c6c07d4241676385cbffbd232f07/extension/tpch/dbgen/queries/q04.sql
-- Copyright 2018-2026 Stichting DuckDB Foundation. SPDX-License-Identifier: MIT
SELECT
    o_orderpriority,
    count(*) AS order_count
FROM
    orders
WHERE
    o_orderdate >= CAST('1993-07-01' AS date)
    AND o_orderdate < CAST('1993-10-01' AS date)
    AND EXISTS (
        SELECT
            *
        FROM
            lineitem
        WHERE
            l_orderkey = o_orderkey
            AND l_commitdate < l_receiptdate)
GROUP BY
    o_orderpriority
ORDER BY
    o_orderpriority;
