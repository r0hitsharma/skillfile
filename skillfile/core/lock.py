import json
from pathlib import Path

from .models import Entry, LockEntry

LOCK_NAME = "Skillfile.lock"


def lock_key(entry: Entry) -> str:
    return f"{entry.source_type}/{entry.entity_type}/{entry.name}"


def read_lock(repo_root: Path) -> dict[str, LockEntry]:
    lock_path = repo_root / LOCK_NAME
    if not lock_path.exists():
        return {}
    data = json.loads(lock_path.read_text())
    return {key: LockEntry(sha=val["sha"], raw_url=val["raw_url"]) for key, val in data.items()}


def write_lock(repo_root: Path, locked: dict[str, LockEntry]) -> None:
    lock_path = repo_root / LOCK_NAME
    data = {key: {"sha": e.sha, "raw_url": e.raw_url} for key, e in locked.items()}
    lock_path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n")
