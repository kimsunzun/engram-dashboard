import { create } from 'zustand'

export type ThemeName = 'dark' | 'light' | 'e-ink'

const THEME_NAMES: readonly ThemeName[] = ['dark', 'light', 'e-ink']

export function isThemeName(raw: unknown): raw is ThemeName {
  return typeof raw === 'string' && (THEME_NAMES as readonly string[]).includes(raw)
}

interface ThemeState {
  theme: ThemeName
  setTheme: (name: ThemeName) => void
}

export const useThemeStore = create<ThemeState>((set) => ({
  theme: 'dark',
  setTheme: (name) => {
    document.documentElement.setAttribute('data-theme', name)
    set({ theme: name })
  },
}))
