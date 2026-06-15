# Releasing fs-ocr

The release pipeline is `.github/workflows/release.yml`. This is the operational
runbook for cutting a release — for a library/CLI the "deployment" is publishing
to PyPI + attaching CLI binaries to the GitHub Release.

<!-- AUTO-GENERATED from .github/workflows/release.yml + Cargo.toml -->

## One-time setup (before the first release)

- **PyPI Trusted Publisher**: on pypi.org, reserve the `fs-ocr` name and register
  this repo + workflow (`release.yml`) + environment `pypi` as a trusted publisher
  (OIDC — no API token is stored).
- **GitHub environment**: create an environment named `pypi` in repo settings
  (the `publish` job runs in it).

## Cutting a release

1. **Bump the version in `Cargo.toml`** (`[package] version`). This — not the tag —
   is the published version.
2. Commit, then **tag `vX.Y.Z`** matching that version exactly and push the tag.
   ```bash
   git tag v0.1.0 && git push origin v0.1.0
   ```
3. CI does the rest on the tag:
   - `check-version` — fails the run if the tag (minus `v`) ≠ `Cargo.toml` version.
   - `build-wheels` — abi3 wheels for linux-x86_64, windows-x64, macos-x86_64,
     macos-aarch64 (static HDF5, embedded OCR model), each smoke-tested (`import fs_ocr`).
   - `build-sdist` — source distribution.
   - `build-cli` / `release-cli` — standalone `fs-ocr` binaries (no libpython) for
     the same 4 platforms, attached to the GitHub Release.
   - `publish` — uploads wheels + sdist to PyPI via trusted publishing.

A non-tag run (manual dispatch / PR touching the workflow) **builds but does not publish**.

## Health checks

- Each wheel job imports the freshly built wheel and prints `fs_ocr.__version__`
  and `fs_ocr.OCR_BACKEND` — a red wheel job means a broken build (bad abi3 tag,
  missing symbol, embedded-model failure).
- After publish: `pip install fs-ocr==X.Y.Z` in a clean venv and run a real scan.

## Common issues

| Symptom | Cause / fix |
|---------|-------------|
| `publish` fails with auth error | Trusted publisher or `pypi` environment not configured (see one-time setup). |
| `check-version` fails | Tag doesn't match `Cargo.toml` version — fix one and re-tag. |
| Wheel build OK but users can't scan | The **template DB (`fs_airborne.h5`) is not bundled** — users get it from the [foxhole-stockpiles](https://github.com/xurxogr/foxhole-stockpiles) repo's `data/` dir (see README → Template Database). |
| HDF5/CMake error on Linux wheel | manylinux container needs `cmake gcc gcc-c++ make` (already in `before-script-linux`). |

## Rollback

- PyPI releases are **immutable** — you cannot overwrite `X.Y.Z`. Yank a bad
  release on PyPI and publish a fixed `X.Y.(Z+1)`.
- GitHub Release assets (CLI binaries) can be deleted/re-uploaded by re-running
  the workflow or editing the Release.

<!-- END AUTO-GENERATED -->
