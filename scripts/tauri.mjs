import { spawn } from 'node:child_process'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const projectRoot = fileURLToPath(new URL('../', import.meta.url))

export function needsRuntimeBuild(args) {
  // Help and runner/app arguments must never trigger an expensive prebuild.
  const options = args.slice(0, args.indexOf('--') < 0 ? args.length : args.indexOf('--'))
  return options.find(arg => !arg.startsWith('-')) === 'dev' &&
    !options.some(arg => ['--help', '-h', '--version', '-V'].includes(arg))
}

export function runBuild(command, args, { cwd = projectRoot, killDelay = 3000 } = {}) {
  return new Promise((resolveBuild, rejectBuild) => {
    // Cargo's rustc children belong to this build only, not the invoking terminal.
    const child = spawn(command, args, {
      cwd, stdio: 'inherit', shell: false, detached: process.platform !== 'win32'
    })
    let interrupted = 0
    let escalation
    const kill = signal => {
      if (!child.pid) return
      if (process.platform === 'win32') {
        // Windows has no POSIX process groups; target only the child we created.
        const cleanup = spawn('taskkill', ['/PID', String(child.pid), '/T', '/F'], {
          stdio: 'ignore', shell: false
        })
        cleanup.on('error', () => child.kill())
      } else {
        try { process.kill(-child.pid, signal) } catch (error) {
          if (error.code !== 'ESRCH') child.kill(signal)
        }
      }
    }
    const onInterrupt = () => stop('SIGINT', 130)
    const onTerminate = () => stop('SIGTERM', 143)
    const stop = (signal, code) => {
      if (interrupted) return
      interrupted = code
      kill(signal)
      escalation = setTimeout(() => kill('SIGKILL'), killDelay)
      escalation.unref()
    }
    const dispose = () => {
      clearTimeout(escalation)
      process.off('SIGINT', onInterrupt)
      process.off('SIGTERM', onTerminate)
    }
    process.on('SIGINT', onInterrupt)
    process.on('SIGTERM', onTerminate)
    child.once('error', error => {
      dispose()
      rejectBuild(error)
    })
    child.once('close', (code, signal) => {
      // Cargo can exit before a rustc descendant that ignored the first signal.
      if (interrupted && process.platform !== 'win32') kill('SIGKILL')
      dispose()
      resolveBuild(interrupted || code || (signal ? 1 : 0))
    })
  })
}

export async function executeTauri(args, { buildRuntime, runCli }) {
  if (needsRuntimeBuild(args)) {
    const code = await buildRuntime()
    if (code !== 0) return code
  }
  // Preserve every Tauri/runner/app argument and let the CLI own signals from here.
  await runCli(args)
  return 0
}

async function main() {
  process.exitCode = await executeTauri(process.argv.slice(2), {
    buildRuntime: () => {
      console.log('[dev] Building chatspeed-runtime before starting the Tauri dev-server timeout...')
      return runBuild('cargo', [
        'build', '--manifest-path', 'src-tauri/crates/daemon/Cargo.toml', '--bin', 'chatspeed-runtime'
      ])
    },
    runCli: async args => {
      const { run } = await import('@tauri-apps/cli')
      await run(args, 'pnpm tauri')
    }
  })
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(error => {
    console.error(error.message)
    process.exitCode = 1
  })
}
