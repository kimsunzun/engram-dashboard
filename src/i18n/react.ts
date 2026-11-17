// ADR-0373: the React binding of the UI language, kept apart from `index.ts` so that importing `t` does not pull React in.

import { useSyncExternalStore } from 'react'

import { getLanguage, subscribeLanguage, type Lang } from './index'

/** The current UI language; the calling component re-renders when it changes. */
export function useLanguage(): Lang {
  return useSyncExternalStore(subscribeLanguage, getLanguage)
}
