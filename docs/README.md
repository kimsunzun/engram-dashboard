# Engram Dashboard — 문서 인덱스

Claude 에이전트들을 한 화면에서 관리하는 네이티브 대시보드.
Tauri v2 + React + xterm.js (프론트) / Rust + portable-pty (백엔드).

> **언제·무엇을·어떻게 만들어왔는지 → [`process/step-log.md`](process/step-log.md) (타임라인)**

## 문서 트리

```
docs/
├── README.md          이 파일 (인덱스)
├── tracking.md        보류 항목·결정 추적 (T-*, D-*) — 현재/미래
├── process/           과정 기록 (step별 응집, 불변)
│   ├── step-log.md        ★ 타임라인 — 언제 무엇을 어떻게 (S0~S8)
│   ├── S0-view-phase/     요구사항·view-spec·research (출발점)
│   ├── S1-design/         architecture·LLD·frontend-integration + 3자 검증 9건
│   ├── S2-phase0-spike/   PTY kill 실측
│   ├── S3-phase1-backend/ 백엔드 코어 브리핑 m1~m6b + stage2 가이드
│   ├── S4-channel-spike/  tauri 핀 실측
│   ├── S5-phase2-tauri/   commands+lib 브리핑
│   └── S7-phase3-frontend/ 프론트 통합 m8a~d
└── reference/         (추후) 완성 통합 정설 — 코드와 동기화되는 현재 동작 문서
```

**구조 원칙:**
- `process/` = "이렇게 만들어왔다" — step별 폴더로 응집. **폴더가 곧 step** (파일만 봐도 소속 자명).
- **새 작업 통합 규칙:** 새 step = `process/SN-name/` 폴더 새로 만들어 그 안에 다 넣고, `step-log.md`에 SN 항목 추가. 종류별로 흩어지지 않음.
- `reference/` = "지금 이렇게 동작한다" — 코드 동기화 정설 (살아있는 문서). **시스템 안정화 후 집필 예정.**
- 정교한 하위 구조(step 내 분류 등)는 나중에 한 번에 정리.

## 진행 상태

진행 상태·타임라인은 이 파일에서 중복 관리하지 않는다(rot 방지). 단일 출처:
- **언제/무엇 (타임라인):** [`process/step-log.md`](process/step-log.md) ★ 항상 최신
- **결정·거부한 대안 (왜):** [`decisions/`](decisions/README.md)
- **보류 항목 (T-*/D-*):** [`tracking.md`](tracking.md)
- `reference/` 정설 문서 = 시스템 안정화 후 집필 예정
