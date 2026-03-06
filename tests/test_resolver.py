import json
from unittest.mock import patch

from skillfile.resolver import fetch_github_file, resolve_github_sha


def test_resolve_github_sha():
    fake_sha = "87321636a1c666283d8f17398b45c2644395044b"
    body = json.dumps({"sha": fake_sha}).encode()

    with patch("skillfile.resolver.urllib.request.urlopen") as mock_urlopen:
        mock_urlopen.return_value.__enter__.return_value.read.return_value = body
        result = resolve_github_sha("owner/repo", "main")

    assert result == fake_sha
    req = mock_urlopen.call_args[0][0]
    assert "api.github.com/repos/owner/repo/commits/main" in req.full_url


def test_fetch_github_file():
    content = b"# Agent content"

    with patch("skillfile.resolver.urllib.request.urlopen") as mock_urlopen:
        mock_urlopen.return_value.__enter__.return_value.read.return_value = content
        result = fetch_github_file("owner/repo", "path/to/agent.md", "abc123")

    assert result == content
    req = mock_urlopen.call_args[0][0]
    assert "raw.githubusercontent.com/owner/repo/abc123/path/to/agent.md" in req.full_url


def test_fetch_github_file_dot_path():
    content = b"# Skill content"

    with patch("skillfile.resolver.urllib.request.urlopen") as mock_urlopen:
        mock_urlopen.return_value.__enter__.return_value.read.return_value = content
        fetch_github_file("owner/repo", ".", "abc123")

    req = mock_urlopen.call_args[0][0]
    assert "/SKILL.md" in req.full_url
