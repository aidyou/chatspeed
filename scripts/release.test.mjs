import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, copyFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { spawnSync } from 'node:child_process'

const root = new URL('../', import.meta.url)
const script = readFileSync(new URL('scripts/release.sh', root), 'utf8')
const functions = script.slice(0, script.lastIndexOf('\nmain "$@"'))
const manifests = ['src-tauri/Cargo.toml', ...['daemon', 'backend', 'cli', 'client', 'contracts'].map(name => `src-tauri/crates/${name}/Cargo.toml`)]

function fixture(callback) {
  const cwd = mkdtempSync(join(tmpdir(), 'chatspeed-release-'))
  const files = [...manifests, 'src-tauri/Cargo.lock', 'src-tauri/tauri.conf.json', 'vcpkg.json']
  try {
    for (const file of files) {
      mkdirSync(dirname(join(cwd, file)), { recursive: true })
      copyFileSync(new URL(file, root), join(cwd, file))
    }
    const version = JSON.parse(readFileSync(join(cwd, 'src-tauri/tauri.conf.json'), 'utf8')).version
    const run = command => spawnSync('bash', ['-c', `${functions}\n${command}`], { cwd, encoding: 'utf8' })
    callback({ cwd, version, run })
  } finally {
    rmSync(cwd, { recursive: true, force: true })
  }
}

test('accepts shared release versions without comparing third-party dependencies', () => fixture(({ version, run }) => {
  const result = run(`verify_version_update '${version}'`)
  assert.equal(result.status, 0, result.stdout + result.stderr)
}))

test('reports all mismatches and missing files before exiting', () => fixture(({ cwd, version, run }) => {
  const daemon = join(cwd, manifests[1])
  writeFileSync(daemon, readFileSync(daemon, 'utf8').replace(`version = "${version}"`, 'version = "0.0.1"'))
  rmSync(join(cwd, manifests[4]))
  const lock = join(cwd, 'src-tauri/Cargo.lock')
  writeFileSync(lock, readFileSync(lock, 'utf8').replace(`name = "chatspeed-cli"\nversion = "${version}"`, 'name = "chatspeed-cli"\nversion = "0.0.2"'))
  const result = run(`verify_version_update '${version}'; echo SHOULD_NOT_RUN`)
  assert.equal(result.status, 1)
  for (const text of [manifests[1], manifests[4], 'Cargo.lock (chatspeed-cli)', '0.0.1', '0.0.2']) {
    assert.ok(result.stdout.includes(text), result.stdout)
  }
  assert.ok(!result.stdout.includes('SHOULD_NOT_RUN'))
}))

test('updates every release package without changing dependency requirements', () => fixture(({ cwd, run }) => {
  const manifest = join(cwd, manifests[0])
  writeFileSync(manifest, readFileSync(manifest, 'utf8') + '\n[dependencies.release-test-only]\nversion = "0.3.0"\n')
  const before = manifests.map(file => readFileSync(join(cwd, file), 'utf8'))
  const result = run('update_cargo_toml 9.8.7')
  assert.equal(result.status, 0, result.stdout + result.stderr)
  manifests.forEach((file, index) => {
    const after = readFileSync(join(cwd, file), 'utf8')
    assert.equal(after, before[index].replace(/^version\s*=\s*"[^"]*"/m, 'version = "9.8.7"'))
  })
}))

test('main rejects drift before file updates, commit, or push', () => fixture(({ cwd, run }) => {
  const daemon = join(cwd, manifests[1])
  writeFileSync(daemon, readFileSync(daemon, 'utf8').replace(/^version = "[^"]*"/m, 'version = "0.0.1"'))
  const config = join(cwd, 'src-tauri/tauri.conf.json')
  const before = readFileSync(config, 'utf8')
  const result = run('git() { return 0; }; main 9.8.7')
  assert.equal(result.status, 1, result.stdout + result.stderr)
  assert.ok(!result.stdout.includes('Release Summary:'))
  assert.equal(readFileSync(config, 'utf8'), before)
}))
