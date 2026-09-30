import assert from 'node:assert/strict'
import test from 'node:test'

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
