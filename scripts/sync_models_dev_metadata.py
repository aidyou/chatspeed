#!/usr/bin/env python3
"""Build deployable models.dev metadata from a checked-out upstream repository."""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import sys
import tomllib
from datetime import UTC, datetime
from pathlib import Path
from typing import Any


MAX_CATALOG_BYTES = 16 * 1024 * 1024


def fail(message: str) -> None:
    print(f"error: {message}", file=sys.stderr)
    raise SystemExit(1)


def load_json(path: Path) -> dict[str, Any]:
    if not path.is_file():
        fail(f"missing catalog: {path}")
    if path.stat().st_size > MAX_CATALOG_BYTES:
        fail(f"catalog exceeds {MAX_CATALOG_BYTES} bytes")
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        fail(f"invalid catalog JSON: {error}")
    if not isinstance(value, dict):
        fail("catalog root must be an object")
    if not isinstance(value.get("models"), dict) or not isinstance(value.get("providers"), dict):
        fail("catalog must contain object-valued models and providers")
    return value


def load_labs(labs_dir: Path) -> dict[str, dict[str, str]]:
    if not labs_dir.is_dir():
        fail(f"missing labs directory: {labs_dir}")

    labs: dict[str, dict[str, str]] = {}
    for lab_file in sorted(labs_dir.glob("*/lab.toml")):
        try:
            value = tomllib.loads(lab_file.read_text(encoding="utf-8"))
        except (OSError, UnicodeDecodeError, tomllib.TOMLDecodeError) as error:
            fail(f"invalid lab metadata {lab_file}: {error}")
        if not isinstance(value, dict):
            fail(f"lab metadata must be a TOML table: {lab_file}")

        description = value.get("description")
        if description is None:
            continue
        if not isinstance(description, str) or not description.strip():
            fail(f"lab description must be a non-empty string: {lab_file}")
        labs[lab_file.parent.name] = {"description": description.strip()}

    if not labs:
        fail("no lab descriptions were found")
    return labs


def write_json(path: Path, value: dict[str, Any]) -> None:
    path.write_text(
        json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", type=Path, required=True, help="models.dev checkout")
    parser.add_argument("--catalog", type=Path, required=True, help="generated models.dev catalog JSON")
    parser.add_argument("--output", type=Path, required=True, help="deployment directory")
    parser.add_argument("--revision", required=True, help="resolved upstream Git revision")
    args = parser.parse_args()

    source = args.source.resolve()
    catalog_path = args.catalog.resolve()
    catalog = load_json(catalog_path)
    labs = load_labs(source / "labs")

    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(catalog_path, output / "catalog.json")

    source_license = source / "LICENSE"
    if not source_license.is_file():
        fail(f"missing upstream license: {source_license}")
    shutil.copyfile(source_license, output / "LICENSE")

    catalog_bytes = (output / "catalog.json").read_bytes()
    write_json(
        output / "labs.json",
        {
            "source": {
                "repository": "https://github.com/anomalyco/models.dev",
                "revision": args.revision,
                "generatedAt": datetime.now(UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z"),
                "catalogSha256": hashlib.sha256(catalog_bytes).hexdigest(),
            },
            "labs": labs,
        },
    )


if __name__ == "__main__":
    main()
