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
import { cmake } from '@codemirror/legacy-modes/mode/cmake'
import { diff } from '@codemirror/legacy-modes/mode/diff'
import { dockerFile } from '@codemirror/legacy-modes/mode/dockerfile'
import { lua } from '@codemirror/legacy-modes/mode/lua'
import { nginx } from '@codemirror/legacy-modes/mode/nginx'
import { perl } from '@codemirror/legacy-modes/mode/perl'
import { powerShell } from '@codemirror/legacy-modes/mode/powershell'
import { properties } from '@codemirror/legacy-modes/mode/properties'
import { protobuf } from '@codemirror/legacy-modes/mode/protobuf'
import { r } from '@codemirror/legacy-modes/mode/r'
import { ruby } from '@codemirror/legacy-modes/mode/ruby'
import { sass } from '@codemirror/legacy-modes/mode/sass'
import { shell } from '@codemirror/legacy-modes/mode/shell'
import { sql } from '@codemirror/legacy-modes/mode/sql'
import { swift } from '@codemirror/legacy-modes/mode/swift'
import { toml } from '@codemirror/legacy-modes/mode/toml'
import { yaml } from '@codemirror/legacy-modes/mode/yaml'

const plainText = () => []

const apache = {
  startState: () => ({ atLineStart: true }),
  token(stream, state) {
    if (stream.sol()) state.atLineStart = true
    if (stream.eatSpace()) return null
    if (stream.match(/^#.*/)) return 'comment'
    if (stream.match(/^<[/!]?[A-Za-z][^>]*>/)) {
      state.atLineStart = false
      return 'tag'
    }
    if (stream.match(/^"[^"\\]*(?:\\.[^"\\]*)*"/) || stream.match(/^'[^'\\]*(?:\\.[^'\\]*)*'/)) {
      state.atLineStart = false
      return 'string'
    }
    if (stream.match(/^\S+/)) {
      const token = state.atLineStart ? 'keyword' : 'string'
      state.atLineStart = false
      return token
    }
    stream.next()
    return null
  }
}

const languageFactories = {
  apache: () => StreamLanguage.define(apache),
  bash: () => StreamLanguage.define(shell),
  c: () => cpp(),
  cmake: () => StreamLanguage.define(cmake),
  cpp: () => cpp(),
  csharp: () => StreamLanguage.define(clike),
  css: () => css(),
  diff: () => StreamLanguage.define(diff),
  dockerfile: () => StreamLanguage.define(dockerFile),
  go: () => go(),
  html: () => html(),
  java: () => java(),
  javascript: options => javascript(options),
  json: () => json(),
  lua: () => StreamLanguage.define(lua),
  markdown: () => markdown(),
  nginx: () => StreamLanguage.define(nginx),
  perl: () => StreamLanguage.define(perl),
  php: () => php(),
  powershell: () => StreamLanguage.define(powerShell),
  properties: () => StreamLanguage.define(properties),
  protobuf: () => StreamLanguage.define(protobuf),
  python: () => python(),
  r: () => StreamLanguage.define(r),
  ruby: () => StreamLanguage.define(ruby),
  rust: () => rust(),
  sass: () => StreamLanguage.define(sass),
  shell: () => StreamLanguage.define(shell),
  sql: () => StreamLanguage.define(sql),
  swift: () => StreamLanguage.define(swift),
  toml: () => StreamLanguage.define(toml),
  tsx: () => javascript({ typescript: true, jsx: true }),
  typescript: () => javascript({ typescript: true }),
  vue: () => vue(),
  xml: () => xml(),
  yaml: () => StreamLanguage.define(yaml),
  zig: plainText
}

export const EDITOR_LANGUAGE_BY_EXTENSION = Object.freeze({
  apacheconf: { id: 'apache', label: 'Apache Config' },
  bash: { id: 'bash', label: 'Shell' },
  bat: { id: 'shell', label: 'Shell' },
  c: { id: 'c', label: 'C' },
  cc: { id: 'cpp', label: 'C++' },
  cfg: { id: 'properties', label: 'Configuration' },
  cmake: { id: 'cmake', label: 'CMake' },
  conf: { id: 'apache', label: 'Apache Config' },
  cpp: { id: 'cpp', label: 'C++' },
  cs: { id: 'csharp', label: 'C#' },
  css: { id: 'css', label: 'CSS' },
  cxx: { id: 'cpp', label: 'C++' },
  diff: { id: 'diff', label: 'Diff' },
  dockerfile: { id: 'dockerfile', label: 'Dockerfile' },
  env: { id: 'properties', label: 'Environment' },
  fish: { id: 'shell', label: 'Shell' },
  go: { id: 'go', label: 'Go' },
  h: { id: 'c', label: 'C' },
  hh: { id: 'cpp', label: 'C++' },
  hpp: { id: 'cpp', label: 'C++' },
  htm: { id: 'html', label: 'HTML' },
  html: { id: 'html', label: 'HTML' },
  hxx: { id: 'cpp', label: 'C++' },
  ini: { id: 'properties', label: 'Configuration' },
  java: { id: 'java', label: 'Java' },
  jjs: { id: 'javascript', label: 'JavaScript' },
  js: { id: 'javascript', label: 'JavaScript' },
  json: { id: 'json', label: 'JSON' },
  jsonc: { id: 'json', label: 'JSON' },
  jsx: { id: 'javascript', label: 'JSX', options: { jsx: true } },
  ksh: { id: 'shell', label: 'Shell' },
  less: { id: 'css', label: 'Less' },
  lua: { id: 'lua', label: 'Lua' },
  markdown: { id: 'markdown', label: 'Markdown' },
  md: { id: 'markdown', label: 'Markdown' },
  mdx: { id: 'markdown', label: 'Markdown' },
  mkd: { id: 'markdown', label: 'Markdown' },
  mkdn: { id: 'markdown', label: 'Markdown' },
  mjs: { id: 'javascript', label: 'JavaScript' },
  nginx: { id: 'nginx', label: 'Nginx Config' },
  patch: { id: 'diff', label: 'Diff' },
  pb: { id: 'protobuf', label: 'Protocol Buffers' },
  perl: { id: 'perl', label: 'Perl' },
  php: { id: 'php', label: 'PHP' },
  pl: { id: 'perl', label: 'Perl' },
  pm: { id: 'perl', label: 'Perl' },
  properties: { id: 'properties', label: 'Configuration' },
  proto: { id: 'protobuf', label: 'Protocol Buffers' },
  ps1: { id: 'powershell', label: 'PowerShell' },
  psm1: { id: 'powershell', label: 'PowerShell' },
  py: { id: 'python', label: 'Python' },
  pyw: { id: 'python', label: 'Python' },
  r: { id: 'r', label: 'R' },
  rake: { id: 'ruby', label: 'Ruby' },
  rb: { id: 'ruby', label: 'Ruby' },
  rs: { id: 'rust', label: 'Rust' },
  sass: { id: 'sass', label: 'Sass' },
  scss: { id: 'sass', label: 'SCSS' },
  sh: { id: 'shell', label: 'Shell' },
  sql: { id: 'sql', label: 'SQL' },
  svg: { id: 'xml', label: 'SVG' },
  swift: { id: 'swift', label: 'Swift' },
  tcc: { id: 'cpp', label: 'C++' },
  toml: { id: 'toml', label: 'TOML' },
  ts: { id: 'typescript', label: 'TypeScript' },
  tsx: { id: 'tsx', label: 'TSX' },
  vhost: { id: 'apache', label: 'Apache Config' },
  vue: { id: 'vue', label: 'Vue' },
  xml: { id: 'xml', label: 'XML' },
  yaml: { id: 'yaml', label: 'YAML' },
  yml: { id: 'yaml', label: 'YAML' },
  zsh: { id: 'shell', label: 'Shell' },
  zig: { id: 'zig', label: 'Zig' }
})

const EDITOR_LANGUAGE_BY_FILE_NAME = Object.freeze({
  '.htaccess': { id: 'apache', label: 'Apache Config' },
  'apache2.conf': { id: 'apache', label: 'Apache Config' },
  'cmakelists.txt': { id: 'cmake', label: 'CMake' },
  dockerfile: { id: 'dockerfile', label: 'Dockerfile' },
  'httpd.conf': { id: 'apache', label: 'Apache Config' },
  'nginx.conf': { id: 'nginx', label: 'Nginx Config' },
  'ports.conf': { id: 'apache', label: 'Apache Config' },
  'ssl.conf': { id: 'apache', label: 'Apache Config' }
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
  'md',
  'markdown',
  'toml',
  'yaml',
  'yml',
  'sh',
  'bash',
  'zsh',
  'apacheconf',
  'conf',
  'ini',
  'cfg',
  'env',
  'properties',
  'dockerfile',
  'sql',
  'proto',
  'ps1',
  'cmake',
  'diff'
])

export function getFileExtension(path = '') {
  const name = path.split(/[\\/]/).pop() || ''
  const index = name.lastIndexOf('.')
  return index > -1 ? name.slice(index + 1).toLowerCase() : ''
}

function getFileName(path = '') {
  return (path.split(/[\\/]/).pop() || '').toLowerCase()
}

export function resolveEditorLanguage(path = '') {
  const extension = getFileExtension(path)
  const entry = EDITOR_LANGUAGE_BY_FILE_NAME[getFileName(path)] || EDITOR_LANGUAGE_BY_EXTENSION[extension]

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
