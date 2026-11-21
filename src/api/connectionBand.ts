// What a window's connection band says (TRD S36 §3-2) — the shell's band plus a start attempt's cause, read by the
//   band (`ConnectionNotice`) and by the LLM's `connection.status`, so both say the same words.
// ADR-0373: `bandView` describes the cause without translating it; the text is made by `causeText` / `bandText` where it is
//   shown, so a band stays in step with a language switch and its identity (the ✕ memory) does not depend on the language.

import type { Band } from '../../src-tauri/bindings/Band'
import { t } from '../i18n'

export type BandKind = Band['kind']

/** Why there is a band, untranslated: our own refusal, version, account or drive words, or the raw start error. */
export type BandCause =
  | { kind: 'refused' }
  | { kind: 'version' }
  | { kind: 'other_account' }
  | { kind: 'account_unverified' }
  | { kind: 'unprotected_volume' }
  | { kind: 'volume_unreadable' }
  | { kind: 'error'; message: string }

export interface BandView {
  kind: BandKind
  /** What follows the head, or null. */
  cause: BandCause | null
}

/**
 * The band to show, or null for none. The shell decides whether there is a band (ADR-0350); `connectionError` is only
 * band ①'s cause, and only when ① carries no `refused` cause — never under ②, the account band or the volume band. ②'s
 * `can_stop` is not read (ADR-0356). The account band (ADR-0395) = the daemon's listener runs under another OS account, or its
 * account could not be read; the app sent nothing. The volume band (ADR-0399) = the last start was refused before a spawn: the
 * data folder's drive does not keep file permissions (FAT / exFAT), or that could not be read.
 */
export function bandView(band: Band | null, connectionError: string | null): BandView | null {
  if (band === null) return null
  if (band.kind === 'version') return { kind: 'version', cause: { kind: 'version' } }
  if (band.kind === 'account') {
    return { kind: 'account', cause: { kind: band.unverified ? 'account_unverified' : 'other_account' } }
  }
  if (band.kind === 'volume') {
    return { kind: 'volume', cause: { kind: band.unreadable ? 'volume_unreadable' : 'unprotected_volume' } }
  }
  if (band.refused) return { kind: 'not_connected', cause: { kind: 'refused' } }
  return { kind: 'not_connected', cause: connectionError ? { kind: 'error', message: connectionError } : null }
}

/** The cause in the current UI language — a raw start error is shown as it came. */
export function causeText(cause: BandCause): string {
  switch (cause.kind) {
    case 'refused':
      return t('connection.refusedCause')
    case 'version':
      return t('connection.versionCause')
    case 'other_account':
      return t('connection.otherAccountCause')
    case 'account_unverified':
      return t('connection.accountUnverifiedCause')
    case 'unprotected_volume':
      return t('connection.unprotectedVolumeCause')
    case 'volume_unreadable':
      return t('connection.volumeUnreadableCause')
    case 'error':
      return cause.message
  }
}

/** The whole sentence the band shows — the head, then the cause when there is one. */
export function bandText(view: BandView): string {
  return view.cause === null ? t('connection.head') : `${t('connection.head')} ${causeText(view.cause)}`
}
