<!-- Generated: 2026-05-21 | Files scanned: 31 | Token estimate: ~650 -->

# fs-ocr API

## Python Module (`fs_ocr`)

### StockpileScanner (main entry point)

```python
scanner = StockpileScanner(database_path: str, data_path: str = "data")

# Core methods
scanner.scan(image: np.ndarray, faction: str = None, config: ScanConfig = None) -> Stockpile
scanner.scan_file(path: str, faction: str = None, config: ScanConfig = None) -> Stockpile
scanner.preload(resolution: int = 2160) -> None   # Warm DB + OCR caches
scanner.is_preloaded() -> bool
scanner.get_config() / scanner.set_config(config)
scanner.database_path() / scanner.data_path()

# Debug methods
scanner.debug_detect_boxes(image) -> List[Tuple[int, int]]            # full-image grey mask
scanner.debug_detect_boxes_roi(image) -> List[Tuple[int, int]]        # ROI pipeline (real path)
scanner.debug_detect_black_boxes(image) -> Optional[Tuple[int,int,int,int]]  # ROI bbox
scanner.debug_detect_all_contours(image) -> List[Tuple[int,int,int,int]]
scanner.debug_detect_regions(image) -> dict                           # type/name/shard regions
scanner.debug_recognize_quantities_template(image) -> List[int]
```

Note: `image` is `H×W×3` uint8 BGR. Constructor's second arg is `data_path`
(OCR models dir), not `tessdata`.

### ScanConfig (tuning parameters)

```python
config = ScanConfig(
    phash_threshold=15,             # Max Hamming distance for pHash filter
    max_ncc_candidates=100,         # Hard cap (upper bound of escalation)
    confidence_gap=0.0,             # >0 returns alternatives within gap
    ncc_tiebreaker_threshold=0.003, # Edge(Sobel)-based tiebreaker; 0 disables
    ncc_initial_candidates=25,      # Initial NCC batch before escalation
    ncc_escalation_threshold=0.90,  # Escalate if best conf below this
)
config.to_json() -> str
ScanConfig.from_json(json: str) -> ScanConfig
```

### Stockpile (scan result)

```python
stockpile.name: Optional[str]
stockpile.type: StockpileType            # serde "type"
stockpile.is_reserve: bool
stockpile.items: List[StockpileItem]
stockpile.timestamp: str                 # ISO 8601 scan time
stockpile.shard: Optional[str]
stockpile.ingame_timestamp: Optional[str]
stockpile.resolution: str                # "WxH"
stockpile.errors: List[str]
stockpile.timing: Optional[Timing]       # per-stage ms (None unless collected)
stockpile.item_count() / matched_count() / crated_count() / is_successful()
stockpile.to_json() / to_json_compact()
```

### StockpileItem / ItemCandidate

```python
item.code: str          # "Unknown" if no match
item.quantity: int      # -1 if recognition failed
item.crated: bool
item.confidence: float  # 0.0-1.0 (serialized rounded to 3 decimals)
item.candidates: Optional[List[ItemCandidate]]  # alternatives within gap
item.is_matched() / item.to_json()

candidate.code: str
candidate.confidence: float
```

### Timing

```python
timing.detection_ms / blackbox_ms / greymask_ms
       / quantity_ms / matching_ms / metadata_ms   # all Optional[float]
```

### Enums

```python
ItemFaction:   Neutral=0, Colonials=1, Wardens=2
ItemCategory:  Invalid=0, Item=1, Vehicle=2, Structure=3, Shippable=4, Liquid=5
StockpileType: Seaport=0, StorageDepot=1, ... Undefined
```

### Module attributes / functions

```python
fs_ocr.__version__, fs_ocr.__author__
fs_ocr.HAS_OCR_BASIC   # True (ocrs always available)
fs_ocr.HAS_OCR_FULL    # True only if built with ocr-full
fs_ocr.OCR_BACKEND     # "ocrs" or "tesseract"
fs_ocr.compute_phash(image: np.ndarray) -> int   # 64-bit pHash
```

## CLI (`fs-ocr` binary)

```bash
fs-ocr scan [IMAGE] -d templates.h5 [options]   # IMAGE omitted/"-" = stdin
fs-ocr version

# Scan options
-f, --faction <wardens|colonials>
--compact                       # one-line JSON
--confidence-gap <F64>          # default 0.0
--phash-threshold <U32>         # default 15
--max-ncc-candidates <USIZE>    # default 100
--ncc-tiebreaker <F64>          # default 0.003
--ncc-initial-candidates <USIZE># default 25
--ncc-escalation-threshold <F64># default 0.85

# Env: FS_OCR_TIMING=1 includes per-stage timing in JSON
# Exit codes: 0 ok, 1 error, 2 bad input
```

## Usage Example

```python
from fs_ocr import StockpileScanner, ScanConfig
import cv2

scanner = StockpileScanner("templates.h5", "data")
scanner.preload(2160)
img = cv2.imread("stockpile.png")
result = scanner.scan(img, faction="wardens")
for item in result.items:
    print(f"{item.code}: {item.quantity} (conf={item.confidence:.2f})")
```
