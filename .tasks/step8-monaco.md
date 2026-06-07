# Step 8 — Monaco DiffEditor

## 패키지 설치
```bash
npm install @monaco-editor/react
```

## 할 일

### 1. `src/components/diff/DiffPanel.tsx` 생성
- `@monaco-editor/react` 의 `DiffEditor` 사용
- 로컬 번들: `loader.config({ monaco })` (CDN 차단 대응)
- 더미 diff:
  ```ts
  const original = `function hello() {\n  console.log("hello")\n}`
  const modified = `function hello(name: string) {\n  console.log(\`hello \${name}\`)\n}`
  ```
- 언어: `typescript`
- 테마: `vs-dark` (dark 테마 시) — CSS 변수 연동은 Step 이후로 미룸
- Accept / Revert 버튼 (더미, 클릭 시 console.log만)
- 높이: 300px 고정

### 2. `src/components/layout/AppLayout.tsx` 수정
- StatusBar 위에 DiffPanel 토글 추가
- 토글 버튼: "Diff ▲" / "Diff ▼"
- 기본 상태: 숨김

## 완료 기준
- `npm run build` 에러 없음
- 완료 후 `orch 4 "⟁dc29 step8 완료"` 로 보고
