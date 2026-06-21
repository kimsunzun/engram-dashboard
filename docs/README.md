# Engram Dashboard — 문서 인덱스

Claude 에이전트들을 한 화면에서 관리하는 네이티브 대시보드.
Tauri v2 + React + xterm.js (프론트) / Rust + portable-pty (백엔드).

> **문서 작업 전 이 파일부터 본다.** 어떤 문서가 있고, 새 내용을 어디에 넣는지가 여기 있다.

## 문서 종류

| 폴더/파일 | 무엇 |
|---|---|
| `process/SN-name/` | "이렇게 만들어왔다" — **폴더가 곧 step**(이름만 봐도 자명). 실재 목록은 `ls docs/process/`, 흐름은 `step-log.md`. *step 목록을 여기에 손으로 베끼지 않는다 — 베끼면 어긋난다(rot).* |
| `process/step-log.md` | ★ 타임라인 — 언제 무엇을 어떻게. **step 흐름의 단일 출처.** |
| `decisions/` | "왜 이렇게 정했나" — 결정 + 거부한 대안(ADR). 영구 누적. |
| `tracking.md` | 보류 항목(T-)·결정 추적(D-). "재도입 시점"이 트리거. |
| `research/` | step 착수 전 선행조사(조사·비교·미결질문). |
| `reference/` | (추후) 코드 동기화 정설. 시스템 안정화 후 집필. |

## 새 내용을 어디에 넣나

- 설계 **결정**을 내렸다(+버린 대안) → `decisions/` ADR (작업 전 관련 ADR 먼저 읽기)
- 지금 안 하고 **나중에** 다룰 것·미결 질문 → `tracking.md` (T-/D-)
- **무엇을 언제** 했나 → `step-log.md`
- step 착수 전 **조사** → `research/`
- 새 기능 **설계 착수** → `process/SN-name/` 새 폴더 (관련 `research/`·ADR 먼저 참조)

**★ 고아 금지:** 새 문서를 만들면 위 경로·`tracking.md`·코드앵커(`// see …`) 중 하나에 반드시 링크한다. 안 하면 다음 세션이 못 찾는다.

## 진행 상태

이 파일에서 중복 관리하지 않는다(rot 방지). 단일 출처:
- **언제/무엇 (타임라인):** `process/step-log.md` ★ 항상 최신
- **왜 (결정·거부 대안):** `decisions/`
- **보류 (T-/D-):** `tracking.md`
