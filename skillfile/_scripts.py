"""Dev script entry points for test suite shortcuts."""

import sys

import pytest


def test_unit() -> None:
    sys.exit(pytest.main(["tests/unit/", "-q"]))


def test_integration() -> None:
    sys.exit(pytest.main(["tests/integration/", "-q"]))


def test_functional() -> None:
    sys.exit(pytest.main(["tests/functional/", "-v"]))


def test_all() -> None:
    sys.exit(pytest.main(["tests/", "-q"]))
