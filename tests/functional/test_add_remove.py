"""Functional test: skillfile add + remove."""

from tests.functional.conftest import run_sf


def test_add_then_remove(tmp_path):
    """add appends an entry; remove deletes it."""
    # Skillfile with no install targets — add will skip resolution
    (tmp_path / "Skillfile").write_text("# empty\n")

    # Add a new entry
    r = run_sf("add", "github", "skill", "my-new-skill", "owner/repo", "skills/test.md", cwd=tmp_path)
    assert r.returncode == 0, r.stderr

    sf_text = (tmp_path / "Skillfile").read_text()
    assert "my-new-skill" in sf_text

    # Remove it
    r = run_sf("remove", "my-new-skill", cwd=tmp_path)
    assert r.returncode == 0, r.stderr

    sf_text = (tmp_path / "Skillfile").read_text()
    assert "my-new-skill" not in sf_text
