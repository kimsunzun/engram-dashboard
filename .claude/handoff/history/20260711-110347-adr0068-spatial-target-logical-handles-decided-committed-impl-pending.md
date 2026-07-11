# 핸드오프: ADR-0068 슬롯 공간 타깃 = 논리 도면 방향·이웃·순서 핸들 결정·커밋·푸시 완료 · 구현 미착수(승계)

## 한 줄 상태 · 다음 첫 액션
- **상태:** slot geometry(ADR-0066 결정 3) 재검 → "우하단→slot id"는 좌표 산술이 아니라 **상대 위치·순서 판정**이라 `ViewManager`(클라이언트 Rust) **논리 도면만으로 충분** 결정. ADR-0068 박제 + ADR-0066 결정3 부분폐기 + step-log, **커밋 `266bfd3` 푸시 완료(origin 동기)**. 코드 변경 0(문서만).
- **다음 첫 액션:** 구현 진입 전 **TRD(구현 세부)부터** — neighbor/ordinal/방향 핸들의 command 표면 설계. **구현 갈림길(사용자 결정 필요) 미해소 3건:** ① 핸들 노출 형태 = 별도 query command vs 레이아웃 스냅샷 필드 ② 방향 토큰 네이밍(tmux `{bottom-right}` 결 vs 자체) ③ neighbor 반환 = slot id만 vs id+메타. 사용자에게 이 forks 제시 → `/implement`.

## 이번 세션 한 것
- **푸시(2건):** 이전 핸드오프 "25커밋 미푸시"는 **stale**(코드는 이미 origin 반영). ① 밀려있던 핸드오프 노트 커밋+푸시(`95caba1` — 이 커밋 제목에 `@` 오타: Bash 툴에서 PowerShell here-string `@'...'@` 오용. 사용자 "그냥 둬"로 방치, cosmetic·force-push 안 함). ② ADR 문서 커밋+푸시(`266bfd3`).
- **리서치:** `/research medium`(설계-결정 모드) — 수집자 2 Sonnet 병렬(①터미널/WM: tmux·zellij·kitty·i3/sway·wezterm ②GUI/웹 split: VS Code·JetBrains·CDP·allotment 등) + **Codex cross-family 적대 리뷰(FIX)**. 결론: 권위=논리좌표 소유, 실제 타깃=심볼릭 방향/이웃. Codex 적출: VS Code #94817 closed(수집자 confident-wrong)·"정규 논리 완전충족" 과장(min-size/collapse/chrome로 렌더≠비율, i3가 percent·rect 둘 다).
- **결정·기록:** ADR-0068(위 커밋). ADR-0066 결정3 부분폐기 — **상태줄 수동 보강**함(`/adr`가 관련줄에만 박아서, 기존 adr feedback 2026-07-11 항목 재확인).

## 검증 상태 (쌍)
- **돌린 것:** `/adr lint` = **clean**(error 0, advisory 5 전부 기존 레거시 = ADR-0016 링크·ADR-0027 폐기앵커). `git push` 성공(`266bfd3`, origin/master 동기 확인). 인덱스 재생성(68번 추가).
- **검증 안 된 것:** **코드/테스트 전무** — 문서만 변경이라 cargo/npm/tsc 안 돌림(돌릴 코드 없음). `/review doc` 안 돌림 — 결정 **내용**은 `/research`의 Codex 적대 게이트로 이미 검증됨(doc-stage prose 리뷰만 생략). 규약상 load-bearing 문서라 다음 세션이 필요시 `/review doc` 가능(내용은 이미 검증됨).

## 실패한 접근 / do-not (재시도 주의)
- **geometry 좌표계 신설 = 보류(재론 금지).** ADR-0068이 결정3 뒤집음 — OSS 논리좌표 선례는 엔진 내부사정이라 LLM 근거 약함. **프론트 `getBoundingClientRect` 1차 = 거부**(권위 역전·staleness·DPI 모호). **백엔드 투영 px = 거부**(gutter/sash/min-size 모델링 = 렌더 엔진 중복). 실측 픽셀은 진짜 픽셀공간 use case(스크린샷 매핑 등) 생길 때만 = 프론트가 **versioned 관측값** 보고(권위는 ViewManager).
- **"백엔드" 용어 = 데몬 아님(혼동 주의).** 레이아웃 권위 = **클라이언트(src-tauri) Rust `ViewManager`**(`src-tauri/src/layout/manager.rs`), 에이전트 호스팅 데몬(서버)과 무관. 이 결정의 좌표 축 = 한 클라이언트 프로세스 안 Rust↔JS 경계.
- **bare `cargo test`·`-p engram-dashboard`/`--lib` = 0xc0000139(WebView2Loader 사망).** member-scoped만(`-core`/`-protocol`). src-tauri 로직 = `cargo build` + GUI 실측이 정본.

## 정지 조건
- **앱 재시작 자유 승인 = 만료.** 이전 세션 한정 grant는 이미 만료(이 세션 로드 시 확인). GUI 실측 위해 client/데몬 재시작 필요하면 **사용자 재확인**.
- **`docs/reference/architecture-overview.md` = 타 세션 작업중, 미커밋(215줄).** 건드리거나 커밋하지 말 것(사용자: "저쪽에서 작업중, 별도 커밋 하지마").
- 비자명 코드 = `/implement`(코더→review→qa) · 굵은 결정 = ADR(`/adr`) · 설계 서베이 = `/research`. 메인 직접 구현 금지. **구현 갈림길 = 사용자 결정**(위 forks 3건).

## 미결 / 다음 갈래
- **[다음 = 구현] ADR-0068 실현:** neighbor(각 슬롯 상하좌우 이웃 slot id)·ordinal(왼쪽부터/위부터 n번째)·방향 토큰을 **`ViewManager`가 논리 트리 순회로 산출** + command registry(§5)로 노출(사람 클릭·LLM 동일 핸들). 픽셀·프론트 왕복 무관. **TRD부터**(forks 3건 사용자 결정).
- 실측 픽셀 capability(보류) · 드래그앤드롭 배치 · 방향 sugar 커맨드 · 키보드 방향 포커스 이동.

## 참조 (읽을 것만)
- **ADR:** **0068**(정본 — 공간 타깃 결정 + 거부 대안) · 0066(결정1 click-to-focus·결정4 65% 링 생존 / 결정2·5 = 0067 폐기, 결정3 = 0068 폐기) · 0035(레이아웃 권위=클라Rust ViewManager) · 0022/0055(command registry) · 0011(assign) · 0067(우클릭 배치). CLAUDE.md §5(LLM-우선 제어).
- **코드 포인터:** `src-tauri/src/layout/manager.rs`(ViewManager — focused_slot_id·tree 순회, 여기에 neighbor/ordinal 산출 추가) · `src-tauri/src/commands/layout.rs`(layout command 패턴) · `src/store/viewStore.ts` · command registry = `src/commands/`(slotCommands 등 §5 핸들).
- **step-log 최근 항목:** "LLM 공간 타깃 재설계 — 논리 도면 방향·이웃·순서 핸들 우선".
