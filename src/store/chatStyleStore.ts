// The applier that attaches the shell setting `chat.style.*` to :root CSS variables — the authority of the value is the shell setting (ADR-0265), and here we only move the received value
//   to a variable name through a whitelist (`CSS_VAR_BY_KEY`) and setProperty it. StructuredTextView/ChatRow/chat.css read only
//   var() (the CSS variable layer of ADR-0051).
//
// ★No default is kept here★ — the canonical source is the shell schema table (`src-tauri/src/settings/registry.rs`), and the first
//   paint before values arrive is handled by the `--chat-*` fallback in `theme.css` (TRD S21-storage §10 F6 (a) — asynchronous).
// ★Nothing is saved★ — writes are only `settings_set`/`settings_reset`, and their result comes back as a notice and is painted here.
//   Do not revive localStorage persistence: the saved copy would diverge from the shell value and become two sources.
// ★There is no key-set cross-check (U2)★ — if the shell table's 11 keys and `CSS_VAR_BY_KEY` diverge, `settings.set chat.style.X`
//   answers `changed:true` but the screen stays the same (ADR-0265 「U2 의 대가」).
// ADR-0265

import {
  settingsClient,
  type ResetOutcome,
  type SetOutcome,
  type SettingsClient,
} from '../api/settingsClient'

/** Chat style keys (spacing + font set). Values are CSS length/number strings (e.g. '1rem', '13px', '1.55'). */
export type ChatStyleKey =
  | 'railRowPt' // rail row top-padding (row-spacing rhythm)
  | 'plainRowPt' // non-rail (user bubble · separator) row top-padding
  | 'userPy' // user bubble vertical padding
  | 'userPx' // user bubble horizontal padding (symmetric with userPy)
  | 'userMy' // user bubble vertical margin (separating turn chunks)
  | 'railGutter' // rail gutter width
  | 'railLineOffset' // connector line top offset (continues to the row above — coupled with railRowPt, usually negative)
  | 'railDotTop' // dot marker top (near the first line of content)
  | 'fontSize' // chat base font-size (in sync with chat.css)
  | 'lineHeight' // chat base line-height
  | 'waitStripH' // waiting-indicator row height above the input (user decision 2026-10-01)

/** Setting key = this prefix + `ChatStyleKey` (the last segment of the shell table is this spelling). */
const KEY_PREFIX = 'chat.style.'

// ADR-0051: StructuredTextView/theme.css/chat.css read these variables with var().
const CSS_VAR_BY_KEY: Record<ChatStyleKey, string> = {
  railRowPt: '--chat-rail-row-pt',
  plainRowPt: '--chat-plain-row-pt',
  userPy: '--chat-user-py',
  userPx: '--chat-user-px',
  userMy: '--chat-user-my',
  railGutter: '--chat-rail-gutter',
  railLineOffset: '--chat-rail-line-offset',
  railDotTop: '--chat-rail-dot-top',
  fontSize: '--chat-font-size',
  lineHeight: '--chat-line-height',
  waitStripH: '--chat-wait-strip-h',
}

/** `chat.style.<k>` → its CSS variable. `undefined` if outside the whitelist or not a chat style key. */
function cssVarOf(settingKey: string): string | undefined {
  if (!settingKey.startsWith(KEY_PREFIX)) return undefined
  const key = settingKey.slice(KEY_PREFIX.length)
  // FIX-2: decide by own key only — `key in CSS_VAR_BY_KEY` walks the prototype chain so keys like `chat.style.constructor` ·
  //   `chat.style.__proto__` pass.
  if (!Object.prototype.hasOwnProperty.call(CSS_VAR_BY_KEY, key)) return undefined
  return CSS_VAR_BY_KEY[key as ChatStyleKey]
}

/** Hook the settings subscription and paint `chat.style.*` onto :root. Returns = unsubscribe. If there is no document (SSR) nothing is painted. */
export function installChatStyleApplier(client: SettingsClient = settingsClient): () => void {
  return client.subscribe(changes => {
    const root = globalThis.document?.documentElement
    if (!root) return
    for (const { key, value } of changes) {
      const cssVar = cssVarOf(key)
      if (cssVar) root.style.setProperty(cssVar, value)
    }
  })
}

/** Write to the shell settings — the screen changes after the notice (or the answer) comes back. A refusal rejects with the shell error string. */
export function setChatStyle(
  key: ChatStyleKey,
  value: string,
  client: SettingsClient = settingsClient,
): Promise<SetOutcome> {
  return client.set(`${KEY_PREFIX}${key}`, value)
}

/** One key, or with `key` omitted all chat style back to defaults. */
export function resetChatStyle(
  key?: ChatStyleKey,
  client: SettingsClient = settingsClient,
): Promise<ResetOutcome> {
  return client.reset(key === undefined ? KEY_PREFIX : `${KEY_PREFIX}${key}`)
}
