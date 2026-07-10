# 핸드오프: 에이전트 트리 + 프리셋 MVP 설계 확정 (사용자 결정 완료) — 다음 = PRD 고정 → /implement 한 번에

## 한 줄 상태 · 다음 첫 액션
- **상태:** pre-PRD 탐색/컨설 완주. `/research medium`(OSS 서베이 4갈래 + Codex 적대 리뷰 FIX) 기반으로 **에이전트 트리 + 프리셋 MVP 설계를 사용자와 전부 확정**. 코드 미착수(설계만). step-log에 전량 기록·커밋.
- **다음 첫 액션(사용자 지시 = "한번에 가자"):** 아래 확정 스펙을 **짧은 PRD로 고정** → **`/implement`(코더→`/review code`→`/qa`)로 에이전트 트리 + 프리셋 MVP를 한 번에 구현.** 설계는 이미 사용자 결정이라 재론 금지 — PRD는 코더에게 줄 스펙 정리 수준.

## 확정 스펙 (사용자 결정 — 그대로 구현, re-litigate 금지)

### 에이전트 트리 = `AgentList` SlotContent variant
- **평평한 목록, 계층 없음(MVP).** 줄 = `[상태 기호][이름]`. **경로(cwd) 표시 없음**(나중에 필요 방향으로 재추가).
- **상태 = 색 아닌 "모양"으로 5-state**(e-ink 대비): `●` 작업중 · `◐` 입력대기 · `○` 유휴 · `◻` 멈춤 · `✗` 에러. (실제 status enum에 매핑 — 현 `AgentTree.tsx`의 상태값 확인해 맞출 것.)
- **이름 = 생성 시 cwd의 폴더 basename.**
- **우클릭 2메뉴:** 에이전트 줄 우클릭 = 에이전트 메뉴(열기/이름변경/재시작/종료) · 빈 공간 우클릭 = 배경 메뉴(에이전트 생성). = 로드맵 data-driven 우클릭 메뉴(§5)와 동일물.
- 현 고정 사이드패널 `AgentTree`를 이 variant로 전환(슬롯 콘텐츠화).
- **변수-only로 구현**(색 리터럴 금지) → e-ink 대비. 상태는 글리프 모양이 담당.

### 생성 흐름
- 배경 우클릭 → "에이전트 생성" → picker: **등록된 경로(프리셋) 목록 + "새 경로 직접"**. 프리셋 강제 아님(raw cwd 경로도 가능).
- 고른 cwd로 claude 스폰(defaults) via 기존 `spawn_into` → 트리에 새 줄.

### 프리셋 = "등록해둔 경로"만 (최소)
- 프리셋 = **cwd만**. `{ id, cwd }`(이름=basename 파생). model·icon·backend·inject 전부 **나중**.
- **저장 = 백엔드 data-dir `.engram-data/presets.json`** — 단일 권위 → 멀티창 동기화 + §5 두뇌(백엔드) 소유. **첫 백엔드-영속 유저 데이터.** front localStorage 거부(창별 desync·멀티모니터와 충돌).
- **command:** `preset.list / create / delete` + `agent.spawn({ preset | cwd, parent? })`(spawn_into 재사용). `parent`는 **시그니처만**(서브에이전트 하위배치 primitive는 열되 nesting 실행은 나중).

### 배치 / 창
- `AgentList`·`PresetPalette` 둘 다 **SlotContent variant**(ADR-0060이 예고한 비-에이전트 variant급, FileTree/ControlPanel과 동류). 슬롯에 렌더, 팝업 창은 슬롯 담는 그릇일 뿐 **특수취급 X**(ADR-0035: 창 생사=src-tauri 소유, 창=순수 렌더러).

### 테마
- **기존 컴포넌트 미수정.** 신규 UI만 **변수-only**. 테마 *선택/전환*은 LLM 제어 유지(§5·`themeCommands`). **창별 테마 실제 구현 = D-7(보류, 이번 MVP 밖).** 슬롯 단위 테마 = 불필요.

## 무엇이 됨 (이번 세션 — 재작업 금지)
- **커밋 2건(master, 푸시 안 함):** `744c293` 렌더모드 디폴트 노트(콘텐츠 종류 기준 내부, 사용자 미노출) · `30c5303` 에이전트 트리·프리셋 MVP 방향 확정 step-log 노트.
- **리서치:** `/research medium` 설계-결정 모드 — 조사 수집자 4명(Sonnet 병렬: ①에이전트 정체성 vs 세션 ②프리셋 UX ③라이브 엔티티 트리 ④마크다운→시스템프롬프트) + Codex 적대 리뷰(FIX 7건 반영). 보고서 = 이 세션 대화(별도 파일 없음).
- **개념 정리 완료:** 슬롯에 임의 콘텐츠(게임 등) 가능(ADR-0060 의도) · 멀티모니터 동기화는 ADR-0035/0029/0041로 대비됨(팝아웃 E2E 기실측) · "트리 = 관리 에이전트"(engram 스폰분만; 셸 안 손으로 킨 프로세스는 핸들 없어 비노출).

## 검증 상태 (쌍으로)
- **돌린 것:** 없음 — **설계/탐색만, 코드 0.** step-log 커밋으로 기록 영속화 완료.
- **검증 안 된 것:** MVP 구현 전부 미착수. 5-state 상태 매핑은 실제 status enum과 대조 필요. 백엔드 프리셋 영속(신규)·spawn_into parent 확장 미검증. Codex 정직표기: agent-view는 tmux 세션매니저라 아이덴티티 근거 아님(UI 표현만) · 프레임워크 계층-오케 세부는 일반지식 비중(트리 오케 착수 시 재리서치).

## 실패한 접근 / do-not (재론 금지)
- **★`cargo test` bare·`-p engram-dashboard --lib` = 0xc0000139(WebView2Loader launch 사망)★** — 선재 환경배리어. 우회 = member-scoped test + throwaway verbatim-mount 하네스(Tauri 무링크). 이전 핸드오프에서 계승.
- **프리셋 저장 = localStorage 거부** — 웹뷰(창)마다 갈려 멀티모니터 동기화와 충돌. 백엔드 data-dir가 정답.
- **표시 폴더를 제어 부모로 겸용 금지** — 장식이 의미를 강제하면 앞서 거부한 cwd-트리 트랩 재판. (폴더 그룹핑·트리 오케 자체가 이번 MVP 밖·나중.)
- **확정 스펙 재론 금지** — 위 스펙은 긴 컨설 끝 사용자 결정. 다음 세션이 "이게 나을 듯" 하고 다시 열지 말 것.

## 미결 / 나중 (이번 MVP 밖 — 파킹)
- **폴더 그룹(장식) / 트리 오케스트레이션(에이전트-하위-에이전트 = 메시지·제어 위상)** — 둘 다 나중. 트리 오케는 자체 PRD/ADR + ADR-0014 + 메시지 시스템.
- **에이전트 auto-handoff 존속 모델**(세션 차면 에이전트 유지; A→B 핸드오프·Q&A·A 세션오프) — 현 ADR-0016/0017(세션-귀속)과 충돌 → 착수 시 ADR. 저장 미정.
- **역할 마크다운 → 시스템프롬프트 주입** — "지시 파일(컨텍스트)" vs 진짜 시스템프롬프트(`--append-system-prompt`) 구분, backend/claude.rs 격리.
- **창별 테마 실제 구현(D-7) + 하드코딩 색 정리**(`TerminalSlot.tsx:56` xterm·`AgentTree.tsx:33` 상태색·`ChatRow` bg-green/red).
- **프리셋 리치화**(model/icon/inject) — Goose recipes 참조.

## 정지 조건 (stop conditions)
- **비자명 코드 = `/implement`(코더→리뷰→QA), 메인 직접 구현 금지.** 자율 모드라도 게이트 스킵 금지. 굵은 결정 표면 시 ADR + (대화)사용자 질문/(자율)태그.
- **데몬/앱 강제종료 = 사용자 승인 후.** 기존 데몬(이전 세션 PID 22832) + 테스트 셸 에이전트 존속 가능(persist 모델). 정리 원하면 확인.
- 구현 중 확정 스펙에 없는 굵은 갈림길 나오면 멈추고 판단 요청. cdp 포트 9223, dev 로그 프로젝트 폴더 리다이렉트 금지.

## 참조 (읽을 것만)
- **정본 스펙:** step-log `docs/process/step-log.md` → "에이전트 트리·프리셋 방향 탐색 (설계-결정 리서치, pre-PRD)" 항목(스펙 + 거부 대안).
- **핵심 ADR:** 0060(SlotContent 유니온 — variant 추가 지점) · 0035(레이아웃 권위=src-tauri·창=렌더러) · 0029(데몬=에이전트 데이터 단일소유) · 0041(구독 소유) · 0016/0017(에이전트 수명 — 충돌 지점) · 0002/0030(capability) · 0004(backend 격리) · 0022/0055(command registry) · 0056(렌더모드). CLAUDE.md §5.
- **코드 포인터:** `src/components/agent/AgentTree.tsx`(현 사이드패널 → AgentList variant 전환) · `src-tauri/src/layout/types.rs`(SlotContent — AgentList/PresetPalette variant 추가) · `src/components/layout/ViewLayoutRenderer.tsx`(`switch(content.type)`) · `src-tauri/src/commands/layout.rs` · `spawn_into`(`manager.rs`) · `themeStore.ts`/`themeCommands.ts`(테마 제어표면) · 프리셋 신규 저장 = `.engram-data/`.
