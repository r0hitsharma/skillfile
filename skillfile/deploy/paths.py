"""Path resolution for platform adapters and installed files."""

from pathlib import Path

from ..core.models import Entry, Manifest
from ..exceptions import ManifestError
from ..sources.strategies import STRATEGIES
from ..sources.sync import vendor_dir_for

# Adapter target directories.
# Paths starting with '~' are global (expanded at runtime).
# Relative paths are local (resolved from repo_root).
ADAPTER_PATHS: dict[str, dict[str, dict[str, str]]] = {
    "claude-code": {
        "agent": {"global": "~/.claude/agents", "local": ".claude/agents"},
        "skill": {"global": "~/.claude/skills", "local": ".claude/skills"},
    },
}

KNOWN_ADAPTERS = list(ADAPTER_PATHS.keys())


def resolve_target_dir(adapter: str, entity_type: str, scope: str, repo_root: Path) -> Path:
    paths = ADAPTER_PATHS[adapter][entity_type]
    raw = paths[scope]
    if raw.startswith("~"):
        return Path(raw).expanduser()
    return repo_root / raw


def installed_path(entry: Entry, manifest: Manifest, repo_root: Path) -> Path:
    """Return the platform-side installed path for a single-file entry (first install target)."""
    if not manifest.install_targets:
        raise ManifestError("no install targets configured — run `skillfile install` first")
    target = manifest.install_targets[0]
    if target.adapter not in ADAPTER_PATHS:
        raise ManifestError(f"unknown adapter '{target.adapter}'")
    target_dir = resolve_target_dir(target.adapter, entry.entity_type, target.scope, repo_root)
    return target_dir / f"{entry.name}.md"


def installed_dir_files(entry: Entry, manifest: Manifest, repo_root: Path) -> dict[str, Path]:
    """Return {relative_path: installed_path} for a directory entry's installed files."""
    if not manifest.install_targets:
        raise ManifestError("no install targets configured — run `skillfile install` first")
    target = manifest.install_targets[0]
    if target.adapter not in ADAPTER_PATHS:
        raise ManifestError(f"unknown adapter '{target.adapter}'")
    target_dir = resolve_target_dir(target.adapter, entry.entity_type, target.scope, repo_root)

    if entry.entity_type == "skill":
        # Skill dirs are installed as a whole directory: target_dir/name/
        installed_dir = target_dir / entry.name
        if not installed_dir.is_dir():
            return {}
        return {str(f.relative_to(installed_dir)): f for f in installed_dir.rglob("*") if f.is_file()}
    else:
        # Agent dirs are exploded: each .md file at target_dir/filename (flat, recursive).
        # Key by relative path within vendor dir so pin/diff lookups match cache_file.relative_to(vdir).
        vdir = vendor_dir_for(entry, repo_root)
        if not vdir.is_dir():
            return {}
        result = {}
        for f in vdir.rglob("*.md"):
            relative_key = str(f.relative_to(vdir))
            installed = target_dir / f.name
            if installed.exists():
                result[relative_key] = installed
        return result


def _source_path(entry: Entry, repo_root: Path) -> Path | None:
    """Return the path to the source file or directory for an entry."""
    strategy = STRATEGIES[entry.source_type]
    if entry.source_type == "local":
        return repo_root / entry.local_path
    vdir = vendor_dir_for(entry, repo_root)
    if strategy.is_dir_entry(entry):
        return vdir if vdir.exists() else None
    filename = strategy.content_file(entry)
    if not filename:
        return None
    return vdir / filename
