#!/usr/bin/env bash
set -euo pipefail
repo="$(cd "$(dirname "$0")/../.." && pwd)"
bin="$repo/target/debug/rusthouse"
if [[ ! -x "$bin" ]]; then (cd "$repo" && cargo build --locked -p rusthouse-cli); fi
db="$(mktemp -d)"
trap 'rm -rf -- "$db"' EXIT
"$bin" sql "CREATE TABLE trial (id UInt64, owner String, amount UInt64)" --database "$db"
"$bin" sql "INSERT INTO trial VALUES (1, 'Ada', 20), (2, 'Grace', 30)" --database "$db"
actual="$("$bin" sql "SELECT id, owner, amount FROM trial ORDER BY id" --format tsv-no-header --database "$db")"
expected=$'1\tAda\t20\n2\tGrace\t30'
if [[ "$actual" != "$expected" ]]; then
    printf 'Wrong RustHouse output:\n%s\n' "$actual" >&2
    exit 1
fi
"$bin" doctor --database "$db"
echo "PASS: create, insert, persisted SELECT, and offline doctor"
