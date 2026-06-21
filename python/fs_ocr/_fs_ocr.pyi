"""Type stubs for the native ``fs_ocr._fs_ocr`` extension module.

These declarations mirror the PyO3 bindings defined in the Rust crate
(``src/lib.rs`` and friends). They exist purely for static type checkers
(mypy, pyright) and editors; they have no runtime effect.
"""

from typing import ClassVar, final

import numpy as np
import numpy.typing as npt

__all__ = [
    "StockpileScanner",
    "ScanConfig",
    "StockpileType",
    "ItemFaction",
    "ItemCategory",
    "Stockpile",
    "StockpileItem",
    "ItemCandidate",
    "Timing",
    "compute_phash",
    "__version__",
    "__author__",
    "OCR_BACKEND",
]

__version__: str
__author__: str

#: OCR backend identifier. Always ``"ocrs"`` (the embedded pure-Rust model).
OCR_BACKEND: str

# A BGR image as an ``H x W x 3`` array of ``uint8``.
_ImageArray = npt.NDArray[np.uint8]

@final
class ItemFaction:
    """Faction variants for items (int-comparable enum)."""

    Neutral: ClassVar[ItemFaction]
    Colonials: ClassVar[ItemFaction]
    Wardens: ClassVar[ItemFaction]

    @staticmethod
    def from_string(value: str | None = ...) -> ItemFaction: ...
    def value(self) -> str: ...
    def __int__(self) -> int: ...

@final
class ItemCategory:
    """Item category classification (int-comparable enum)."""

    Item: ClassVar[ItemCategory]
    Vehicle: ClassVar[ItemCategory]
    Shippable: ClassVar[ItemCategory]
    Invalid: ClassVar[ItemCategory]

    @staticmethod
    def from_string(value: str) -> ItemCategory: ...
    def value(self) -> str: ...
    def __int__(self) -> int: ...

@final
class StockpileType:
    """Detected stockpile type (int-comparable enum)."""

    Encampment: ClassVar[StockpileType]
    Keep: ClassVar[StockpileType]
    SafeHouse: ClassVar[StockpileType]
    RelicBase: ClassVar[StockpileType]
    BunkerBase: ClassVar[StockpileType]
    BorderBase: ClassVar[StockpileType]
    TownBase: ClassVar[StockpileType]
    UndergroundFortress: ClassVar[StockpileType]
    BmsLonghook: ClassVar[StockpileType]
    StorageDepot: ClassVar[StockpileType]
    Seaport: ClassVar[StockpileType]
    AircraftDepot: ClassVar[StockpileType]
    Undefined: ClassVar[StockpileType]

    @staticmethod
    def from_string(value: str) -> StockpileType: ...
    def has_custom_name(self) -> bool: ...
    def display_name(self) -> str: ...
    def __int__(self) -> int: ...

@final
class ScanConfig:
    """Configuration for the stockpile scanner."""

    phash_threshold: int
    max_ncc_candidates: int
    ncc_initial_candidates: int
    ncc_escalation_threshold: float
    confidence_gap: float
    ncc_tiebreaker_threshold: float

    def __new__(
        cls,
        phash_threshold: int | None = ...,
        max_ncc_candidates: int | None = ...,
        confidence_gap: float = ...,
        ncc_tiebreaker_threshold: float | None = ...,
        ncc_initial_candidates: int | None = ...,
        ncc_escalation_threshold: float | None = ...,
    ) -> ScanConfig: ...
    @staticmethod
    def from_json(json: str) -> ScanConfig: ...
    def to_json(self) -> str: ...

@final
class ItemCandidate:
    """An alternative candidate match for an item."""

    @property
    def code(self) -> str: ...
    @property
    def confidence(self) -> float: ...
    def __new__(cls, code: str, confidence: float) -> ItemCandidate: ...

@final
class StockpileItem:
    """A single item detected in a stockpile."""

    @property
    def code(self) -> str: ...
    @property
    def quantity(self) -> int: ...
    @property
    def crated(self) -> bool: ...
    @property
    def confidence(self) -> float: ...
    @property
    def x(self) -> int: ...
    @property
    def y(self) -> int: ...
    @property
    def candidates(self) -> list[ItemCandidate] | None: ...
    def __new__(
        cls,
        code: str,
        quantity: int,
        crated: bool = ...,
        confidence: float = ...,
        candidates: list[ItemCandidate] | None = ...,
    ) -> StockpileItem: ...
    @staticmethod
    def unknown(quantity: int, crated: bool) -> StockpileItem: ...
    def is_matched(self) -> bool: ...
    def to_json(self) -> str: ...

@final
class Timing:
    """Per-stage timing for a scan (all values in milliseconds)."""

    detection_ms: float | None
    blackbox_ms: float | None
    greymask_ms: float | None
    quantity_ms: float | None
    matching_ms: float | None
    metadata_ms: float | None

    def __new__(cls) -> Timing: ...

@final
class Stockpile:
    """Complete stockpile scan result."""

    name: str | None
    type: StockpileType
    is_reserve: bool
    shard: str | None
    ingame_timestamp: str | None
    timing: Timing | None

    @property
    def items(self) -> list[StockpileItem]: ...
    @property
    def timestamp(self) -> str: ...
    @property
    def resolution(self) -> str: ...
    @property
    def errors(self) -> list[str]: ...
    def __new__(
        cls,
        resolution: str,
        stockpile_type: StockpileType = ...,
    ) -> Stockpile: ...
    def item_count(self) -> int: ...
    def matched_count(self) -> int: ...
    def crated_count(self) -> int: ...
    def is_successful(self) -> bool: ...
    def to_json(self) -> str: ...
    def to_json_compact(self) -> str: ...
    def __len__(self) -> int: ...

@final
class StockpileScanner:
    """Main stockpile scanner interface."""

    def __new__(
        cls, database_path: str, data_path: str | None = ...
    ) -> StockpileScanner: ...
    def scan(
        self,
        image: _ImageArray,
        faction: str | None = ...,
        config: ScanConfig | None = ...,
    ) -> Stockpile: ...
    def scan_file(
        self,
        image_path: str,
        faction: str | None = ...,
        config: ScanConfig | None = ...,
    ) -> Stockpile: ...
    def get_config(self) -> ScanConfig: ...
    def set_config(self, config: ScanConfig) -> None: ...
    def database_path(self) -> str: ...
    def data_path(self) -> str: ...
    def preload(self, resolution: int = ...) -> None: ...
    def is_preloaded(self) -> bool: ...
    def warmup(self) -> None: ...

def compute_phash(image: _ImageArray) -> int:
    """Compute the 64-bit perceptual hash for a BGR image."""
    ...
