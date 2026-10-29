-- Full TPC-H Q11 validation query. Source: https://raw.githubusercontent.com/duckdb/duckdb/6287a4e595d1c6c07d4241676385cbffbd232f07/extension/tpch/dbgen/queries/q11.sql
-- Copyright 2018-2026 Stichting DuckDB Foundation. SPDX-License-Identifier: MIT
SELECT
    ps_partkey,
    sum(ps_supplycost * ps_availqty) AS value
FROM
    partsupp,
    supplier,
    nation
WHERE
    ps_suppkey = s_suppkey
    AND s_nationkey = n_nationkey
    AND n_name = 'GERMANY'
GROUP BY
    ps_partkey
HAVING
    sum(ps_supplycost * ps_availqty) > (
        SELECT
            sum(ps_supplycost * ps_availqty) * 0.0001000000
        FROM
            partsupp,
            supplier,
            nation
        WHERE
            ps_suppkey = s_suppkey
            AND s_nationkey = n_nationkey
            AND n_name = 'GERMANY')
ORDER BY
    value DESC;
