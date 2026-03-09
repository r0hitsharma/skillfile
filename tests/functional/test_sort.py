"""Functional test: skillfile sort golden path."""

from tests.functional.conftest import run_sf


def test_sort_golden_path(repo):
    """sort reorders entries alphabetically within entity type."""
    # Write entries in reverse order
    (repo / "Skillfile").write_text(
        "install  claude-code  local\n"
        "github  skill  zebra  owner/repo  skills/z.md\n"
        "github  skill  alpha  owner/repo  skills/a.md\n"
    )
    r = run_sf("sort", cwd=repo)
    assert r.returncode == 0

    lines = (repo / "Skillfile").read_text().strip().splitlines()
    entry_lines = [line for line in lines if line.startswith("github")]
    assert "alpha" in entry_lines[0]
    assert "zebra" in entry_lines[1]
