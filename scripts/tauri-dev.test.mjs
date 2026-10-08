import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { PassThrough } from 'node:stream'
import { createServer as createHttpServer } from 'node:http'
import test from 'node:test'
import { executeTauri, needsRuntimeBuild, runBuild } from './tauri.mjs'
import { startVite } from './tauri-vite.mjs'

const root = new URL('../', import.meta.url)
const deadline = { timeout: 20000 }
const tick = () => new Promise(resolve => setImmediate(resolve))

function fakeServer() {
  const events = []
  return {
    events,
    listen: async () => events.push('listen'),
    close: async () => events.push('close'),
    printUrls: () => events.push('urls')
  }
}

test('only a real dev command prebuilds; help, version and other commands remain cheap', () => {
  assert.equal(needsRuntimeBuild(['dev']), true)
  assert.equal(needsRuntimeBuild(['-v', 'dev', '--target', 'x']), true)
  for (const args of [['dev', '--help'], ['dev', '-h'], ['--version'], ['build'], ['info'], []]) {
    assert.equal(needsRuntimeBuild(args), false)
  }
  assert.equal(needsRuntimeBuild(['dev', '--', '--help']), true)
})

test('CLI starts only after the prebuild finishes and receives unchanged arguments', async () => {
  const args = ['dev', '--target', 'x', '--', '--features', 'y', '--', 'app arg']
  const events = []
  let release
  const result = executeTauri(args, {
    buildRuntime: () => new Promise(resolve => { events.push('build'); release = resolve }),
    runCli: received => { assert.equal(received, args); events.push('cli') }
  })
  await tick()
  assert.deepEqual(events, ['build'])
  release(0)
  assert.equal(await result, 0)
  assert.deepEqual(events, ['build', 'cli'])
})

test('failed/cancelled prebuild does not start Tauri or Vite', async () => {
  for (const code of [1, 7, 130, 143]) {
    assert.equal(await executeTauri(['dev'], {
      buildRuntime: async () => code,
      runCli: () => assert.fail('CLI must not start')
    }), code)
  }
})

test('non-dev arguments bypass the build and CLI errors propagate', async () => {
  const args = ['build', '--target', 'x']
  await executeTauri(args, {
    buildRuntime: () => assert.fail('must not build runtime'),
    runCli: received => assert.equal(received, args)
  })
  await assert.rejects(executeTauri(['info'], {
    runCli: async () => { throw new Error('cli failed') }
  }), /cli failed/)
})

test('build returns exit status and rejects a missing executable', async () => {
  assert.equal(await runBuild(process.execPath, ['-e', 'process.exit(7)']), 7)
  await assert.rejects(runBuild('chatspeed-nonexistent-command', []), /ENOENT/)
})

test('parent EOF closes the server once; closing before createServer resolves never listens', async () => {
  const input = new PassThrough()
  const server = fakeServer()
  const handle = await startVite({ createServer: async () => server, input })
  input.end()
  await tick()
  await handle.shutdown()
  assert.deepEqual(server.events, ['listen', 'urls', 'close'])

  const earlyInput = new PassThrough()
  let release
  const earlyServer = fakeServer()
  const starting = startVite({
    input: earlyInput,
    createServer: () => new Promise(resolve => { release = resolve })
  })
  earlyInput.end()
  await tick()
  release(earlyServer)
  await starting
  assert.deepEqual(earlyServer.events, ['close'])
})

test('EOF during listen closes after listen completes without publishing URLs', async () => {
  const input = new PassThrough()
  const server = fakeServer()
  let release
  server.listen = () => new Promise(resolve => { release = resolve })
  const started = startVite({ createServer: async () => server, input })
  await tick()
  input.end()
  await tick()
  release()
  await started
  assert.deepEqual(server.events, ['close'])
})

test('listen/create failures close handles and remove signal listeners', async () => {
  const count = process.listenerCount('SIGTERM')
  const server = fakeServer()
  server.listen = async () => { throw new Error('port occupied') }
  await assert.rejects(startVite({ createServer: async () => server, input: new PassThrough() }), /port occupied/)
  assert.deepEqual(server.events, ['close'])
  await assert.rejects(startVite({
    createServer: async () => { throw new Error('config failed') }, input: new PassThrough()
  }), /config failed/)
  assert.equal(process.listenerCount('SIGTERM'), count)
})

async function startRealHook(t) {
  const { createServer } = await import('vite')
  const probe = await createServer({
    configFile: false, server: { host: '127.0.0.1', port: 0, strictPort: true }, logLevel: 'silent'
  })
  await probe.listen()
  const port = probe.httpServer.address().port
  await probe.close()
  const script = `import {createServer} from 'vite';
    import {startVite} from './scripts/tauri-vite.mjs';
    await startVite({createServer,options:{configFile:false,logLevel:'silent',server:{host:'127.0.0.1',port:${port},strictPort:true}}});
    console.log('READY');`
  const child = spawn(process.execPath, ['--input-type=module', '-e', script], {
    cwd: root, stdio: ['pipe', 'pipe', 'pipe']
  })
  t.after(() => { if (child.exitCode === null) child.kill('SIGKILL') })
  const exited = once(child, 'exit')
  let output = ''
  await new Promise((resolve, reject) => {
    child.stdout.on('data', data => { output += data; if (output.includes('READY')) resolve() })
    child.once('error', reject)
    child.once('exit', code => reject(new Error(`hook exited before ready: ${code}`)))
  })
  return { child, exited, port }
}

async function assertPortReusable(port) {
  const server = createHttpServer()
  await new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(port, '127.0.0.1', resolve)
  })
  await new Promise(resolve => server.close(resolve))
}

for (const action of ['EOF', 'SIGINT', 'SIGTERM']) {
  test(`real Vite releases its port after ${action}`, deadline, async t => {
    const { child, exited, port } = await startRealHook(t)
    assert.equal((await fetch(`http://127.0.0.1:${port}/`)).status, 200)
    if (action === 'EOF') child.stdin.end()
    else child.kill(action)
    const [code] = await exited
    assert.equal(code, 0)
    await assertPortReusable(port)
  })
}

test('the real project Vite configuration loads and can restart after shutdown', deadline, async () => {
  const { createServer } = await import('vite')
  for (let attempt = 0; attempt < 2; attempt++) {
    const input = new PassThrough()
    const handle = await startVite({
      createServer, input,
      options: { logLevel: 'silent', server: { host: '127.0.0.1', port: 0, strictPort: true } }
    })
    const port = handle.server.httpServer.address().port
    try {
      assert.equal((await fetch(`http://127.0.0.1:${port}/@vite/client`)).status, 200)
    } finally {
      await handle.shutdown()
    }
    await assertPortReusable(port)
  }
})

test('strictPort rejects a busy port without terminating its owner', deadline, async () => {
  const owner = createHttpServer((_request, response) => response.end('original owner'))
  await new Promise(resolve => owner.listen(0, '127.0.0.1', resolve))
  const port = owner.address().port
  try {
    const { createServer } = await import('vite')
    await assert.rejects(startVite({
      createServer, input: new PassThrough(),
      options: { configFile: false, logLevel: 'silent', server: { host: '127.0.0.1', port, strictPort: true } }
    }), /already in use/)
    assert.equal(await (await fetch(`http://127.0.0.1:${port}/`)).text(), 'original owner')
  } finally {
    await new Promise(resolve => owner.close(resolve))
  }
})

test('SIGINT during a build prevents later CLI startup', deadline, async t => {
  const script = `import {runBuild} from './scripts/tauri.mjs';
    const build = runBuild(process.execPath,['-e',"console.log('BUILDING');setInterval(()=>{},1000)"]);
    process.exitCode = await build;`
  const child = spawn(process.execPath, ['--input-type=module', '-e', script], {
    cwd: root, stdio: ['ignore', 'pipe', 'pipe']
  })
  t.after(() => { if (child.exitCode === null) child.kill('SIGKILL') })
  const exited = once(child, 'exit')
  await new Promise(resolve => child.stdout.once('data', resolve))
  child.kill('SIGINT')
  const [code] = await exited
  assert.equal(code, 130)
})
