# 핸드오프 — Engram Dashboard S12 phase4-2 (이벤트 배선까지 완료)

작성 2026-06-15. 직전 핸드오프(`2026-06-15-S12-phase4-2-daemonclient.md`) 후속. 본문(`docs/process/step-log.md`·`docs/decisions/`·`CLAUDE.md`·`docs/testing-strategy.md`)이 항상 우선.

## 0. 한 줄 요약
**phase4-2 거의 완료** — 데몬 모드 프론트 연결(DaemonClient)·재연결 resume·**상태/목록/복원 이벤트 배선**까지 끝. 로직은 전부 vitest/cargo test 로 검증됨. **남은 건 "데몬 모드 트리 갱신 GUI 실측"(보안 CDP 통과 후) + 자동부팅 배선 + nit.** git `67accd0`까지 커밋, 작업트리 깨끗(`.ccb/` 제외). **push 미실행**(사용자 승인 시 `git push origin master`).

## 1. 이번 세션 커밋 (master, 미push)
| 커밋 | 내용 |
|---|---|
| `7ed756b` | phase4-2 #6 반환 event(Created/Spawned) |
| `45af572` | 프론트 DaemonClient(WS) + clientFactory 두-모드 토글 |
| `6998404` | DaemonClient 재연결 resume high-water dedup 수정(GUI 실측이 적출) |
| `114f9ac` | step-log |
| `e469e67` | testing-strategy 문서 신설(3층+배치 일원화) |
| `68d8295` | core examples→tests 이관(단언, cargo test 58) |
| `6ed9bc3` | 프론트 vitest 도입 + DaemonClient/clientFactory 테스트 |
| `0ccd3fa` | 상태/목록/복원 이벤트 AgentClient 인터페이스로 통일(데몬 트리 갱신) |
| `67accd0` | step-log |
| `b79a7a4` | core `pty/`→`agent/` rename(git mv 18파일, 실 PTY 항목 이름 유지) |
| `f6c9ce8` | CLAUDE.md 모듈맵·step-log rename 반영 + 데몬 콘솔 nit |

author: 이 repo local `user.email=kimsunzun@naver.com`(개인). 건드리지 말 것. 트레일러 `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.

## 2. 구현 실행 규약(불변, 준수 중)
메인=오케스트레이터. 코더(opus)→**reviewer-deep**(무겁게, mutation 검증)→QA(test+tsc, GUI 실측)→게이트 후 메인 커밋. 에이전트 프롬프트에 "포그라운드 동기·폴링/백그라운드/sleep 금지·끝나면 즉시 반환" 명시.
- **이번 사고**: reviewer 가 mutation 검증 중 `git checkout` 으로 **미커밋 변경을 날렸다가 patch 재적용으로 복원**. → 미커밋 작업트리에서 mutation 검증 시 `checkout` 대신 백업+patch 경로 강제. 검증 후 작업트리 무결성(test/tsc/diff) 직접 재확인할 것.
- **교훈**: 와이어 의미 의존 로직(dedup/resume)은 정적 리뷰가 놓친다 — 반드시 **실측(또는 mock 소켓 vitest)으로 드롭→재연결 재현**. 이번 재연결 버그가 그 증거.

## 3. 테스트 체계 (★ docs/testing-strategy.md 정독 ★)
3층 + 배치 일원화로 정리됨:
- **① 단위**: Rust `src/#[cfg(test)]` · 프론트 `*.test.ts`(vitest, mock WS/invoke).
- **② 통합(단언)**: `crates/<c>/tests/*.rs`(실 PTY 포함) · 프론트 vitest.
- **③ 실/시각**: 실프로세스 `#[ignore]` · **CDP(`scripts/cdp.mjs` shot/eval)=시각 전용**.
- `examples/` = 데모·스파이크만(게이트 아님). `cargo test`=Rust 단일 진실원, `npm test`=프론트.
- 명령: `cargo test -p {protocol|core|daemon}` / `npm test`(35) / `npx tsc --noEmit` / `cargo clippy --workspace --all-targets -- -D warnings`.

## 4. ★ EDR / CDP 보안 사건 (중요) ★
`scripts/cdp.mjs`(`--remote-debugging-port`)로 WebView2 제어 → 보안관제가 **"Chrome/Edge 프로세스 메모리 접근"으로 탐지**(2026-06-15). 보안관제(명동호)에 개발 예외 문의함. "한번 해보라" 받아 재연결+스샷 2회 테스트 통과(popup 캡처 확인) — **추가 탐지 없는지 확인 대기 중**.
- 원칙: 로직은 ①(vitest)에서 잡아 CDP 의존 최소화. CDP 는 시각 확인 꼭 필요할 때만.
- **다음 GUI 실측(데몬 모드 트리 갱신) 전에 보안 통과 확인**할 것. 통과되면 자유 사용.
- 띄울 때: `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223" npm run tauri dev` → `node scripts/cdp.mjs {info|eval|shot}`.
- **stale daemon.json 주의**: 기능 이전 옛 파일+죽은 PID 면 discover 가 죽은 포트 반환(daemon 모드 'reconnecting' 고착). 해소 `rm "$APPDATA/com.engram.dashboard/daemon.json"`.

## 5. 다음 작업 (우선순위)
1. **GUI 실측(보안 통과 후)** — 데몬 모드로 앱 띄워 spawn→**트리에 실시간 추가**, kill→**제거** 확인(cdp eval 로 트리 DOM 텍스트 또는 스샷). 이게 #1 이벤트 배선의 마지막 실측 확인. (로직은 vitest 35 green 으로 이미 검증됨.)
2. **자동 부팅 daemon 모드 배선** — 현재 `localStorage engram_client_mode='daemon'` 수동. 정식 설정 UI/부팅 자동 discover_daemon.
3. **nit 후속(저위험·장기, §0 깔 만함)**: ①list 조회(getAgents/listProfiles/getSnapshot) 응답에 request_id variant 추가(broadcast 편승 매칭 제거 — #6 과 동형) ②SubscribeAck.truncated UI 전파(인터페이스 확장) ③데몬 재연결 후 status 재동기 책임(백엔드가 재연결 시 AgentListUpdated 재전송하는지 — 백엔드 계약 확인) ④core `pid_alive` 부재(ERROR_INVALID_PARAMETER=dead)/권한(ACCESS_DENIED=live) 구분.

## 5-b. ★ 데몬 수명/콘솔/복원 정책 — 설계 결정 필요(다음 세션, 사용자 방향) ★
이번에 데몬 콘솔 빈 창 + respawn 루프(콘솔 닫으면 데몬 죽고→재discover→재spawn→복원 churn)를 겪고 정리한 방향:
- **콘솔 = 데몬 생명줄 아님. "떠 있는 데몬에 옵션으로 붙였다 떼는 뷰어"로** 재정의. 데몬은 기본 headless/detached 로 살고, 콘솔(출력 뷰)은 옵션으로 attach/detach. **콘솔 닫아도 데몬 생존**, 다시 열면 재attach(tmux attach 처럼). → 단순 "windowless" 가 아니라 "detachable 콘솔 뷰" 로 설계.
- **닫은 콘솔을 다시 여는 경로** 필요(메뉴/커맨드로 살아있는 데몬에 콘솔 뷰 재부착) — 어떻게 둘지 고민.
- **데몬 재기동 시 복원 정책을 옵션으로** — "기존 claude 세션 다시 실행할까?"를 사용자가 고를 수 있게(현재 per-profile auto_restore 는 있으나, 데몬 수명·재기동 맥락의 정책으로 정리 필요).
- **데몬 종료 정책**(영구 생존 vs 에이전트0+클라0 자동종료) 함께 결정 → ADR 후보.
- 전제(맞는 설계): UI 열림=데몬 확보(있으면 attach/없으면 start). UI 닫아도 데몬 생존·재attach. 복원은 데몬 부팅 1회.

## 6. 그 뒤 (모바일/별건, 한참 뒤)
- 모바일: VT-framebuffer 화면상태 동기(Mosh 갭2)·roaming TCP→UDP(갭3, transport 교체급)·predictive echo(갭4). **Mosh 턴키 없음→아이디어만 차용(브라우저 UDP 불가)**. 로컬/근거리는 현 WS+replay 로 충분, 모바일 답답할 때 갭별 보강.
- 별건: `pty/`→`agent/` 폴더 rename, core `AgentCommand`→`SpawnSpec` 개명, ts-rs `bindings/*.ts` 프론트 직접 소비(현재 손-미러 드리프트), Interrupt-lease 정책(긴급중단 누구나).

## 7. 핵심 불변식 (변경 금지)
- **DaemonClient dedup**: 클라 실배달 high-water(`lastDeliveredSeq`) 기준. `replay_from`=데몬이 보내는 첫 seq(dedup 기준 아님). resubscribe 는 알려진 epoch 전송(tail-only Resume). epoch 변경 시만 리셋.
- **이벤트 통일**: 상태/목록/복원은 `agentClient.on*` 경유(eventBus 가 Tauri 직접 listen 안 함). 두 구현이 동일 표면 — Embedded=Tauri listen 래핑, Daemon=WS 이벤트 라우팅.
- 데몬 SubscribeAck FIFO(on_ready→replay), kill 2동사, finalize 1회, 코어 격리(tauri/protocol import 0), epoch 재구독 — 이전 핸드오프 §7 동일.

## 8. 시작 첫 행동 제안
1. `docs/process/step-log.md` 맨 끝 + 이 핸드오프 대조. `docs/testing-strategy.md` 정독.
2. 보안 CDP 예외 상태 확인 → 통과면 **#1 GUI 실측**(데몬 모드 트리 갱신) 먼저 닫기.
3. 그 뒤 #2 자동부팅 배선. 매 코드 변경은 코더→reviewer-deep→QA 규약.
