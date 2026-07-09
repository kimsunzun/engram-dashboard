# 핸드오프: 채팅 슬롯 오버레이 스크롤바+헤더제거 커밋 완료 — 다음 = 채팅 대기표시 재설계(Thought 제거+Wait) & 레이아웃

## 한 줄 상태 · 다음 첫 액션
- **상태:** 채팅 슬롯(RichSlot) UI 정비 완료·커밋(`af2063b`). 컨텍스트 절반 넘어 깨끗한 지점서 핸드오프.
- **다음 첫 액션:** 사용자 우선순위 대기 — 두 갈래: **(A) 채팅 대기표시 재설계**(아래 ★설계 방향) · **(B) 레이아웃 착수**. 사용자가 정함.

## 이번 세션 커밋 (master · 로컬만, 미푸시)
- `ab536f1` docs: continue→handoff 참조 정정(CLAUDE.md:16 + step-log:4)
- `0edf28e` docs(ADR-0053): 채팅 슬롯 오버레이 스크롤바 = Radix ScrollArea 결정 박제
- `af2063b` feat(chat): 오버레이 스크롤바(Radix type=scroll) + 헤더 제거 + 렌더 분할

## 완료 (커밋됨)
- **스킬 continue→handoff 개명 여파 정리:** CLAUDE.md:16 + step-log:4 정정(내 몫). global-rules:34 + REVIEW-NOTES 4개는 skill-lab가 처리(`b330806`, orchestra 분담). `.claude/skills`는 심링크 확인(SSOT drift 우려 철회). **+ 이 핸드오프 세션서 `.claude/continue`→`.claude/handoff` 폴더 이동 + `.claude/skill-bindings/handoff.md` 바인딩 신설**(사용자 결정).
- **ADR-0053 채팅 슬롯 오버레이 스크롤바** (`/research`→`/adr`→`/implement`→`/review`→GUI 실측 전 과정):
  - 헤더 "JSON ● idle" **제거**.
  - 스크롤 컨테이너 = Radix ScrollArea `ChatScrollArea` seam. **`type="scroll"`** + scrollHideDelay=500(스크롤 시에만 표시·멈추면 0.5s 뒤 숨김). overlay 공간0, 테마별 `--scrollbar-thumb`, auto-scroll은 seam이 Viewport를 forwardRef로 노출해 보존.
  - StructuredTextView(643줄) **분할**: `chat/railPositions.ts`(순수 util) + `chat/ChatRow.tsx`(leaf). 미사용 `StructuredItemStream`은 참조(자체 테스트·롤백 주석) 있어 미삭제.
  - 순서목록 2자리 숫자 좌측 잘림 수정(`chat/chat.css` ol padding 2.2em).

## ★다음 세션 설계 방향 — 채팅 대기표시 재설계 (사용자 지시)★
- **"Thought" 행 제거** — opus의 빈 "Thought(내용 비공개)"는 매 응답 붙는 낭비 clutter. 없앤다.
- **send 시 transient "Wait…"류 플레이스홀더** — 전송~응답 사이 그 자리에 대기 표시.
- **응답 콘텐츠 도착 시 "Wait…" 삭제/교체.**
- 효과: 지속 clutter 제거 + "메시지 바닥 flush" + "send 즉시 인디케이터 안 뜸" 3가지 동시 해결. **dead 패딩 불필요**(사용자 명시 거부).
- 관련 코드: `src/components/slot/chat/ThoughtRow.tsx`(Thought/Thinking… 렌더) · `StructuredTextView.tsx:497-522`(`showTail = streaming && hasContent`, 스트림 tail) · `RichSlot.tsx:150,165`(awaiting/streaming 파생 — `streaming = awaiting || (!turnDone && items.length>0)`).

## 검증 상태 (쌍으로)
- **돌린 것 (PASS):** tsc 0 · vitest 279/279 · 코어 격리(`rg "use tauri" crates/engram-dashboard-core/src` → lib.rs:9 주석 1건=문서화된 가짜양성) · `/review code full`(reviewer-deep+Codex, F1 테마색 FIX 반영) · Codex 최종 PASS · GUI 실측(cdp+사용자 실조작: 헤더제거·overlay공간0(gutterPx 0)·자동스크롤 하단고정·스크롤시표시·숫자온전). 재실행: `npx tsc --noEmit` · `npm test`.
- **미검증/안 됨:** workspace 전체 `cargo test`는 프론트 전용 변경이라 미실행(Rust 미변경, 스코프 밖). **send 즉시 "Thinking…" 안 뜸** — cdp 실측서 전송 후 130/700ms 둘 다 hasThinking:false. awaiting(RichSlot:150)이 실 유저 send에서 뜨는지 미확정(cdp 합성 send가 실 send() 경로 안 탔을 수 있음). → 위 재설계로 대체 예정이라 별도 디버깅보다 재설계 권장.

## 실패한 접근 (do-not)
- **하단 dead 패딩으로 flush 해결** = 사용자 거부(공간 상시 낭비). pb-4 넣었다 pb-3 원복함.
- **Radix hover/scroll 표시 상태를 CDP로 검증** = 재현 불가. hover=합성 pointer 무시, scroll=신뢰 wheel도 content만 스크롤될 뿐 scrollbar 상태전이 안 됨. **Radix 상호작용 표시 실측은 사용자 실조작으로만** — 다음 세션도 cdp로 스크롤바 표시 검증 시도 말 것(구조·CSS·auto-scroll은 cdp로 측정 가능, 표시 토글만 불가).
- **type="scroll"을 hover로 되돌려 thumb 드래그 살리기** = 사용자가 "화면 전체 뜸" 반려. **되돌리지 말 것**(드래그 불가는 사용자 수용). 원래 원했던 건 C안(우측 가장자리 근처 hover)이나 A안(scroll 표준) 수용.

## repo 상태
- 브랜치 master. 이번 3커밋 **로컬만, 미푸시**(원하면 push).
- **미커밋 이월(이 작업 밖 — 커밋 말 것):** `run-dashboard-clean.bat`(M) · `docs/reference/architecture-overview.md`(untracked). 세션 시작부터 존재.
- `.claude/handoff/`·`.claude/skill-bindings/handoff.md` = 미추적(핸드오프 데이터·바인딩, gitignore 대상 추정).

## 앱 상태
- dev 앱 실행 중(background 태스크 `bc4nzk35b`, CDP 9223, 빌드=af2063b). JSON 구조화 에이전트(`8e28e25e`) 슬롯 배정+테스트 메시지 있음 — 재사용 or 재기동. ★주의: 포트 1420 스테일 Vite 있으면 kill 후 재기동(이번 세션 겪음: `Get-NetTCPConnection -LocalPort 1420` → Stop-Process).

## 레이아웃 (다음 큰 주제)
- 사용자 "다음에 레이아웃 가보자". 큰 설계 영역 → dev-step(PRD/TRD·**사용자 결정**) 밟아야. backlog: 멀티뷰/split · 창 레이아웃 영속화(저장위치=프론트 localStorage 결정됨, tracking D-7) · §5 LLM 제어 표면(command 버스, ADR-0022 제안) · ADR-0014(오케스트레이션 제안). 착수 전 사용자와 스코프 확정.

## 참조 (읽을 것만)
- ADR-0053 `docs/decisions/0053-*.md` · step-log 2026-07-07~08 "채팅 슬롯 UI 정비" 섹션
- 재설계 코드 포인터 = 위 ★설계 방향 절.
