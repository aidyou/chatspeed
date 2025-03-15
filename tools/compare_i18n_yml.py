#!/usr/bin/env python3
# -*- coding: utf-8 -*-

"""
Compare key differences between two backend i18n YAML files.
Usage: python3 compare_backend_i18n_keys.py <language1> <language2>

Example:
python3 compare_backend_i18n_keys.py zh-Hans en

Output:
- Keys present in file2.yml but missing in file1.yml
- Keys present in file1.yml but missing in file2.yml
"""

import sys
import yaml
from typing import Dict, Set
import os

language_dict = os.path.join(os.path.dirname(__file__), "../src-tauri/i18n")


def get_all_keys(obj: Dict, prefix: str = "") -> Set[str]:
    """Recursively get all keys from a YAML object, supporting nested objects.

    Args:
        obj: YAML object
        prefix: Current key prefix for building the full key path

    Returns:
        A set containing all keys
    """
    keys = set()
    for key, value in obj.items():
        full_key = f"{prefix}.{key}" if prefix else key
        if isinstance(value, dict):
            keys.update(get_all_keys(value, full_key))
        else:
            keys.add(full_key)
    return keys


def compare(language1: str, language2: str):
    file1_path = os.path.join(language_dict, f"{language1}.yml")
    file2_path = os.path.join(language_dict, f"{language2}.yml")

    try:
        with open(file1_path, "r", encoding="utf-8") as f1, open(
            file2_path, "r", encoding="utf-8"
        ) as f2:
            # Use safe_load to prevent arbitrary code execution
            data1 = yaml.safe_load(f1)
            data2 = yaml.safe_load(f2)
    except FileNotFoundError as e:
        print(f"Error: File not found - {e.filename}")
        sys.exit(1)
    except yaml.YAMLError as e:
        print(f"Error: YAML parsing failed - {e}")
        sys.exit(1)

    # Handle empty files
    if data1 is None:
        data1 = {}
    if data2 is None:
        data2 = {}

    keys1 = get_all_keys(data1)
    keys2 = get_all_keys(data2)

    # Find key differences between the two files
    only_in_file2 = keys2 - keys1
    only_in_file1 = keys1 - keys2

    # Output results
    print(f"Keys in {language2} but not in {language1}:")
    if only_in_file2:
        for key in sorted(only_in_file2):
            print(f"  {key}")
    else:
        print()

    print(f"\nKeys in {language1} but not in {language2}:")
    if only_in_file1:
        for key in sorted(only_in_file1):
            print(f"  {key}")
    else:
        print()


def compare_all(base_language: str):
    """
    Validate key differences in all language files against the base language

    Args:
        base_language (str): Base language code (e.g., "zh-Hans")
    """
    # Get all language files
    files = [f for f in os.listdir(language_dict) if f.endswith(".yml")]
    base_file = f"{base_language}.yml"

    if base_file not in files:
        print(f"Error: Base language file {base_file} not found in {language_dict}")
        return

    # Iterate over all language files and compare with the base language
    for file in files:
        if file == base_file:
            continue

        language = file[:-4]
        print("=" * 30)
        print(f"Comparing {base_language} with {language}:")
        compare(base_language, language)
        print("\n")


if __name__ == "__main__":
    args_len = len(sys.argv)
    if args_len == 2:
        print(f"Comparing all languages with {sys.argv[1]}:")
        compare_all(sys.argv[1])
    elif args_len == 3:
        print(f"Comparing {sys.argv[1]} with {sys.argv[2]}:")
        compare(sys.argv[1], sys.argv[2])
    else:
        print(f"Usage: python3 {sys.argv[0]} <base_language> [language1 language2]")
        print(f"Example: python3 {sys.argv[0]} zh-Hans")
        print(f"Example: python3 {sys.argv[0]} zh-Hans en")
        print("=" * 30)
        compare_all("zh-Hans")
