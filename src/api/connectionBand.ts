// What a window's connection band says (TRD S36 §3-2) — the shell's band plus a start attempt's cause, read by the
//   band (`ConnectionNotice`) and by the LLM's `connection.status`, so both say the same words.

import type { Band } from '../../src-tauri/bindings/Band'
import { t } from '../i18n'

export type BandKind = Band['kind']

export interface BandView {
  kind: BandKind
  /** The text after the head, or null. */
  cause: string | null
}

/**
 * The band to show, or null for none. The shell decides whether there is a band (ADR-0350); `connectionError` is only
 * band ①'s cause, and only when ① carries no `refused` cause — never under ②. ②'s `can_stop` is not read (ADR-0356).
 */
export function bandView(band: Band | null, connectionError: string | null): BandView | null {
  if (band === null) return null
  if (band.kind === 'version') return { kind: 'version', cause: t('connection.versionCause') }
  const cause = band.refused ? t('connection.refusedCause') : connectionError || null
  return { kind: 'not_connected', cause }
}

/** The whole sentence the band shows — the head, then the cause when there is one. */
export function bandText(view: BandView): string {
  return view.cause === null ? t('connection.head') : `${t('connection.head')} ${view.cause}`
}
