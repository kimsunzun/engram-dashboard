# 핸드오프: JSON 렌더 Claude Code 스타일 전환 + 백엔드 입력-즉시 에코 (프리뷰·미커밋·게이트 일부 잔여)

## 한 줄 상태 · 다음 첫 액션
JSON(StreamJson) 슬롯 렌더를 **VS Code Claude Code 스타일**로 재작성(① 프론트)하고, 사용자 입력을 **백엔드가 입력 즉시 에코**(② 백엔드 echo-on-input)하도록 구현. 전부 **미커밋 프리뷰**. tauri dev가 ②를 자동 리빌드·재시작해 라이브 반영됨.
**다음 첫 액션:** JSON 에이전트를 슬롯에 배치 → 메시지 전송해 **② 라이브 실측**(내 메시지가 응답 전에 즉시, 정확히 1번만 뜨는지). OK면 → `/review code`(백엔드 적대 게이트) → 커밋 판단(사용자).

## 사용자 결정 (지속)
- **렌더 = VS Code Claude Code 스타일** (/research medium로 업계 표준 확인 — Cline·Roo·Claude Code 수렴). 정책: text·user·usage = 표시 / thinking·tool = 기본 접힘(클릭 펼침).
- **user 즉시 에코 = 백엔드 echo-on-input(B)**, 프론트 optimistic hack(A) 아님 — 사용자 지정. 근거: 터미널 PTY가 입력을 즉시 에코하는 것과 동일 원리, 프론트는 순수 미러 유지(§5).
- 커밋 = 명시 승인만 (이번 세션 승인 없음 → 전부 미커밋).

## 완료 + repo 상태 (브랜치 master, 전부 미커밋)
**프론트 (HMR 반영됨):**
- `src/components/slot/StructuredTextView.tsx` (+ `structuredTextView.css`) = **활성 라이브 렌더**. cc-* 클래스, Claude Code 스타일. (파일명은 레거시 — 내용은 Claude Code 렌더)
- `src/components/slot/RichSlot.tsx` = LiveRichSlot이 `<StructuredTextView items={items}/>` 사용. `StructuredItemStream` import는 원복 대비 **주석 처리**.
- `src/components/layout/Sidebar.tsx` = "+" 생성 폼에 **JSON 모드(StreamJson) 체크박스** 추가(테스트용 임시 — 정식은 §5 커맨드화, 백로그 M2).

**백엔드 (tauri dev 자동 리빌드·재시작 반영됨):**
- `crates/.../agent/session.rs` = `write_input`에서 `send_input` 성공 후 `encoder.input_echo_event(bytes)`로 emit-on-input.
- `crates/.../agent/backend/mod.rs` = `InputEncoder::input_echo_event` (json → Some(Structured{kind:"user"}), Raw → None).
- `crates/.../agent/backend/claude.rs` = `user_text_echo_json` 헬퍼 + 디코더에서 **user-role `type=="text"` 블록만 억제**(중복 방지), tool_result 등 다른 user-role 블록은 보존.

**오펀(정리 대상):** `src/components/slot/structuredToRichMessages.ts`(chat-layout 실험 잔재, 현재 미사용) · `StructuredItemStream.tsx`(주석 처리됨, 미사용). 이전 스킬 feedback 3개(`.claude/skills/...`)는 이번 작업 무관 미커밋.

## 검증 상태 (쌍)
**돌린 것(PASS):**
- ① `npx tsc --noEmit` clean · cdp 실측(`.cc-stream`/`.cc-text`/`.cc-user`/`.cc-usage`/`.cc-aside` 렌더, thinking 접힘 open:false, 옛 stv-/lay-/si- 0) · 스크린샷 확인.
- ② `cargo test -p engram-dashboard-core` **139 passed 0 failed**(신규 9: emit-on-input json/terminal, 디코더 text 억제, tool_result 회귀가드, 헬퍼 shape) · `cargo fmt --check` clean · `rg "use tauri" core/src` 0 · `cargo check --workspace` clean · tauri dev 자동 리빌드 성공(≈15s, 앱 재시작 running).
**재실행 명령:** `cargo test -p engram-dashboard-core` · `npx tsc --noEmit` · cdp = `node scripts/cdp.mjs eval "..."`(포트 9223).
**검증 안 된 것(정직):**
- ★**② 라이브 cdp 실측 미완**★ — 백엔드 재시작으로 슬롯 레이아웃 리셋(비어있음). 라이브 JSON 에이전트를 슬롯에 붙여 send→**즉시·1회 에코·중복없음** 확인 아직 못 함. (로직은 유닛테스트가 커버하나 end-to-end 스모크 잔여.)
- ★**`/review code`(백엔드 ② 적대 게이트) 미실행**★ — 비자명 백엔드·불변식 인접 → 커밋 전 필수.
- full-workspace `cargo build`는 앱 실행 중 exe 파일락으로 미완(cargo check는 clean이라 컴파일 OK).
- 스트리밍-펼침 규칙(thinking/tool 스트리밍 중 펼침→완료 접힘) 미구현.

## 실패한 접근 (do-not)
- **`--replay-user-messages` 제거로 중복 억제 = 회피.** tool_result·resume(ADR-0008) 영향 미검증 → ADR-0038(추측 금지). 정답 = 디코더에서 user-role `type=="text"` 블록만 억제(fixture 검증).
- **lab `ChatLayout` 직접 적용 = 폐기.** usage/error/separator를 구조적으로 드롭 + lab 팔레트(`--lay-*`)가 앱 테마와 안 맞아 텍스트 흐림. → StructuredItem 직접 렌더(cc-*, 앱 테마 `--text`).
- **user 턴 완전 드롭 = 오해였음.** "그룹핑 필요없다" = 칩 말고 텍스트로 보이라는 뜻 → user 텍스트 표시로 복원.

## 다음 단계 (순서)
1. **② 라이브 실측** (send → 즉시·1회 user 에코, 중복 없음; tool 있으면 tool_result 정상 표시).
2. **`/review code`** — 백엔드 ② 적대 리뷰(디코더 억제·emit·불변식 ADR-0006/0044/0045/0004).
3. **오펀 정리** — structuredToRichMessages.ts 삭제, StructuredItemStream 결정(삭제/보존), StructuredTextView 명명 정합(→ClaudeChatView?).
4. **스트리밍-펼침 규칙** 추가(선택).
5. **커밋 판단**(사용자).

## 참조 (읽을 것만)
- 활성 렌더: `src/components/slot/StructuredTextView.tsx` + `structuredTextView.css` (cc-* 클래스, 접힘 정책).
- 백엔드 ②: `session.rs::write_input` · `backend/mod.rs::input_echo_event` · `backend/claude.rs`(user_text_echo_json + 디코더 억제, `// ADR-0044/0045` 앵커).
- 리서치 결론(포맷 근거): Cline/Roo/Claude Code 공통 = text 크게·thinking/tool 접힘·usage 뱃지·명시 구분선 없음·스트리밍 스피너. optimistic echo는 백엔드 비에코라 dedup 불필요(우리는 백엔드 에코라 디코더 억제로 처리).

## 앱 상태
dev 앱 백그라운드 실행 중(task `bgnqwnub7`, CDP 9223, 백엔드 ② 반영본). 재시작으로 메인 슬롯 비어있음 — 실측하려면 JSON 에이전트를 슬롯에 다시 배치 필요. 종료 무방.
