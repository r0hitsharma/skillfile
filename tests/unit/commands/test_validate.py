import argparse

import pytest

from skillfile.commands.validate import cmd_validate
from skillfile.exceptions import ManifestError
from tests.helpers import write_manifest


def _make_args():
    return argparse.Namespace()


# ---------------------------------------------------------------------------
# No manifest
# ---------------------------------------------------------------------------


def test_no_manifest(tmp_path):
    with pytest.raises(ManifestError, match="not found"):
        cmd_validate(_make_args(), tmp_path)


# ---------------------------------------------------------------------------
# Valid manifests
# ---------------------------------------------------------------------------


def test_valid_empty_manifest(tmp_path, capsys):
    write_manifest(tmp_path, "")
    cmd_validate(_make_args(), tmp_path)
    assert "OK" in capsys.readouterr().out


def test_valid_github_entry(tmp_path, capsys):
    write_manifest(tmp_path, "github  agent  owner/repo  agents/agent.md\n")
    cmd_validate(_make_args(), tmp_path)
    assert "OK" in capsys.readouterr().out


def test_valid_local_entry_existing_path(tmp_path, capsys):
    source = tmp_path / "skills" / "foo.md"
    source.parent.mkdir()
    source.write_text("# Foo")
    write_manifest(tmp_path, "local  skill  skills/foo.md\n")
    cmd_validate(_make_args(), tmp_path)
    assert "OK" in capsys.readouterr().out


def test_valid_with_known_install_target(tmp_path, capsys):
    write_manifest(tmp_path, "install  claude-code  global\n")
    cmd_validate(_make_args(), tmp_path)
    assert "OK" in capsys.readouterr().out


# ---------------------------------------------------------------------------
# Duplicate names
# ---------------------------------------------------------------------------


def test_duplicate_name_errors(tmp_path, capsys):
    write_manifest(tmp_path, ("local  skill  skills/foo.md\ngithub  agent  owner/repo  skills/foo.md\n"))
    with pytest.raises(ManifestError):
        cmd_validate(_make_args(), tmp_path)
    assert "duplicate" in capsys.readouterr().err


# ---------------------------------------------------------------------------
# Missing local path
# ---------------------------------------------------------------------------


def test_missing_local_path_errors(tmp_path, capsys):
    write_manifest(tmp_path, "local  skill  skills/nonexistent.md\n")
    with pytest.raises(ManifestError):
        cmd_validate(_make_args(), tmp_path)
    assert "not found" in capsys.readouterr().err


# ---------------------------------------------------------------------------
# Unknown platform
# ---------------------------------------------------------------------------


def test_unknown_platform_errors(tmp_path, capsys):
    write_manifest(tmp_path, "install  unknown-platform  global\n")
    with pytest.raises(ManifestError):
        cmd_validate(_make_args(), tmp_path)
    assert "unknown platform" in capsys.readouterr().err


# ---------------------------------------------------------------------------
# Multiple errors reported
# ---------------------------------------------------------------------------


def test_multiple_errors_all_reported(tmp_path, capsys):
    write_manifest(tmp_path, ("install  unknown-platform  global\nlocal  skill  skills/missing.md\n"))
    with pytest.raises(ManifestError):
        cmd_validate(_make_args(), tmp_path)
    err = capsys.readouterr().err
    assert "unknown platform" in err
    assert "not found" in err


# ---------------------------------------------------------------------------
# v0.9.0 — Duplicate install targets (#12)
# ---------------------------------------------------------------------------


def test_duplicate_install_target_errors(tmp_path, capsys):
    write_manifest(tmp_path, "install  claude-code  global\ninstall  claude-code  global\n")
    with pytest.raises(ManifestError):
        cmd_validate(_make_args(), tmp_path)
    assert "duplicate install target" in capsys.readouterr().err


def test_different_scopes_not_duplicate(tmp_path, capsys):
    write_manifest(tmp_path, "install  claude-code  global\ninstall  claude-code  local\n")
    cmd_validate(_make_args(), tmp_path)
    assert "OK" in capsys.readouterr().out


# ---------------------------------------------------------------------------
# v0.9.0 — Orphaned lock entries (#10)
# ---------------------------------------------------------------------------


def test_orphaned_lock_entry_errors(tmp_path, capsys):
    import json

    write_manifest(tmp_path, "local  skill  skills/foo.md\n")
    (tmp_path / "skills").mkdir()
    (tmp_path / "skills" / "foo.md").write_text("# Foo")
    # Write a lock entry for a removed manifest entry
    lock_data = {"github/agent/removed-entry": {"sha": "abc123", "raw_url": "https://example.com"}}
    (tmp_path / "Skillfile.lock").write_text(json.dumps(lock_data))
    with pytest.raises(ManifestError):
        cmd_validate(_make_args(), tmp_path)
    assert "orphaned" in capsys.readouterr().err


def test_no_orphans_when_lock_matches(tmp_path, capsys):
    write_manifest(tmp_path, "github  agent  owner/repo  agents/agent.md\n")
    import json

    lock_data = {"github/agent/agent": {"sha": "abc123", "raw_url": "https://example.com"}}
    (tmp_path / "Skillfile.lock").write_text(json.dumps(lock_data))
    cmd_validate(_make_args(), tmp_path)
    assert "OK" in capsys.readouterr().out
