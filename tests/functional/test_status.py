"""Functional test: skillfile status after install."""

from tests.functional.conftest import run_sf


def test_status_after_install(repo, github_token):
    """status shows entries as installed after install."""
    run_sf("install", cwd=repo)
    r = run_sf("status", cwd=repo)
    assert r.returncode == 0
    assert "code-refactorer" in r.stdout
    assert "requesting-code-review" in r.stdout
