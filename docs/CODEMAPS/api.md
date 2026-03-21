<!-- Generated: 2026-03-21 | Files scanned: 24 | Token estimate: ~500 -->

# fs-ocr Python API

## Classes Exported

### StockpileScanner (main entry point)

```python
scanner = StockpileScanner(database_path: str, tessdata_path: str = "tessdata")

# Core methods
scanner.scan(image: np.ndarray, faction: str = None, config: ScanConfig = None) -> Stockpile
scanner.scan_file(path: str, faction: str = None, config: ScanConfig = None) -> Stockpile
scanner.preload(resolution: int = 2160)  # Warm up caches

# Debug methods
scanner.debug_detect_boxes(image) -> List[Tuple[int, int]]
scanner.debug_detect_all_contours(image) -> List[Tuple[int, int, int, int]]
```

### ScanConfig (tuning parameters)

```python
config = ScanConfig(
    phash_threshold=20,         # Max Hamming distance for pHash filter
    max_ncc_candidates=50,      # Max templates to run NCC on
    confidence_gap=0.02,        # Gap for alternative matches
    ncc_tiebreaker_threshold=0.005,  # Edge-based tiebreaker
)
config.to_json() -> str
ScanConfig.from_json(json: str) -> ScanConfig
```

### Stockpile (scan result)

```python
stockpile.items: List[StockpileItem]
stockpile.stockpile_type: StockpileType
stockpile.name: Optional[str]
stockpile.shard: Optional[str]
stockpile.ingame_timestamp: Optional[str]
stockpile.errors: List[str]
stockpile.timing_detection_ms: float
stockpile.timing_quantity_ms: float
stockpile.timing_matching_ms: float
stockpile.to_json() -> str
stockpile.to_json_compact() -> str
```

### StockpileItem

```python
item.code: str          # "Unknown" if no match
item.quantity: int      # -1 if OCR failed
item.crated: bool
item.confidence: float  # 0.0 - 1.0
item.candidates: Optional[List[ItemCandidate]]  # Alternatives within gap
item.to_json() -> str
```

### Enums

```python
ItemFaction: Neutral=0, Colonials=1, Wardens=2
ItemCategory: Invalid=0, Item=1, Vehicle=2, Structure=3, Shippable=4, Liquid=5
StockpileType: Seaport=0, StorageDepot=1, ... Undefined=12
```

### Standalone Functions

```python
fs_ocr.compute_phash(image: np.ndarray) -> int  # 64-bit pHash
```

## Usage Example

```python
from fs_ocr import StockpileScanner, ScanConfig
import cv2

scanner = StockpileScanner("templates.h5", "tessdata")
scanner.preload(2160)  # Optional: warm caches

img = cv2.imread("stockpile.png")
result = scanner.scan(img, faction="wardens")

for item in result.items:
    print(f"{item.code}: {item.quantity} (conf={item.confidence:.2f})")
```
