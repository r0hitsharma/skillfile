"""Unit tests for source strategies."""

from skillfile.sources.strategies import _decode_safe, _fetch_files_parallel


def test_decode_safe_utf8():
    """Valid UTF-8 bytes decode to string."""
    assert _decode_safe(b"hello world") == "hello world"


def test_decode_safe_binary():
    """Binary data (invalid UTF-8) returns raw bytes."""
    binary_data = b"\x89PNG\r\n\x1a\n\x00\x00"
    result = _decode_safe(binary_data)
    assert isinstance(result, bytes)
    assert result == binary_data


def test_fetch_files_parallel_empty():
    assert _fetch_files_parallel([]) == {}
