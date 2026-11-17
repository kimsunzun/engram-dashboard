// ADR-0373: date and number formatting follows the UI language. A formatter is built per language when first used, never
//   once at module load — one built at load would keep the boot language after a switch.

import { getLanguage, type Lang } from './index'

const LOCALE: Record<Lang, string> = { en: 'en-US', ko: 'ko-KR' }

/**
 * A getter for a value made from the current UI language's locale (typically an `Intl` formatter). `make` runs on the
 * first call in each language; later calls in that language return the same value.
 */
export function perLanguage<T>(make: (locale: string) => T): () => T {
  const made = new Map<Lang, T>()
  return () => {
    const lang = getLanguage()
    let value = made.get(lang)
    if (value === undefined) {
      value = make(LOCALE[lang])
      made.set(lang, value)
    }
    return value
  }
}
