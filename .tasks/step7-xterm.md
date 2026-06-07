# Step 7 — xterm.js 더미 출력

## 패키지 설치
```bash
npm install @xterm/xterm @xterm/addon-fit react-xtermjs
```

## 할 일

### 1. `src/components/slot/TerminalSlot.tsx` 생성
- `react-xtermjs` 의 `XTerm` 컴포넌트 사용
- FitAddon 연결 → 컨테이너 리사이즈 시 `fit()` 호출
- 마운트 시 더미 텍스트 write:
  ```ts
  terminal.write("Claude 비서 에이전트 v1.0\r\n")
  terminal.write("\x1b[32m✓\x1b[0m 작업 완료: requirements.md 분석\r\n")
  terminal.write("\x1b[33m⚠\x1b[0m 파일 경로: I:/Engram/core/dashboard/\r\n")
  terminal.write("\x1b[31m✗\x1b[0m 오류: 파일을 찾을 수 없습니다\r\n")
  terminal.write("\x1b[90m>\x1b[0m 대기 중...\r\n")
  ```
- xterm 테마: `{ background: 'var(--bg)', foreground: 'var(--text)', cursor: 'var(--accent)' }` — CSS 변수 값은 `getComputedStyle(document.documentElement).getPropertyValue(...)` 로 읽어서 적용
- fontFamily: `getComputedStyle(document.documentElement).getPropertyValue('--font-terminal').trim()`

### 2. `src/components/layout/AppLayout.tsx` 수정
- SlotPane 내부를 `<TerminalSlot />` 으로 교체
- allotment 리사이즈 이벤트 → FitAddon.fit() 트리거

## 완료 기준
- `npm run build` 에러 없음
- 완료 후 `orch 4 "⟁dc29 step7 완료"` 로 보고
