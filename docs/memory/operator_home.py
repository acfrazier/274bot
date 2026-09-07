#!/usr/bin/env python3
"""Operator home path selection matching Rust client::operator_home.

Windows: explicit HOME wins including empty string; USERPROFILE only when HOME
is absent. Non-Windows: HOME only (USERPROFILE ignored). Pure helpers take an
environ mapping so tests never mutate process env.
"""
from __future__ import annotations

import os
import sys
from typing import Mapping, Optional


def operator_home_from(
    home: Optional[str],
    userprofile: Optional[str],
    *,
    windows: bool,
) -> Optional[str]:
    """Select operator home string (may be empty) or None if unavailable.

    ``None`` for *home* / *userprofile* means the env key is absent (Rust
    ``Err``). A present empty string is ``Ok("")`` and must win on Windows.
    """
    if windows:
        if home is not None:
            return home
        return userprofile
    return home


def _env_get_present(environ: Mapping[str, str], key: str) -> Optional[str]:
    """Return value if key is present (including empty); else None."""
    if key not in environ:
        return None
    return environ[key]


def operator_home(
    *,
    environ: Optional[Mapping[str, str]] = None,
    windows: Optional[bool] = None,
) -> Optional[str]:
    """Read operator home from *environ* (default ``os.environ``).

    *windows* defaults to ``sys.platform == 'win32'``. Does not allocate beyond
    the returned string reference from the mapping.
    """
    env = os.environ if environ is None else environ
    is_windows = (sys.platform == "win32") if windows is None else bool(windows)
    home = _env_get_present(env, "HOME")
    if is_windows:
        if home is not None:
            return home
        return _env_get_present(env, "USERPROFILE")
    return home


def bot_home_path(
    *,
    environ: Optional[Mapping[str, str]] = None,
    windows: Optional[bool] = None,
) -> str:
    """Path base for ``~/.274bot/...`` defaults (Rust ``bot_home`` shape).

    Non-empty operator home → that string; empty or missing → ``\".\"``.
    """
    home = operator_home(environ=environ, windows=windows)
    if isinstance(home, str) and home:
        return home
    return "."


def unpack_root_path(
    *,
    cwd=None,
    environ: Optional[Mapping[str, str]] = None,
    windows: Optional[bool] = None,
):
    """Client ``unpack_dir`` shape: non-empty home → ``$home/.274bot/unpack``.

    Empty or missing home → ``{cwd or .}/.274bot/unpack``.
    """
    import pathlib

    home = operator_home(environ=environ, windows=windows)
    if isinstance(home, str) and home:
        return pathlib.Path(home) / ".274bot" / "unpack"
    base = pathlib.Path(cwd) if cwd is not None else pathlib.Path.cwd()
    return base / ".274bot" / "unpack"
