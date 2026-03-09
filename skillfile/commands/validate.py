import argparse
import sys
from pathlib import Path

from skillfile.core.lock import lock_key, read_lock
from skillfile.core.parser import MANIFEST_NAME, parse_manifest
from skillfile.deploy.adapter import ADAPTERS
from skillfile.exceptions import ManifestError


def cmd_validate(args: argparse.Namespace, repo_root: Path) -> None:
    manifest_path = repo_root / MANIFEST_NAME
    if not manifest_path.exists():
        raise ManifestError(f"{MANIFEST_NAME} not found in {repo_root}. Create one and run `skillfile init`.")

    # parse_manifest already emits warnings for malformed lines.
    manifest = parse_manifest(manifest_path)
    errors: list[str] = []

    # Duplicate entry names.
    seen: dict[str, str] = {}
    for entry in manifest.entries:
        if entry.name in seen:
            errors.append(f"duplicate name '{entry.name}' ({seen[entry.name]} and {entry.source_type})")
        else:
            seen[entry.name] = entry.source_type

    # Missing local paths.
    for entry in manifest.entries:
        if entry.source_type == "local":
            p = repo_root / entry.local_path
            if not p.exists():
                errors.append(f"local path not found: '{entry.local_path}' (entry: {entry.name})")

    # Unknown platforms.
    for target in manifest.install_targets:
        if target.adapter not in ADAPTERS:
            errors.append(f"unknown platform: '{target.adapter}'")

    # Duplicate install targets.
    seen_targets: set[tuple[str, str]] = set()
    for target in manifest.install_targets:
        key = (target.adapter, target.scope)
        if key in seen_targets:
            errors.append(f"duplicate install target: '{target.adapter} {target.scope}'")
        else:
            seen_targets.add(key)

    # Orphaned lock entries (lock key has no matching manifest entry).
    locked = read_lock(repo_root)
    manifest_keys = {lock_key(e) for e in manifest.entries}
    for key in sorted(locked):
        if key not in manifest_keys:
            errors.append(f"orphaned lock entry: '{key}' (not in Skillfile)")

    if errors:
        for msg in errors:
            print(f"error: {msg}", file=sys.stderr)
        raise ManifestError()

    n = len(manifest.entries)
    t = len(manifest.install_targets)
    print(f"Skillfile OK — {n} entr{'y' if n == 1 else 'ies'}, {t} install target{'s' if t != 1 else ''}")
