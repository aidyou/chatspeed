#!/usr/bin/env python3
# -*- coding: utf-8 -*-

"""
Compare key differences between two backend i18n YAML files.
Usage: python3 compare_backend_i18n_keys.py <file1.yml> <file2.yml>

Example:
python3 compare_backend_i18n_keys.py src-tauri/i18n/zh-Hans.yml src-tauri/i18n/en.yml

Output:
- Keys present in file2.yml but missing in file1.yml
- Keys present in file1.yml but missing in file2.yml
"""

import sys
import yaml
from typing import Dict, Set


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


def main():
    if len(sys.argv) != 3:
        print(f"Usage: {sys.argv[0]} <file1.yml> <file2.yml>")
        sys.exit(1)

    file1_path = sys.argv[1]
    file2_path = sys.argv[2]

    try:
        with open(file1_path, 'r', encoding='utf-8') as f1, \
             open(file2_path, 'r', encoding='utf-8') as f2:
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
    print(f"Keys in {file2_path.split('/')[-1]} but not in {file1_path.split('/')[-1]}:")
    if only_in_file2:
        for key in sorted(only_in_file2):
            print(f"  {key}")
    else:
        print()

    print(f"\nKeys in {file1_path.split('/')[-1]} but not in {file2_path.split('/')[-1]}:")
    if only_in_file1:
        for key in sorted(only_in_file1):
            print(f"  {key}")
    else:
        print()


if __name__ == "__main__":
    main()
