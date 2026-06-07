# Step 3 — 폰트 시스템

## 할 일

### 1. `src/styles/font.css` 생성
```css
:root {
  --font-ui:           'JetBrains Mono', monospace;
  --font-terminal:     'Cascadia Code', monospace;
  --font-code:         'Fira Code', monospace;
  --font-claude-prose: 'Inter', sans-serif;
  --font-claude-code:  'JetBrains Mono', monospace;
  --font-claude-path:  'Cascadia Code', monospace;
  --font-claude-header:'Inter', sans-serif;
}
```

### 2. `src/index.css` 수정
- `@import './styles/font.css';` 추가 (theme.css 아래)

### 3. `src/App.tsx` 수정
- 현재 버튼 3개(dark/light/e-ink) 아래에 폰트 미리보기 섹션 추가
- 각 `--font-*` 변수 적용 예시 텍스트 표시:
  ```tsx
  <p style={{ fontFamily: 'var(--font-ui)' }}>UI: The quick brown fox</p>
  <p style={{ fontFamily: 'var(--font-terminal)' }}>Terminal: ls -la /home</p>
  <p style={{ fontFamily: 'var(--font-code)' }}>Code: const x = 42;</p>
  <p style={{ fontFamily: 'var(--font-claude-prose)' }}>Prose: Claude 응답 텍스트</p>
  <p style={{ fontFamily: 'var(--font-claude-code)' }}>Claude Code: `npm install`</p>
  <p style={{ fontFamily: 'var(--font-claude-path)' }}>Path: I:/Engram/core/</p>
  ```

## 완료 기준
- `npm run build` 에러 없음
- 완료 후 `orch 4 "⟁dc29 step3 완료"` 로 보고
