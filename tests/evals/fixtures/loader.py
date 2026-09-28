# -*- coding: utf-8 -*-
# pyright: strict
import os  # noqa: F401
from typing import Any


def load(path: str) -> Any:  # type: ignore[override]
    # Read the file
    with open(path) as fh:
        return fh.read()  # prolix-ignore: kept on purpose
