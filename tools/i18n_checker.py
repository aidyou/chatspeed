import yaml
import os
import re

def parse_yaml_keys(filepath):
    with open(filepath, 'r', encoding='utf-8') as f:
        data = yaml.safe_load(f)

    keys = set()
    def flatten_dict(d, parent_key=''):
        for k, v in d.items():
            new_key = f"{parent_key}.{k}" if parent_key else k
            if isinstance(v, dict):
                flatten_dict(v, new_key)
            elif isinstance(v, (str, int, float, bool)):
                # Only add keys that have a direct string value
                keys.add(new_key)
    flatten_dict(data)
    return keys

def extract_rust_i18n_keys(directory):
    rust_keys = set()
    for root, _, files in os.walk(directory):
        for file in files:
            if file.endswith('.rs'):
                filepath = os.path.join(root, file)
                with open(filepath, 'r', encoding='utf-8') as f:
                    content = f.read()
                    # Regex to find t!("key") with potential whitespace/newlines and other params
                    matches = re.findall(r'(?:rust_i18n::)?t!\(\s*\"([a-zA-Z0-9_.]+)\"', content, re.MULTILINE)
                    for match in matches:
                        rust_keys.add(match)
    return rust_keys

def main():
    script_dir = os.path.dirname(os.path.abspath(__file__))
    project_root = os.path.join(script_dir, '..') # Assuming tools/ is directly under project root

    yaml_file = os.path.join(project_root, "src-tauri", "i18n", "zh-Hans.yml")
    rust_src_dir = os.path.join(project_root, "src-tauri", "src")
    output_dir = os.path.join(project_root, "tools")

    defined_keys = parse_yaml_keys(yaml_file)
    used_keys = extract_rust_i18n_keys(rust_src_dir)

    unused_keys = defined_keys - used_keys
    missing_keys = used_keys - defined_keys

    output_file = os.path.join(output_dir, "i18n_check_results.txt")

    with open(output_file, "w", encoding='utf-8') as f:
        f.write("# 缺失的语言项\n")
        for key in sorted(list(missing_keys)):
            f.write(f"{key}\n")

        f.write("\n# 多余的语言项\n")
        for key in sorted(list(unused_keys)):
            f.write(f"{key}\n")

    print(f"Results written to {output_file}")

if __name__ == "__main__":
    main()
