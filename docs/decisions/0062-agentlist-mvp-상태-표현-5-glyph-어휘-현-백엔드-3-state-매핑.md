# ADR-0062: AgentList MVP 상태 표현 = 5-glyph 어휘 / 현 백엔드 3-state 매핑

- 상태: 확정 (2026-07-10, 근거: pre-PRD 사용자 결정(5-glyph 어휘) + grounding으로 확인한 실제 status enum)
- 관련: CLAUDE.md §5 · ADR-0005(상태 알림 분담) · ADR-0002/0030(capability) · `crates/engram-dashboard-core/src/agent/*`(AgentStatus enum) · `src/components/agent/AgentList.tsx`(statusGlyph) · step-log "에이전트 트리·프리셋"

## 맥락

에이전트 트리(AgentList) MVP에서 각 줄의 상태를 **색이 아닌 "모양"(글리프)으로** 표시하기로 사용자가 결정했다(e-ink 대비 — 흑백에서도 상태가 구분돼야 함). 결정된 어휘는 5-state:

- `●` 작업중 · `◐` 입력대기 · `○` 유휴 · `◻` 멈춤 · `✗` 에러

그런데 grounding으로 실제 백엔드 상태 enum을 확인하니 `AgentStatus = Running | Exiting | Exited{code} | Failed{message} | Killed` 뿐이다 — **"작업중 vs 입력대기 vs 유휴"를 구분하는 신호가 없다**(모두 `Running`으로 뭉쳐 있고, PTY 출력 활동·입력 대기 여부를 추적하지 않는다). 즉 사용자가 결정한 5-glyph 어휘를 실제 상태에 1:1로 매핑할 수 없다.

## 결정

**5-glyph 어휘는 그대로 pure 매핑 함수(`statusGlyph`)로 구현하되, 현 백엔드가 구분 가능한 3개만 실제로 점등한다.**

현 매핑:
- `Running` → `●` (작업중)
- `Exited` / `Killed` → `◻` (멈춤)
- `Failed` → `✗` (에러)
- `Exiting` → `◻` (멈춤으로 전이 중 — terminal 직전)

`◐`(입력대기)·`○`(유휴)는 **어휘로만 정의해 두고 현재는 미점등**한다 — 백엔드가 나중에 "출력 활동/입력 대기" 신호를 내보내면 그때 매핑을 채운다. `statusGlyph`는 외부 의존 없는 pure 함수라 headless 단위테스트로 전 분기를 고정한다.

프론트가 terminal 판정을 `status_changed`가 아니라 `agent-list-updated` 목록으로 한다는 기존 불변식(ADR-0005)은 그대로 — AgentList는 목록 갱신으로 상태를 받는다.

## 거부한 대안

- **지금 출력-활동 추적을 추가해 작업중/유휴/입력대기를 구분** — `Running`을 쪼개려면 OutputRouter/코어에 마지막-출력-시각·입력-대기 감지를 심어야 한다. 이는 **실측 안 된 백엔드 내부**(무엇이 "입력 대기"인지 백엔드별로 다르고, TUI/JSON 모드별 판정이 제각각)라, CLAUDE.md §0 판단기준상 "고비용·불확실 → 껍데기/정의만 두고 실측 때 채운다"에 해당. MVP에서 조기 도입하면 잘못된 휴리스틱을 박제할 위험. → 어휘만 깔고 미점등.
- **어휘를 3개로 줄여 실제 enum에 맞춤**(●◻✗만) — 사용자가 5-glyph를 결정했고, 어휘를 미리 5개로 깔아 두면 백엔드 신호가 생길 때 매핑만 채우면 된다(저위험·장기 = over-engineering 허용, CLAUDE.md §0). 3개로 줄이면 나중에 어휘를 다시 늘리는 재작업 + 사용자 결정 번복.
- **색으로 상태 표시**(기존 `AgentTree.tsx:33` statusColor 방식) — e-ink(흑백)에서 색이 소실돼 상태 구분 불가. 사용자가 명시적으로 "색 아닌 모양"을 결정. 신규 UI는 변수-only + 글리프 모양이 상태를 담당.

## 근거

- **사용자 결정 = 5-glyph 어휘**(step-log, pre-PRD 컨설). 어휘는 사용자 소관, 매핑 가능 범위는 백엔드 현실.
- **grounding 사실:** 실제 `AgentStatus`에 작업중/입력대기/유휴 구분 없음(Running 단일). 해석으로 메우지 않고 "3개만 점등 + 2개 미점등 어휘"로 정직하게 구현.
- **pure 함수 격리(ADR-0012):** `statusGlyph(status) → glyph`는 외부 의존 0 → headless 단위테스트로 전 분기 고정, 백엔드 신호 확장 시 이 함수만 수정.

## 영향 / 불변식

- **`statusGlyph`는 pure 함수 + 전 분기 단위테스트** — 매핑 변경은 이 함수 + 테스트만 건드린다.
- **◐·○ 미점등은 결함이 아니라 의도** — 다음 세션이 "왜 입력대기/유휴가 안 뜨나"로 이 매핑을 재론하지 말 것. 백엔드가 활동/입력-대기 신호를 낼 때 매핑을 채우는 것이 정규 경로(그 신호 추가 = 별도 결정).
- **색 리터럴 금지(변수-only)** — 상태는 글리프 모양이 담당하고 색에 의존하지 않는다(e-ink 대비). 기존 `AgentTree` statusColor는 AgentList 전환 시 모양 기반으로 대체.
