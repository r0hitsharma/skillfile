import re
import shlex
import sys
from pathlib import Path

from skillfile.core.models import Entry, InstallTarget, Manifest
from skillfile.sources.strategies import STRATEGIES

MANIFEST_NAME = "Skillfile"

# Names must be filesystem-safe: alphanumeric, dot, hyphen, underscore.
_VALID_NAME_RE = re.compile(r"^[a-zA-Z0-9._-]+$")

_VALID_SCOPES = {"global", "local"}


def find_entry_in(name: str, manifest: Manifest) -> Entry:
    from skillfile.exceptions import ManifestError

    matching = [e for e in manifest.entries if e.name == name]
    if not matching:
        raise ManifestError(f"no entry named '{name}' in {MANIFEST_NAME}")
    return matching[0]


def find_entry(name: str, manifest_path: Path) -> Entry:
    return find_entry_in(name, parse_manifest(manifest_path))


def _is_valid_name(name: str) -> bool:
    """Check if a name is filesystem-safe."""
    return bool(_VALID_NAME_RE.match(name))


def _split_line(line: str) -> list[str]:
    """Split a manifest line respecting double-quoted fields.

    Uses shlex to handle double-quoted fields (e.g. "path with spaces/foo.md").
    Unquoted lines split identically to str.split().
    """
    try:
        lex = shlex.shlex(line, posix=True)
        lex.whitespace_split = True
        return list(lex)
    except ValueError:
        # Malformed quotes — fall back to simple split
        return line.split()


def _strip_inline_comment(parts: list[str]) -> list[str]:
    """Remove inline comment (# ...) from field list.

    A field that starts with '#' and everything after it is stripped.
    Full-line comments are already handled before this is called.
    """
    for i, part in enumerate(parts):
        if part.startswith("#"):
            return parts[:i]
    return parts


def parse_manifest(manifest_path: Path) -> Manifest:
    entries: list[Entry] = []
    install_targets: list[InstallTarget] = []
    seen_names: set[str] = set()

    with open(manifest_path, encoding="utf-8-sig") as f:
        for lineno, raw in enumerate(f, 1):
            line = raw.strip()
            if not line or line.startswith("#"):
                continue
            parts = _split_line(line)
            parts = _strip_inline_comment(parts)
            if len(parts) < 2:
                print(f"warning: line {lineno}: too few fields, skipping", file=sys.stderr)
                continue

            source_type = parts[0]

            match source_type:
                case "install":
                    if len(parts) < 3:
                        print(f"warning: line {lineno}: install line needs: adapter scope", file=sys.stderr)
                    else:
                        scope = parts[2]
                        if scope not in _VALID_SCOPES:
                            print(
                                f"warning: line {lineno}: invalid scope '{scope}', "
                                f"must be one of: {', '.join(sorted(_VALID_SCOPES))}",
                                file=sys.stderr,
                            )
                        else:
                            install_targets.append(InstallTarget(adapter=parts[1], scope=scope))

                case _ if source_type in STRATEGIES:
                    if len(parts) < 3:
                        print(f"warning: line {lineno}: too few fields, skipping", file=sys.stderr)
                    else:
                        entry = STRATEGIES[source_type].parse(parts, lineno)
                        if entry is not None:
                            if not _is_valid_name(entry.name):
                                print(
                                    f"warning: line {lineno}: invalid name '{entry.name}' "
                                    f"— names must match [a-zA-Z0-9._-], skipping",
                                    file=sys.stderr,
                                )
                            elif entry.name in seen_names:
                                print(
                                    f"warning: line {lineno}: duplicate entry name '{entry.name}'",
                                    file=sys.stderr,
                                )
                                entries.append(entry)
                            else:
                                seen_names.add(entry.name)
                                entries.append(entry)

                case _:
                    print(f"warning: line {lineno}: unknown source type '{source_type}', skipping", file=sys.stderr)

    return Manifest(entries=entries, install_targets=install_targets)
