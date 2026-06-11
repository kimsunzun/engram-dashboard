# Engram Dashboard — 문서 인덱스

Claude 에이전트들을 한 화면에서 관리하는 네이티브 대시보드.
Tauri v2 + React + xterm.js (프론트) / Rust + portable-pty (백엔드).

## 문서 트리

```
docs/
├── README.md          ← 이 파일 (전체 인덱스)
├── spec/              요구사항·출발점
│   ├── requirements.md            제품 요구사항
│   ├── research.md                기술 조사
│   ├── view-spec.md               View(UI) 설계 스펙
│   ├── view-spec-gpt-review.md    View 스펙 GPT 리뷰
│   └── README.md                  (초기 dashboard 개요)
├── design/            확정 설계 (구현 기준)
│   ├── backend-architecture.md        백엔드 아키텍처
│   ├── backend-lld-stage1.md          백엔드 상세설계 (LLD) — 구현 계약서
│   └── frontend-integration-lld.md    프론트 통합 설계 (invoke/Channel/event)
├── reviews/           3자 검증 기록 (fable/Gemini/GPT)
│   ├── backend-architecture-{gemini,gpt}-review.md
│   ├── lld-review-{fable,gemini,gpt}.md + lld-request-guide-fable.md
│   ├── frontend-review-{fable,gemini,gpt}.md
│   └── backend-review-feedback.md
├── briefings/         구현 지시서 (코더 에이전트용)
│   ├── phase0-spike.md, channel-spike.md   사전 실측 스파이크
│   ├── m1~m7 (백엔드), m8a~m8d (프론트)    모듈별 브리핑
│   └── backend-stage2-briefing.md          백엔드 구현 진입 가이드
├── history/           초안·폐기본 (참고용)
│   └── backend-architecture-draft.md
└── tracking.md        보류 항목·결정 추적 (T-*, D-*)
```

## 진행 상태 (2026-06-11)

| 단계 | 상태 | 커밋 |
|------|------|------|
| Phase 1 — 백엔드 PTY 코어 (types/session/drain/windows/manager/logging) | ✅ | 575e36d |
| Phase 2 — Tauri 연결 (commands/lib.rs) | ✅ | f959304 |
| 백엔드 마감 — 로그 마스킹 + 병렬 kill | ✅ | 26dc649 |
| Phase 3 — 프론트 통합 3a~3c (API/eventBus/TerminalSlot, E2E claude 기동) | ✅ | ca61cbd |
| Phase 3d — popup + monaco | ⏸ 보류 | — |
| **세션 저장/복원** (핵심 기능, LLD에 "추후 결정"이던 것) | 📐 설계 예정 | — |

검증 방식: **dco23(Opus)/dcs24(Sonnet) 코딩 → dr26(Fable) LLD 리뷰 → dq25(Sonnet) QA** 3-게이트.

## 보류·결정 (tracking.md 상세)

- **T-5** monaco optimizeDeps (3d) / **T-6** cwd 검증 — claude 권한과 중복이라 스킵
- **T-7** snapshot wire 포맷 / **D-5** frontend LLD 경로 동기화 / **D-6** tauri 버전 표기 갱신
- **T-1**(로그 마스킹)·**T-8**(병렬 kill)·**T-3**(tauri 핀) — ✅ 해소

## 다음 작업

1. **세션 저장/복원 설계** — 떠있던 에이전트(명령/cwd/레이아웃) persist + claude `--continue` 대화 복원
2. spawn 설정화 (에이전트 프로필) — 세션 복원과 연결
3. 프론트 마무리 (3d + 상세설계)
