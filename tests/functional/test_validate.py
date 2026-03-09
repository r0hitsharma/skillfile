"""Functional test: skillfile validate golden path."""

from tests.functional.conftest import run_sf


def test_validate_golden_path(repo):
    """validate reports no errors on a well-formed Skillfile."""
    r = run_sf("validate", cwd=repo)
    assert r.returncode == 0
    # No errors in output
    assert "error" not in r.stdout.lower()
    assert "error" not in r.stderr.lower()
