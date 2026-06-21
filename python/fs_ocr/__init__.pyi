"""Type stubs for the ``fs_ocr`` package.

The package re-exports everything from the native ``_fs_ocr`` extension; the
real declarations live in ``_fs_ocr.pyi``.
"""

from ._fs_ocr import (
    OCR_BACKEND as OCR_BACKEND,
    ItemCandidate as ItemCandidate,
    ItemCategory as ItemCategory,
    ItemFaction as ItemFaction,
    ScanConfig as ScanConfig,
    Stockpile as Stockpile,
    StockpileItem as StockpileItem,
    StockpileScanner as StockpileScanner,
    StockpileType as StockpileType,
    Timing as Timing,
    compute_phash as compute_phash,
)

__version__: str
__author__: str

__all__ = [
    "ItemCandidate",
    "ItemCategory",
    "ItemFaction",
    "OCR_BACKEND",
    "ScanConfig",
    "Stockpile",
    "StockpileItem",
    "StockpileScanner",
    "StockpileType",
    "Timing",
    "compute_phash",
]
