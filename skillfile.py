#!/usr/bin/env python3
"""skillfile v0.1 — tool-agnostic AI skill & agent manager

Usage:
  skillfile.py sync [--dry-run] [--entry NAME]
"""

import argparse
import json
import sys
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path

MANIFEST_NAME = "Skillfile"
VENDOR_DIR = "vendor"


# ---------------------------------------------------------------------------
# Data model
# ---------------------------------------------------------------------------

@dataclass
class Entry:
    source_type: str   # local | github | url
    entity_type: str   # skill | agent
    name: str
    # github
    owner_repo: str = ""
    path_in_repo: str = ""
    ref: str = ""
    # local
    local_path: str = ""
    # url
    url: str = ""


# ---------------------------------------------------------------------------
# Manifest parser
# ---------------------------------------------------------------------------

def parse_manifest(manifest_path: Path) -> list[Entry]:
    entries = []
    with open(manifest_path) as f:
        for lineno, raw in enumerate(f, 1):
            line = raw.strip()
            if not line or line.startswith("#"):
                continue
            parts = line.split()
            if len(parts) < 3:
                print(f"warning: line {lineno}: too few fields, skipping", file=sys.stderr)
                continue

            source_type, entity_type, name = parts[0], parts[1], parts[2]

            if source_type == "local":
                if len(parts) < 4:
                    print(f"warning: line {lineno}: local entry missing path", file=sys.stderr)
                    continue
                entries.append(Entry(source_type, entity_type, name, local_path=parts[3]))

            elif source_type == "github":
                if len(parts) < 6:
                    print(f"warning: line {lineno}: github entry needs: owner/repo path ref", file=sys.stderr)
                    continue
                entries.append(Entry(
                    source_type, entity_type, name,
                    owner_repo=parts[3],
                    path_in_repo=parts[4],
                    ref=parts[5],
                ))

            elif source_type == "url":
                if len(parts) < 4:
                    print(f"warning: line {lineno}: url entry missing url", file=sys.stderr)
                    continue
                entries.append(Entry(source_type, entity_type, name, url=parts[3]))

            else:
                print(f"warning: line {lineno}: unknown source type '{source_type}', skipping", file=sys.stderr)

    return entries


# ---------------------------------------------------------------------------
# Network helpers
# ---------------------------------------------------------------------------

def _get(url: str) -> bytes:
    req = urllib.request.Request(url, headers={"User-Agent": "skillfile/0.1"})
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            return resp.read()
    except urllib.error.HTTPError as e:
        print(f"error: HTTP {e.code} fetching {url}", file=sys.stderr)
        sys.exit(1)
    except urllib.error.URLError as e:
        print(f"error: {e.reason} fetching {url}", file=sys.stderr)
        sys.exit(1)


def resolve_github_sha(owner_repo: str, ref: str) -> str:
    """Resolve a branch/tag/SHA ref to a full commit SHA via GitHub API."""
    url = f"https://api.github.com/repos/{owner_repo}/commits/{ref}"
    req = urllib.request.Request(
        url,
        headers={
            "Accept": "application/vnd.github.v3+json",
            "User-Agent": "skillfile/0.1",
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            data = json.loads(resp.read())
            return data["sha"]
    except urllib.error.HTTPError as e:
        print(f"error: could not resolve {owner_repo}@{ref}: HTTP {e.code}", file=sys.stderr)
        sys.exit(1)


def fetch_github_file(owner_repo: str, path_in_repo: str, sha: str) -> bytes:
    """Fetch raw file bytes from raw.githubusercontent.com."""
    effective_path = "SKILL.md" if path_in_repo == "." else path_in_repo
    url = f"https://raw.githubusercontent.com/{owner_repo}/{sha}/{effective_path}"
    return _get(url)


# ---------------------------------------------------------------------------
# Sync logic
# ---------------------------------------------------------------------------

def vendor_dir_for(entry: Entry, repo_root: Path) -> Path:
    return repo_root / VENDOR_DIR / f"{entry.entity_type}s" / entry.name


def sync_entry(entry: Entry, repo_root: Path, dry_run: bool) -> None:
    label = f"  {entry.source_type}/{entry.entity_type}/{entry.name}"

    if entry.source_type == "local":
        print(f"{label}: local — no vendoring needed")
        return

    vdir = vendor_dir_for(entry, repo_root)

    if entry.source_type == "github":
        print(f"{label}: resolving {entry.owner_repo}@{entry.ref} ...", end=" ", flush=True)

        if dry_run:
            print("[dry-run]")
            return

        sha = resolve_github_sha(entry.owner_repo, entry.ref)
        print(f"sha={sha[:12]}", end=" ", flush=True)

        content = fetch_github_file(entry.owner_repo, entry.path_in_repo, sha)

        effective_path = "SKILL.md" if entry.path_in_repo == "." else entry.path_in_repo
        filename = Path(effective_path).name

        vdir.mkdir(parents=True, exist_ok=True)
        (vdir / filename).write_bytes(content)

        meta = {
            "source_type": "github",
            "owner_repo": entry.owner_repo,
            "path_in_repo": entry.path_in_repo,
            "ref": entry.ref,
            "sha": sha,
            "raw_url": f"https://raw.githubusercontent.com/{entry.owner_repo}/{sha}/{effective_path}",
        }
        (vdir / ".meta").write_text(json.dumps(meta, indent=2) + "\n")

        print(f"-> {vdir / filename}")

    elif entry.source_type == "url":
        print(f"{label}: fetching {entry.url} ...", end=" ", flush=True)

        if dry_run:
            print("[dry-run]")
            return

        content = _get(entry.url)
        filename = Path(entry.url).name or "content.md"

        vdir.mkdir(parents=True, exist_ok=True)
        (vdir / filename).write_bytes(content)

        meta = {
            "source_type": "url",
            "url": entry.url,
        }
        (vdir / ".meta").write_text(json.dumps(meta, indent=2) + "\n")

        print(f"-> {vdir / filename}")


def cmd_sync(args: argparse.Namespace, repo_root: Path) -> None:
    manifest_path = repo_root / MANIFEST_NAME
    if not manifest_path.exists():
        print(f"error: {MANIFEST_NAME} not found in {repo_root}", file=sys.stderr)
        sys.exit(1)

    entries = parse_manifest(manifest_path)

    if args.entry:
        entries = [e for e in entries if e.name == args.entry]
        if not entries:
            print(f"error: no entry named '{args.entry}' in {MANIFEST_NAME}", file=sys.stderr)
            sys.exit(1)

    if not entries:
        print(f"No entries found in {MANIFEST_NAME}.")
        return

    mode = " [dry-run]" if args.dry_run else ""
    print(f"Syncing {len(entries)} entr{'y' if len(entries) == 1 else 'ies'}{mode}...")

    for entry in entries:
        sync_entry(entry, repo_root, args.dry_run)

    if not args.dry_run:
        print("Done.")


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def main() -> None:
    parser = argparse.ArgumentParser(
        prog="skillfile",
        description="Tool-agnostic AI skill & agent manager",
    )
    sub = parser.add_subparsers(dest="command")

    sync_p = sub.add_parser("sync", help="Fetch community entries into vendor/")
    sync_p.add_argument("--dry-run", action="store_true", help="Show planned actions without fetching")
    sync_p.add_argument("--entry", metavar="NAME", help="Sync only this named entry")

    args = parser.parse_args()
    if args.command is None:
        parser.print_help()
        sys.exit(1)

    repo_root = Path.cwd()

    if args.command == "sync":
        cmd_sync(args, repo_root)


if __name__ == "__main__":
    main()
