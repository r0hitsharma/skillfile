"""Functional test: skillfile install --update."""

from tests.functional.conftest import run_sf


def test_install_update(repo, github_token):
    """install --update re-resolves refs and re-deploys."""
    # First install
    run_sf("install", cwd=repo)

    # Update (SHAs should stay the same since repo hasn't changed)
    r = run_sf("install", "--update", cwd=repo)
    assert r.returncode == 0
    assert "Done" in r.stdout
