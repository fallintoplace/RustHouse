"""TPC-H dbgen schema mapped to RustHouse's supported fixed-width column types.

Money uses integer cents, quantity integer hundredths, and fractions basis points.
All scaled values must be converted before COPY; this is not a certified schema.
"""
from collections import OrderedDict

# Fields are (name, RustHouse type, decimal multiplier).
TABLES = OrderedDict({
    "region": [
        ("r_regionkey", "UInt64", 1), ("r_name", "String", 1),
        ("r_comment", "String", 1),
    ],
    "nation": [
        ("n_nationkey", "UInt64", 1), ("n_name", "String", 1),
        ("n_regionkey", "UInt64", 1), ("n_comment", "String", 1),
    ],
    "supplier": [
        ("s_suppkey", "UInt64", 1), ("s_name", "String", 1),
        ("s_address", "String", 1), ("s_nationkey", "UInt64", 1),
        ("s_phone", "String", 1), ("s_acctbal", "Int64", 100),
        ("s_comment", "String", 1),
    ],
    "customer": [
        ("c_custkey", "UInt64", 1), ("c_name", "String", 1),
        ("c_address", "String", 1), ("c_nationkey", "UInt64", 1),
        ("c_phone", "String", 1), ("c_acctbal", "Int64", 100),
        ("c_mktsegment", "String", 1), ("c_comment", "String", 1),
    ],
    "part": [
        ("p_partkey", "UInt64", 1), ("p_name", "String", 1),
        ("p_mfgr", "String", 1), ("p_brand", "String", 1),
        ("p_type", "String", 1), ("p_size", "Int64", 1),
        ("p_container", "String", 1), ("p_retailprice", "Int64", 100),
        ("p_comment", "String", 1),
    ],
    "partsupp": [
        ("ps_partkey", "UInt64", 1), ("ps_suppkey", "UInt64", 1),
        ("ps_availqty", "Int64", 1), ("ps_supplycost", "Int64", 100),
        ("ps_comment", "String", 1),
    ],
    "orders": [
        ("o_orderkey", "UInt64", 1), ("o_custkey", "UInt64", 1),
        ("o_orderstatus", "String", 1), ("o_totalprice", "Int64", 100),
        ("o_orderdate", "Date", 1), ("o_orderpriority", "String", 1),
        ("o_clerk", "String", 1), ("o_shippriority", "Int64", 1),
        ("o_comment", "String", 1),
    ],
    "lineitem": [
        ("l_orderkey", "UInt64", 1), ("l_partkey", "UInt64", 1),
        ("l_suppkey", "UInt64", 1), ("l_linenumber", "UInt64", 1),
        ("l_quantity", "Int64", 100), ("l_extendedprice", "Int64", 100),
        ("l_discount", "Int64", 10000), ("l_tax", "Int64", 10000),
        ("l_returnflag", "String", 1), ("l_linestatus", "String", 1),
        ("l_shipdate", "Date", 1), ("l_commitdate", "Date", 1),
        ("l_receiptdate", "Date", 1), ("l_shipinstruct", "String", 1),
        ("l_shipmode", "String", 1), ("l_comment", "String", 1),
    ],
})


def names(table):
    return [column[0] for column in TABLES[table]]


def create_sql(table):
    columns = TABLES[table]
    return "CREATE TABLE " + table + " (" + ", ".join(
        f"{name} {kind}" for name, kind, _ in columns
    ) + ")"


def explain_scaling():
    return {
        "money": "integer cents; multiply original monetary value by 100",
        "quantity": "integer hundredths; multiply original quantity by 100",
        "fractions": "integer basis points; multiply discount/tax by 10000",
        "q6_revenue": "sum(price_cents * discount_bps) divided by 1000000",
        "certified": False,
    }
