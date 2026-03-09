"""Functional test: skillfile install --dry-run."""

from tests.functional.conftest import run_sf


def test_install_dry_run(repo, github_token):
    """install --dry-run shows plan but writes nothing."""
    r = run_sf("install", "--dry-run", cwd=repo)
    assert r.returncode == 0
    assert "dry-run" in r.stdout.lower() or "dry run" in r.stdout.lower()

    # Nothing written
    assert not (repo / "Skillfile.lock").exists()
    assert not (repo / ".claude").exists()
