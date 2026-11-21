// scripts/engram-account.mjs — the listener account check of `scripts/engram.mjs` (ADR-0395 decision 6): before the script sends this
//   account's token, the process listening at the recorded loopback port must run under this Windows account; anything else — another
//   account, an owner that cannot be read (another account's process denies it), no listening socket, a failed tool — refuses and
//   nothing is sent.
// ★Mechanism without a native module★ — one PowerShell run (by its System32 path, `-NoProfile -NonInteractive`) that prints one JSON
//   line, so no localized text or console code page is parsed:
//   - `Get-NetTCPConnection -LocalAddress 127.0.0.1 -LocalPort <port> -State Listen` → the owning PID and the listening socket's
//     creation time;
//   - `Get-CimInstance Win32_Process` → `GetOwnerSid()` → the owner's SID, compared with this process's SID;
//   - `Get-Process -Id <pid>` → the process's start time.
//   It passes only when the SIDs are equal AND the process started no later than the socket was created — the same creation-time rule
//   as the app's lookup (platform `account::tcp_server_account` · `created_in_time`, no tolerance): the TCP table keeps the creating
//   process's PID after it exits while another process holds the socket, so a later process of this account that took that PID number
//   must not vouch for the socket. Times travel as FILETIME strings (beyond JavaScript's exact integers) and compare as BigInt.
// ★Compared in whole seconds (measured 2026-11-21)★: `Get-NetTCPConnection`'s `CreationTime` is truncated to the second (`…:29.0000000`
//   for a socket made at `…:29.57`), while the process start time is exact. Compared exactly, a daemon that listens within the second
//   it started — every daemon: it binds early in its start-up — would read as a reused PID. So both are cut to the second; what this
//   leaves open is a reused PID whose process started within the same wall-clock second the socket was created. Another account
//   chooses when its sockets are created and can repeat that, so the remaining barrier is PID reuse within one second, not luck.
//   Accepted for this dev CLI because a token leaked that way is still useless: the daemon's own accept-time check (exact table
//   timestamps, `platform::account`) refuses that account's connections. The app's lookup has no such window.
// Pure decision is split from the tool run so `src/util/engramAccount.test.ts` can drive it with recorded outputs.
import path from 'node:path'
import { execFileSync } from 'node:child_process'

export const OTHER_ACCOUNT =
  'this daemon belongs to another Windows account — use a separate folder per account'

/** The PowerShell program for `port` — prints exactly one JSON line (UTF-8). */
export function probeScript(port) {
  if (!Number.isInteger(port) || port <= 0 || port > 65535) throw new Error(`not a port: ${port}`)
  return [
    "$ErrorActionPreference = 'Stop'",
    '[Console]::OutputEncoding = [Text.Encoding]::UTF8',
    '$out = [ordered]@{}',
    'try {',
    '  $out.me = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value',
    `  $rows = @(Get-NetTCPConnection -LocalAddress 127.0.0.1 -LocalPort ${port} -State Listen -ErrorAction SilentlyContinue)`,
    '  $out.listeners = $rows.Count',
    '  if ($rows.Count -ge 1) {',
    '    $owner = [int]$rows[0].OwningProcess',
    '    $out.pid = $owner',
    '    $out.pids = @($rows | ForEach-Object { [int]$_.OwningProcess } | Sort-Object -Unique)',
    '    $out.socketCreated = [string]$rows[0].CreationTime.ToFileTimeUtc()',
    '    $proc = Get-CimInstance Win32_Process -Filter "ProcessId=$owner"',
    '    if ($proc) { $out.sid = (Invoke-CimMethod -InputObject $proc -MethodName GetOwnerSid).Sid }',
    '    try { $out.processStarted = [string](Get-Process -Id $owner).StartTime.ToFileTimeUtc() } catch { $out.startError = $_.Exception.Message }',
    '  }',
    '} catch { $out.error = $_.Exception.Message }',
    '$out | ConvertTo-Json -Compress',
  ].join('\n')
}

/** The decision on the probe's JSON line for `port`: null = pass, else why it refuses. */
export function decide(output, port) {
  const line = String(output)
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l.startsWith('{'))
    .pop()
  let r
  try {
    r = JSON.parse(line ?? '')
  } catch {
    return `the listener check printed no result (${JSON.stringify(String(output).slice(0, 200))})`
  }
  if (r.error) return `the listener's account could not be read (${r.error})`
  if (!r.listeners) return `no process listens on 127.0.0.1:${port}`
  if (!Array.isArray(r.pids) || r.pids.length !== 1) return `more than one process listens on 127.0.0.1:${port}`
  if (typeof r.me !== 'string' || !r.me.startsWith('S-')) return "this account's SID could not be read"
  if (typeof r.sid !== 'string' || !r.sid.startsWith('S-')) {
    return `the owner of pid ${r.pid} could not be read (another account's process)`
  }
  if (r.sid !== r.me) return `pid ${r.pid} listening on 127.0.0.1:${port} runs under ${r.sid}, not ${r.me}`
  let started, created
  try {
    started = BigInt(r.processStarted)
    created = BigInt(r.socketCreated)
  } catch {
    return `pid ${r.pid}'s start time or its socket's creation time could not be read${r.startError ? ` (${r.startError})` : ''}`
  }
  const SECOND = 10_000_000n // FILETIME units (100 ns)
  if (created <= 0n || started / SECOND > created / SECOND) {
    return `pid ${r.pid} started after its listening socket was created — the PID was reused, so it does not prove the socket's owner`
  }
  return null
}

/**
 * Checks that the process listening at `127.0.0.1:<port>` runs under this account. Returns null when it does, else the reason it does
 * not (the caller refuses with [`OTHER_ACCOUNT`] and sends nothing). `run(script)` returns PowerShell's stdout — the seam the tests fake.
 */
export function listenerAccountProblem(port, run = runPowerShell) {
  let output
  try {
    output = run(probeScript(port))
  } catch (e) {
    return `the listener's account could not be read (${e.message})`
  }
  return decide(output, port)
}

function runPowerShell(script) {
  const exe = path.join(process.env.SystemRoot || 'C:\\Windows', 'System32', 'WindowsPowerShell', 'v1.0', 'powershell.exe')
  return execFileSync(exe, ['-NoProfile', '-NonInteractive', '-Command', script], {
    encoding: 'utf8',
    windowsHide: true,
  })
}
