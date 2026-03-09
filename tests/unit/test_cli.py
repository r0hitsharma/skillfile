"""Tests for CLI argument parsing and error handling."""

from unittest.mock import patch

import pytest

from skillfile.cli import main


def test_no_command_exits_with_help(capsys):
    with patch("sys.argv", ["skillfile"]):
        with pytest.raises(SystemExit) as exc:
            main()
        assert exc.value.code == 1


def test_sync_missing_skillfile(tmp_path, capsys, monkeypatch):
    monkeypatch.chdir(tmp_path)
    with patch("sys.argv", ["skillfile", "sync"]):
        with pytest.raises(SystemExit) as exc:
            main()
        assert exc.value.code == 1
    assert "not found" in capsys.readouterr().err.lower()


def test_install_missing_skillfile(tmp_path, capsys, monkeypatch):
    monkeypatch.chdir(tmp_path)
    with patch("sys.argv", ["skillfile", "install"]):
        with pytest.raises(SystemExit) as exc:
            main()
        assert exc.value.code == 1
    assert "not found" in capsys.readouterr().err.lower()


def test_status_missing_skillfile(tmp_path, capsys, monkeypatch):
    monkeypatch.chdir(tmp_path)
    with patch("sys.argv", ["skillfile", "status"]):
        with pytest.raises(SystemExit) as exc:
            main()
        assert exc.value.code == 1


def test_validate_ok(tmp_path, capsys, monkeypatch):
    monkeypatch.chdir(tmp_path)
    (tmp_path / "Skillfile").write_text("install  claude-code  global\nlocal  skill  skills/test.md\n")
    (tmp_path / "skills").mkdir()
    (tmp_path / "skills" / "test.md").write_text("# test\n")
    with patch("sys.argv", ["skillfile", "validate"]):
        main()
    assert "OK" in capsys.readouterr().out


def test_add_no_source_exits(tmp_path, capsys, monkeypatch):
    monkeypatch.chdir(tmp_path)
    (tmp_path / "Skillfile").write_text("install  claude-code  global\n")
    with patch("sys.argv", ["skillfile", "add"]):
        with pytest.raises(SystemExit) as exc:
            main()
        assert exc.value.code == 1


def test_remove_missing_entry(tmp_path, capsys, monkeypatch):
    monkeypatch.chdir(tmp_path)
    (tmp_path / "Skillfile").write_text("install  claude-code  global\n")
    with patch("sys.argv", ["skillfile", "remove", "nonexistent"]):
        with pytest.raises(SystemExit) as exc:
            main()
        assert exc.value.code == 1
    assert "no entry" in capsys.readouterr().err.lower()


def test_pin_missing_entry(tmp_path, capsys, monkeypatch):
    monkeypatch.chdir(tmp_path)
    (tmp_path / "Skillfile").write_text("install  claude-code  global\n")
    with patch("sys.argv", ["skillfile", "pin", "nonexistent"]):
        with pytest.raises(SystemExit) as exc:
            main()
        assert exc.value.code == 1


def test_diff_missing_entry(tmp_path, capsys, monkeypatch):
    monkeypatch.chdir(tmp_path)
    (tmp_path / "Skillfile").write_text("install  claude-code  global\n")
    with patch("sys.argv", ["skillfile", "diff", "nonexistent"]):
        with pytest.raises(SystemExit) as exc:
            main()
        assert exc.value.code == 1


def test_resolve_missing_entry(tmp_path, capsys, monkeypatch):
    monkeypatch.chdir(tmp_path)
    (tmp_path / "Skillfile").write_text("install  claude-code  global\n")
    with patch("sys.argv", ["skillfile", "resolve", "nonexistent"]):
        with pytest.raises(SystemExit) as exc:
            main()
        assert exc.value.code == 1


def test_sort_dry_run(tmp_path, capsys, monkeypatch):
    monkeypatch.chdir(tmp_path)
    (tmp_path / "Skillfile").write_text("install  claude-code  global\nlocal  skill  skills/test.md\n")
    with patch("sys.argv", ["skillfile", "sort", "--dry-run"]):
        main()
    out = capsys.readouterr().out
    assert "skill" in out


def test_unpin_not_pinned(tmp_path, capsys, monkeypatch):
    monkeypatch.chdir(tmp_path)
    (tmp_path / "Skillfile").write_text("install  claude-code  global\ngithub  agent  o/r  agents/a.md\n")
    with patch("sys.argv", ["skillfile", "unpin", "a"]):
        main()  # Should not error, just say "not pinned"
    assert "not pinned" in capsys.readouterr().out


def test_skillfile_error_empty_message(tmp_path, capsys, monkeypatch):
    """SkillfileError with empty message should exit 1 without printing 'error:'."""
    monkeypatch.chdir(tmp_path)
    (tmp_path / "Skillfile").write_text("install  claude-code  global\n")
    # Validate with duplicate names raises ManifestError() with no message
    (tmp_path / "Skillfile").write_text(
        "install  claude-code  global\ngithub  agent  o/r  agents/a.md\ngithub  agent  o/r  agents/a.md\n"
    )
    with patch("sys.argv", ["skillfile", "validate"]):
        with pytest.raises(SystemExit) as exc:
            main()
        assert exc.value.code == 1
