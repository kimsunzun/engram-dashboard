# 핸드오프: 리로드 replay 버그 — resync 수정으로 tag0(터미널) 복원 실증 성공 / tag1(JSON) 미해결 · 미커밋(커밋 승인 대기) — 다음=tag1 클라 재충전 경로 진단 or 사용자 커밋 결정

## ⭐ 규약 — 커밋 금지 (사용자 결정) ⭐
자율 모드에서도 자동 커밋 금지. 커밋은 사용자 명시 지시 때만("계속 진행"·"최대한 작업"≠커밋 승인). 커밋 전 diff 보여주고 승인받기. implement/qa/review 스킬 전부 이 규약 내장.

## 한 줄 상태 · 다음 첫 액션
S15 리로드 replay 버그를 진단→수정(coder→review deep→qa full)했다. **`subscribe_output`에 `client.resync()` 1줄 추가 = 터미널(tag0) 리로드 복원 실증 성공(cdp 3회 재현).** 그러나 **JSON/구조화(tag1)는 여전히 복원 실패** — 별개 축(클라측 tag1 재충전/flush 경로)에 막힘. **미커밋(review PASS, qa tag0 PASS·tag1 FAIL) — 커밋 승인 대기.**
**다음 첫 액션 = 둘 중 사용자 결정:** (A) tag0 수정만 채택 커밋(tag1은 별도 후속) (B) tag1까지 이번에 진단·수정 (C) 롤백. 기술적으로 다음은 **tag1 클라 재충전/flush 경로 진단**(아래 ⭐⭐).

## repo 상태 (미커밋 — 커밋 승인 대기)
- **HEAD = `da9f948`(master).** 이 세션 커밋 0건(규약).
- **미커밋 내 변경 2건:**
  - `src-tauri/src/commands/agent.rs` — `subscribe_output` 끝(registry 락 드롭 후, `replay_slots(slots,true)` 다음)에 `client.resync();` 1줄 + 상세 주석(FIX-1: resync=연결 전량 재구독 명시·정확성 안전 근거·최적화 여지).
  - `src-tauri/src/daemon_client/tests.rs` — 신규 테스트 2건 + 헬퍼(+275줄): `reload_resubscribe_repulls_daemon_ring`(배선 그물), `reload_resync_repull_refills_buffer_and_restores_replay`(FIX-2, 버그 본체 재현: on_frame content 채움→drop 리로드→replay 0 확인→resync→재충전→복원 단언+멱등), 헬퍼 `spawn_ring_replay_server`/`wait_until`.
- **미커밋 = `.claude/skills/research/SKILL.md` 1건은 외부/Fable 편집 — 내 것 아님, 손대지 마.**

## ⭐⭐ tag1(JSON) 리로드 복원 실패 — 다음 세션 진단 대상 ⭐⭐
**증상(cdp 실측 3회 재현):** JSON 에이전트 스폰→프롬프트→라이브 렌더 정상(lds -1→N, DOM 렌더). **리로드 후 lds=-1 고착, 이전 출력 DOM 미복원("○ streaming").** 리로드 후 새 프롬프트 라이브는 정상 = **이전 seq replay만 실패**. 터미널(tag0)은 같은 시나리오에서 **복원 성공**(결정적 대조).
**근본원인 방향(미확정 — 코드 vs 실측 어긋남):**
- resync가 쓰는 `subscribe_from`(replay) 경로는 **코드상 payload-generic으로 tag1(구조화) 전송하게 되어 있음**(`crates/engram-dashboard-core/src/agent/output_core.rs:400-413` — TerminalBytes→Bytes, 그 외→Event 전송). 데몬 Ring 저장도 종류 무관(`output_core.rs:122-129` emit이 event.clone() 무조건 push). **즉 코드상 tag1은 통과해야 하는데 실측 배달 0** = 어긋남.
- **후보(다음 세션 진단):** 클라측 content ring 재충전(`on_frame`)이 tag1 원본 bytes를 담는지 / replay flush가 tag1을 프론트로 흘리는지 / tag0는 되고 tag1만 막히는 클라 소비 경로 차이. 코드 정적분석상 통과하므로 배선 버그면 작은 수정 가능성.
- **볼 곳:** 클라 tag1 재충전/flush — `src-tauri/src/daemon_client/` on_frame(tag1 bytes append 여부)·output_view_store replay flush(tag0 vs tag1)·output_channel. 프론트 tag1 소비 — `src/components/slot/RichSlot.tsx`(subscribeOutput만 씀, getSnapshot 안 씀)·`structuredAccumulator.ts`·`protocolClient.ts`.

## ⚠️ 관측 함정 (do-not — 재현 시 주의)
- **`getSnapshot(json_agent)=0`을 tag1 판정에 쓰지 마라.** `get_snapshot` 경로는 구조화를 **명시적 drop**한다(`output_core.rs:434-468` filter_map TerminalBytes만 = do-not #5 DEFER "get_snapshot 구조화 wire 매핑 후속"). tag1 agent엔 Ring에 있어도 **항상 0** 반환 = 관측 사각지대. **tag1 판정은 DOM 텍스트 + subs `lastDeliveredSeq`로** 하라(리로드 복원 경로=subscribe_from replay, getSnapshot 아님).
- **`subs`·`pendingBuffers`는 JS `Map`** — `Object.keys()` 쓰지 마라(항상 `[]`). `[...map.entries()]`/`[...map.keys()]`. (이전 실측 오염원)

## 실패한 접근 (do-not — 재시도 금지)
1. **원 가설(caps-게이트發 pre-subscribe 버퍼 유실) = 기각** — pre-subscribe 버퍼는 tag0에서 정상 작동 실측.
2. **①안(프론트 등록 트리거 분리) = 기각** — subscribe_output은 리로드 후 정상 발화·Ok(()) 완주 실측. 프론트 등록 무죄.
3. **"프론트 소비(RichSlot/accumulator) 문제" = 기각** — 라이브 fan-out 정상, 데몬 Ring 보존(seq 연속). 
4. **B(registry label 불일치)·C(slots_for_window 빈) = 기각** — 리로드 후 라이브 도착 실측(flush/registry/router 건강).
5. **08:14부터 떠있던 오래된 앱 인스턴스로 진단 = 오염** — write_input 무응답·getSnapshot 등 오염. fresh 재기동 필수.

## 검증 상태 (쌍)
**돌린 것(PASS):** `/review code deep`(opus doc-aware + Codex cross-family). 1차 FIX vs BLOCK 불일치 → deep 추가 렌즈가 Codex의 seq역전(F-B)·데몬 double-sink(F-C)를 "기존 결함·resync 무관·dedup 흡수"로 판별 → FIX 2건(주석 오도·테스트 약함) 반영 → **재리뷰 2인 PASS**. `/qa full`: build·test(core 182/protocol/discovery 44/daemon --lib 38)·fmt·격리(use tauri 0)·tsc·npm(vitest 212) **전부 PASS**. **cdp 실측: tag0 리로드 복원 YES(3회)** / tag1 복원 **NO(3회)**.
**재실행:** 프로세스 종료(engram-dashboard·engram-dashboard-daemon·vite 1420) → `cargo build` · `cargo test -p engram-dashboard-{core,protocol,discovery}` + `-p engram-dashboard-daemon --lib` · `cargo fmt --check` · `rg "use tauri" crates/engram-dashboard-core/src/`(→0) · `npx tsc --noEmit` · `npm test`. cdp: `$env:RUST_LOG="debug"; $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223"; npm run tauri dev` → `node scripts/cdp.mjs eval "..."`.
**검증 안 된 것:** tag1 리로드 복원(FAIL — 미해결). src-tauri 통합 테스트 실행(WebView2 로더 `STATUS_ENTRYPOINT_NOT_FOUND` = compile-net only, step-log:494 기존 이슈). 데몬 double-sink(F-C) 별도 이슈 미착수. resync 전량 재구독 다창 성능(멱등이라 정확성 안전, 실부하 미측).

## 별도 이슈 (이번 범위 밖 — 기록만)
- **F-C 데몬 double-sink**(기존 결함, resync 무관): `crates/engram-dashboard-daemon/src/connection_core.rs:1080` same-socket resubscribe가 새 sink를 옛 sink 제거(1090-1097) 전 설치 → concurrent emit이 같은 seq 이중 전송. dedup 흡수라 정확성 안전, 대역폭 낭비. resync가 호출 빈도↑. 별도 ADR/이슈 후보.
- **do-not #5**: get_snapshot 구조화 wire 매핑 미구현(S15 DEFER). tag1 리로드 복원과 별개(복원은 subscribe_from replay). 단 구현하려면 output_core.rs snapshot() 반환 payload-generic 확장 + connection_core GetSnapshot arm에서 output_event_to_wire 사용 + protocol wire 스키마 확장(ts-rs·golden 동반) = 중간 규모.

## 앱 상태
CDP 9223 앱 **살아있음**(qa 서브 16:xx 재기동, RUST_LOG=debug, 슬롯 split 뷰 + JSON·터미널 에이전트 각 1). 다음 세션 재활용 가능(단 새 코드=resync 반영본). agents.json 영속이라 재기동해도 복원.

## 근본원인 정리 (확정분)
리로드 시 데몬 재구독(`resubscribe_and_sweep` → visible agent `Subscribe{after_seq}` 재전송)은 **connect/재연결/Resync 진입에서만** 돎(`connection.rs:849`). 웹뷰 리로드는 소켓 생존이라 미트리거 → 클라 content ring 재충전 안 됨 → replay 0. **resync() 배선이 이 갭을 메움 = tag0 복원(확정).** tag1은 이 재충전 경로를 코드상 통과하나 실측 배달 0(미확정 잔여).

## 협업 메모
- 사용자: "쭉쭉/최대한 진행" 강함. 커밋·굵은 결정·순서위험만 올림. 순수 내부 구현·진단 절차(RUST_LOG 확정 여부, 수정안 선택 등)는 메인이 정하고 **보고**(결정 떠넘기기 금물 — 이 세션 초반 지적받음). 커밋 전 승인.
- 구현 규약: 코더(opus/sonnet)→`/review code`(opus+Codex 다른 family)→`/qa`→**커밋 승인 후**. 메인 직접 구현 금지. 조사·실측·대량읽기 서브에이전트 위임(결론만 회수).
- 이 세션 서브에이전트 ~15기(진단 다수·코더 2·리뷰어 5·qa 1). implement/review/qa/continue 스킬 파이프라인 완주.

## 참조 (읽을 것만)
- **tag1 진단 착안:** `crates/engram-dashboard-core/src/agent/output_core.rs`(subscribe_from 400-413·emit 122-129·snapshot drop 434-468) · `src-tauri/src/daemon_client/`(on_frame·output_view_store replay flush·output_channel) · `src/components/slot/RichSlot.tsx`·`structuredAccumulator.ts`·`src/api/protocolClient.ts`.
- **수정분:** `src-tauri/src/commands/agent.rs`(resync 198행+주석) · `src-tauri/src/daemon_client/tests.rs`(신규 테스트 2970~).
- ADR: 0040(서버버퍼)·0043(mount-replay)·0044·0045(S15)·0007(epoch)·0006(락)·0037(seq dedup)·0038(근본원인 우선). S15 TRD `docs/process/S15-backend-output-refine/`.
- step-log `docs/process/step-log.md` — 이 세션 결과(resync tag0 수정·tag1 잔여) 아직 미기록 → 커밋 결정 후 기록.
