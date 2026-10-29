#!/usr/bin/env python3
"""Verify that Q1-Q22 are unchanged full upstream SQL, not derived substitutes."""
import argparse
import hashlib
import json
from pathlib import Path

UPSTREAM_REVISION = "6287a4e595d1c6c07d4241676385cbffbd232f07"
SOURCE = "https://github.com/duckdb/duckdb/tree/" + UPSTREAM_REVISION + "/extension/tpch/dbgen/queries"
EXPECTED_GIT_BLOBS = {
    "q01.sql": "1513b8c3a40743d3bc3b8271afe231ac131f2c07",
    "q02.sql": "206a4727f089343dd631a24bc3fbd2f99ac1aa31",
    "q03.sql": "b05a73b475e01981cfa3e2b8ce3380adc02f8a28",
    "q04.sql": "4c57789ac3ce4816c9b8ecfab0a607534dd3a39f",
    "q05.sql": "411596ed34e21f3157797637a3a5cc61f562b88b",
    "q06.sql": "162e516b2df14753351ff90efbaafa8f0c7a2ba3",
    "q07.sql": "695b953bcc40dd3a2e19eccfc48602f653ee6c07",
    "q08.sql": "6c80028f4686f3ecd3b441e38dfabf6e6555929d",
    "q09.sql": "4c166599100fbcb0f2906d12654b5f9ce862f4b8",
    "q10.sql": "ae37ab9997f583ff6f7a14938a4acc03c2e5ea5c",
    "q11.sql": "527e2d1c873a5386db68ed11e187beb3da05eef4",
    "q12.sql": "479cd3edf856830eb371d285cd220d787731e046",
    "q13.sql": "e78f21203b231f1fa700edc038f97d3ed0663f86",
    "q14.sql": "327d5cd81c38adc4930ba2005987455faa7c2b23",
    "q15.sql": "87b3e723c6617085314362ea1e9c2d1a3b7d24ff",
    "q16.sql": "bfb8c74b7fd53db18fc7ad47ff0e64c8b5e7c44f",
    "q17.sql": "964fea1f1031d85ff4bcfb3cf6e6df4f81744acf",
    "q18.sql": "3f6886a48b2470523ceaa34c74513c7a01fe056c",
    "q19.sql": "104650135dda9a0c6c6460654ab3b026c85c8596",
    "q20.sql": "8760453a1e3def6efa1cecdb5ad14b770ae08730",
    "q21.sql": "19d18fd37c0d8f939243a8ae2b3c1bb3b901653f",
    "q22.sql": "7ce29a597297c904953ad3e245baaef96552577a",
}


def source_bytes(raw):
    """Strip the two RustHouse provenance comments, preserving SQL bytes."""
    lines = raw.splitlines(keepends=True)
    if len(lines) < 3:
        raise ValueError("missing pinned query provenance or SQL")
    if not lines[0].startswith(b"-- Full TPC-H Q") and not lines[0].startswith(b"-- TPC-H Q"):
        raise ValueError("missing pinned query provenance")
    if b"SPDX-License-Identifier: MIT" not in lines[1]:
        raise ValueError("missing upstream license attribution")
    return b"".join(lines[2:])


def git_blob_hash(data):
    return hashlib.sha1(b"blob " + str(len(data)).encode("ascii") + b"\0" + data).hexdigest()


def verify(directory, expected=None):
    directory = Path(directory)
    expected = EXPECTED_GIT_BLOBS if expected is None else expected
    if not directory.is_dir():
        raise ValueError("canonical SQL directory does not exist")
    found = {path.name for path in directory.iterdir() if path.name.lower().endswith(".sql")}
    expected_names = set(expected)
    if found != expected_names:
        raise ValueError("query files differ from exact canonical Q1-Q22 set: "
                         f"missing={sorted(expected_names - found)}, extra={sorted(found - expected_names)}")
    checked = []
    for name, digest in sorted(expected.items()):
        file = directory / name
        if file.is_symlink() or not file.is_file():
            raise ValueError(f"{name}: source must be a regular file")
        raw = file.read_bytes()
        if len(raw) > 64 * 1024 or b"\0" in raw:
            raise ValueError(f"{name}: SQL exceeds CLI size bound or contains NUL")
        sql = source_bytes(raw)
        if git_blob_hash(sql) != digest:
            raise ValueError(f"{name}: SQL differs from pinned canonical source")
        checked.append(name)
    return {"query_count": len(checked), "queries": checked,
            "upstream_revision": UPSTREAM_REVISION, "source": SOURCE,
            "canonical_sql_verified": len(checked) == 22}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--queries-dir", type=Path,
                        default=Path(__file__).resolve().parent / "queries")
    args = parser.parse_args(argv)
    try:
        print(json.dumps(verify(args.queries_dir), indent=2))
    except (ValueError, OSError) as exc:
        parser.exit(1, f"canonical SQL validation failed: {exc}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
