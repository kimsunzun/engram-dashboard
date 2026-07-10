# 핸드오프: 에이전트 트리+프리셋 MVP — Slice A(백엔드 foundation) 커밋 완료, Slice B/C = fresh 승계

## 한 줄 상태 · 다음 첫 액션
- **상태:** 확정 스펙(사용자 결정) 기반으로 MVP를 3 세로 슬라이스로 쪼개 진행 중. **Slice A(백엔드 foundation) = /implement critical 게이트 전부 통과 + 커밋 완료.** Slice B/C 미착수(코드 0). ADR-0061 박제 완료.
- **다음 첫 액션:** **Slice B** 착수 — 프론트 프리셋 클라이언트 + `PresetPalette` SlotContent variant. `/implement standard`(코더→`/review code full`→`/qa`). 그다음 **Slice C**(AgentList variant + spawn + 우클릭 메뉴, `/implement critical`). 설계는 이미 사용자 결정 = 재론 금지. **B/C 시작 전 아래 "정지 조건"의 데몬 재시작 이슈를 반드시 읽을 것.**

## 무엇이 됨 (이번 세션 — 재작업 금지)
- **커밋 2건(master, 푸시 안 함):** `570e9b6` feat(Slice A 코드 + ts-rs 바인딩) · `0311a44` docs(ADR-0061 + 인덱스 + step-log). 이전 커밋 `30c5303`(방향 확정) 위.
- **ADR-0061 박제:** 프리셋 = 데몬 소유 영속(presets.json), 프로필 패턴 미러. 거부 대안 4개 기록(localStorage·src-tauri소유·프로필겸용·variant-config).
- **Slice A 구현(프로필 시스템 end-to-end 미러):**
  - core: `crates/engram-dashboard-core/src/agent/preset.rs`(신규 — `Preset{id,cwd}`·`PresetRegistry` list/create(cwd canonicalize)/remove) · `persistence/presets.rs`(신규 — `FilePresetStore` atomic write+버전게이트+손상보존).
  - protocol: `AgentCommand::{ListPresets,CreatePreset{cwd},DeletePreset{preset_id}}` + `AgentEvent::{PresetList{request_id,presets}(reply), PresetListUpdated{presets}(broadcast)}` + wire `Preset`(domain.rs)·`PresetId`(ids.rs)·ts-rs 바인딩(`bindings/Preset.ts` 등).
  - daemon: `connection_core.rs` 3-arm(ListPresets→요청자만 reply · Create/Delete→mutate→`broadcast_preset_list` 전 연결) + `build_manager` 배선(`AgentManager.presets()` — `profiles()` 미러).
  - src-tauri: `daemon_client`에 신규 command/event request_id 매칭 + `emit_broadcast`에 `preset-list-updated` 이벤트 배선.
  - SlotContent 유니온: `src-tauri/src/layout/types.rs`에 `AgentList`/`PresetPalette` unit variant(ADR-0060 앵커) + `manager.rs` resolve_spawn_slot/agent_id match 처리 + `bindings/SlotContent.ts` 갱신.
- **게이트:** `/review code deep` PASS(doc-aware 2인 PASS + Codex blind FIX 4건 → 아래 follow-up으로 분리) · `/qa` 코드게이트 PASS(member-scoped 빌드·test·fmt·코어격리·tsc·npm 전부 green).

## 확정 스펙 (사용자 결정 — 그대로 구현, re-litigate 금지)
> 정본 = step-log `docs/process/step-log.md` "에이전트 트리·프리셋 방향 탐색" 항목. 아래는 B/C에 필요한 요지.

### Slice B — 프론트 프리셋 클라이언트 + PresetPalette variant
- **ProtocolClient/agentClient(`src/api/`):** `listPresets()`/`createPreset(cwd)`/`deletePreset(id)` + `onPresetListUpdated(cb)` 추가. 프로필 메서드(`listProfiles`/`createClaudeProfile`/`deleteProfile` + `onProfileListUpdated`)를 미러 — 백엔드 wire(위)가 준비됨. `ptyApi` 직접 호출 금지, `agentClient` 단일 표면만(ADR-0011).
- **eventBus:** `preset-list-updated` 구독 등록(agent-list-updated 패턴 미러, src-tauri가 이미 emit).
- **PresetPalette SlotContent variant:** `src/components/layout/ViewLayoutRenderer.tsx`의 콘텐츠 dispatch에 `content.type === 'preset_palette'` case 추가 → 프리셋 목록 렌더(이름 = cwd basename 파생) + 등록/삭제 UI. **변수-only(색 리터럴 금지) — e-ink 대비.**
- **presetCommands 레지스트리(`src/commands/`):** `preset.list/create/delete` 등록(themeCommands.ts 패턴 미러, `register({id,title,run})`). handler는 agentClient 호출로 라우팅.

### Slice C — AgentList variant + spawn 흐름 + 우클릭 메뉴
- **AgentTree→AgentList variant:** 현 `src/components/agent/AgentTree.tsx`(react-arborist 트리)를 **평평한 목록** `AgentList` SlotContent variant로 전환. 줄 = `[상태 기호][이름]`, cwd 표시 없음, 이름 = cwd basename.
- **5-glyph 상태 = 색 아닌 모양(pure 매핑 함수):** `●`작업중 `◐`입력대기 `○`유휴 `◻`멈춤 `✗`에러. ★실제 백엔드 enum = `Running|Exiting|Exited|Failed|Killed`뿐 — 입력대기(◐)·유휴(○) 신호 없음★. → 매핑: Running→● · Exited/Killed→◻ · Failed→✗ · Exiting→◻(전이). ◐·○는 미래 백엔드 신호 대비 어휘로만 두고 지금은 미점등. **이건 ADR-0062로 박을 것**(거부 대안 = 출력활동 추적을 지금 도입 — 미검증 백엔드 내부라 보류). 현 `AgentTree.tsx:30-38` statusColor는 색 기반 3-state → 모양 기반으로 교체.
- **우클릭 2메뉴:** 에이전트 줄 우클릭 = 에이전트 메뉴(열기/이름변경/재시작/종료) · 빈 공간 우클릭 = 배경 메뉴(에이전트 생성). 재사용: `src/components/slot/SlotContextMenu.tsx`(범용 fixed-position 패턴) + AgentTree 인라인 메뉴(`AgentTree.tsx:259-291`). data-driven 메뉴(§5 로드맵)와 동일물이나 MVP는 하드코딩 항목 OK.
- **spawn 흐름:** 배경 우클릭 "에이전트 생성" → picker(등록 프리셋 목록 + "새 경로 직접") → 고른 cwd로 스폰 → AgentList에 새 줄. `agent.spawn({preset|cwd, parent?})` command 신설 — preset→cwd 해석 후 기존 스폰 경로 재사용(`agentClient.spawnAgent(cwd)` = 데몬 SpawnByCwd; `parent`는 **시그니처만**, 세팅 시 reject·nesting 나중).
- **배치:** AgentList·PresetPalette 둘 다 SlotContent variant, 팝업 창은 슬롯 담는 그릇(ADR-0035, 특수취급 X).

## 검증 상태 (쌍으로)
- **돌린 것(Slice A):** member-scoped `cargo test -p engram-dashboard-core`(153+9신규) `-p engram-dashboard-protocol`(신규 golden/roundtrip) PASS · `cargo build` member-scoped `--lib`(core/protocol/daemon/src-tauri) PASS · `cargo fmt --check` PASS · `rg "use tauri" crates/engram-dashboard-core/src/`=0(doc-comment 1건뿐) · `npx tsc --noEmit` 0에러 · `npm test` 352 PASS. **재실행 명령 = 그대로.**
- **검증 안 된 것:** ★전체 `cargo build` 최종 링크 미검증 — 실행 중 데몬.exe 락(os error 5, env). ★신규 프리셋 wire·SlotContent variant의 **GUI/런타임 실측 0** — 프론트 소비자가 없고(Slice B/C) 신규 데몬이 안 떠서. B/C는 프론트 소비를 붙인 뒤 **반드시 GUI 실측(cdp)** 필요 — 단 아래 데몬 재시작 이슈 걸림.

## 실패한 접근 / do-not (재론 금지)
- **★bare `cargo test`·`-p engram-dashboard --lib` = 0xc0000139(WebView2Loader 사망)★** — 선재 환경배리어. 우회 = member-scoped test만.
- **프리셋 localStorage 거부**(멀티창 desync) · **표시 폴더를 제어 부모로 겸용 금지**(cwd-트리 트랩) · **확정 스펙 재론 금지.**
- **프리셋만 하드닝 금지** — 프로필과 미러라 프리셋만 고치면 비대칭. 하드닝은 양 경로 동시(follow-up, 사용자 결정).

## 정지 조건 (stop conditions)
- **★B/C GUI 실측 = 데몬 재시작 필요 = 사용자 승인★** — 신규 프리셋 handling은 **새로 빌드한 데몬**이 떠야 동작한다. 현재 실행 중 데몬(이전 빌드, 프리셋 미지원) + 앱이 `engram-dashboard-daemon.exe`를 잡아 전체 빌드 링크·신규 동작 실측 둘 다 막힘. 데몬/앱 강제종료·재시작은 **사용자 승인 후**(persist 모델 — 존속 테스트 에이전트 유실 주의). B/C 코드는 code-gate까지 구현·커밋 가능하나 **end-to-end GUI 실측은 재시작 승인 뒤**로 미룰 것(정직 라벨).
- **비자명 코드 = `/implement`(코더→리뷰→QA), 메인 직접 구현 금지.** 자율 모드라도 게이트 스킵 금지. 굵은 갈림길 = ADR + (자율)태그.
- **[사용자 결정 대기] 영속 하드닝(TaskList #5)** — Codex blind가 적출, 전부 프로필 shipped 패턴 미러: ①Registry::mutate가 lock 밖 save → 동시 mutate 시 disk 순서 역전(in-memory 정확, severity LOW) ②Store::save가 Result 없이 log-only → save 실패해도 Ack 성공 ③ws broadcast try_send drop ④PROTOCOL_VERSION 미bump. 고치려면 프로필+프리셋 양 경로 동시 = 사용자 결정 전 착수 금지. 근거 = Codex thread 019f482f.

## 참조 (읽을 것만)
- **정본 스펙:** step-log `docs/process/step-log.md` → "에이전트 트리·프리셋 방향 탐색"(확정 스펙+거부 대안) + "Slice A: 백엔드 foundation"(이번 회수물).
- **핵심 ADR:** 0061(프리셋 데몬 소유 — 방금) · 0060(SlotContent 유니온 — variant 추가 지점) · 0011(agentClient 단일 제어표면) · 0055/0022(command registry) · 0035(레이아웃 권위=src-tauri) · 0016/0017(에이전트 수명 — auto-handoff 충돌 지점, MVP 밖) · 0002/0030(capability) · 0056(렌더모드). CLAUDE.md §5.
- **코드 포인터(B/C 진입점):** `src/api/`(agentClient/ProtocolClient — 프리셋 메서드 미러 추가) · `src/api/eventBus`(preset-list-updated 구독) · `src/components/layout/ViewLayoutRenderer.tsx`(SlotContent dispatch — agent_list/preset_palette case) · `src/components/agent/AgentTree.tsx`(→AgentList 전환·statusColor→모양) · `src/components/slot/SlotContextMenu.tsx`(우클릭 재사용) · `src/commands/registry.ts`+`themeCommands.ts`(presetCommands 패턴) · `src/store/agentStore.ts`/`viewStore.ts`. 백엔드 spawn 재사용 = `agentClient.spawnAgent(cwd)`(데몬 SpawnByCwd).
- **미착수 파킹(MVP 밖):** 폴더 그룹 · 트리 오케스트레이션(에이전트-하위-에이전트, ADR-0014+메시지시스템) · auto-handoff 존속 모델(ADR-0016/0017 충돌) · 역할 마크다운→시스템프롬프트 · 창별 테마(D-7) · 프리셋 리치화(model/icon/inject, Goose recipes).
