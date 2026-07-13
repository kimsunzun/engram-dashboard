# ADR-0078: 렌더 모드는 에이전트 생성 시 결정·고정 (per-activation 활성화 오버라이드 폐기)

- 상태: 확정 (2026-07-13, 근거: 사용자 결정 + 라이브 GUI 실측 + /review code full·/qa PASS)
- 관련: CLAUDE.md §5(LLM 제어) · ADR-0044(output_format 렌더 모드) · ADR-0064/0065(slot 메뉴 단일 기여 API·1단 서브메뉴 container) · ADR-0076/0077(활성화 resume·fresh-fallback) · `src/commands/agentCommands.ts` · step-log 2026-07-13

## 맥락
예약 노드(claude reserved 프로필)를 활성화할 때 렌더 모드 — Terminal(PTY/xterm) vs StreamJson(헤드리스 NDJSON→RichSlot 챗) — 를 어디서 정할지가 문제였다. 두 모드 자체는 ADR-0044로 이미 존재하고, `createClaudeProfile`은 생성 시 `output_format`을 받아 프로필에 영속한다. 초기 구현은 트리 우클릭 "활성화"에 [클로드 터미널/클로드 JSON] 서브메뉴를 달아 **매 활성화마다** 모드를 고르는 per-activation 오버라이드였다(저장 프로필의 output_format은 불변으로 유지하려는 의도).

## 결정
렌더 모드는 **에이전트(프로필) 생성 시점에 결정하고 이후 불변**으로 한다 — 모드는 에이전트의 본질 속성이다.

- 트리 pane 배경 우클릭 "에이전트 생성"을 1단 서브메뉴 **[클로드 터미널 생성 / 클로드 JSON 생성]**(ADR-0064/0065 slot 메뉴 container)로 확장한다. 각 자식이 `createClaudeProfile`을 `'Terminal'`/`'StreamJson'`으로 호출해 프로필에 output_format을 영속한다(ADR-0044 재사용).
- 활성화는 단일 "활성화"(모드 선택 없음) — 저장된 모드 그대로 스폰한다.
- §5(LLM 제어): 생성 command 3종 — `agentlist.createTerminal`(`'Terminal'`) · `agentlist.createJson`(`'StreamJson'`) · `agentlist.createAgent`(파라미터화 프리미티브, `args.outputFormat ?? 'StreamJson'`, 유효값 외 입력은 다이얼로그 열기 전 fail-loud throw). 메뉴는 command id만 참조한다(ADR-0064 불변식).

## 거부한 대안
- **per-activation 렌더모드 오버라이드** — 활성화 우클릭 서브메뉴로 매 활성화마다 모드를 고르고, 저장 프로필은 불변으로 두는 방식(`effective_command` threading + 프로토콜 `SpawnProfile.output_format` 오버라이드 필드 + 코어 `with_output_format_override`). 세 가지 이유로 버렸다:
  - **① 개념 부정합** — 렌더 모드가 에이전트의 본질 속성인데 같은 예약 노드가 활성화마다 성격이 바뀌는 건 부정합이다(사용자 결정 2026-07-13).
  - **② 구현 복잡도** — 저장 프로필 불변을 지키려 프론트를 넘어 프로토콜·코어까지 오버라이드를 배선해야 했다(생성-고정 방식은 프론트 변경만으로 끝난다 — createClaudeProfile이 이미 output_format을 받으므로).
  - **③ 실측 결함** — resume 조기종료 → fresh-fallback(ADR-0077) 경로에서 오버라이드가 떨어져, Terminal로 활성화해도 StreamJson으로 스폰되는 **반쯤-작동** 상태였다(라이브 실측 2026-07-13). 생성-고정 방식은 저장값이 유일 출처라 이 결함이 원천 소거된다.
  - **보존** — 시도 코드는 `git stash@{0}`("adr-0078 per-activation override")에 보존.

## 근거
- 사용자 결정(2026-07-13): "한번 생성하면 다른 모드로는 안 되어야, 생성 이후 고정이 맞다."
- 라이브 GUI 실측: per-activation 오버라이드가 fresh-fallback 경로에서 Terminal→StreamJson으로 새어 반쯤만 작동함을 확인(process command line ground truth).
- 게이트: /review code full(cross-family, doc-aware+blind) PASS(입력 검증 fail-loud FIX 반영) · /qa PASS(vitest 613 green·tsc 0·코어 격리 0) · 생성 서브메뉴 라이브 GUI 실측 PASS.

## 영향 / 불변식
- **프론트 전용 변경** — protocol/core/daemon(crates/**) diff 없음. ADR-0044(output_format 렌더)·ADR-0064/0065(slot 메뉴)·ADR-0076/0077(활성화 resume·fresh-fallback)는 유효(폐기 아님) — 활성화 경로는 HEAD 그대로다.
- 모드 변경은 **재생성**으로 한다. 기존 프로필의 output_format을 사후 편집하는 command는 현재 범위 밖(필요 시 별도 ADR).
- 앵커: `src/commands/agentCommands.ts`(`createReservedProfile` · `create*` commands · `registerSlotMenu('agent_list', ...)` container). 이 파일이 생성-시-고정 계약의 단일 지점 — 활성화에 모드 선택 서브메뉴를 다시 붙이면 이 결정을 어기는 것이다.
