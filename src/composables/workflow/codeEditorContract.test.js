import assert from 'node:assert/strict'
import { readFile, readdir } from 'node:fs/promises'
import test from 'node:test'
import { EditorState } from '@codemirror/state'
import { insertTab } from '@codemirror/commands'
import { ref } from 'vue'

import {
  EDITOR_LANGUAGE_BY_EXTENSION,
  USER_REQUESTED_EDITOR_EXTENSIONS,
  resolveEditorLanguage
} from './codeEditorLanguages.js'
import {
  WORKFLOW_EDITOR_MAX_BYTES,
  useWorkflowCodeEditor
} from './useWorkflowCodeEditor.js'

const read = path => readFile(new URL(`../../../${path}`, import.meta.url), 'utf8')

test('editor language mapping covers user requested file extensions', () => {
  for (const extension of USER_REQUESTED_EDITOR_EXTENSIONS) {
    assert.ok(EDITOR_LANGUAGE_BY_EXTENSION[extension], `${extension} should be mapped`)
    const language = resolveEditorLanguage(`example.${extension}`)
    assert.equal(language.supported, true, `${extension} should be supported`)
    assert.equal(typeof language.load, 'function')
    assert.doesNotThrow(() => language.load(), `${extension} should load`)
  }

  assert.equal(resolveEditorLanguage('main.rs').label, 'Rust')
  assert.equal(resolveEditorLanguage('main.go').label, 'Go')
  assert.equal(resolveEditorLanguage('index.ts').label, 'TypeScript')
  assert.equal(resolveEditorLanguage('README.md').label, 'Markdown')
  assert.equal(resolveEditorLanguage('config.toml').label, 'TOML')
  assert.equal(resolveEditorLanguage('deployment.yaml').label, 'YAML')
  assert.equal(resolveEditorLanguage('script.sh').label, 'Shell')
  assert.equal(resolveEditorLanguage('httpd.conf').label, 'Apache Config')
  assert.equal(resolveEditorLanguage('.htaccess').label, 'Apache Config')
  assert.equal(resolveEditorLanguage('nginx.conf').label, 'Nginx Config')
  assert.equal(resolveEditorLanguage('Dockerfile').label, 'Dockerfile')
  assert.equal(resolveEditorLanguage('program.zig').label, 'Zig')

  for (const [path, label] of [
    ['README.mdx', 'Markdown'],
    ['config.toml', 'TOML'],
    ['deployment.yml', 'YAML'],
    ['script.zsh', 'Shell'],
    ['sites-enabled/example.conf', 'Apache Config'],
    ['.htaccess', 'Apache Config'],
    ['nginx.conf', 'Nginx Config'],
    ['Dockerfile', 'Dockerfile'],
    ['app.env', 'Environment'],
    ['application.properties', 'Configuration'],
    ['query.sql', 'SQL'],
    ['schema.proto', 'Protocol Buffers'],
    ['script.ps1', 'PowerShell'],
    ['CMakeLists.txt', 'CMake']
  ]) {
    const language = resolveEditorLanguage(path)
    assert.equal(language.label, label, `${path} should use ${label}`)
    assert.doesNotThrow(() => language.load(), `${path} extension should load`)
  }
})

test('tab command inserts a tab at the cursor without indenting the line', () => {
  let state = EditorState.create({ doc: 'alpha', selection: { anchor: 2 } })

  const handled = insertTab({
    state,
    dispatch(transaction) {
      state = transaction.state
    }
  })

  assert.equal(handled, true)
  assert.equal(state.doc.toString(), 'al\tpha')
  assert.equal(state.selection.main.head, 3)
})

test('editor state handles tabs, dirty state, successful save, and conflicts', async () => {
  const calls = []
  const notifications = []
  const saveErrors = []
  const editor = useWorkflowCodeEditor({
    t: (key, params = {}) => `${key}${params.name ? `:${params.name}` : ''}${params.error ? `:${params.error}` : ''}`,
    usesCommandKey: ref(false),
    notify: (message, type) => notifications.push({ message, type }),
    saveError: (message, title, options) => saveErrors.push({ message, title, options }),
    confirm: async () => true,
    invoke: async (command, payload) => {
      calls.push({ command, payload })
      if (command === 'read_text_file_for_editor') {
        return {
          name: payload.filePath.split('/').pop(),
          content: 'initial',
          size: 7,
          modified_at_ms: 100
        }
      }
      if (command === 'write_text_file_for_editor') {
        return { size: payload.content.length, modified_at_ms: 200 }
      }
      throw new Error(`unexpected command ${command}`)
    }
  })

  await editor.openFile('/tmp/example.rs')
  await editor.openFile('/tmp/example.rs')
  assert.equal(editor.tabs.value.length, 1)
  assert.equal(calls.filter(call => call.command === 'read_text_file_for_editor').length, 1)

  editor.updateContent('/tmp/example.rs', 'changed')
  assert.equal(editor.activeTab.value.dirty, true)
  await editor.saveTab('/tmp/example.rs')
  const saveCall = calls.find(call => call.command === 'write_text_file_for_editor')
  assert.ok(saveCall)
  assert.equal(typeof saveCall.payload.expectedModifiedAtMs, 'number')
  assert.equal(saveCall.payload.expectedModifiedAtMs, 100)
  assert.equal(editor.activeTab.value.dirty, false)
  assert.equal(editor.activeTab.value.modifiedAtMs, 200)
  assert.deepEqual(notifications, [{ message: 'workflow.codeEditor.saveSuccess:example.rs', type: 'success' }])

  const conflictEditor = useWorkflowCodeEditor({
    t: key => key,
    notify: (message, type) => notifications.push({ message, type }),
    saveError: (message, title, options) => saveErrors.push({ message, title, options }),
    confirm: async () => true,
    invoke: async command => {
      if (command === 'read_text_file_for_editor') {
        return { name: 'conflict.ts', content: 'base', size: 4, modified_at_ms: 10 }
      }
      throw new Error('File changed on disk. Reload before saving.')
    }
  })
  await conflictEditor.openFile('/tmp/conflict.ts')
  conflictEditor.updateContent('/tmp/conflict.ts', 'user edit')
  await assert.rejects(() => conflictEditor.saveTab('/tmp/conflict.ts'))
  assert.equal(conflictEditor.activeTab.value.conflict, true)
  assert.equal(conflictEditor.activeTab.value.content, 'user edit')
  assert.equal(conflictEditor.activeTab.value.dirty, true)
  assert.deepEqual(saveErrors, [{
    message: 'workflow.codeEditor.externalChangeMessage',
    title: 'common.warning',
    options: { confirmButtonText: 'common.confirm', type: 'warning' }
  }])

  const failedSaveEditor = useWorkflowCodeEditor({
    t: (key, params = {}) => `${key}${params.error ? `:${params.error}` : ''}`,
    saveError: (message, title, options) => saveErrors.push({ message, title, options }),
    invoke: async command => {
      if (command === 'read_text_file_for_editor') {
        return { name: 'failed.ts', content: 'base', size: 4, modified_at_ms: 10 }
      }
      throw new Error('Permission denied')
    }
  })
  await failedSaveEditor.openFile('/tmp/failed.ts')
  failedSaveEditor.updateContent('/tmp/failed.ts', 'user edit')
  await assert.rejects(() => failedSaveEditor.saveTab('/tmp/failed.ts'))
  assert.deepEqual(saveErrors[1], {
    message: 'workflow.codeEditor.saveFailed:Permission denied',
    title: 'common.error',
    options: { confirmButtonText: 'common.confirm', type: 'error' }
  })
})

test('workflow code editor source contracts keep CodeMirror, shortcuts, and tab escape hint', async () => {
  const [component, composable, workflow, sidebar, fileTree, fsCommands, lib] = await Promise.all([
    read('src/components/workflow/WorkflowCodeEditor.vue'),
    read('src/composables/workflow/useWorkflowCodeEditor.js'),
    read('src/views/Workflow.vue'),
    read('src/components/workflow/WorkflowSidebar.vue'),
    read('src/components/workflow/FileTree.vue'),
    read('src-tauri/src/commands/fs.rs'),
    read('src-tauri/src/lib.rs')
  ])

  assert.equal(WORKFLOW_EDITOR_MAX_BYTES <= 5 * 1024 * 1024, true)
  assert.match(component, /EditorView/)
  assert.match(component, /EditorView\.updateListener/)
  assert.match(component, /key: 'Tab'/)
  assert.match(component, /run: insertTab/)
  assert.doesNotMatch(component, /indentWithTab/)
  assert.match(component, /key: 'Mod-s'/)
  assert.match(component, /EditorView\.lineWrapping/)
  assert.match(component, /\.cm-gutters/)
  assert.match(component, /\.cm-search/)
  assert.match(component, /tabEscapeHint/)
  assert.match(component, /role="tablist"/)
  assert.match(composable, /read_text_file_for_editor/)
  assert.match(composable, /write_text_file_for_editor/)
  assert.match(composable, /expectedModifiedAtMs/)
  assert.match(composable, /expectedSize/)
  assert.match(workflow, /<WorkflowCodeEditor/)
  assert.match(workflow, /workflow-workspace/)
  assert.match(workflow, /code-editor-resize-handle/)
  assert.match(workflow, /onCodeEditorResizeStart/)
  assert.match(workflow, /codeEditor\.hasTabs\.value/)
  assert.match(workflow, /workflow-chat-pane/)
  assert.match(workflow, /<TerminalPanel :terminal="terminal" :preferences="terminalPreferences" \/>/)
  assert.match(workflow, /codeEditorFocused/)
  assert.match(sidebar, /open-editor-file/)
  assert.match(fileTree, /emit\('openFile', path\)/)
  assert.doesNotMatch(fileTree, /previewContent\.value = `\\`\\`\\`\$\{ext\}/)
  assert.match(fsCommands, /const EDITOR_MAX_FILE_BYTES: u64 = 5 \* 1024 \* 1024/)
  assert.match(fsCommands, /ensure_editor_file_size\(&metadata, resolved_max_bytes\)\?;/)
  assert.match(fsCommands, /metadata\.len\(\) != expected_size/)
  assert.match(lib, /get_text_file_info/)
  assert.match(lib, /read_text_file_for_editor/)
  assert.match(lib, /write_text_file_for_editor/)
})

test('all frontend locales contain identical workflow editor and file preview keys', async () => {
  const dir = new URL('../../../src/i18n/locales/', import.meta.url)
  const files = (await readdir(dir)).filter(file => file.endsWith('.json'))
  const keyShape = value => Object.keys(value.workflow.codeEditor).sort()
  const previewKeyShape = value => Object.keys(value.workflow.filePreview).sort()
  let expected = null
  let expectedPreview = null

  for (const file of files) {
    const content = JSON.parse(await read(`src/i18n/locales/${file}`))
    assert.ok(content.workflow?.codeEditor, `${file} should include workflow.codeEditor`)
    assert.ok(content.workflow?.filePreview, `${file} should include workflow.filePreview`)
    const shape = keyShape(content)
    const previewShape = previewKeyShape(content)
    expected ||= shape
    expectedPreview ||= previewShape
    assert.deepEqual(shape, expected, `${file} should match codeEditor key structure`)
    assert.deepEqual(previewShape, expectedPreview, `${file} should match filePreview key structure`)
  }
})
