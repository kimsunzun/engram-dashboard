# Step 2 — 테마 시스템

## 할 일

### 1. `src/styles/theme.css` 생성
```css
:root[data-theme='dark']  { --bg: #0a0a0a; --bg-secondary: #111; --text: #e0e0e0; --text-muted: #888; --border: #333; --accent: #4a9eff; }
:root[data-theme='light'] { --bg: #f5f5f5; --bg-secondary: #fff; --text: #1a1a1a; --text-muted: #666; --border: #ccc; --accent: #0066cc; }
:root[data-theme='e-ink']  { --bg: #ffffff; --bg-secondary: #f0f0f0; --text: #000000; --text-muted: #444; --border: #000; --accent: #000; }
```

### 2. `src/store/themeStore.ts` 생성 (Zustand)
- state: `theme: 'dark' | 'light' | 'e-ink'`
- action: `setTheme` → `document.documentElement.setAttribute('data-theme', theme)` 호출

### 3. `src/theme/ThemeManager.ts` 생성
- 싱글턴, 테마 변경 단일 진입점
- `apply(theme)` 메서드 → themeStore.setTheme 호출

### 4. `src/App.tsx` 수정
- 앱 시작 시 `ThemeManager.apply('dark')` 기본 적용
- 화면에 버튼 3개 [dark] [light] [e-ink] 표시
- 클릭 시 테마 즉시 전환 (배경/텍스트 색상 변경 확인용)
- CSS 변수 사용: `background: var(--bg)`, `color: var(--text)`

### 5. `src/index.css` 수정
- 상단에 `@import './styles/theme.css';` 추가

## 완료 기준
- `npm run build` 에러 없음
- 완료 후 보고
