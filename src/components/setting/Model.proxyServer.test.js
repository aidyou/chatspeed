import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

const readModelComponent = () => readFile(new URL('./Model.vue', import.meta.url), 'utf8')

const loadNormalizeProxyServerAddress = async () => {
  const source = await readModelComponent()
  const match = source.match(
    /const normalizeProxyServerAddress = value => \{([\s\S]*?)\n\}\n\nconst isValidUrl/
  )
  assert.ok(match, 'proxy server address normalizer must remain independently testable')
  return new Function('value', match[1])
}

test('adds http protocol to bare IPv4 proxy addresses', async () => {
  const normalizeProxyServerAddress = await loadNormalizeProxyServerAddress()

  assert.equal(normalizeProxyServerAddress('127.0.0.1:8080'), 'http://127.0.0.1:8080')
  assert.equal(normalizeProxyServerAddress(' 192.168.1.10:3128 '), 'http://192.168.1.10:3128')
})

test('preserves explicit protocols and does not guess for non-IP hosts', async () => {
  const normalizeProxyServerAddress = await loadNormalizeProxyServerAddress()

  assert.equal(normalizeProxyServerAddress('http://127.0.0.1:8080'), 'http://127.0.0.1:8080')
  assert.equal(normalizeProxyServerAddress('https://10.0.0.1:443'), 'https://10.0.0.1:443')
  assert.equal(normalizeProxyServerAddress('proxy.example.com:8080'), 'proxy.example.com:8080')
  assert.equal(normalizeProxyServerAddress(''), '')
})

test('normalizes proxy addresses before validating dialog and provider saves', async () => {
  const source = await readModelComponent()

  assert.match(
    source,
    /const saveProxyServer = \(\) => \{\n  const server = normalizeProxyServerAddress\(proxyServerForm\.value\.server\)[\s\S]*?if \(!isValidUrl\(server\)\)/
  )
  assert.match(
    source,
    /modelForm\.value\.proxyServers = modelForm\.value\.proxyServers\.map\(server => \(\{[\s\S]*?server: normalizeProxyServerAddress\(server\.server\)[\s\S]*?if \(modelForm\.value\.proxyServers\.some\(server => !isValidUrl\(server\.server\)\)\)/
  )
})
