import { create } from 'zustand'

export type ThemeName = 'dark' | 'light' | 'e-ink'

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
