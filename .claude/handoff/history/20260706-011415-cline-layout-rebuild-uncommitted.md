# 핸드오프: 채팅 렌더 Cline 포트 — 레이아웃 Cline 구조로 재구성(미커밋·게이트 미실행) · 마크다운 실측 검증 · usage칩 제거 등 후속

## 한 줄 상태 · 다음 첫 액션
- **상태:** Cline 잎 포트+dispatch는 **커밋·푸시 완료(4ef0fe1)**. 그 위 **레이아웃을 Cline 구조로 재구성 + MarkdownBlock 견고성 수정 = 전부 미커밋(디스크 저장됨, /review·/qa 미실행)**. 마크다운 렌더는 GUI 실측으로 검증됨(표·코드·제목 정상).
- **다음 첫 액션:** ① **usage 칩 제거**(아래 사용자 피드백) → ② 미커밋 변경 전체를 `/review code` + `/qa` 거쳐 커밋.

## 사용자 핵심 피드백 (놓치면 또 헛발질 — 최우선)
- **★"프론트 화면 갈아끼우려는데 화면 안 바뀌고 내부만 바뀜"★** — 첫 커밋(4ef0fe1)은 렌더 엔진(Cline MarkdownBlock)·dispatch 토대라 **겉모습 거의 무변화**였음. 원인: Cline의 *보이는 룩*을 쥔 `ChatRow`가 VSCode gRPC 강결합이라 **verbatim 복사 불가** → 복사 가능한 건 *안 보이는* 잎 렌더러뿐. "잎만 포트=레이아웃은 내 손디자인 유지=Cline처럼 안 보임". 이걸 결정 때 세게 안 짚은 게 실수.
- **★usage 칩 제거★** — per-message 토큰 표시 `in X · out Y`는 **Cline이 안 하는 것**. 우리 `usage` 칩임. Cline 충실이면 제거(StructuredTextView `case 'usage'`).
- **레이아웃/dispatch는 우리가 Cline 구조 *참조해 재구성***(복붙 아님, ChatRow 복사불가라). 뽑아둔 Cline 구조: 행 `relative pt-2.5 px-4` · 헤더 `HEADER_CLASSNAMES="flex items-center gap-2.5 mb-3"`(아이콘 size-2 + bold 제목) · 툴 `bg-code rounded-sm border` 박스(클릭 헤더 `py-2 px-2.5`) · 유저 `p-2.5 my-1 rounded-xs` badge박스 · 어시스턴트 = MarkdownRow(헤더 없음). (원본: `I:\Engram_Workspace\references\cline\apps\vscode\webview-ui\src\components\chat\ChatRow.tsx`·`UserMessage.tsx`)
- 사용자 "이상한데 일단 핸드오프" — 아직 완전히 Cline스럽지 않다고 느낌. 다듬을 후보: **usage칩 제거**, 유저 박스 스타일, 툴 블록 실측, 간격/색.

## 완료 + repo 상태 (브랜치 master)
- **커밋·푸시됨:** `4ef0fe1` (origin/master, 8b06953..4ef0fe1) — Cline 잎 verbatim/적응 포트(MarkdownBlock·MarkdownRow·ThinkingRow·CopyButton·ui/button) + StructuredItem→dispatch(StructuredTextView, InertCode 안전렌더) + Apache-2.0 귀속(LICENSES/cline-Apache-2.0.txt+헤더) + send() awaiting 버그픽스 + deps 7 + ADR-0048(ADR-0047 채팅조항 부분폐기). 커밋 메시지에 "레이아웃은 후속" 명시.
- **미커밋 — 이번 세션 작업(디스크 저장됨, 게이트 미실행):**
  - `src/components/slot/StructuredTextView.tsx` — **레이아웃 Cline 구조 재구성**: 점선 레일 제거 → flat 스택(`pt-2.5 px-4`), RowHeader(아이콘+bold), 유저 badge박스, 툴 `bg-surface rounded-sm border` 박스, MarkdownRow. usage는 아직 muted 칩(제거 대상).
  - `src/components/slot/StructuredTextView.test.tsx` — 구조 테스트 갱신(레일 없음·유저박스·툴헤더).
  - `src/components/slot/cline/MarkdownBlock.tsx` — **견고성 수정**: `parseMarkdownIntoBlocks`(marked 블록쪼개기) 제거 → 단일 `<ReactMarkdown>` + block `<div>`(옛 inline span 제거); `stripZeroWidth()`(U+200B/200C/200D/2060/FEFF) sanitize 추가.
  - `src/components/slot/cline/MarkdownBlock.test.tsx` — **신규**(제목+표+코드 렌더 + U+200B 케이스).
  - `THIRD_PARTY_NOTICES.md` — **신규**: Cline 출처 고정 = `cli-v3.0.37` commit `25ef0939` + 포트 파일 표 + 재동기 절차.
- **미커밋 — 무관, 건드리지 말 것:** backend echo(`agent/backend/claude.rs`·`mod.rs`·`agent/session.rs`), `Sidebar.tsx`, `run-dashboard-clean.bat`, `structuredToRichMessages.ts`(오펀), skills feedback 3개, `phase2-*.png`(스샷 아티팩트).

## 검증 상태 (쌍)
- **돌린 것(PASS):** `npx tsc --noEmit` clean · `npx vitest run` **242** · `npm run build` 성공(마지막 코더 라운드 기준). **GUI 실측(cdp):** 레이아웃 = 점선레일 제거·Cline flat 구조 렌더 확인. 마크다운 = **자연 프롬프트로 표(remark-gfm)·python 코드하이라이트·제목 정상 렌더 실측**(스샷 `phase2-md-works.png`).
- **재실행:** `npx tsc --noEmit` · `npm test` · `npm run build` · cdp = `node scripts/cdp.mjs eval|shot`(포트 9223, 앱 재기동 필요).
- **검증 안 된 것(정직):** 미커밋 변경에 **`/review code`·`/qa` 게이트 미실행**. 툴 블록·thinking 행 **라이브 미실측**(코드/테스트만 — 이 테스트대화엔 툴/thinking 없음). usage 칩 제거 **미반영**.

## 실패한 접근 / 함정 (do-not)
- **★마크다운 렌더 테스트 프롬프트 함정(큰 시간낭비 원인)★:** "마크다운만 출력해줘" 류 = claude가 응답 전체를 ` ```markdown ` 코드펜스로 래핑 → react-markdown이 **정확히** 코드블록(마크다운 소스)으로 렌더 = raw처럼 보임(**버그 아님**). 부산물로 중첩펜스 앞 U+200B(ZWSP)도 생김. **마크다운 렌더 검증은 자연 프롬프트로**("표로 정리해줘, 코드예제 보여줘, 코드블록으로 감싸지 말고 답변에 포함"). 이 함정으로 멀쩡한 엔진을 오래 헛디버깅함 → 다음 세션 반복 금지.
- **앱 재기동 함정:** 창 X=hide라 창 닫아도 프로세스(`engram-dashboard.exe`) 살아있고 vite(node)가 포트 1420 점유 → 재기동 시 "Port 1420 in use"로 실패. **재기동 전 kill**: `Get-NetTCPConnection -LocalPort 1420 -State Listen`의 OwningProcess(vite) + `Get-Process engram-dashboard` 둘 다 Stop-Process. (다른 node 무차별 kill 금지 — 머신에 남 세션 node 많음, 포트로 특정.)
- **HMR/reload 불안정:** MarkdownBlock 수정 후 HMR이 메모된 기존 아이템 재렌더 안 하거나 스테일. 확실히 하려면 **dev 서버 재시작**. 서빙 코드 확인 = `curl http://localhost:1420/src/components/slot/cline/MarkdownBlock.tsx | grep stripZeroWidth`.
- **★라이브 렌더 확인법★:** 우리 렌더러(StructuredTextView)는 **라이브 RichSlot에서만** 보임 = JSON 에이전트 슬롯 배치 필요(`__richslot.mount`=fixture라 우리 렌더 아님). 절차: `list_views`→view id→`get_view`→slot id(**재기동마다 UUID 바뀜**) · agentId=`window.__engram.agent.getState().agents`에서 name="Json"(`181e99d7`, 데몬 persist) · `invoke("assign_agent",{viewId,slotId,agentId})`(slotId=UUID) · 메시지=textarea(placeholder "메시지 입력"=배열 [0]번, [2]번 아님) native value setter+input 이벤트 → "전송" 버튼 click · **mount replay 빈 버그로 배치 직후 공백 → 새 메시지 보내야 참**.

## 다음 단계 (순서)
1. **usage 칩 제거** — StructuredTextView `case 'usage'` (Cline 미표시).
2. 미커밋 변경 전체(레이아웃+MarkdownBlock+NOTICES+usage제거)를 **`/review code full` + `/qa`** 거쳐 **커밋**(사용자 승인).
3. 툴 블록·thinking 행 **라이브 실측**(툴 쓰는 대화/thinking 유도 어려우면 fixture 경로 고려).
4. 사용자와 화면 디테일 조율(유저박스·간격·색 — "완전 Cline"까지).
5. **replay 빈 버그** 별도 조사(ADR-0046 view-direct replay).

## 앱 상태
dev 서버 백그라운드 실행 중(task `bs9b75cfg`, CDP 9223). Json 에이전트 슬롯 배치·테스트 대화(표/코드 렌더 확인분) 있음. 종료 무방(다음 세션 위 "재기동 함정"대로 재기동).

## 참조 (읽을 것만)
- 우리: `src/components/slot/StructuredTextView.tsx`(dispatch·레이아웃) · `cline/MarkdownBlock.tsx` · `structuredAccumulator.ts`(StructuredItem 모델·불변).
- `docs/decisions/0048-*.md` · `THIRD_PARTY_NOTICES.md`.
- Cline 원본(레이아웃 참조): `I:\Engram_Workspace\references\cline\apps\vscode\webview-ui\src\components\chat\{ChatRow,UserMessage}.tsx`.
