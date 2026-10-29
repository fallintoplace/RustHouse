-- Full TPC-H Q16 validation query. Source: https://raw.githubusercontent.com/duckdb/duckdb/6287a4e595d1c6c07d4241676385cbffbd232f07/extension/tpch/dbgen/queries/q16.sql
-- Copyright 2018-2026 Stichting DuckDB Foundation. SPDX-License-Identifier: MIT
SELECT
    p_brand,
    p_type,
    p_size,
    count(DISTINCT ps_suppkey) AS supplier_cnt
FROM
    partsupp,
    part
WHERE
    p_partkey = ps_partkey
    AND p_brand <> 'Brand#45'
    AND p_type NOT LIKE 'MEDIUM POLISHED%'
    AND p_size IN (49, 14, 23, 45, 19, 3, 36, 9)
    AND ps_suppkey NOT IN (
        SELECT
            s_suppkey
        FROM
            supplier
        WHERE
            s_comment LIKE '%Customer%Complaints%')
GROUP BY
    p_brand,
    p_type,
    p_size
ORDER BY
    supplier_cnt DESC,
    p_brand,
    p_type,
    p_size;
