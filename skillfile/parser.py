import sys
from pathlib import Path

from .models import Entry

MANIFEST_NAME = "Skillfile"


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
