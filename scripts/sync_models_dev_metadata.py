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
PROVIDER_PROTOCOLS = {
    "@ai-sdk/anthropic": "claude",
    "@ai-sdk/google": "gemini",
    "@ai-sdk/openai": "openai",
    "@ai-sdk/openai-compatible": "openai",
    "@openrouter/ai-sdk-provider": "openai",
}


def fail(message: str) -> None:
    print(f"error: {message}", file=sys.stderr)
    raise SystemExit(1)


def load_json(path: Path) -> dict[str, Any]:
    if not path.is_file():
        fail(f"missing JSON file: {path}")
    if path.stat().st_size > MAX_CATALOG_BYTES:
        fail(f"JSON file exceeds {MAX_CATALOG_BYTES} bytes: {path}")
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        fail(f"invalid JSON {path}: {error}")
    if not isinstance(value, dict):
        fail(f"JSON root must be an object: {path}")
    return value


def load_catalog(path: Path) -> dict[str, Any]:
    catalog = load_json(path)
    if not isinstance(catalog.get("models"), dict) or not isinstance(catalog.get("providers"), dict):
        fail("catalog must contain object-valued models and providers")
    return catalog


def load_lab_descriptions(labs_dir: Path) -> dict[str, str]:
    if not labs_dir.is_dir():
        fail(f"missing labs directory: {labs_dir}")

    labs: dict[str, str] = {}
    for lab_file in sorted(labs_dir.glob("*/lab.toml")):
        try:
            value = tomllib.loads(lab_file.read_text(encoding="utf-8"))
        except (OSError, UnicodeDecodeError, tomllib.TOMLDecodeError) as error:
            fail(f"invalid lab metadata {lab_file}: {error}")
        description = value.get("description")
        if description is None:
            continue
        if not isinstance(description, str) or not description.strip():
            fail(f"lab description must be a non-empty string: {lab_file}")
        labs[lab_file.parent.name] = description.strip()
    return labs


def load_overrides(path: Path) -> dict[str, dict[str, Any]]:
    value = load_json(path)
    entries = value.get("providers")
    if not isinstance(entries, list):
        fail("provider overrides must contain a providers array")

    overrides: dict[str, dict[str, Any]] = {}
    allowed = {
        "id", "protocol", "name", "description", "logo", "api", "documentationUrl",
        "modelListUrl", "keyApplyUrl", "responses", "enabled",
    }
    for entry in entries:
        if not isinstance(entry, dict):
            fail("each provider override must be an object")
        unknown = set(entry) - allowed
        if unknown:
            fail(f"unsupported provider override fields: {', '.join(sorted(unknown))}")
        provider_id = entry.get("id")
        if not isinstance(provider_id, str) or not provider_id.strip():
            fail("each provider override requires a non-empty id")
        if provider_id in overrides:
            fail(f"duplicate provider override id: {provider_id}")
        overrides[provider_id] = entry
    return overrides


def infer_protocol(provider: dict[str, Any]) -> str | None:
    npm = provider.get("npm")
    if not isinstance(npm, str):
        return None
    return PROVIDER_PROTOCOLS.get(npm.lower())


def supports_responses(provider: dict[str, Any]) -> bool:
    models = provider.get("models")
    return isinstance(models, dict) and any(
        isinstance(model, dict)
        and isinstance(model.get("provider"), dict)
        and model["provider"].get("shape") == "responses"
        for model in models.values()
    )


def merge_providers(
    catalog: dict[str, Any], labs: dict[str, str], overrides: dict[str, dict[str, Any]]
) -> list[dict[str, Any]]:
    catalog_providers = catalog["providers"]
    assert isinstance(catalog_providers, dict)
    merged: dict[str, dict[str, Any]] = {}

    for provider_id, value in catalog_providers.items():
        if not isinstance(provider_id, str) or not isinstance(value, dict):
            continue
        models = value.get("models")
        if not isinstance(models, dict) or not models:
            continue
        provider: dict[str, Any] = {
            "id": provider_id,
            "name": value.get("name", provider_id),
            "logo": f"https://models.dev/logos/{provider_id}.svg",
            "documentationUrl": value.get("doc"),
            "api": value.get("api"),
            "protocol": infer_protocol(value),
            "responses": supports_responses(value),
            "modelCount": len(models),
        }
        if provider_id in labs:
            provider["description"] = labs[provider_id]
        merged[provider_id] = {key: value for key, value in provider.items() if value is not None}

    for provider_id, override in overrides.items():
        provider = merged.setdefault(provider_id, {"id": provider_id, "modelCount": 0, "responses": False})
        provider.update(override)

    return sorted(
        (provider for provider in merged.values() if provider.get("enabled", True)),
        key=lambda provider: str(provider["id"]).casefold(),
    )


def write_json(path: Path, value: dict[str, Any]) -> None:
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", type=Path, required=True, help="models.dev checkout")
    parser.add_argument("--catalog", type=Path, required=True, help="generated models.dev catalog JSON")
    parser.add_argument("--overrides", type=Path, required=True, help="manual provider override JSON")
    parser.add_argument("--output", type=Path, required=True, help="deployment directory")
    parser.add_argument("--revision", required=True, help="resolved upstream Git revision")
    args = parser.parse_args()

    source = args.source.resolve()
    catalog_path = args.catalog.resolve()
    catalog = load_catalog(catalog_path)
    overrides = load_overrides(args.overrides.resolve())
    providers = merge_providers(catalog, load_lab_descriptions(source / "labs"), overrides)

    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(catalog_path, output / "catalog.json")

    source_license = source / "LICENSE"
    if not source_license.is_file():
        fail(f"missing upstream license: {source_license}")
    shutil.copyfile(source_license, output / "LICENSE")

    catalog_bytes = (output / "catalog.json").read_bytes()
    source_metadata = {
        "repository": "https://github.com/anomalyco/models.dev",
        "revision": args.revision,
        "generatedAt": datetime.now(UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z"),
        "catalogSha256": hashlib.sha256(catalog_bytes).hexdigest(),
    }
    write_json(output / "providers.json", {"source": source_metadata, "providers": providers})


if __name__ == "__main__":
    main()
