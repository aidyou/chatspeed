import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

export async function startVite({ createServer, input = process.stdin, options = {} }) {
  let server
  let stopping = false
  let starting = true
  let closing
  const detach = () => {
    input.off('end', onParentExit)
    input.off('close', onParentExit)
    process.off('SIGINT', onInterrupt)
    process.off('SIGTERM', onInterrupt)
    input.pause()
  }
  const shutdown = () => {
    stopping = true
    if (!server || starting) return Promise.resolve()
    closing ??= server.close().finally(detach)
    return closing
  }
  const onParentExit = () => { void shutdown() }
  const onInterrupt = () => { void shutdown() }
  // Tauri pipes this hook's stdin. EOF also closes Vite when the CLI exits via
  // a timeout/error path that does not execute its normal process-tree cleanup.
  input.once('end', onParentExit)
  input.once('close', onParentExit)
  process.once('SIGINT', onInterrupt)
  process.once('SIGTERM', onInterrupt)
  input.resume()
  try {
    server = await createServer(options)
    if (!stopping) await server.listen()
    starting = false
    if (stopping) {
      await shutdown()
    } else {
      server.printUrls()
    }
    return { server, shutdown }
  } catch (error) {
    starting = false
    await shutdown()
    detach()
    throw error
  }
}

async function main() {
  const { createServer } = await import('vite')
  await startVite({ createServer })
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(error => {
    console.error(error.message)
    process.exitCode = 1
  })
}
