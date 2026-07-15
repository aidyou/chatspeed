import json
import os
import subprocess
import sys


NODE_EXTRACTOR = r"""
const fs = require('fs');
const path = require('path');
const ts = require('typescript');
const { parse: parseSfc } = require('@vue/compiler-sfc');
const { parse: parseTemplate, NodeTypes } = require('@vue/compiler-dom');

const sourceDir = process.argv[1];
const keys = new Set();
const keyLiterals = new Set();
const dynamic = [];
const dynamicPatterns = [];
const extensions = new Set(['.js', '.ts', '.vue']);

function isTranslationCall(expression) {
  if (ts.isIdentifier(expression)) {
    return ['t', '$t', 'translateOrFallback', 'buildToolText'].includes(expression.text);
  }
  if (!ts.isPropertyAccessExpression(expression)) return false;
  if (expression.name.text !== 't' && expression.name.text !== '$t') return false;
  const text = expression.getText();
  return text === 'this.$t' || text === 'i18n.global.t' || expression.name.text === '$t';
}

function staticValues(node) {
  if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) return [node.text];
  if (ts.isParenthesizedExpression(node)) return staticValues(node.expression);
  if (ts.isConditionalExpression(node)) {
    return [...staticValues(node.whenTrue), ...staticValues(node.whenFalse)];
  }
  if (ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.PlusToken) {
    const left = staticValues(node.left);
    const right = staticValues(node.right);
    if (left.length * right.length > 32) return [];
    return left.flatMap(prefix => right.map(suffix => prefix + suffix));
  }
  if (ts.isTemplateExpression(node)) {
    let values = [node.head.text];
    for (const span of node.templateSpans) {
      const parts = staticValues(span.expression);
      if (!parts.length || values.length * parts.length > 32) return [];
      values = values.flatMap(prefix => parts.map(part => prefix + part + span.literal.text));
    }
    return values;
  }
  return [];
}

function dynamicPattern(node) {
  if (!ts.isTemplateExpression(node)) return null;
  return {
    prefix: node.head.text,
    suffix: node.templateSpans.at(-1)?.literal.text || ''
  };
}

function collectExpression(source, fileName, lineOffset = 0) {
  const sourceFile = ts.createSourceFile(fileName, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  function visit(node) {
    if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) {
      keyLiterals.add(node.text);
    }
    if (ts.isTemplateExpression(node)) {
      const pattern = dynamicPattern(node);
      if (pattern && /^(common|settings|workflow|menu|chat)\./.test(pattern.prefix)) {
        dynamicPatterns.push(pattern);
      }
    }
    if (ts.isCallExpression(node) && isTranslationCall(node.expression) && node.arguments.length) {
      const values = staticValues(node.arguments[0]);
      if (values.length) values.forEach(value => keys.add(value));
      else {
        const pattern = dynamicPattern(node.arguments[0]);
        if (pattern) dynamicPatterns.push(pattern);
        const position = sourceFile.getLineAndCharacterOfPosition(node.arguments[0].getStart(sourceFile));
        dynamic.push(`${fileName}:${position.line + lineOffset + 1}: ${node.arguments[0].getText(sourceFile)}`);
      }
    }
    ts.forEachChild(node, visit);
  }
  visit(sourceFile);
}

function collectTemplate(template, fileName, lineOffset) {
  const ast = parseTemplate(template);
  function visit(node) {
    if (node.type === NodeTypes.INTERPOLATION) {
      collectExpression(node.content.content, fileName, lineOffset + node.loc.start.line - 1);
    }
    if (node.type === NodeTypes.ELEMENT) {
      for (const prop of node.props) {
        if (prop.type === NodeTypes.DIRECTIVE && prop.exp) {
          collectExpression(prop.exp.content, fileName, lineOffset + prop.exp.loc.start.line - 1);
        }
      }
    }
    if (node.children) node.children.forEach(visit);
    if (node.branches) node.branches.forEach(visit);
  }
  visit(ast);
}

function collectFile(fileName) {
  const content = fs.readFileSync(fileName, 'utf8');
  if (path.extname(fileName) !== '.vue') return collectExpression(content, fileName);
  const { descriptor, errors } = parseSfc(content, { filename: fileName });
  if (errors.length) throw new Error(`Unable to parse Vue SFC: ${fileName}`);
  const lineOffset = block => content.slice(0, block.loc.start.offset).split('\n').length - 1;
  if (descriptor.script) collectExpression(descriptor.script.content, fileName, lineOffset(descriptor.script));
  if (descriptor.scriptSetup) {
    collectExpression(descriptor.scriptSetup.content, fileName, lineOffset(descriptor.scriptSetup));
  }
  if (descriptor.template) {
    collectTemplate(descriptor.template.content, fileName, lineOffset(descriptor.template));
  }
}

function walk(directory) {
  for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
    const fileName = path.join(directory, entry.name);
    if (entry.isDirectory()) walk(fileName);
    else if (extensions.has(path.extname(entry.name))) collectFile(fileName);
  }
}

walk(sourceDir);
console.log(JSON.stringify({ keys: [...keys], keyLiterals: [...keyLiterals], dynamic, dynamicPatterns }));
"""


def parse_json_keys(filepath):
    with open(filepath, "r", encoding="utf-8") as f:
        data = json.load(f)

    keys = set()

    def flatten_dict(value, parent_key=""):
        for key, item in value.items():
            full_key = f"{parent_key}.{key}" if parent_key else key
            if isinstance(item, dict):
                flatten_dict(item, full_key)
            elif isinstance(item, (str, int, float, bool)):
                keys.add(full_key)

    flatten_dict(data)
    return keys


def extract_frontend_i18n_keys(project_root, directory):
    """Use Vue and TypeScript ASTs instead of regular expressions.

    Static string, template, concatenation, and conditional expressions are resolved.
    Calls whose key cannot be statically resolved are reported separately rather than
    being mistaken for missing or unused locale entries.
    """
    result = subprocess.run(
        ["node", "-e", NODE_EXTRACTOR, directory],
        cwd=project_root,
        check=True,
        capture_output=True,
        text=True,
    )
    data = json.loads(result.stdout)
    return (
        set(data["keys"]),
        set(data["keyLiterals"]),
        sorted(set(data["dynamic"])),
        {(item["prefix"], item["suffix"]) for item in data["dynamicPatterns"]},
    )


def write_section(output, title, values):
    output.write(f"## {title}:\n")
    if values:
        for value in values:
            output.write(f"- {value}\n")
    else:
        output.write("- 无\n")


def main():
    script_dir = os.path.dirname(os.path.abspath(__file__))
    project_root = os.path.abspath(os.path.join(script_dir, ".."))
    zh_hans_file = os.path.join(project_root, "src", "i18n", "locales", "zh-Hans.json")
    rust_copy_file = os.path.join(project_root, "src", "i18n", "do_not_edit", "copy_from_rust_src_i18n.json")
    src_dir = os.path.join(project_root, "src")
    output_file = os.path.join(script_dir, "frontend_i18n_check_results.txt")

    defined_keys = parse_json_keys(zh_hans_file) | parse_json_keys(rust_copy_file)
    used_keys, key_literals, dynamic_calls, dynamic_patterns = extract_frontend_i18n_keys(
        project_root, src_dir
    )
    used_keys |= defined_keys & key_literals

    # Runtime values prevent exact key resolution. Keep matching locale keys out of
    # the unused list, while retaining the source location for manual review.
    dynamic_keys = {
        key
        for key in defined_keys
        if any(key.startswith(prefix) and key.endswith(suffix) for prefix, suffix in dynamic_patterns)
    }

    missing_keys = used_keys - defined_keys
    unused_keys = defined_keys - used_keys - dynamic_keys

    with open(output_file, "w", encoding="utf-8") as output:
        output.write("# 前端 i18n 检查结果（Vue/TypeScript AST 分析）\n\n")
        write_section(output, "缺失的语言项（代码中使用但 JSON 中未定义）", sorted(missing_keys))
        output.write("\n")
        write_section(output, "多余的语言项（JSON 中定义但未发现静态引用）", sorted(unused_keys))
        output.write("\n")
        write_section(output, "无法静态解析的动态调用（需人工核对）", dynamic_calls)

    print(f"Results written to {output_file}")


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as error:
        sys.stderr.write(error.stderr)
        raise
