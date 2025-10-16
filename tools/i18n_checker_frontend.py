import json
import os
import re


def parse_json_keys(filepath):
    with open(filepath, "r", encoding="utf-8") as f:
        data = json.load(f)

    keys = set()

    def flatten_dict(d, parent_key=""):
        for k, v in d.items():
            new_key = f"{parent_key}.{k}" if parent_key else k
            if isinstance(v, dict):
                flatten_dict(v, new_key)
            elif isinstance(v, (str, int, float, bool)):
                keys.add(new_key)

    flatten_dict(data)
    return keys


def extract_frontend_i18n_keys(directory):
    used_keys = set()
    for root, _, files in os.walk(directory):
        for file in files:
            if file.endswith((".vue", ".js", ".ts")):
                filepath = os.path.join(root, file)
                with open(filepath, "r", encoding="utf-8") as f:
                    content = f.read()
                    # Enhanced regex to capture i18n keys, focusing on i18n contexts with word boundaries to avoid false positives
                    # For standalone t('key') or t("key") or t(`key`) from useI18n, with params or end
                    matches1 = re.findall(
                        r'\bt\s*\(\s*["`\']([a-zA-Z][a-zA-Z0-9_.]*)["`\']\s*(?:,|\))',
                        content,
                        re.DOTALL,
                    )
                    # For $t('key') or $t("key") or $t(`key`)
                    matches2 = re.findall(
                        r'\$t\s*\(\s*["`\']([a-zA-Z][a-zA-Z0-9_.]*)["`\']\s*(?:,|\))',
                        content,
                        re.DOTALL,
                    )
                    # For this.$t('key') or this.$t("key") or this.$t(`key`)
                    matches3 = re.findall(
                        r'this\.\$t\s*\(\s*["`\']([a-zA-Z][a-zA-Z0-9_.]*)["`\']\s*(?:,|\))',
                        content,
                        re.DOTALL,
                    )
                    # For i18n.global.t('key') or i18n.global.t("key") or i18n.global.t(`key`)
                    matches6 = re.findall(
                        r'i18n\.global\.t\s*\(\s*["`\']([a-zA-Z][a-zA-Z0-9_.]*)["`\']\s*(?:,|\))',
                        content,
                        re.DOTALL,
                    )
                    # For {{ $t('key') }} in template
                    matches4 = re.findall(
                        r'{{[^}]*\$t\s*\(\s*["`\']([a-zA-Z][a-zA-Z0-9_.]*)["`\']\s*\)[^}]*}}',
                        content,
                        re.DOTALL,
                    )
                    # For {{ t('key') }} in template with word boundary
                    matches5 = re.findall(
                        r'{{[^}]*\bt\s*\(\s*["`\']([a-zA-Z][a-zA-Z0-9_.]*)["`\']\s*\)[^}]*}}',
                        content,
                        re.DOTALL,
                    )
                    all_matches = (
                        matches1 + matches2 + matches3 + matches4 + matches5 + matches6
                    )
                    for match in all_matches:
                        if (
                            match and len(match) > 1 and match[0].isalpha()
                        ):  # Filter: starts with letter, length >1
                            used_keys.add(match)
    return used_keys


def main():
    script_dir = os.path.dirname(os.path.abspath(__file__))
    project_root = os.path.join(script_dir, "..")

    zh_hans_file = os.path.join(project_root, "src", "i18n", "locales", "zh-Hans.json")
    rust_copy_file = os.path.join(
        project_root, "src", "i18n", "do_not_edit", "copy_from_rust_src_i18n.json"
    )
    src_dir = os.path.join(project_root, "src")
    output_dir = os.path.join(project_root, "tools")

    defined_keys = parse_json_keys(zh_hans_file) | parse_json_keys(rust_copy_file)

    # Whitelist for dynamically constructed keys and imports
    whitelist = set(
        [
            "chat.collapseSidebar",
            "chat.enableContext",
            "chat.disableContext",
            "chat.expandSidebar",
            "chat.mcpDisabled",
            "chat.mcpEnabled",
            "chat.networkDisabled",
            "chat.networkEnabled",
            "common.autoHide",
            "common.pin",
            "common.unpin",
            "settings.agent.addSuccess",
            "settings.agent.updateSuccess",
            "settings.mcp.addFailed",
            "settings.mcp.addSuccess",
            "settings.mcp.disableFailed",
            "settings.mcp.enableFailed",
            "settings.mcp.updateFailed",
            "settings.mcp.updateSuccess",
            "settings.mcp.statusRunning",
            "settings.mcp.statusStarting",
            "settings.mcp.enableServer",
            "settings.mcp.disableServer",
            "settings.mcp.statusStopped",
            "settings.model.addSuccess",
            "settings.model.disable",
            "settings.model.disableFailed",
            "settings.model.disableSuccess",
            "settings.model.enable",
            "settings.model.enableFailed",
            "settings.model.enableSuccess",
            "settings.model.updateSuccess",
            "settings.model.proxyTypes.bySetting",
            "settings.model.proxyTypes.http",
            "settings.model.proxyTypes.none",
        ]
    )
    # Extract chat.date.* keys
    for key in defined_keys:
        if (
            key.startswith("chat.date.")
            or key.startswith("menu.")
            or key.startswith("settings.skill.type.")
            or key.startswith("languages.")
        ):
            whitelist.add(key)

    used_keys = extract_frontend_i18n_keys(src_dir)

    # Filter used_keys to remove languages.* if mis-matched (though regex should prevent)
    used_keys = {k for k in used_keys if not k.startswith("languages.")}

    missing_keys = used_keys - defined_keys
    unused_keys = defined_keys - used_keys - whitelist

    output_file = os.path.join(output_dir, "frontend_i18n_check_results.txt")

    with open(output_file, "w", encoding="utf-8") as f:
        f.write(
            "# 前端 i18n 检查结果 (合并 zh-Hans.json 和 copy_from_rust_src_i18n.json)\n\n"
        )
        f.write("## 缺失的语言项 (代码中使用但 JSON 中未定义):\n")
        for key in sorted(missing_keys):
            f.write(f"- {key}\n")

        f.write("\n## 多余的语言项 (JSON 中定义但代码中未使用，排除白名单):\n")
        for key in sorted(unused_keys):
            f.write(f"- {key}\n")

    print(f"Results written to {output_file}")


if __name__ == "__main__":
    main()
