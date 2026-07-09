# 핸드오프: ADR-0052 json 유저 에코 uuid/isReplay dedup — 구현·review PASS·로컬커밋 완료. 남은 것 = GUI 실측 + push

## 한 줄 상태 · 다음 첫 액션
- **상태:** json(StreamJson) 모드 유저 에코 중복 제거를 **blunt suppress → uuid/isReplay 기반 dedup**으로 교체(ADR-0052). 구현·`/review code deep` PASS(FIX 1회)·코드 게이트 전부 PASS·**로컬 커밋 2개**(`dbced5a` 기능 + `f9d40da` docs). **GUI 실측 미완 → push 보류(동작 미확인).**
- **다음 첫 액션:** 앱 띄워 **GUI 실측** → JSON 모드 에이전트 spawn(Sidebar "JSON 모드 (StreamJson)" 체크박스 or `window.__ENGRAM_AGENT__.spawnProfile`) → 메시지 전송 → **유저 버블이 1개만** 뜨는지(이중 렌더 없음) + tool 호출 시 tool_result(OUT) 보존 확인. OK면 `git push origin master`(커밋 2개). 앱 기동 = `run-dashboard-clean.bat`(데몬 재빌드 포함) 권장, 포트 9223.

## 이 세션 커밋 (로컬 — push 보류)
- `dbced5a` feat(json): 유저 에코 dedup을 uuid/isReplay 기반으로 교체 (6 files, +628/−24)
- `f9d40da` docs(ADR-0052): 결정 박제 + step-log (3 files, +40)
- **push 안 함** — GUI 실측(동작 확인) 미완이라 보류. 실측 PASS면 `git push origin master`.

## 무엇을 했나 (핵심)
- 기존 blunt suppress(user text 무조건 억제)는 json resume 켜면 claude가 되울린 **과거 유저 메시지까지 전부 삭제** → 대화 이력 소실. cross-family(Codex)가 `/review code full`에서 BLOCK.
- 사용자 "OSS는 어떻게 푸나" → `/research medium`: 표준 = correlation-id/pending 매칭. **claude CLI 실측 확정** — `--replay-user-messages`에서 우리 stdin `uuid` 그대로 보존 + `isReplay:true` 태그. **공식 VSCode Claude 확장 설치본 코드 실측** — 메시지 `uuid`로 dedup + transcript 저장.
- 채택(S1) = 우리 생성 uuid를 stdin(`wrap_user_turn`)·합성 에코 양쪽 부착 → decoder는 억제 없이 uuid 실어 통과 → 프론트 `structuredAccumulator`가 **`type=="text"` user 블록만** uuid dedup. tool_result·과거·비매칭·다른 uuid 전부 보존.

## 검증 상태 (쌍으로)
- **돌린 것 PASS:** full workspace `cargo build` · `cargo test -p engram-dashboard-core`(144+통합) · `-p engram-dashboard-protocol`(42) · `cargo fmt --check` · 격리(`rg "use tauri" crates/engram-dashboard-core/src/` → 0 real) · `npx tsc --noEmit` · `npm test`(vitest 277). 재실행 = 이 명령들.
- **uuid 왕복 실측 PASS:** `echo '{"type":"user","message":{...},"uuid":"X"}' | claude -p --output-format stream-json --input-format stream-json --replay-user-messages --verbose` → 되돌아온 `type:user` 라인에 `uuid:X` + `isReplay:true`.
- **미검증(핵심 잔여) = GUI 실측:** 실제 화면에서 유저 메시지 1개 렌더(dedup 동작) 확인 안 됨. **자율 백그라운드 `npm run tauri dev` launch가 안 떠서 미수행**(engram 프로세스 안 뜸·9223 리스너 없음). 사람이 앱 띄워 확인 필요.
- **주의(무시 가능):** workspace 전체 `cargo test`는 src-tauri lib 테스트 `STATUS_ENTRYPOINT_NOT_FOUND(0xc0000139)`로 실패 = **안 건드린 crate의 DLL 엔트리포인트 환경성 이슈**(논리 실패 X). CLAUDE.md 정본 테스트 명령은 per-crate(`-p core`·`-p protocol`)이며 그건 PASS.

## 함정 (do-not)
- **dedup 키 = `type=="text"` user 블록의 client uuid만.** `extractUserUuid`(structuredAccumulator.ts)를 **uuid 단독 키로 되돌리지 말 것** — multi-block(같은 uuid의 text+tool_result)에서 tool_result 소실(리뷰 3인 일치 적출한 결함).
- **합성 에코와 stdin 메시지는 반드시 같은 uuid**(`write_input`이 `Uuid::new_v4()` 하나를 양쪽 주입). 어긋나면 이중 렌더.
- **ADR-0004 격리:** claude json 스키마(uuid/isReplay 위치)는 `backend/claude.rs`에만. session/mod는 불투명 `Uuid` 토큰만.
- **GUI 실측 전 push 금지**(동작 미확인).
- 관찰(비차단·지금 조치 X): `seenUserUuids` 단일 epoch 무한 증가(reset마다 비움, 저위험).

## repo 미커밋 (이 라운드 밖 — 커밋 말 것)
- `run-dashboard-clean.bat`(데몬 재빌드 dev 스크립트) · `docs/reference/architecture-overview.md`(미추적 초안). 세션 시작부터 미커밋, 이 기능 무관.

## 참조 (읽을 것만)
- `docs/decisions/0052-*.md`(결정 + 거부 대안 A blunt/B backend pending-set/C echo 제거) · step-log 2026-07-07 섹션
- 코드: `src/components/slot/structuredAccumulator.ts`(extractUserUuid·seenUserUuids) · `crates/engram-dashboard-core/src/agent/backend/claude.rs`(wrap_user_turn·consume_block) · `backend/mod.rs`(input_echo_event) · `agent/session.rs`(write_input)

## rate limit 참고
- 실측 프로브 1회에 7일 quota **0.85**(경고 임계 0.75 초과, `allowed_warning`·오버리지 아님). GUI 실측도 claude 세션 1개 필요 — quota 유의(막히면 실측 보류·코드 게이트만으로 판단).
