// ADR-0395: the listener account check of `scripts/engram.mjs` (`scripts/engram-account.mjs`) — the script sends this account's token
//   only to a loopback listener whose process runs under this account's SID and started no later than its listening socket was made;
//   everything else refuses. Driven with recorded PowerShell outputs (one JSON line) through the `run` seam, so it runs anywhere.
// Placement exception = the same as `devConfigOverlay.test.ts` (the target is in `scripts/`, and vitest only collects `src/**`).
import { describe, expect, it } from 'vitest'

declare function require(id: string): {
  join(...parts: string[]): string
  createRequire(filename: string): (id: string) => unknown
}
declare const process: { cwd(): string }

const { join } = require('node:path')
const { createRequire } = require('node:module')

// The plain Node loader, not `import` (vite would transform the script — see `devConfigOverlay.test.ts`).
const nodeRequire = createRequire(join(process.cwd(), 'package.json'))
const { decide, probeScript, listenerAccountProblem } = nodeRequire(
  join(process.cwd(), 'scripts', 'engram-account.mjs'),
) as {
  decide: (output: string, port: number) => string | null
  probeScript: (port: number) => string
  listenerAccountProblem: (port: number, run: (script: string) => string) => string | null
}

const ME = 'S-1-5-21-111-222-333-1001'
const OTHER = 'S-1-5-21-111-222-333-1002'
// FILETIME (100 ns) values; one second = 10 000 000.
const SOCKET_AT = 134396720690000000n // `Get-NetTCPConnection` truncates to the second
const SAME_SECOND_LATER = SOCKET_AT + 5_759_808n
const NEXT_SECOND = SOCKET_AT + 10_000_000n

function probe(fields: Record<string, unknown>): string {
  return (
    JSON.stringify({
      me: ME,
      listeners: 1,
      pid: 4242,
      pids: [4242],
      socketCreated: String(SOCKET_AT),
      sid: ME,
      processStarted: String(SOCKET_AT - 30_000_000n),
      ...fields,
    }) + '\r\n'
  )
}

describe('engram.mjs listener account check', () => {
  it('passes a listener of this account that started before its socket', () => {
    expect(decide(probe({}), 5123)).toBeNull()
  })

  it('passes a listener that started within the second its socket was made (the socket time is whole seconds)', () => {
    expect(decide(probe({ processStarted: String(SAME_SECOND_LATER) }), 5123)).toBeNull()
  })

  it("refuses a reused PID: this account's process started after the socket was made", () => {
    expect(decide(probe({ processStarted: String(NEXT_SECOND) }), 5123)).toContain('PID was reused')
  })

  it('refuses another account, and an owner that cannot be read', () => {
    expect(decide(probe({ sid: OTHER }), 5123)).toContain(OTHER)
    expect(decide(probe({ sid: null }), 5123)).toContain('could not be read')
  })

  it('refuses when the start time cannot be read (another account denies it)', () => {
    expect(decide(probe({ processStarted: undefined, startError: 'Zugriff verweigert' }), 5123)).toContain(
      'Zugriff verweigert',
    )
  })

  it('compares SIDs, never names — a non-ASCII account passes on its SID and a localized error text still refuses', () => {
    // The probe line is UTF-8; a localized message (here German with an umlaut) only rides along in the refusal text.
    expect(decide(probe({ owner: 'DÜSSELDORF\\Müller' }), 5123)).toBeNull()
    expect(decide(JSON.stringify({ error: 'Zugriff für Benutzer verweigert' }), 5123)).toContain(
      'Zugriff für Benutzer verweigert',
    )
  })

  it('refuses no listener, more than one, an unreadable own SID, and output that is not one JSON line', () => {
    expect(decide(JSON.stringify({ me: ME, listeners: 0 }), 6000)).toContain('no process listens')
    expect(decide(probe({ pids: [4242, 5151] }), 5123)).toContain('more than one')
    expect(decide(probe({ me: null }), 5123)).toContain("this account's SID")
    expect(decide('Get-NetTCPConnection : not recognized', 5123)).toContain('no result')
  })

  it('runs one probe for the asked port and fails closed when PowerShell cannot run', () => {
    let seen = ''
    expect(
      listenerAccountProblem(5123, (script) => {
        seen = script
        return probe({})
      }),
    ).toBeNull()
    expect(seen).toContain('-LocalPort 5123')
    expect(() => probeScript(0)).toThrow()
    expect(
      listenerAccountProblem(5123, () => {
        throw new Error('powershell missing')
      }),
    ).toContain('powershell missing')
  })
})
