"""Shared test helpers."""

import textwrap
from pathlib import Path

from skillfile.core.models import Entry


def write_manifest(tmp_path: Path, content: str = "") -> Path:
    p = tmp_path / "Skillfile"
    p.write_text(textwrap.dedent(content))
    return p


def make_github_entry(
    name: str = "test-agent",
    entity_type: str = "agent",
    owner_repo: str = "owner/repo",
    path_in_repo: str = "agents/test.md",
    ref: str = "main",
) -> Entry:
    return Entry(
        source_type="github",
        entity_type=entity_type,
        name=name,
        owner_repo=owner_repo,
        path_in_repo=path_in_repo,
        ref=ref,
    )
