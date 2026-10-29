-- TPC-H Q3 full validation SQL. Source: https://raw.githubusercontent.com/duckdb/duckdb/6287a4e595d1c6c07d4241676385cbffbd232f07/extension/tpch/dbgen/queries/q03.sql
-- Copyright 2018-2026 Stichting DuckDB Foundation. SPDX-License-Identifier: MIT
SELECT
    l_orderkey,
    sum(l_extendedprice * (1 - l_discount)) AS revenue,
    o_orderdate,
    o_shippriority
FROM
    customer,
    orders,
    lineitem
WHERE
    c_mktsegment = 'BUILDING'
    AND c_custkey = o_custkey
    AND l_orderkey = o_orderkey
    AND o_orderdate < CAST('1995-03-15' AS date)
    AND l_shipdate > CAST('1995-03-15' AS date)
GROUP BY
    l_orderkey,
    o_orderdate,
    o_shippriority
ORDER BY
    revenue DESC,
    o_orderdate
LIMIT 10;
