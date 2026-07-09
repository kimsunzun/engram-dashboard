# 핸드오프: 메시징 data-plane PRD 초안 완료 → /review prd 직전 체크포인트 (dashboard1)

**한 줄 상태:** db2 작업배정(메시징 data-plane 설계, design-only) — **PRD 초안 작성·커밋(`d9ed173`)·push 완료.** (b) 선택대로 **`/review prd` 직전에서 끊음**(컨텍스트예산 ~42%). ADR-0014 + 적대검증 + 제출 = 다음 세션.

**다음 첫 액션:** `/review prd` (PRD 초안 적대검증) → 오너 옵션 선택 → `/adr new`로 **ADR-0014**(오케스트레이션 후보) 생성 → **db2(pane11)에 제출 회신**.

> ⚠️ **멀티스트림:** 이 `latest.md`는 내(db1) 것으로 덮음. **db2 핸드오프(S14 T4 미완)는 `history/20260628-162651-T4-재연결-미완-FIX2테스트-깨짐.md`에 보존** — db2는 그 파일로 로드.

## repo 상태 (★ 이번 세션 git 정리 완료)
- **머지 완료:** a1→master(`6e1d307`) push — 내 5커밋(RichSlot lab·TerminalSlot fix·리서치2·draft2)이 main에 있음. wip/a1은 FF로 master 따라잡아 **db2 S14(T4 재연결·protocol_state·ADR-0038·debugging-conventions) 받음.**
- **현재:** wip/a1 = master +1 (PRD `d9ed173`). origin/wip/a1 동기 push됨. **미커밋 0.**
- 작업 = `engram-dashboard-a1`(wip/a1). continue 파일은 main 공유 트리.

## 완료·검증
- **PRD 초안:** `docs/research/messaging-data-plane-prd-draft-2026-06-28.md` — **D1=전달클래스(ephemeral/durable/state_latest/command_rpc) 표면화 핵심결정** + C1~C7 옵션셋(control/data 분리·supervised actor small·agentic봉투·peer금지/오케스트레이터·A2A외부·전송로 seam·영속모델) 각 거부대안 1줄.
- 입력 digest: `agent-messaging-survey-2026-06-28.md`(양 family 강수렴: tokio now/NATS future·actor supervision·에이전틱 레이어 분리) + `llm-control-surface-message-command-scope-2026-06-28.md`.

## 검증 안 됨 (="된 것"으로 적지 말 것)
- **PRD `/review prd` 미통과**(draft). 적대검증 안 함.
- **ADR-0014 미작성** — a1 `docs/decisions/` glob이 비게 나옴(원인 미확인) → 작성 전 인덱스·디렉토리 실재 확인. `/adr new`가 채번·템플릿·인덱스 처리.
- **응답인식**(PTY stdout→reply 판정) = 본 PRD 비목표, 별 seam.

## 작업 제약 (db2 배정 — 재확인)
docs-only · **protocol crate/src-tauri/ViewManager 코드 무접촉(메인 소유)** · wip/a1 유지 · `/review prd` 후 제출 · **결정권 오너, 산출은 옵션셋까지** · 제출 시 **pane11(db2) 회신**.

## do-not
- protocol/src-tauri/ViewManager 편집 금지(db2 S14 도메인).
- git 머지 블로커를 *stale 상태로 과하게 미루지 말 것* — 이번 세션 dirty-master 블로커를 오래 붙들었다가 상태 재확인하니 이미 해소돼 있었음. **추측 말고 상태 먼저 확인 후 행동.**

## 참조 (읽을 것만)
- PRD 초안 + 입력 survey 2 + scope (전부 `docs/research/`).
- ADR-0014 후보: Erlang OTP/Ractor · Temporal/Restate · LangGraph/MS Agent Framework · A2A. 거부: Redis/Rabbit/MQTT/Actix/ZeroMQ/A2A내부. (근거 = survey §거부후보.)
- ADR-0020(전송의미론)·0028(이벤트버스 single-push)·0038(OSS-조사 규약).

## 커뮤니케이션 메모
결론 먼저·짧게·평이, 리스트 남발 금지(org). 사용자=메시지 시스템 경험 많음 → peer, 기초강의 X.
