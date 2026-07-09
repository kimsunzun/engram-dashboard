# 핸드오프: 리로드 replay 버그 완전 수정 + 커밋푸쉬 완료(ca3f325) — 근본원인=프론트 subscribeOutput StrictMode 레이스 / 후속=split+assign tag1 고착·step-log·ADR

## ⭐ 규약 — 커밋 금지 (사용자 결정) ⭐
자율 모드에서도 자동 커밋 금지. 커밋은 사용자 명시 지시 때만. 커밋 전 diff 승인. (이번 커밋 ca3f325는 사용자 "커밋푸쉬해" 명시 승인 받음.)

## 한 줄 상태 · 다음 첫 액션
리로드 replay 버그(웹뷰 리로드 후 이전 출력 복원 실패)를 **완전 수정 + origin/master 푸쉬 완료(ca3f325).** tag0(터미널)·tag1(JSON) 둘 다 리로드 복원 cdp 실증. 워킹트리 clean(미커밋 = research/SKILL.md 외부 1건뿐).
**다음 첫 액션(택1):** (A) **후속 버그** — split+assignAgent 시 tag1 일시 고착 조사(아래 ⭐). (B) **step-log·ADR 기록**(이번 결과 미기록). (C) 새 작업.

## 이번 세션 결과 (커밋됨)
- **HEAD = `ca3f325`(master), origin 동기.** 커밋 1건: 프론트 subscribeOutput 레이스 수정(`src/api/protocolClient.ts` + `protocolClient.test.ts`, 226+/26-). 백엔드(src-tauri/crates) 무변경(HEAD 유지).
- **근본원인:** 프론트 `ProtocolClient.subscribeOutput`의 pre-subscribe 버퍼(1회성)가 React StrictMode 이중구독 레이스로 폐기될 첫 인스턴스에 소진·삭제 → 생존 구독자 빈 화면. **tag 무관(tag0/tag1 공통).**
- **수정 4조치:** ① subs.set을 `await ensureReady()` 이전으로(생존 구독자 동기 확정) ② flush를 token 가드로(생존 구독자만, 옛 st 버퍼 보존) ③ `SubState.ready` 게이트(ready 전 프레임 pendingBuffers 경유 → early set의 seq 조기 전진→낮은 seq drop 방지, 타이밍 무관 순서 보존) ④ ensureReady reject 시 자기 등록 롤백(좀비 구독 방지).

## ⭐⭐ 후속 버그 (다음 세션 조사 후보) ⭐⭐
**split+assignAgent 로 슬롯 재구성 시 tag1 일시 "○ streaming" 고착** (qa full 중 1회 관측). 리로드하면 복원됨 → **이번 리로드 복원 경로와 별개.** 의심: slot 재마운트(unsubscribe→resubscribe) 시 replay 누락 or 재구독 타이밍. 미조사(추측). 착안: `RichSlot.tsx`/`TerminalSlot.tsx` effect 재마운트 생명주기 + protocolClient subscribeOutput/unsubscribe + 데몬 replay_slots 트리거(slot 재배정 경로).

## ⚠️ do-not (재현·재조사 시 주의)
- **`getSnapshot(json_agent)`을 tag1 판정에 쓰지 마라** — `get_snapshot`은 구조화 명시적 drop(`output_core.rs:434-468` = do-not #5 DEFER "구조화 wire 매핑 후속"). tag1엔 Ring에 있어도 항상 0 반환 = 관측 사각지대. **tag1 판정 = DOM 텍스트 + subs `lastDeliveredSeq`.** (리로드 복원 경로 = subscribe_from replay, getSnapshot 아님.)
- **`subs`·`pendingBuffers`는 JS `Map`** — `Object.keys()` 금지(항상 `[]`). `[...map.entries()]`.

## 실패한 접근 (do-not — 재시도 금지)
1. **백엔드 resync 접근 = 오진** — 이전 세션이 "리로드 시 데몬 재구독 미트리거"로 보고 `subscribe_output`에 `client.resync()` 추가했으나, 진짜는 프론트 레이스였다. resync는 롤백(불필요 전량 재구독 부작용). **근본은 프론트, 백엔드 아님.**
2. caps-게이트 버퍼 유실 가설 · 프론트 등록 트리거 분리(①안) · B(registry label)·C(slots_for_window) — 전부 실측 기각.
3. 오래된 앱 인스턴스로 진단 = 오염(fresh 재기동 필수).

## 검증 상태 (쌍)
**돌린 것(PASS·커밋 근거):** `/review code deep` 2인(opus doc-aware + Codex cross-family) — 1차 FIX 3건(좀비 구독·early set seq 꼬임·테스트 약함) → 반영 → **재리뷰 2인 PASS(findings 0)**. `/qa full`: tsc·vitest(218, protocolClient 45)·격리(use tauri 0)·cargo build PASS + **cdp 실측 tag0/tag1 리로드 복원 YES(3·5회 재현, 중복/seq 역전 없음).**
**재실행:** `npx tsc --noEmit` · `npm test` · `rg "use tauri" crates/engram-dashboard-core/src/`(→0). cdp: `$env:RUST_LOG="debug"; $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223"; npm run tauri dev` → `node scripts/cdp.mjs eval "window.__richslot.spawnJson('.')"` 등.
**검증 안 된 것:** split+assign tag1 고착(후속 버그) · 프로덕션 빌드(StrictMode 미적용) 재현(수정은 dev/prod 안전 설계라 무해 예상, 미실측).

## 기록 미완 (다음 세션 or 후속)
- **step-log 미기록** — 이번 결과(리로드 replay 수정, 근본원인=프론트 레이스, resync 오진 폐기) `docs/process/step-log.md`에 추가 필요.
- **ADR 판단** — resync 백엔드 접근 폐기 + subscribeOutput ready 게이트 도입이 ADR감인지 판단(버그픽스 수준이면 step-log만으로 충분할 수도).

## 앱 상태
CDP 9223 앱 **살아있음**(qa 재기동, background ID `bqdadnyp9`, RUST_LOG=debug, 슬롯 split + 에이전트 다수). 새 프론트(ca3f325 수정) 반영본. 다음 세션 재활용 가능.

## 협업 메모
- 사용자: "쭉쭉/ㄱㄱ/최대한" 강함. 커밋·굵은 결정·순서위험만 올림. 순수 내부 구현·진단 절차는 메인이 정하고 **보고**(결정 떠넘기기 금물 — 세션 초반 지적). 커밋은 명시 승인 필요("ㄱㄱ"≠커밋, 단 "커밋푸쉬해"는 명시 승인).
- 구현 규약: 코더(opus/sonnet)→`/review code`(opus+Codex 다른 family)→`/qa`→커밋 승인 후. 메인 직접 구현 금지. 조사·실측·대량읽기 서브에이전트 위임(결론만).
- 이 세션 서브에이전트 ~22기(진단 8·코더 3·리뷰어 8·qa 2·기타). implement/review/qa/continue 스킬 완주.

## 참조 (읽을 것만)
- **수정 정본:** `src/api/protocolClient.ts`(subscribeOutput 428~·handleOutput ready 게이트 206·SubState.ready 45~) + `protocolClient.test.ts`.
- **후속 버그 착안:** `src/components/slot/{RichSlot,TerminalSlot}.tsx`(effect 재마운트 생명주기) · `src/api/protocolClient.ts`(subscribeOutput/unsubscribe) · 데몬 replay_slots(slot 재배정 경로).
- ADR: 0037(seq dedup)·0007(epoch 재구독)·0043(mount-replay)·0040(서버버퍼)·0035(slot 배정)·0011(agentClient)·0038(근본원인 우선). do-not #5 = ADR-0045/S15 DEFER.
- step-log `docs/process/step-log.md`(이번 결과 미기록).
