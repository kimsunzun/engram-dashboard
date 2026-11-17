// ADR-0373: applies the shell setting `ui.language` to this window — `setLanguage`, then the per-window memory
//   (`langMemory.ts`). The authority of the value is the shell setting (ADR-0372); nothing is written back from here.
// `settingsClient` replays the applied values on subscribe and pulls after listening, so a boot language planted from a
//   stale value corrects itself.

import { settingsClient, type SettingsClient } from '../api/settingsClient'
import { isLang, setLanguage } from './index'
import { rememberLanguage } from './langMemory'

/** The shell setting key that selects the UI language (values `en` | `ko`). */
export const LANGUAGE_SETTING_KEY = 'ui.language'

/** Subscribes to `ui.language` and applies each value. Returns the unsubscribe function. */
export function installLanguageApplier(client: SettingsClient = settingsClient): () => void {
  return client.subscribe(changes => {
    for (const { key, value } of changes) {
      if (key !== LANGUAGE_SETTING_KEY) continue
      // The shell accepts only `en` | `ko` for this key; another value is a contract break, not a language to try.
      if (!isLang(value)) {
        console.warn(`[i18n] ignored ${LANGUAGE_SETTING_KEY} value ${JSON.stringify(value)}`)
        continue
      }
      setLanguage(value)
      rememberLanguage(value)
    }
  })
}
