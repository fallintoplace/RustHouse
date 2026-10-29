-- Full TPC-H Q9 validation query. Source: https://raw.githubusercontent.com/duckdb/duckdb/6287a4e595d1c6c07d4241676385cbffbd232f07/extension/tpch/dbgen/queries/q09.sql
-- Copyright 2018-2026 Stichting DuckDB Foundation. SPDX-License-Identifier: MIT
SELECT
    nation,
    o_year,
    sum(amount) AS sum_profit
FROM (
    SELECT
        n_name AS nation,
        extract(year FROM o_orderdate) AS o_year,
        l_extendedprice * (1 - l_discount) - ps_supplycost * l_quantity AS amount
    FROM
        part,
        supplier,
        lineitem,
        partsupp,
        orders,
        nation
    WHERE
        s_suppkey = l_suppkey
        AND ps_suppkey = l_suppkey
        AND ps_partkey = l_partkey
        AND p_partkey = l_partkey
        AND o_orderkey = l_orderkey
        AND s_nationkey = n_nationkey
        AND p_name LIKE '%green%') AS profit
GROUP BY
    nation,
    o_year
ORDER BY
    nation,
    o_year DESC;
