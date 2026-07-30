import { StreamLanguage } from '@codemirror/language'
import { cpp } from '@codemirror/lang-cpp'
import { css } from '@codemirror/lang-css'
import { go } from '@codemirror/lang-go'
import { html } from '@codemirror/lang-html'
import { java } from '@codemirror/lang-java'
import { javascript } from '@codemirror/lang-javascript'
import { json } from '@codemirror/lang-json'
import { markdown } from '@codemirror/lang-markdown'
import { php } from '@codemirror/lang-php'
import { python } from '@codemirror/lang-python'
import { rust } from '@codemirror/lang-rust'
import { vue } from '@codemirror/lang-vue'
import { xml } from '@codemirror/lang-xml'
import { clike } from '@codemirror/legacy-modes/mode/clike'
import { lua } from '@codemirror/legacy-modes/mode/lua'
import { shell } from '@codemirror/legacy-modes/mode/shell'
import { toml } from '@codemirror/legacy-modes/mode/toml'
import { yaml } from '@codemirror/legacy-modes/mode/yaml'

const plainText = () => []

const languageFactories = {
  bash: () => StreamLanguage.define(shell),
  c: () => cpp(),
  cpp: () => cpp(),
  csharp: () => StreamLanguage.define(clike),
  css: () => css(),
  go: () => go(),
  html: () => html(),
  java: () => java(),
  javascript: options => javascript(options),
  json: () => json(),
  lua: () => StreamLanguage.define(lua),
  markdown: () => markdown(),
  php: () => php(),
  python: () => python(),
  rust: () => rust(),
  shell: () => StreamLanguage.define(shell),
  toml: () => StreamLanguage.define(toml),
  tsx: () => javascript({ typescript: true, jsx: true }),
  typescript: () => javascript({ typescript: true }),
  vue: () => vue(),
  xml: () => xml(),
  yaml: () => StreamLanguage.define(yaml),
  zig: plainText
}

export const EDITOR_LANGUAGE_BY_EXTENSION = Object.freeze({
  bash: { id: 'bash', label: 'Shell' },
  c: { id: 'c', label: 'C' },
  cc: { id: 'cpp', label: 'C++' },
  cpp: { id: 'cpp', label: 'C++' },
  cs: { id: 'csharp', label: 'C#' },
  css: { id: 'css', label: 'CSS' },
  cxx: { id: 'cpp', label: 'C++' },
  go: { id: 'go', label: 'Go' },
  h: { id: 'c', label: 'C' },
  hpp: { id: 'cpp', label: 'C++' },
  htm: { id: 'html', label: 'HTML' },
  html: { id: 'html', label: 'HTML' },
  java: { id: 'java', label: 'Java' },
  js: { id: 'javascript', label: 'JavaScript' },
  json: { id: 'json', label: 'JSON' },
  jsx: { id: 'javascript', label: 'JSX', options: { jsx: true } },
  lua: { id: 'lua', label: 'Lua' },
  md: { id: 'markdown', label: 'Markdown' },
  mjs: { id: 'javascript', label: 'JavaScript' },
  php: { id: 'php', label: 'PHP' },
  py: { id: 'python', label: 'Python' },
  rs: { id: 'rust', label: 'Rust' },
  sh: { id: 'shell', label: 'Shell' },
  toml: { id: 'toml', label: 'TOML' },
  ts: { id: 'typescript', label: 'TypeScript' },
  tsx: { id: 'tsx', label: 'TSX' },
  vue: { id: 'vue', label: 'Vue' },
  xml: { id: 'xml', label: 'XML' },
  yaml: { id: 'yaml', label: 'YAML' },
  yml: { id: 'yaml', label: 'YAML' },
  zig: { id: 'zig', label: 'Zig' }
})

export const USER_REQUESTED_EDITOR_EXTENSIONS = Object.freeze([
  'rs',
  'go',
  'php',
  'html',
  'js',
  'ts',
  'css',
  'py',
  'lua',
  'zig',
  'c',
  'cpp',
  'java',
  'cs',
  'sh',
  'bash'
])

export function getFileExtension(path = '') {
  const name = path.split(/[\\/]/).pop() || ''
  const index = name.lastIndexOf('.')
  return index > -1 ? name.slice(index + 1).toLowerCase() : ''
}

export function resolveEditorLanguage(path = '') {
  const extension = getFileExtension(path)
  const entry = EDITOR_LANGUAGE_BY_EXTENSION[extension]

  if (!entry) {
    return {
      id: 'text',
      label: 'Plain Text',
      extension,
      supported: false,
      load: plainText
    }
  }

  const factory = languageFactories[entry.id] || plainText

  return {
    ...entry,
    extension,
    supported: true,
    load: () => factory(entry.options)
  }
}
