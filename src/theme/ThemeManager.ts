import { type ThemeName, useThemeStore } from '../store/themeStore'
import { rememberTheme } from './themeMemory'

class ThemeManager {
  private static instance: ThemeManager

  static getInstance(): ThemeManager {
    if (!ThemeManager.instance) {
      ThemeManager.instance = new ThemeManager()
    }
    return ThemeManager.instance
  }

  apply(theme: ThemeName): void {
    useThemeStore.getState().setTheme(theme)
    rememberTheme(theme)
  }
}

export const themeManager = ThemeManager.getInstance()
