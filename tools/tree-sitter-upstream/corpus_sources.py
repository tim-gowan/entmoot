"""Stream pinned source blobs without copying or changing a foreign checkout."""
from contextlib import contextmanager
from pathlib import Path
import subprocess


@contextmanager
def source_blobs(root, commit, *, suffix, prefix=""):
    root = Path(root).resolve(strict=True)
    listing = subprocess.check_output(["git", "-C", str(root), "ls-tree", "-rz", "--full-tree", commit])
    with subprocess.Popen(["git", "-C", str(root), "cat-file", "--batch"], stdin=subprocess.PIPE,
                          stdout=subprocess.PIPE, stderr=subprocess.DEVNULL) as git:
        def entries():
            for entry in listing.split(b"\0"):
                if not entry:
                    continue
                metadata, raw_path = entry.split(b"\t", 1)
                mode, kind, object_id = metadata.decode().split()
                path = raw_path.decode("utf-8")
                if not path.endswith(suffix) or not path.startswith(prefix):
                    continue
                item = {"path": path, "git_object": object_id, "mode": mode}
                if mode not in ("100644", "100755") or kind != "blob":
                    yield item, None
                    continue
                git.stdin.write(object_id.encode() + b"\n")
                git.stdin.flush()
                header = git.stdout.readline().split()
                if len(header) != 3 or header[0].decode() != object_id or header[1] != b"blob":
                    raise ValueError("pinned source object unavailable: " + path)
                size = int(header[2])
                source = git.stdout.read(size)
                if len(source) != size or git.stdout.read(1) != b"\n":
                    raise ValueError("truncated pinned source object: " + path)
                yield item, source
        try:
            yield entries()
        finally:
            git.stdin.close()
            # Closing stdout also makes early iterator termination safe.
            git.stdout.close()
            if git.wait(timeout=30):
                raise ValueError("Git source reader failed")
