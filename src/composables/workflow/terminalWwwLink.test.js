import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'
import { runInNewContext } from 'node:vm'

import { findWwwHosts } from './terminalWwwLink.js'

const texts = line => findWwwHosts(line).map(host => host.text)

test('recognises bare www hosts including multi-part ccTLDs', () => {
  assert.deepEqual(texts('visit www.example.com now'), ['www.example.com'])
  assert.deepEqual(texts('see www.foo.com.cn'), ['www.foo.com.cn'])
  assert.deepEqual(texts('WWW.Example.COM'), ['WWW.Example.COM'])
})

test('ignores scheme URLs and non-www hosts', () => {
  assert.deepEqual(texts('https://www.example.com'), [])
  assert.deepEqual(texts('http://www.example.com'), [])
  assert.deepEqual(texts('abc.def.eft.com'), [])
  assert.deepEqual(texts('subwww.example.com'), [])
  assert.deepEqual(texts('www.com'), [])
  assert.deepEqual(texts('www.example'), [])
})

test('stops before trailing punctuation and reports the column range', () => {
  const [host] = findWwwHosts('open www.example.com, ok')
  assert.equal(host.text, 'www.example.com')
  assert.equal(host.start, 5)
  assert.equal(host.end, 5 + 'www.example.com'.length - 1)
})

test('finds every host on a line', () => {
  assert.deepEqual(texts('www.a.com and www.b.org'), ['www.a.com', 'www.b.org'])
})

test('rejects embedded hosts without consuming adjacent link boundaries', () => {
  for (const prefix of ['a', '1', '_', '.', ':', '/', '-']) {
    assert.deepEqual(texts(`${prefix}www.a.com`), [])
  }
  const line = '(www.a.com),www.b.org https://www.c.net www.d.dev'
  assert.deepEqual(texts(line), ['www.a.com', 'www.b.org', 'www.d.dev'])
  for (const host of findWwwHosts(line)) {
    assert.equal(line.slice(host.start, host.end + 1), host.text)
  }
  assert.deepEqual(texts('www.a.com'), ['www.a.com'])
})

test('workflow route helpers avoid lookbehind and preserve sentence previews', () => {
  const chat = readFileSync(new URL('./useWorkflowChat.ts', import.meta.url), 'utf8')
  const links = readFileSync(new URL('./terminalWwwLink.js', import.meta.url), 'utf8')
  for (const source of [chat, links]) {
    assert.ok(!source.includes('(?<=') && !source.includes('(?<!'))
  }
  const start = chat.indexOf('const getLastSentence =')
  const end = chat.indexOf('// Get preview text', start)
  assert.ok(start >= 0 && end > start)
  const getLastSentence = runInNewContext(`${chat.slice(start, end)}; getLastSentence`)
  for (const [input, expected] of [
    ['', ''],
    ['Hello. Next sentence!', 'Next sentence!'],
    ['第一句。第二句！第三句？', '第三句？'],
    ['Hello.\n\tNext sentence', 'Next sentence'],
    ['Hello.   ', 'Hello.'],
    ['Hello?!', '!'],
    ['  no punctuation', '  no punctuation'],
    ['   ', '   ']
  ]) {
    assert.equal(getLastSentence(input), expected, JSON.stringify(input))
  }

  // Compare against the former behavior on modern Node, including empty splits
  // between punctuation marks and whitespace after each sentence boundary.
  const boundary = new RegExp('(?<=' + '[。！？.!?])\\s*')
  const fragments = ['word', '中文', '.', '。', '!', '?', ' ', '\n', '\t', '']
  for (const first of fragments) {
    for (const second of fragments) {
      for (const third of fragments) {
        const input = first + second + third
        const sentences = input.split(boundary).filter(sentence => sentence.trim())
        const expected = sentences[sentences.length - 1] || input.slice(-50)
        assert.equal(getLastSentence(input), expected, JSON.stringify(input))
      }
    }
  }
})
