// The connection band's actions on the LLM surface (TRD S36 §3-6 · ADR-0350) — `connection.status`,
//   `connection.reconnect` and `daemon.stop` on `window.__engramCmd`.
//
// ★Main window only★: popouts and the spare window show the band text only.
// ★No `help`★ — they stay off the daemon bus: in every state with a band the shell has no daemon link
//   (ADR-0344 d7 · ADR-0345 d5), so a bus call could not reach them.

import type { Band } from '../../src-tauri/bindings/Band'
import type { DaemonShutdownReply } from '../../src-tauri/bindings/DaemonShutdownReply'
import type { AgentClient, ConnectionState } from '../api/agentClient'
import { agentClient, getDaemonControl } from '../api/clientFactory'
import { bandText, bandView } from '../api/connectionBand'
import { shutdownDaemon } from '../api/daemonControl'
import { t } from '../i18n'
import { MAIN_WINDOW_LABEL, readWindowLabelFromHash } from '../store/viewStore'
import { register } from './registry'

/** `text` = the sentence the band shows, or null when there is no band. */
export interface ConnectionStatusReply {
  state: ConnectionState
  band: Band | null
  text: string | null
}

export interface ConnectionCommandDeps {
  client: AgentClient
  /** `DaemonControl.start()` — the band's 「다시 연결」. */
  start: () => Promise<unknown>
  /** `shutdownDaemon` — the shell's one stop path. */
  shutdown: () => Promise<DaemonShutdownReply>
}

function statusReply(client: AgentClient): ConnectionStatusReply {
  const band = client.connectionBand
  const view = bandView(band, client.connectionError)
  return { state: client.connectionState, band, text: view === null ? null : bandText(view) }
}

export function registerConnectionCommands({ client, start, shutdown }: ConnectionCommandDeps): void {
  register({
    id: 'connection.status',
    title: t('connection.statusCmd'),
    category: 'connection',
    run: (): ConnectionStatusReply => statusReply(client),
  })

  register({
    id: 'connection.reconnect',
    title: t('connection.reconnectCmd'),
    category: 'connection',
    // A failure rethrows the start's error rather than replying a status: the band in a status read right after a
    //   failure can still be the one before it — the shell answers before it emits that answer's band.
    run: async (): Promise<ConnectionStatusReply> => {
      await start()
      return statusReply(client)
    },
  })

  register({
    id: 'daemon.stop',
    title: t('connection.stopCmd'),
    category: 'connection',
    // No confirm on the LLM path. The reply carries no status (TRD S21 trd-3-2-u5 U6) — read `connection.status`.
    run: (): Promise<DaemonShutdownReply> => shutdown(),
  })
}

if (readWindowLabelFromHash() === MAIN_WINDOW_LABEL) {
  registerConnectionCommands({
    client: agentClient,
    start: () => getDaemonControl().start(),
    shutdown: shutdownDaemon,
  })
}
