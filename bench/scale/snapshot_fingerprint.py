"""Build a stable SHA256 fingerprint of an offline snapshot's complete file tree.

Not a RustHouse part format validator. Requires all writers to be stopped.
"""
import argparse
import hashlib
import json
import os
import re
import stat
from pathlib import Path


def fingerprint(root, *, max_files=1_000_000, block_size=1024 * 1024,
                expected_sha256=None):
    root = Path(root)
    if not root.is_dir() or root.is_symlink():
        raise ValueError("expected an offline snapshot directory")
    if max_files < 1 or not 4096 <= block_size <= 16 * 1024 * 1024:
        raise ValueError("invalid max_files or block_size")
    if expected_sha256 is not None and (
            not isinstance(expected_sha256, str)
            or not re.fullmatch(r"[0-9a-f]{64}", expected_sha256)):
        raise ValueError("expected_sha256 must be lowercase SHA256 hex")
    aggregate = hashlib.sha256()
    count = 0
    total_bytes = 0
    def fail_walk(error):
        raise error

    for directory, dirs, files in os.walk(root, followlinks=False, onerror=fail_walk):
        dirs.sort()
        files.sort()
        for name in dirs:
            if (Path(directory) / name).is_symlink():
                raise ValueError("snapshot contains a symlink")
        for name in files:
            path = Path(directory) / name
            before = path.lstat()
            if not stat.S_ISREG(before.st_mode):
                raise ValueError("snapshot contains a nonregular file")
            digest = hashlib.sha256()
            with path.open("rb") as stream:
                opened = os.fstat(stream.fileno())
                if (opened.st_dev, opened.st_ino) != (before.st_dev, before.st_ino):
                    raise ValueError(f"snapshot file replaced while opening: {path}")
                for block in iter(lambda: stream.read(block_size), b""):
                    digest.update(block)
                finished = os.fstat(stream.fileno())
                if (opened.st_size, opened.st_mtime_ns) != (finished.st_size, finished.st_mtime_ns):
                    raise ValueError(f"snapshot file changed during hashing: {path}")
            after = path.lstat()
            if ((before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns)
                    != (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns)):
                raise ValueError(f"snapshot file changed while hashing: {path}")
            rel = path.relative_to(root).as_posix()
            aggregate.update(rel.encode("utf-8") + b"\0"
                             + str(before.st_size).encode("ascii") + b"\0"
                             + digest.digest())
            count += 1
            total_bytes += before.st_size
            if count > max_files:
                raise ValueError("snapshot exceeds max_files")
    if count == 0:
        raise ValueError("snapshot has no files")
    actual = aggregate.hexdigest()
    if expected_sha256 is not None and actual != expected_sha256:
        raise ValueError("snapshot SHA256 differs from expected_sha256")
    result = {"workload": "offline-snapshot-fingerprint",
              "files": count, "bytes": total_bytes,
              "sha256": actual, "hash_scope": "all regular files in path"}
    if expected_sha256 is not None:
        result["verified"] = True
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--part", type=Path, required=True)
    parser.add_argument("--max-files", type=int, default=1_000_000)
    parser.add_argument("--expected-sha256")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        report = fingerprint(args.part, max_files=args.max_files,
                             expected_sha256=args.expected_sha256)
        rendered = json.dumps(report, indent=2) + "\n"
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(rendered, encoding="utf-8")
    except (OSError, ValueError) as error:
        parser.exit(1, f"snapshot fingerprint failed: {error}\n")
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
