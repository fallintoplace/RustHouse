"""Bounded, race-checked benchmark artifacts and atomic PR summaries."""
import os
import stat
import tempfile
from pathlib import Path


def _identity(info):
    return (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns)


def read_bounded_artifact(path, max_bytes):
    path = Path(path)
    before = path.lstat()
    if not stat.S_ISREG(before.st_mode):
        raise ValueError("PR comparison artifact must be regular and non-symlinked")
    fd = os.open(path, os.O_RDONLY | getattr(os, "O_BINARY", 0)
                 | getattr(os, "O_NOFOLLOW", 0))
    with os.fdopen(fd, "rb") as stream:
        opened = os.fstat(stream.fileno())
        if not stat.S_ISREG(opened.st_mode) or _identity(before) != _identity(opened):
            raise ValueError("PR comparison artifact changed while opening")
        contents = stream.read(max_bytes + 1)
        if _identity(opened) != _identity(os.fstat(stream.fileno())):
            raise ValueError("PR comparison artifact changed while reading")
    if _identity(before) != _identity(path.lstat()):
        raise ValueError("PR comparison artifact changed while reading")
    if len(contents) > max_bytes:
        raise ValueError("PR comparison artifact exceeds 16 MiB")
    return contents.decode("utf-8")


def write_summary_atomically(path, content, *sources):
    path = Path(path)
    if path.resolve() in {Path(source).resolve() for source in sources}:
        raise ValueError("PR comparison output must not replace an input artifact")
    path.parent.mkdir(parents=True, exist_ok=True)
    pending = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w", encoding="utf-8", dir=path.parent,
            prefix=f".{path.name}.", suffix=".tmp", delete=False,
        ) as output:
            pending = Path(output.name)
            output.write(content)
            output.flush()
            os.fsync(output.fileno())
        os.replace(pending, path)
    finally:
        if pending is not None:
            pending.unlink(missing_ok=True)
