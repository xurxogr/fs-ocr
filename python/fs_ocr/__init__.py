"""fs-ocr: fast OCR for Foxhole stockpile screenshots.

This import package (``fs_ocr``) is shipped by two mutually exclusive PyPI
distributions, both of which provide the same compiled module:

* ``fs-ocr``            - pure-Rust OCR backend (no system OCR libraries)
* ``fs-ocr-tesseract``  - Tesseract backend (requires system Tesseract)

Install exactly one. They share the same files, so installing both leaves the
environment in an inconsistent state. pip cannot enforce this on its own, so the
guard below turns the "both installed" case into a clear error instead of a
silent, hard-to-debug breakage.
"""

import importlib.metadata as _metadata

_DISTRIBUTIONS = ("fs-ocr", "fs-ocr-tesseract")


def _conflicting_distributions():
    """Return the subset of _DISTRIBUTIONS whose metadata is installed."""
    found = []
    for name in _DISTRIBUTIONS:
        try:
            _metadata.distribution(name)
        except _metadata.PackageNotFoundError:
            continue
        found.append(name)
    return found


_installed = _conflicting_distributions()
if len(_installed) > 1:
    raise ImportError(
        "Conflicting fs_ocr distributions installed: "
        + ", ".join(_installed)
        + ".\nBoth provide the 'fs_ocr' module and must not coexist. "
        "Uninstall all of them, then install exactly one:\n\n"
        "    pip uninstall -y " + " ".join(_DISTRIBUTIONS) + "\n"
        "    pip install <fs-ocr | fs-ocr-tesseract>\n"
    )

del _conflicting_distributions, _installed, _metadata, _DISTRIBUTIONS

from . import _fs_ocr as _native  # noqa: E402
from ._fs_ocr import *  # noqa: E402,F401,F403

__doc__ = _native.__doc__ or __doc__
__version__ = _native.__version__
__author__ = _native.__author__
__all__ = [name for name in dir(_native) if not name.startswith("_")]
