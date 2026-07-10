# 핸드오프: 에이전트 트리+프리셋 MVP — 3슬라이스(A/B/C) 전부 구현·게이트·커밋 완료

## 한 줄 상태 · 다음 첫 액션
- **상태:** 에이전트 트리+프리셋 MVP **완성**. Slice A(백엔드 영속+wire)·B(프론트 프리셋 클라+PresetPalette)·C(AgentList+spawn+메뉴) 전부 `/implement`(코더→`/review code`→`/qa`) 게이트 통과 + 커밋. GUI 실측까지 live PASS. ADR-0061·0062 박제.
- **다음 첫 액션:** MVP 자체는 완료 — 새 작업은 **미해결 플래그(아래)** 중 사용자가 고르는 것부터. 급한 것 없으면 **영속·ProtocolClient 하드닝(사용자 결정, TaskList #5)** 판단이 첫 후보.

## 무엇이 됨 (커밋 — master 로컬, 푸시 안 함)
- `570e9b6` feat Slice A(백엔드): 데몬 소유 프리셋 영속 `presets.json`(프로필 패턴 미러) + SlotContent `AgentList`/`PresetPalette` variant.
- `0311a44` docs: ADR-0061(프리셋=데몬 소유) + step-log.
- `38ba194` feat Slice B(프론트): agentClient 프리셋 메서드 + `preset-list-updated` 배선 + `PresetPalette` variant + presetCommands.
- `03d1733` feat Slice C(프론트): `AgentTree`→`AgentList`(평면) 전환 + `statusGlyph` pure fn + `agent_list` variant + `agent.spawn` command + 우클릭 2메뉴 + `util/basename`. AgentTree.tsx 제거.
- `b1715f5` docs: ADR-0062(5-glyph 상태 매핑) + step-log B·C.

## 게이트·실측 결과
- 전 슬라이스 `/review code`(doc-aware Opus + cross-family blind Codex 2인 적대) PASS. `/qa` 코드게이트(member-scoped 빌드·test·fmt·코어격리·tsc·npm) 전부 green.
- **GUI 실측 live PASS(cdp):** 프리셋 CRUD end-to-end(client↔데몬↔`.engram-data/presets.json`↔broadcast↔store) · `agent.spawn`→AgentList `●`(Running) 등장 · Reserved `○` · basename 표시 · kill 전이.
- Slice A 이월됐던 전체 `cargo build` 링크 = 데몬 재빌드로 PASS 확인.

## 검증 상태 (쌍으로)
- **돌린 것:** `cargo test -p engram-dashboard-core`/`-p engram-dashboard-protocol` · `cargo build`(전체, 링크 OK) · `cargo fmt --check` · `rg "use tauri" crates/engram-dashboard-core/src/`=0 · `npx tsc --noEmit`=0 · `npm test`=416 · cdp GUI 실측. **재실행 = 그대로**(bare `cargo test` 금지 — member-scoped만, 0xc0000139).
- **검증 안 된 것:** 멀티창(팝아웃) 프리셋 동기화는 단일창에서만 실측(broadcast 경로는 프로필과 동형이라 高신뢰지만 2창 동시 미실측). 우클릭 메뉴 실제 마우스 동작은 cdp 대신 command/agentClient 경유로 실측(메뉴 렌더는 unit 테스트).

## 미해결 / 플래그 (다음 — 대부분 사용자 결정)
- **[사용자 결정 · TaskList #5] 영속·ProtocolClient 하드닝(프로필+프리셋 양 경로)** — Codex blind 지적: ①Registry::mutate lock 밖 save→동시 mutate disk 순서 역전(in-memory 정확, LOW) ②Store::save Result 없이 log-only→save 실패해도 Ack ③ws broadcast try_send drop ④`sendCommand` pending 타임아웃 부재(응답 없으면 무한대기·맵 누수) ⑤PROTOCOL_VERSION 미bump. 프리셋만 고치면 미러 비대칭이라 양-경로 동시. 근거 = Codex threads 019f482f·019f499b.
- **비-에이전트 SlotContent variant 슬롯 placement command** — 현재 AgentList/PresetPalette는 고정 사이드패널 마운트 + variant case만. 임의 슬롯/팝업에 배치하려면 `set_slot_content` 류 command(§5 LLM 제어표면) 필요. "고정 사이드패널 완전 제거→슬롯-only 전환"은 이게 있어야.
- **agent rename** — 이름이 cwd basename 파생(미저장, ADR-0061)이라 rename하려면 name-override 저장 필드 필요. 현재 메뉴 '이름변경' disabled '준비 중'.
- **agent restart 전용 command** — 현재 kill→re-spawn 조합만. 메뉴 '재시작' disabled '준비 중'. (step-log "다음"의 [게이트] 자동 재시작과 연계.)
- **메뉴 위치 화면-밖 클램프** — AgentList·SlotContextMenu 등 모든 fixed 메뉴 공통. cross-cutting 폴리시.

## 정지 조건 (stop conditions)
- **실행 중 데몬/앱 = 신규 빌드**(프리셋 handling 有). 데몬 PID 13160 + 앱 PID 27316(CDP 9223). 강제종료는 사용자 승인(persist 모델 — 존속 에이전트 유실 주의). 재빌드 필요하면 이 프로세스가 exe 락.
- 비자명 코드 = `/implement`(코더→리뷰→QA), 메인 직접 구현 금지. 굵은 결정 = ADR + (자율)태그.
- 확정 스펙(에이전트 트리·프리셋 MVP) 재론 금지 — 완료됨.

## 참조 (읽을 것만)
- **ADR:** 0061(프리셋 데몬 소유)·0062(상태 매핑)·0060(SlotContent 유니온)·0011(agentClient 표면)·0055/0022(command registry)·0035(레이아웃 권위)·0016/0017(에이전트 수명)·0024(data-dir)·0029(데몬 소유). CLAUDE.md §5.
- **step-log:** "Slice A: 백엔드 foundation" + "Slice B·C" 항목.
- **코드 포인터:** `crates/engram-dashboard-core/src/{agent/preset.rs,persistence/presets.rs}` · protocol `messages.rs`/`domain.rs` 프리셋 wire · daemon `connection_core.rs`(preset arms+broadcast) · `src/api/{agentClient,protocolClient,tauriTransport}.ts`(프리셋 메서드) · `src/components/agent/AgentList.tsx`(statusGlyph·메뉴) · `src/components/slot/PresetPalette.tsx` · `src/commands/{presetCommands,agentCommands}.ts` · `src/util/basename.ts` · `src/components/layout/ViewLayoutRenderer.tsx`(variant case).
