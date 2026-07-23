# 핸드오프: 릴리즈 실행 가능화 완주 — 패키징(0100)·메시징 이름(0101)·부팅 레이스(0102) 3커밋, 다음 = push/후속 선택

## 한 줄 상태 · 다음 첫 액션
- **상태:** 이번 세션 3개 작업 **완주**(전부 `/review code full` 2R + `/qa` 게이트 통과, 각 ADR 박제·step-log 기록). 전부 **로컬 커밋(master), 미push.** 워킹트리 클린. `release/` 폴더 = 3커밋 전부 반영본으로 재빌드됨(ADR-0102 QA에서 `scripts/build-release.ps1` 실행) → `release/engram-dashboard.exe` 실행 시 실 UI 부팅 **3/3 확인**.
- **다음 첫 액션(사용자 선택):** ① **push**(GitLab, 코드) 여부 결정 → ② 후속 중 택1: **DaemonClient 부팅-레이스 resilience 확인**(아래 미결 #1) / **이름 유일성 ②**(동명 자동 suffix) / **릴리즈 GUI 실사용 추가 확인**(메시지 전송 클릭 경로).

## 이번 세션 커밋 (master, 미push)
- `336299f` **feat(release): 포터블 릴리즈 폴더 조립 스크립트 (ADR-0100)** — `scripts/build-release.ps1`(Windows PS): 프론트+릴리즈 3바이너리 빌드 후 `release/`에 정확히 3 exe(engram-dashboard·daemon·send)+`prompts/`(agent-priming[-cli].md)만 조립+매니페스트 tripwire. UI 앱은 `npm run tauri -- build --no-bundle`(프로덕션 컨텍스트·frontendDist embed)로 빌드. `.gitignore`에 `release/`.
- `5f9adf0` **fix(messaging): canonical 이름 통일 (ADR-0101)** — 메시지 주소가 트리 표시 이름으로 안 가던 버그. `crates/engram-dashboard-core/src/agent/name.rs`(신규) 공유 코어(`cwd_basename`[프론트 `basename.ts` 1:1]·`canonical_name_or_id_fallback`). `AgentInfo.name` = `display_name ?? basename(session.cwd)`로 통일 — manager(`agent_info`/`resolve_canonical_name`/`canonical_name`)·reaper(`session_info`)·ingress(`sender_display_name`)·`src-tauri/src/cli.rs` 전부 **session.cwd 기반 공유 resolution** 사용(WYSIWYA). id-우선 매칭(ADR-0087) 유지.
- `45b6ed8` **fix(boot): 부팅 레이스 (ADR-0102)** — release exe main 창 "창 로딩 중" 영구 고착. 근본(가능성 높음)=Tauri v2 부팅 레이스(webview가 build 중 `setup()`의 `app.manage(LayoutState)` 전에 `invoke('list_tabs')` 조기 발화→state 미존재 Err→프론트 삼킴+재시도 없음+main 복구이벤트 없음). fix: (1)`LayoutState`를 `builder.manage()`(build 전) 이동=레이스 by-construction 제거(`lib.rs` `// ADR-0102` 앵커, setup으로 되돌리지 말 것) (2)`src/util/retryInvoke.ts` 유계 재시도+실패 표면화(WindowLayout·initMainWindowFromBackend 부팅 pull).

## 검증 상태 (쌍)
- **돌린 것:** 각 작업 `/review code full` 2인 적대(doc-aware 주도 상급 + codex blind, 2~3R) + `/qa`. 부팅 fix qa=코드게이트 6/6+`release/` 재빌드+exe **3/3 실 UI 부팅**(`tauri.localhost`·"창 로딩 중" 미발생·`list_tabs('main')` OK·데몬 release/서 spawn). 메시징 fix qa=코드게이트+`roundtrip-smoke` 하네스로 실 claude 트리이름 send→**양방향 배달**·발신자명=트리이름.
  - **재실행 명령:** `cargo test -p engram-dashboard-core -p engram-dashboard-protocol -p engram-dashboard-discovery -p engram-dashboard-daemon`(bare `cargo test` 금지) · `npm test`(vitest) · `npx tsc --noEmit` · `cargo fmt --check` · 격리 = Grep `use tauri` in `crates/engram-dashboard-core/src/`(lib.rs 주석 1건=baseline PASS) · **release 재빌드** `pwsh scripts/build-release.ps1` · **release 부팅 실측** bash `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223" ./release/engram-dashboard.exe &` → `node scripts/cdp.mjs eval "location.href"`(=tauri.localhost).
- **안 된 것(검증 안 됨):** ① **메시징 GUI 클릭-스폰 라이브 턴** — QA서 stream-json 에이전트가 이 dev 런치에서 턴 미실행(스타트업 스톨=환경). 라우팅은 하네스로 증명됐으나 release GUI 클릭→send 경로는 미관측. ② **DaemonClient 부팅-레이스 resilience** — codex 지적, out-of-scope로 미수정·미확인. ③ 이름 유일성 ②(동명 시 `RECIPIENT_AMBIGUOUS` 여전).

## do-not (실패한 접근 / 재현 방지)
- **옛 release/ 그대로 실행 = fix 없음.** 소스 변경 반영엔 `pwsh scripts/build-release.ps1` 재빌드 필수(release/는 gitignore·자동 안 따라옴).
- **순수 `cargo build --release -p engram-dashboard`로 Tauri 앱 빌드 금지** — 프론트 미임베드+devUrl(`localhost:1420`) 로드로 connection-refused. `tauri build --no-bundle` 써야 함(ADR-0100 스크립트가 이미 그리 함).
- **`release/`를 git repo 안에 두면** `find_install_root`가 `.git`까지 walk-up해 prompts를 repo 루트서 해석(ADR-0101 QA 관측). **실배포는 release 폴더를 repo 밖에** 둘 것.
- **재실행 전 stray 데몬 kill** — `Stop-Process -Name engram-dashboard,engram-dashboard-daemon -Force`. 안 하면 새 앱이 옛 데몬(옛 코드)에 붙음.
- codex blind 리뷰 = CLI `codex exec --sandbox read-only -c model_reasoning_effort="high" "..." < /dev/null`(bash, stdin 닫기). 출력 크면 tail.

## 정지 조건
- 리뷰 정면 대립·근거 없는 BLOCK = 사용자 에스컬레이션.
- 굵은 결정(②이름 유일성 방향·DaemonClient resilience 착수 여부·push) = 사용자.

## 미결 (carry-over)
- **[신규·후속 1순위] DaemonClient 부팅-레이스 resilience** — daemon_connect/daemon_connection_state/daemon_ensure 조기 invoke가 clientFactory서 swallow, 재시도 경로 점검 필요(별 서브시스템, out-of-scope였음). 단 관찰된 증상(로딩 고착)과 다름(연결 끊김).
- **[신규] 이름 유일성 ②** — 동명 라이브 에이전트 자동 suffix(`-2`/`-3`·가동 중 재사용 금지·이름을 AgentId에 묶어 epoch 유지, ADR-0087 미구현분). ADR-0101이 ①만 했음.
- **[신규·경미] `name::resolve_display_name` vestigial 제거**(자기 테스트만 호출) · retry in-flight/backoff 취소 granularity 경계 한계(문서화됨).
- **[이월]** 봉투 포맷 영속화(저장 위치=사용자 결정) · CI(사용자 직접·windows-latest) · codex/gemini CLI spike→백엔드 연결 · 전 LLM 공용 제약 레이어 · 풀 메일박스(수신함·영속·ACK) · S17 제어 표면(UI relay ADR-0081) · 정식 설치본(MSI/NSIS — ADR-0100 supersede/amend 시).

## 참조 (읽을 것만)
- **ADR:** 0100(릴리즈 패키징) · 0101(canonical 이름·거부대안 id-only/profile.name유지/프론트-only) · 0102(부팅 레이스·거부대안 visible:false/retry-only) · 0087(send 시맨틱·이름규칙·id-우선) · 0086(듀얼입구) · 0024(데이터위치) · 0057(레이아웃/창).
- **코드:** `scripts/build-release.ps1` · `src/util/retryInvoke.ts` · `crates/engram-dashboard-core/src/agent/name.rs`(canonical) + `manager.rs::resolve_canonical_name` · `src-tauri/src/lib.rs`(`builder.manage(LayoutState)` `// ADR-0102`) · `crates/engram-dashboard-daemon/src/control/ingress.rs`(`resolve_recipient` id-우선 · `sender_display_name`) · `bin/engram-send.rs`(CLI 입구·토큰 신원).
- `docs/process/step-log.md` 최근 3 항목(0100/0101/0102) · `docs/decisions/README.md`.
- **메시징 테스트/검증:** `roundtrip-smoke`(test-harness feat) = `cargo run -q -p engram-dashboard-daemon --features test-harness --bin roundtrip-smoke -- --model <m>`. GUI 실측 = `scripts/cdp.mjs`.
