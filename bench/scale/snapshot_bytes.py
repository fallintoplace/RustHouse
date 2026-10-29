"""Inventory bytes in frozen snapshot files, grouped by stream kind.

Does not read or hash file contents. Call this only on offline snapshots.
"""
import argparse
import json
import os
import stat
from pathlib import Path


def classify(path):
    name = Path(path).name
    if name.endswith(".bin"):
        return "data_bytes"
    if name.endswith((".mrk", ".mrk2", ".mrk3")):
        return "mark_bytes"
    if name.endswith((".idx", ".cidx")):
        return "index_bytes"
    return "metadata_bytes"


def inventory(root, *, max_files=1_000_000):
    root = Path(root)
    if not root.is_dir() or root.is_symlink():
        raise ValueError("expected a real offline snapshot directory")
    if max_files < 1:
        raise ValueError("max_files must be positive")
    groups = dict.fromkeys(("data_bytes", "mark_bytes", "index_bytes", "metadata_bytes"), 0)
    count = 0
    def fail_walk(error):
        raise error

    for directory, dirs, files in os.walk(root, followlinks=False, onerror=fail_walk):
        for name in dirs:
            if (Path(directory) / name).is_symlink():
                raise ValueError("symlinked directory is unsafe")
        for name in files:
            path = Path(directory) / name
            info = path.lstat()
            if not stat.S_ISREG(info.st_mode):
                raise ValueError(f"unsafe snapshot entry: {path}")
            groups[classify(name)] += info.st_size
            count += 1
            if count > max_files:
                raise ValueError("snapshot exceeds max_files")
    return {"workload": "snapshot-stream-bytes", "total_bytes": sum(groups.values()),
            "file_count": count, "byte_groups": groups,
            "scope": "filesystem inventory; verify active parts separately"}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--part", type=Path, required=True)
    parser.add_argument("--max-files", type=int, default=1_000_000)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        result = inventory(args.part, max_files=args.max_files)
        rendered = json.dumps(result, indent=2) + "\n"
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(rendered, encoding="utf-8")
    except (OSError, ValueError) as error:
        parser.exit(1, f"snapshot inventory failed: {error}\n")
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
