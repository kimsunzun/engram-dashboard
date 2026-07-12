# 핸드오프: 프리셋·에이전트 rename 기능 게이트통과·**미커밋**(첫 액션=커밋) + i18n slice③ 푸시완료 + 오케스트레이션 서베이 커밋(미푸시)

## 한 줄 상태 · 다음 첫 액션
- **상태:** ① ADR-0069 i18n 3슬라이스 완료·**푸시**(b982b02). ② 오케스트레이션 OSS 서베이 커밋·**미푸시**(27844b8) + 핸드오프 커밋(44ba149) = `origin/master` 대비 **ahead 2**. ③ **프리셋·에이전트 [우클릭 메뉴 + 디스플레이 이름 변경] 기능 = 구현·리뷰 deep·qa full 전부 통과, 그러나 미커밋**(사용자가 "일단 핸드오프" — 커밋 보류).
- **다음 첫 액션(순서 중요):** **rename 기능을 커밋한다** — 게이트 통과분이라 방치하면 stale/뒤섞임 위험. 커밋 전: (a) 아래 결정 2건으로 **새 ADR 작성**(`/adr new` — 채번·인덱스 자동) (b) **step-log 기록** (c) 코드 24 M + 신규 `src/commands/presetCommands.test.ts` + ts-rs 바인딩 3개 스테이징(**`.tauri-dev-qa.log`는 제외** — QA 앱 로그). 그 다음 push 여부(ahead 2 + rename) 사용자 확인.

## ⚠️ 미커밋 게이트통과 산출물 — rename 기능 (유실 금지)
**무엇:** 프리셋 패널·에이전트 트리에 우클릭 컨텍스트 메뉴(이름변경·삭제) + 디스플레이 이름 인라인 편집. 탭 rename(ADR-0057) 미러 + ADR-0061(프리셋 이름 리치화) 확장. 풀스택 additive.

**미커밋 파일(24 M + 1 신규 + 바인딩 3):**
- core: `agent/preset.rs`·`agent/profile.rs`·`persistence/presets.rs`
- protocol: `domain.rs`·`messages.rs` + bindings `AgentCommand.ts`·`AgentProfile.ts`·`Preset.ts`
- daemon: `connection_core.rs` · src-tauri: `daemon_client/protocol_state.rs`
- front: `api/{agentClient,protocolClient,types}.ts`·`commands/{agentCommands,presetCommands}.ts`·`components/agent/{AgentList,mergeTreeNodes}.tsx/.ts`·`components/slot/PresetPalette.tsx`·`i18n/ko.ts`
- 테스트: `presetCommands.test.ts`(신규)·`protocolClient.test.ts`·`agentCommands.test.ts`·`AgentList.test.tsx`·`PresetPalette.test.tsx`·`mergeTreeNodes.test.ts` + core/protocol 내 rename·동시성 테스트.

**결정 1 (ADR 소재) — 디스플레이 이름 = 백엔드 저장 override:** `Preset.name: Option<String>` + `AgentProfile.display_name: Option<String>`(둘 다 `#[serde(default)]` — 기존 무필드 JSON → None, 마이그레이션 없음). 표시: Some→사용, None→cwd basename 파생(기존 동작 유지). **거부한 대안:** ①프론트 전용 localStorage(§5 LLM 제어 안 됨·비영속) ②기존 `AgentProfile.name` 재사용 — 오염됨(claude=CreateProfile명, SpawnByCwd=cwd 전체 문자열, connection_core.rs:869)이라 별도 `display_name` 신설. **§5:** rename이 command(`preset.rename`·`agent.rename`)로 노출돼 LLM도 이름 변경 가능(=백엔드 저장 이유).

**결정 2 (ADR 소재) — persistence 락 규율(동시성 fix):** 공유 헬퍼 `PresetRegistry::mutate`/`ProfileRegistry::mutate`가 `store.save`를 **map 락 보유 중** 실행(Option A). **거부한 대안 = IO-바깥-락**(기존 코드 주석 설계) — §5로 LLM/오케스트레이터가 rename·create·delete를 **동시/연속** 부르면 stale-snapshot 저장 race(메모리·broadcast=최신, 디스크=stale → 재시작 시 옛값). **cross-family(Codex)가 적출, 사용자가 §5 도달성 지적으로 확정.** deadlock 없음(map락→store write_lock 단방향 leaf, ADR-0006 sessions 락 도메인과 분리 — reaper·manager가 sessions guard 해제 후 registry 호출). advisory: save 중 map read 블록(로컬 작은 파일 무해). **같은 racy 패턴 3곳 수정**(preset mutate, profile mutate, `observe_session_id`→신규 `mutate_if` 조건부 저장). 코드 주석도 새 규율로 갱신됨.

## repo 상태
- 브랜치 master, `origin/master` 대비 **ahead 2**(`27844b8` 오케스트레이션 서베이 + `44ba149` 핸드오프, 미푸시) + **위 rename 미커밋**. `b982b02`(i18n slice③)는 푸시됨.
- **동시 세션 주의:** master 타 세션 동시작업 가능. push 전 `git fetch` ff 재확인.
- **앱 실행 중:** QA용으로 `npm run tauri dev`(디버그 포트 9223, 새 빌드) 백그라운드 기동해둠 — 사용자 사용 가능. `.tauri-dev-qa.log`(untracked)가 그 로그(무해·커밋 제외).

## 검증 상태 (게이트 — 전부 통과)
- **리뷰 deep 3인:** doc-aware(Opus) PASS · wire-contract 렌즈(Opus) PASS · **cross-family blind(Codex) FIX 1(동시성)→fix 후 재검증 PASS.**
- **qa full PASS:** `cargo build`(EXE 링크 15.30s) · `cargo test -p engram-dashboard-core`(166) `-p engram-dashboard-protocol`(48) · `cargo fmt --check` · 코어격리(`use tauri` 0 — lib.rs `//!` 주석만) · `npx tsc --noEmit` · `npm test`(vitest **550**) · **GUI 실측**(cdp 포트 9223): 프리셋·에이전트 rename 양쪽 §5 핸들로 실 daemon 라운드트립(null↔override↔null, broadcast 반영).
- **재실행:** Rust는 member-scoped만(`-p ...-core`/`-p ...-protocol`) · 프론트 `npx tsc --noEmit`+`npm test`. GUI = cdp `node scripts/cdp.mjs eval`(포트 9223 라이브 필요).
- **검증 안 된 것:** rename의 컨텍스트-메뉴/인라인편집 UI 조작을 cdp로 직접 시뮬레이션은 안 함(vitest 컴포넌트 테스트가 커버). 재시작-후-영속은 unit 테스트(serde default + 동시성)가 커버(라이브 재시작 실측은 안 함).

## do-not / 주의
- **bare `cargo test`·`-p engram-dashboard` = WebView2 0xc0000139 크래시** — member-scoped `-core`/`-protocol`만.
- 이 변경은 additive — 기존 create/delete/list 동작 회귀 금지(fix가 공유 헬퍼 건드려 create/delete/set-autorestore도 경유하니 그쪽 회귀 특히 확인). 순환 테스트 금지.
- **리뷰어 바인딩:** cross-family blind = `mcp__codex__codex`(effort high 명시 `config:{model_reasoning_effort:"high"}`, sandbox read-only, approval never). **SendMessage 툴 없음** → Claude 코더/리뷰어 이어가기는 fresh 스폰 + 이전 산출 주입, Codex만 `codex-reply`(threadId).
- 커밋/푸시는 master 직접이라 push는 명시 승인 필요(auto-mode가 무단 push 차단 — 이번 세션 실측). 로컬 커밋은 통과.

## 파킹된 큰 설계 논의 (사용자가 "나중에 얘기하자" — 캡처)
**주제: 다이나믹 스킬 loadout 주입** — 에이전트 역할별로 컨텍스트/스킬을 engram이 시스템 주도로 주입(폴더 복붙 대신 SSOT). 이번 세션 spike로 확정된 사실:
- **정적 주입 = 시스템 프롬프트**(`--append-system-prompt[-file]`, 실측 확인·append라 claude 기본 유지·큰 건 `-file`로 커맨드라인 길이 회피), **동적 주입 = 유저 메시지 push**(`WriteStdin`/`-p stream-json`). 재시작 재주입은 engram이 스폰마다 CommandSpec에 넣어 자동.
- **loadout = 3층 합성**(전부 per-spawn 주입, engram이 역할별 조합): 정체성=`--append-system-prompt-file`(soft) · 권한/capability=`--settings`(파일 OK, **hard 강제** — worker에 오케스트레이터 툴 deny) · 팀=`--agents <json>`(inline만, 파일변형 없음) 또는 `.claude/agents/*.md`(파일 기반).
- **스킬 subset:** 네이티브 allowlist 없음. `permissions.deny Skill(...)`(denylist, 문서화·사용자 settings.json이 실증)로 중앙 라이브러리 유지 + 역할별 제외. **engram이 allowlist("이것만")를 denylist(나머지 deny)로 자동 변환** = 사용자 경험 allowlist. (`skillOverrides` allowlist 여부 미검증.)
- **세션 저장 위치:** claude 세션은 cwd별 버킷 `~/.claude/projects/<encoded-cwd>/`(실측). `CLAUDE_CONFIG_DIR`로 옮길 수 있으나 **config dir 전체**(settings·skills·agents·projects·creds) 이동(세션만 따로 불가·실측). **세션만 커스텀 폴더로**는 `~/.claude/projects` 심링크(사용자가 이미 skills/agents를 심링크로 씀 — 같은 방식, 가능성 높음·미실측). PRD 미작성.
- **PRD 스코프(미착수):** loadout 바인딩을 프리셋 vs 에이전트 트리 어디서, 스킬 프리셋 등록 슬롯 필요 여부, 합성 모델. spike는 끝, PRD가 다음.

## 미결 결정 (사용자)
- rename 기능 커밋(다음 첫 액션) → push 여부(ahead 2 + rename).
- 원래 로드맵: **트리·프리셋 고도화**(사용자: 미완·다음 실작업) vs 오케스트레이션 옵션(서베이 §6) vs 위 다이나믹 loadout PRD.

## 정지 조건 (다음 세션)
- rename 커밋 전 ADR(결정 2건)·step-log 먼저. 커밋은 로컬, push는 사용자 확인.
- 다이나믹 loadout·세션저장은 **PRD/설계 미확정** — 구현 진입 전 사용자 결정(순서 불변).

## 참조 (읽을 것만)
- 미커밋 diff: `git diff`(rename 기능 전체) + 위 파일 목록.
- **결정 근거:** 위 "결정 1·2" (ADR 작성용). 미러 = ADR-0057(tab rename)·ADR-0061(preset 이름).
- 오케스트레이션 서베이: `docs/research/orchestration-survey-2026-07-12.md`.
- 재사용 패턴: `TabBar.tsx`(인라인 편집)·`AgentList.tsx`(행 컨텍스트 메뉴).
