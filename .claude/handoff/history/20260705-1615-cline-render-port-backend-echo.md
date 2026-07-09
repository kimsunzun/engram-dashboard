# 핸드오프: JSON 렌더 = Cline 실코드 이식 완료 + 백엔드 입력-즉시 에코 확정 (프리뷰·미커밋)

## 한 줄 상태 · 다음 첫 액션
JSON(StreamJson) 슬롯 렌더를 **실제 Cline 코드(github.com/cline/cline, 로컬 클론)에서 이식**하고, 사용자 입력을 **백엔드가 입력 즉시 에코**(②)하도록 완성. **② 라이브 확정**(데몬 리빌드 후 내 메시지 즉시 뜸 확인), **툴 렌더도 실측 통과**(Error 뱃지·tool_result 병합). 전부 **미커밋 프리뷰**.
**다음 첫 액션:** 사용자 계획 = **"다 따라하기 완료 → 그 이후 수정"**. 즉 이제부터 Cline 베이스 위에서 커스터마이즈. 커밋 전엔 `/review code`(백엔드 ② + 렌더) 필수.

## ★핵심 함정 (진단으로 확정 — 시간 크게 날림, 반드시 기억)★
- **백엔드/Rust 변경은 `tauri dev`로 반영 안 됨.** `beforeDevCommand=npm run dev`(프론트만), `cargo run`(클라이언트 셸 `engram-dashboard.exe`만) — **데몬 바이너리(`engram-dashboard-daemon.exe`) 안 만듦.** 에이전트 I/O는 데몬에서 돌고(ADR-0029), `ensure_daemon`이 살아있는 호환 데몬 재사용 → **옛 데몬에 계속 붙어 Rust 변경이 무반영.**
- **해결:** 백엔드 변경 후 **`run-dashboard-clean.bat`으로 재기동**(이 배치에 `cargo build -p engram-dashboard-daemon` 넣어둠). 프론트만 바꿀 땐 HMR이라 그냥 두면 됨.
- ②가 "안 되는 것처럼" 보인 실제 원인이 이거였음(코드는 정상, 실행 데몬이 stale — reviewer-deep 진단).

## 사용자 결정 (지속)
- **렌더 = 실제 Cline 코드 이식**(흉내/근사 금지 — 여러 번 지적받음). Claude Code CLI/앱은 클로즈드라 못 베낌 → Cline(Apache-2.0)이 "VS Code의 Claude 에이전트 UI" 정본.
- **user 즉시 에코 = 백엔드 echo-on-input(B)**, 프론트 optimistic hack 아님 — 터미널 PTY 에코와 동일 원리, 프론트는 순수 미러 유지(§5).
- 커밋 = 명시 승인만.

## 완료 + repo 상태 (브랜치 master, 전부 미커밋)
**Cline 클론:** `I:\Engram_Workspace\references\cline`(git 추적 밖). 참조 컴포넌트 = `apps/vscode/webview-ui/src/components/chat/{ChatRow,UserMessage,ThinkingRow,RequestStartRow,MarkdownRow}.tsx` + `common/{CodeAccordian,MarkdownBlock}.tsx`.

**프론트 (HMR 반영, cl-* 클래스):**
- `src/components/slot/StructuredTextView.tsx` (+ `structuredTextView.css`) = **활성 라이브 렌더 = Cline 이식**. 매핑: user박스(UserMessage)·assistant마크다운(MarkdownRow)·thinking(ThinkingRow, 내용없으면 숨김)·tool(CodeAccordian, tool_result를 tool_use_id로 병합+에러/성공 색)·usage뱃지(RequestStartRow). Apache-2.0 출처 주석 있음. (파일명 레거시 — 내용은 Cline 렌더)
- `src/components/slot/RichSlot.tsx` = `<StructuredTextView items={items}/>` 사용. `StructuredItemStream` import 주석.
- `src/components/layout/Sidebar.tsx` = "+" 폼에 JSON모드(StreamJson) 체크박스(임시 — 정식은 §5 커맨드화, 백로그 M2).

**백엔드 ② (데몬 리빌드 반영):**
- `crates/.../agent/session.rs::write_input` = send_input 성공 후 `encoder.input_echo_event(bytes)`로 emit-on-input.
- `crates/.../agent/backend/mod.rs::InputEncoder::input_echo_event` (json→Some(Structured{kind:"user"}), Raw→None).
- `crates/.../agent/backend/claude.rs` = `user_text_echo_json` + 디코더에서 **user-role `type=="text"` 블록만 억제**(tool_result 등 보존). ※tool_result는 user-role로 와서 `structured{label:"user", json:{type:"tool_result",...}}`로 흐름 → 프론트가 json.type으로 구분해 tool 박스에 병합.
- `run-dashboard-clean.bat` = 데몬 리빌드 스텝 추가.

**오펀(정리 대상):** `src/components/slot/structuredToRichMessages.ts`(chat-layout 실험 잔재, 미사용) · `StructuredItemStream.tsx`(주석 처리, 미사용). 스킬 feedback 3개는 무관 미커밋.

## 검증 상태 (쌍)
**돌린 것(PASS):** ② `cargo test -p engram-dashboard-core` 139 passed(신규 9) · fmt clean · use-tauri 0 · **② 라이브 확정**(메시지 즉시 뜸). ① `npx tsc --noEmit` clean · cdp 실측(user박스·tool accordion·Error 뱃지·tool_result 병합 `rawToolResult:0 toolBoxes:4`·빈 thinking 숨김·옛 cc-/stv- 0). 툴 데모(Read/Glob/PowerShell) 실렌더 스크린샷 확인.
**재실행:** `cargo test -p engram-dashboard-core` · `npx tsc --noEmit` · cdp = `node scripts/cdp.mjs eval/shot`(9223).
**검증 안 된 것(정직):**
- **`/review code`(백엔드 ② + 렌더 적대 게이트) 미실행** — 커밋 전 필수.
- 코드 블록 문법 하이라이팅 미구현(Cline은 rehype-highlight — dep 필요, follow-up).
- 스트리밍-펼침 규칙(스트리밍 중 펼침→완료 접힘) 미구현.
- full-workspace `cargo build`는 앱 실행 중이면 exe 파일락(clean.bat이 데몬만 리빌드).

## 실패한 접근 (do-not)
- **백엔드 변경 후 데몬 리빌드 없이 디버깅** = 위 핵심 함정. 코드 의심 전에 데몬 stale부터 의심.
- **"VS Code 스타일"을 직접 CSS 근사(cc-*)** = 사용자 반복 지적. → 실제 Cline 코드 이식(references/cline).
- **`--replay-user-messages` 제거로 중복 억제** = 회피(tool_result/resume 미검증, ADR-0038).
- **lab ChatLayout 직접 적용** = 폐기(usage/error/separator 구조 드롭 + 팔레트 흐림).
- user 턴 완전 드롭 = 오해였음(칩 말고 텍스트로 보이라는 뜻).

## 다음 단계 (순서)
1. **커스터마이즈** — 사용자 "그 이후 수정" 단계. Cline 베이스 위에서.
2. 코드 문법 하이라이팅(dep 추가) + 스트리밍-펼침 규칙(선택).
3. **`/review code`** — 백엔드 ②(디코더/emit/불변식 ADR-0006/0044/0045/0004) + Cline 렌더.
4. 오펀 정리(structuredToRichMessages 삭제, StructuredItemStream 결정) + StructuredTextView 명명 정합(→ClineChatView?).
5. 커밋 판단(사용자).
6. (별개) 테스트 에이전트 cwd가 `I:\` 루트라 프로젝트 파일 못 찾음 — 에이전트 프로필 cwd 설정 이슈.

## 참조 (읽을 것만)
- 활성 렌더: `src/components/slot/StructuredTextView.tsx` + `.css` (cl-* 클래스, tool_result 병합·에러색·skip-empty).
- Cline 원본: `I:\Engram_Workspace\references\cline\apps\vscode\webview-ui\src\components\{chat,common}\`.
- 백엔드 ②: `session.rs::write_input` · `backend/mod.rs::input_echo_event` · `backend/claude.rs`(user_text_echo_json + 디코더 억제, `// ADR-0044/0045` 앵커).

## 앱 상태
`run-dashboard-clean.bat`으로 실행 중(CDP 9223, 데몬 ② 반영본). 슬롯에 "Json" 에이전트(Running) 배치돼 대화+툴데모 남아있음. 프론트=HMR, 백엔드=clean.bat 재기동. 종료 무방.
