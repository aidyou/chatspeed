import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { runInNewContext } from 'node:vm'
import test from 'node:test'

const source = await readFile(new URL('./directive.js', import.meta.url), 'utf8')

test('failed diagram input is inserted as text without parsing markup', () => {
  const helper = source.match(/function renderDiagramError\(diagram, title, content\) \{[\s\S]*?\n\}/)?.[0]
  assert.ok(helper)
  const created = []
  const document = {
    createElement(tag) {
      const element = {
        tag, children: [], textContent: '',
        appendChild(child) { this.children.push(child) },
        set innerHTML(_) { assert.fail('error fallback must never parse HTML') }
      }
      created.push(element)
      return element
    }
  }
  const render = runInNewContext(`(${helper})`, { document })
  const diagram = { replaceChildren(child) { this.child = child } }
  const content = '<img src=x onerror="window.secret=1"><script>attack()</script>'
  const title = '<b>diagram failure</b>'
  render(diagram, title, content)
  assert.equal(created.length, 3)
  assert.equal(diagram.child.tag, 'pre')
  assert.equal(diagram.child.className, 'mermaid-error')
  assert.equal(diagram.child.children[0].textContent, title)
  assert.equal(diagram.child.children[1].textContent, content)
  for (const key of ['diagramError', 'mindmapError']) {
    assert.ok(source.includes(`renderDiagramError(diagram, i18n.global.t('chat.${key}'), content)`))
  }
})

test('math and tool decode error fallbacks retain text-only localized output', () => {
  assert.match(source, /element\.textContent = element\.getAttribute\('data-formula'\) \|\| ''/)
  assert.doesNotMatch(source, /element\.innerHTML = element\.getAttribute\('data-formula'\)/)
  assert.match(source, /codeElement\.textContent = i18n\.global\.t\('common\.errorFallback'\)/)
  assert.doesNotMatch(source, /Error displaying tool result\./)
})
