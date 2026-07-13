# 핸드오프: ADR-0078 재정의(렌더 모드 = 생성 시 고정) 구현·검증·커밋·푸쉬 **완료**(origin/master `863f9ec`). per-activation 시도는 `stash@{0}` 보존(폐기 여부 미정)

## 한 줄 상태 · 다음 첫 액션
- **상태:** 트리 pane 배경 "에이전트 생성"을 1단 서브메뉴 [클로드 터미널 생성 / 클로드 JSON 생성]로 확장(생성 시 output_format 고정·이후 불변). 활성화는 단일 "활성화" 유지. **구현→/review code full(cross-family)→/qa→라이브 GUI 실측→ADR-0078 본문→커밋→푸쉬 전부 완료.** origin/master = `863f9ec`.
- **다음 첫 액션:** 이 작업은 종료. 남은 결정 하나 — **`stash@{0}`(per-activation 오버라이드 시도) 폐기 여부**(사용자 확인 대기, 미결). 새 갈래는 `docs/process/step-log.md` "## 다음 (미진행)".

## 완료분 (커밋+푸쉬)
- **커밋 `863f9ec`** (6파일): 프론트 3 — `src/commands/agentCommands.ts`(createReservedProfile 헬퍼 + createTerminal/createJson/파라미터화 createAgent + coerceOutputFormat + registerSlotMenu container) · `src/commands/agentCommands.test.ts` · `src/i18n/ko.ts`(createTerminal/createJson 라벨). 문서 3 — `docs/decisions/0078-*.md`(신규 본문) · `docs/decisions/README.md`(인덱스) · `docs/process/step-log.md`.
- **설계 재정의(ADR-0078):** 렌더 모드는 에이전트 생성 시 결정·이후 불변. 거부 대안 = per-activation 오버라이드(활성화 서브메뉴). createClaudeProfile(...,output_format)은 ADR-0044부터 존재라 신규는 프론트만.

## 되돌림/보존
- **`stash@{0}`** = "adr-0078 per-activation override (abandoned; superseded by creation-time mode)" — 미커밋이던 16파일(코어/데몬/프로토콜/프론트 오버라이드 배선). ADR-0078 거부-대안 근거. **폐기(`git stash drop stash@{0}`) 여부는 사용자 판단 — 아직 안 지움.**

## 검증 상태
- **PASS:** `/review code full` 2인(doc-aware worker-senior=PASS · cross-family Codex high=FIX→반영→재리뷰 PASS) · `/qa`(프론트 전용): `npx tsc --noEmit` 0 · `npm test`(vitest **613**, 플래그된 ViewLayoutRenderer·slotContentCommands 포함 green) · 코어 격리 실 import 0 · **라이브 GUI 실측(cdp 9223)**: pane 배경 우클릭→"에이전트 생성" container→flyout [클로드 터미널 생성/클로드 JSON 생성] 렌더 + command 3종 등록 + invalid outputFormat fail-loud throw(다이얼로그 열기 전).
- **미검증(경미):** 실사용 **create 클릭 왕복**(메뉴→native 폴더 다이얼로그→예약 노드 생성→활성화 렌더)은 CDP가 native 다이얼로그를 못 몰아 자동 실측 안 함 — 단위테스트(올바른 output_format 호출) + ADR-0044 기존 영속 메커니즘으로 커버. **사용자 수동 클릭 테스트 권장.**
- **재실행:** 프론트 `npx tsc --noEmit` + `npx vitest run` · GUI `node scripts/cdp.mjs eval "<js>"`(dev 앱 포트 9223).

## do-not / 주의
- **bare `cargo test`·`-p engram-dashboard` = WebView2 크래시** — member-scoped만.
- 실행 중 데몬/클라 있으면 `cargo build` link 파일락 — 재빌드 전 `taskkill /IM engram-dashboard-daemon.exe /F` + `engram-dashboard.exe`.
- **활성화에 모드 선택 서브메뉴를 다시 붙이지 말 것 = ADR-0078 위반.** 모드는 생성 시 고정.
- **실행 중 daemon(pid 1904)은 stashed per-activation 빌드(11:58) = 소스(HEAD)보다 stale.** 생성 기능(CreateProfile 경로)엔 무영향이나, Rust 만지려면 재빌드+재시작해 HEAD 데몬으로 정합.

## 정지 조건
- `stash@{0}` 폐기는 **사용자 확인 후**(되돌리기 어려움).
- 커밋/푸쉬는 사용자 승인 후(이번 건은 "모두 다 커밋 푸쉬" 승인받아 완료).

## 참조 (읽을 것)
- ADR-0078 본문 · `src/commands/agentCommands.ts`(앵커 `// ADR-0078`) · `src/commands/slotMenu.ts`(container API, ADR-0064/0065) · ADR-0044(output_format 렌더) · ADR-0076/0077(활성화 resume/fallback — 유효).
- 앱 실행: `run-dashboard.bat`(dev, 포트 9223).
