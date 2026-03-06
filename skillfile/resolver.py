import json
import sys
import urllib.error
import urllib.request


def _get(url: str) -> bytes:
    req = urllib.request.Request(url, headers={"User-Agent": "skillfile/0.2"})
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            return resp.read()
    except urllib.error.HTTPError as e:
        print(f"error: HTTP {e.code} fetching {url}", file=sys.stderr)
        sys.exit(1)
    except urllib.error.URLError as e:
        print(f"error: {e.reason} fetching {url}", file=sys.stderr)
        sys.exit(1)


def resolve_github_sha(owner_repo: str, ref: str) -> str:
    """Resolve a branch/tag/SHA ref to a full commit SHA via GitHub API."""
    url = f"https://api.github.com/repos/{owner_repo}/commits/{ref}"
    req = urllib.request.Request(
        url,
        headers={
            "Accept": "application/vnd.github.v3+json",
            "User-Agent": "skillfile/0.2",
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            data = json.loads(resp.read())
            return data["sha"]
    except urllib.error.HTTPError as e:
        print(f"error: could not resolve {owner_repo}@{ref}: HTTP {e.code}", file=sys.stderr)
        sys.exit(1)


def fetch_github_file(owner_repo: str, path_in_repo: str, sha: str) -> bytes:
    """Fetch raw file bytes from raw.githubusercontent.com."""
    effective_path = "SKILL.md" if path_in_repo == "." else path_in_repo
    url = f"https://raw.githubusercontent.com/{owner_repo}/{sha}/{effective_path}"
    return _get(url)
