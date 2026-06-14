"""fs-ocr: fast OCR for Foxhole stockpile screenshots.

A single distribution (``fs-ocr``) ships the pure-Rust OCR backend with the
recognition model embedded — no system OCR libraries are required.

Chinese *custom* stockpile names are the one feature that needs an external
dependency: when the system ``tesseract`` binary (plus its ``chi_sim`` language
data) is installed, it is detected at runtime and used to read those names. When
it is absent, Chinese custom names are left unread and everything else still
scans normally. See the module docs for the ``FS_OCR_TESSERACT`` /
``FS_OCR_TESSERACT_LANG`` overrides.
"""

from . import _fs_ocr as _native
from ._fs_ocr import *  # noqa: F401,F403

__doc__ = _native.__doc__ or __doc__
__version__ = _native.__version__
__author__ = _native.__author__
__all__ = [name for name in dir(_native) if not name.startswith("_")]
