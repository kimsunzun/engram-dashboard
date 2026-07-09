# 핸드오프: 채팅 UI = Cline 실코드 verbatim 포트 결정 (Phase1 Tailwind 커밋 완료 · Phase2 첫 cut 미커밋·폐기예정)

## 한 줄 상태 · 다음 첫 액션
- **상태:** Tailwind 도입(ADR-0047) **커밋 완료(4d34c45)**. 내가 만든 CC-룩 "첫 cut"(손디자인)은 미커밋·검증됨이나 **폐기 예정** — 사용자 결정으로 **Cline 실제 코드를 verbatim 포트**(근사 금지, 실코드 복사)하기로 확정.
- **다음 첫 액션:** Cline 채팅 컴포넌트 verbatim 포트 실행 — `/implement`로 코더 스폰(아래 "다음 단계" 순서). 착수 전 Cline 파일/헤더/NOTICE 유무 확인.

## 사용자 결정 (지속)
- **채팅 렌더 = Cline 실코드 verbatim 포트 + 우리 데이터 어댑터.** 내 손디자인 근사 = 반복 지적받음 → 금지. "완전히 따라하기 → 거기서부터 커스텀."
- **Tailwind 도입 확정(ADR-0047)** — 이제 Cline의 Tailwind/shadcn/lucide 코드를 **근사 없이 그대로** 옮길 수 있음(이게 리서치의 "네이티브 구현" 추천 전제[= 우리가 Tailwind 안 씀]를 뒤집음).
- **Apache-2.0 귀속(verbatim 복사 시):** 저작권 헤더 유지 + 변경 파일에 "Modified by…"(§4b) + **LICENSE 전문 동봉(필수)** + NOTICE(**Cline에 NOTICE 있으면 필수·없으면 옵션 — 유무 미확인, 포트 시 확인**).
- **법무 게이트 = 지금은 스킵(사용자 결정):** "개인 로컬 사용, 공개(업로드) 시점에 직접 검토." org/CLAUDE.md는 verbatim=법무 확인을 권고하나 사용자가 개인 개발로 판단·공개 전 검토로 연기.
- **Thought for N seconds(소요시간) = 미룸**(스트림 아이템에 타이밍 없음 → 백엔드 변경 필요). 대신 **"Thinking …" 라이브 애니 표시**(생각 중 신호)는 유지·구현.
- 커밋 = 명시 승인만.

## 완료 + repo 상태 (브랜치 master)
- **커밋됨:** `4d34c45` — ADR-0047 Phase1: `@theme inline`로 data-theme 3종→Tailwind 색토큰 매핑 + cn 유틸 + lucide-react. (테마 전환 cdp 실측 확인.)
- **미커밋 — Phase2 첫 cut(검증됨이나 폐기예정):**
  - `src/components/slot/StructuredTextView.tsx` — 내 CC-룩 손디자인(점선 타임라인 Row·접힘 ThinkingRow·ToolRow IN/OUT·LiveThinkingRow "…"). **Cline 포트로 교체 예정**(또는 fallback 참조).
  - `src/components/slot/RichSlot.tsx` — Tailwind 재스타일 + `streaming` prop 배선. **구독/누산/send 데이터흐름 무변경(ADR-0044/45/46).**
  - `src/components/slot/structuredTextView.css` — **삭제됨**.
  - `tsconfig.json`·`vite.config.ts`·`vitest.config.ts` — `@/*`→`src/*` alias. **keeper(Cline 포트도 씀).**
- **미커밋 — 이전 세션(무관, 건드리지 말 것):** backend echo(`agent/backend/claude.rs`·`mod.rs`·`agent/session.rs`), `Sidebar.tsx`, `run-dashboard-clean.bat`, `structuredToRichMessages.ts`(오펀), 스킬 feedback 3개.

## 검증 상태 (쌍)
- **돌린 것(PASS):** Phase1 = `/review code full`(Advocate+Codex) + `/qa full`(tsc·vitest 214·vite build·코어격리·cdp 테마 실측). Phase2 첫 cut = `npx tsc --noEmit`·`npm test`(214)·`npm run build` PASS + **cdp 실측**(Json 에이전트 슬롯 배치→CC 룩 렌더 확인: 사용자메시지 박스·assistant 마크다운·Read[Error 배지]·Glob·"Thinking…" 라이브·좌측 점선 레일).
- **재실행:** `npx tsc --noEmit` · `npm test` · `npm run build` · cdp = `node scripts/cdp.mjs eval|shot`(포트 9223).
- **검증 안 된 것(정직):** Phase2 첫 cut은 `/review code`·`/qa` 게이트 **미실행**(어차피 폐기). **Cline verbatim 포트 = 미착수.** replay 빈 문제(아래) 미해결.

## 실패한 접근 / 함정 (do-not)
- **내 손디자인 CC-룩 근사** = 사용자 반복 지적 → **verbatim Cline 포트**로 확정.
- **Claude Code 확장 복사 시도 금지** = 폐쇄소스(All Rights Reserved, 복사 불가). 복사 가능한 실코드 = **Cline뿐**(Apache-2.0, 클론 `I:\Engram_Workspace\references\cline`).
- **★라이브 렌더 확인법(시간 크게 절약)★:** 앱은 `npm run tauri dev`(env `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223`)로 기동. **우리 렌더러(StructuredTextView)는 라이브 RichSlot에서만 보임 = JSON 에이전트가 슬롯에 배치돼야 함.** "JSON 스파이크" 버튼(FixtureRichSlot)은 lab 레이아웃이라 우리 렌더러 아님. 배치법: `invoke("get_view",{viewId})`로 슬롯 **UUID** 얻어 `invoke("assign_agent",{viewId, slotId, agentId})`. **slotId는 정수(1) 아니라 UUID** — 정수면 "expected UUID string" 에러. 현재 값: agent=`181e99d7-c8f9-4760-83d6-c6b1cac8051b`, view=`69f9e9d9-32ff-4158-9e14-f0633942e9e4`, slot=`29ac2fa7-7029-4be2-b231-1f90f84fe332`. 메시지 전송은 textarea에 native value setter+input 이벤트 후 "전송" 버튼 click(별도 eval, React 리렌더 후).
- **`vite.config.ts` 변경 → dev 서버 재시작 필요**(Vite가 자동 재시작하기도 함; 안 되면 수동).
- Cline은 React18·shadcn ui/button·heroui·styled-components 혼용 — verbatim 포트 시 우리 데이터모델(StructuredItem)로 **어댑터** 필요(컴포넌트가 ClineMessage 소비). "룩(JSX+Tailwind) 복사 + props 어댑터"가 실제 방식.

## 버그/미결 (별건)
- **마운트 시 이전 대화 replay가 빔** — 데몬에 에이전트 Running인데 히스토리 replay 안 됨(새 메시지 보내야 내용 참). ADR-0046 view-direct replay 경로 별도 조사 필요.

## 다음 단계 (순서)
1. (선택) **Cline 포트 계획 + 귀속 감사** 서브에이전트: 복사할 Cline 파일 목록 · 각 헤더 · **Cline NOTICE 유무 확정** · StructuredItem→props 어댑터 설계 · 붙일 귀속 세트(LICENSE/NOTICE/변경헤더) 초안.
2. **verbatim 포트 실행** — `/implement standard|critical`: Cline 채팅 컴포넌트(`chat/ChatRow`·`ThinkingRow`·`CommandOutputRow`, `common/CodeAccordian`·`MarkdownBlock`, `ui/button`) JSX+Tailwind **그대로 복사** + StructuredItem 어댑터 + **Apache-2.0 귀속 부착**(헤더 유지·LICENSE 동봉·NOTICE·변경표시). → `/review code full` → `/qa full`(cdp, 위 배치법으로 실측).
3. 첫 cut `StructuredTextView`는 포트가 대체(또는 참조로 남김).
4. **ADR 작성** — "채팅 렌더 = Cline verbatim 포트" 결정(거부 대안: 손디자인 네이티브·Claude Code 직접복사) — ADR-0047 amend 또는 신규 채번(`/adr`).
5. replay 빈 버그 별도 조사.
6. 커밋(사용자 승인). 공개 전 법무 검토는 사용자가 공개 시점에.

## 참조 (읽을 것만)
- Cline 원본: `I:\Engram_Workspace\references\cline\apps\vscode\webview-ui\src\components\{chat,common}\` + `ui/button`.
- 우리: `src/components/slot/{StructuredTextView.tsx(첫cut·교체예정), RichSlot.tsx, structuredAccumulator.ts(StructuredItem 모델·불변)}` · `src/lib/utils.ts`(cn) · `src/index.css`(@theme).
- `docs/decisions/0047-*.md` · CLAUDE.md 기술스택(프론트).

## 앱 상태
`npm run tauri dev` 백그라운드 실행 중(task `bjbtddf4a`, CDP 9223). Json 에이전트가 슬롯 배치돼 첫 cut CC 룩 렌더 중(방금 보낸 대화 있음). 종료 무방(다음 세션 재기동 가능).
