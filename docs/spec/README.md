# Engram Dashboard — 설계 문서

Engram 에이전트 관리 데스크톱 앱의 설계·결정 기록.
실제 앱 코드는 `apps/engram-dashboard/` 에 있다.

## 문서 목록

- [architecture.md](architecture.md) — 기술 스택, 컴포넌트 구조
- [requirements.md](requirements.md) — 기능 요구사항

## 핵심 결정

- **스택**: Tauri + React + xterm.js + Monaco DiffEditor
- **코드 위치**: `I:\Engram\apps\engram-dashboard\`
- WezTerm 임베드 불가 → xterm.js로 직접 터미널 렌더링
- 기존 CCB(Claude Code Bridge) 포크 아닌 처음부터 제작
