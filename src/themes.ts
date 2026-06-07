export type ThemeName = 'dark' | 'light' | 'e-ink'

export interface Theme {
  bg: string
  fg: string
  surface: string
  border: string
}

export const themes: Record<ThemeName, Theme> = {
  dark: {
    bg: '#0a0a0a',
    fg: '#e0e0e0',
    surface: '#1a1a1a',
    border: '#333333',
  },
  light: {
    bg: '#f5f5f5',
    fg: '#1a1a1a',
    surface: '#ffffff',
    border: '#dddddd',
  },
  'e-ink': {
    bg: '#f0ede4',
    fg: '#1a1814',
    surface: '#e8e4d8',
    border: '#bbbbbb',
  },
}
