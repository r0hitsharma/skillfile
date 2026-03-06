import argparse
from pathlib import Path

from .exceptions import ManifestError
from .lock import lock_key, read_lock, write_lock
from .models import Entry, LockEntry
from .parser import MANIFEST_NAME, parse_manifest
from .strategies import STRATEGIES

VENDOR_DIR = ".skillfile"


def vendor_dir_for(entry: Entry, repo_root: Path) -> Path:
    return repo_root / VENDOR_DIR / f"{entry.entity_type}s" / entry.name


def sync_entry(
    entry: Entry,
    repo_root: Path,
    dry_run: bool,
    locked: dict[str, LockEntry],
    update: bool,
) -> dict[str, LockEntry]:
    """Sync a single entry. Returns updated locked dict."""
    label = f"  {entry.source_type}/{entry.entity_type}/{entry.name}"
    vdir = vendor_dir_for(entry, repo_root)
    key = lock_key(entry)
    return STRATEGIES[entry.source_type].sync(entry, vdir, key, label, dry_run, locked, update)


def cmd_sync(args: argparse.Namespace, repo_root: Path) -> None:
    manifest_path = repo_root / MANIFEST_NAME
    if not manifest_path.exists():
        raise ManifestError(f"{MANIFEST_NAME} not found in {repo_root}")

    entries = parse_manifest(manifest_path).entries

    if args.entry:
        entries = [e for e in entries if e.name == args.entry]
        if not entries:
            raise ManifestError(f"no entry named '{args.entry}' in {MANIFEST_NAME}")

    if not entries:
        print(f"No entries found in {MANIFEST_NAME}.")
        return

    mode = " [dry-run]" if args.dry_run else ""
    print(f"Syncing {len(entries)} entr{'y' if len(entries) == 1 else 'ies'}{mode}...")

    locked = read_lock(repo_root)
    update = getattr(args, "update", False)

    for entry in entries:
        locked = sync_entry(entry, repo_root, args.dry_run, locked, update)

    if not args.dry_run:
        write_lock(repo_root, locked)
        print("Done.")
