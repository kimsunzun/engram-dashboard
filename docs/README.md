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

## 진행 상태 (2026-06-11)

| 단계 | 상태 | 커밋 |
|------|------|------|
| S2 Phase 0 — Spike (PTY kill 실측) | ✅ | — |
| S3 Phase 1 — 백엔드 PTY 코어 | ✅ | 575e36d |
| S4 Channel spike — tauri 핀 | ✅ | — |
| S5 Phase 2 — Tauri 연결 | ✅ | f959304 |
| S6 백엔드 마감 — 로그 마스킹 + 병렬 kill | ✅ | 26dc649 |
| S7 Phase 3 — 프론트 통합 3a~3c (E2E claude 기동) | ✅ | ca61cbd |
| Phase 3d — popup + monaco | ⏸ 보류 | — |
| **세션 저장/복원** (핵심 기능) | 📐 설계 예정 | — |

검증: dco23(Opus)/dcs24(Sonnet) 코딩 → dr26(Fable) LLD 리뷰 → dq25(Sonnet) QA 3-게이트.

## 보류·결정 ([tracking.md](tracking.md) 상세)

- **T-5** monaco optimizeDeps(3d) / **T-7** snapshot wire / **D-5** frontend LLD 경로 / **D-6** tauri 표기·마스킹 규칙
- ✅ 해소: T-1(로그 마스킹) · T-3(tauri 핀) · T-8(병렬 kill) · T-6(cwd — claude 권한 중복으로 스킵)

## 다음 작업

1. **세션 저장/복원 설계** — 떠있던 에이전트(명령/cwd/레이아웃) persist + claude `--continue` 대화 복원
2. spawn 설정화 (에이전트 프로필) — 세션 복원과 연결
3. 프론트 마무리 (3d + 상세설계)
4. `reference/` 정설 문서 집필 (안정화 후)
