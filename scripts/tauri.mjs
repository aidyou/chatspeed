import { execFile, spawn } from 'node:child_process'
import { copyFile, mkdir } from 'node:fs/promises'
import { join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { promisify } from 'node:util'

const projectRoot = fileURLToPath(new URL('../', import.meta.url))
const inspectCommand = promisify(execFile)

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

export async function buildDevBinaries(args, {
  build = runBuild, inspect = inspectCommand, cwd = projectRoot
} = {}) {
  const options = args.slice(0, args.indexOf('--') < 0 ? args.length : args.indexOf('--'))
  const targetIndex = options.indexOf('--target')
  const explicitTarget = targetIndex >= 0
    ? options[targetIndex + 1]
    : options.find(arg => arg.startsWith('--target='))?.slice('--target='.length)
  const { stdout: rustcInfo } = await inspect('rustc', ['-vV'], { cwd })
  const target = explicitTarget || /^host: (.+)$/m.exec(rustcInfo)?.[1].trim()
  if (!target) throw new Error('Cannot determine the Rust target for development sidecars')

  console.log('[dev] Building cscli and chatspeed-runtime before starting the Tauri dev-server timeout...')
  const buildArgs = [
    'build', '--manifest-path', 'src-tauri/Cargo.toml',
    '-p', 'chatspeed-cli', '-p', 'chatspeed-runtime',
    '--bin', 'cscli', '--bin', 'chatspeed-runtime'
  ]
  if (explicitTarget) buildArgs.push('--target', explicitTarget)
  const code = await build('cargo', buildArgs, { cwd })
  if (code !== 0) return code

  const { stdout: metadata } = await inspect('cargo', [
    'metadata', '--no-deps', '--format-version', '1', '--manifest-path', 'src-tauri/Cargo.toml'
  ], { cwd })
  const targetDir = JSON.parse(metadata).target_directory
  const debugDir = explicitTarget ? join(targetDir, target, 'debug') : join(targetDir, 'debug')
  const sidecarDir = join(cwd, 'src-tauri', 'target', 'release')
  const extension = target.includes('windows') ? '.exe' : ''
  // externalBin uses release paths even in dev; publish real debug binaries there.
  await mkdir(sidecarDir, { recursive: true })
  for (const name of ['cscli', 'chatspeed-runtime']) {
    await copyFile(join(debugDir, `${name}${extension}`), join(sidecarDir, `${name}-${target}${extension}`))
  }
  return 0
}

export async function executeTauri(args, { buildRuntime, runCli }) {
  if (needsRuntimeBuild(args)) {
    const code = await buildRuntime(args)
    if (code !== 0) return code
  }
  // Preserve every Tauri/runner/app argument and let the CLI own signals from here.
  await runCli(args)
  return 0
}

async function main() {
  process.exitCode = await executeTauri(process.argv.slice(2), {
    buildRuntime: buildDevBinaries,
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
