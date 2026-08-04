# 테스트 전략 (Test Strategy)

> 이 문서는 **살아있는 운영 지도**다 — "각 층을 무엇으로·어디까지·어떻게 검증하고, 빈칸이 어디인가". 결정의 *근거(why)* 는 **ADR-0012**(테스트 전략 — 격리 하네스 + TDD)에 있고, 이 문서는 그 위에서 실제 인벤토리·역할 분담·갭을 추적한다. 진행 흐름은 `docs/process/step-log.md`.
>
> 새 모듈·기능을 추가할 때 이 문서의 "층별 역할"에 맞는 테스트를 **함께** 짠다(TDD, ADR-0012). 테스트 방식이 바뀌면 이 문서를 갱신한다.

## 0. 핵심 원칙 — 테스트 피라미드 + 층별 역할 분리

세 층으로 나누고, **싸고 빠른 아래층에서 최대한 잡는다.** 위층(실앱/시각)은 아래층이 못 보는 것만.

| 층 | 무엇을 검증 | 도구 | 속도 | 보안(EDR) |
|---|---|---|---|---|
| **① 로직 단위** | 순수 로직(디코드/dedup/상태전이/변환) | Rust `#[test]` · (프론트)vitest | 빠름 | 무관 |
| **② 격리 통합** | 모듈 경계 인과(seam으로 외부의존 끊고) | examples/smoke bin · `tests/` 통합 · in-process WS E2E | 중간 | 무관 |
| **③ 실앱/시각** | 진짜 프로세스·진짜 렌더링·실제 OS | 실프로세스(#[ignore]) · CDP(`scripts/cdp.mjs`) | 느림 | **CDP는 EDR 탐지 대상** |

**역할 분리 규칙(중요):**
- **로직은 ①에서.** "데이터를 제대로 처리하나"는 단위테스트로. 브라우저·실프로세스 불필요.
- **CDP/Chrome은 ③ 전용** — 스샷·레이아웃·"진짜 앱이 실제로 뜨고 붙나" 최종 스모크. **로직 검증에 CDP eval 을 쓰지 않는다**(과용 = 느리고, EDR 마찰, 버그를 늦게 잡음).
- 근거: 재연결 resume 버그(2026-06-15)는 ①에 있어야 할 로직 버그였는데 프론트 단위테스트가 없어 ③(CDP)까지 가서야 잡혔다. mock 소켓 단위테스트였으면 브라우저 없이 즉시 걸렸을 버그.

### 0-a. 배치 일원화 — 용도로 폴더를 고정한다 (시기 아님)

과거엔 같은 "모듈 격리 검증"이 초기엔 `examples/`(실행해서 로그 eyeball), S12엔 `tests/`(단언)로 갈렸다. 이는 시기 차이일 뿐이고, **검증 하네스가 `cargo test` 밖(examples)에 있으면 자동 회귀 게이트에서 빠지는 구멍**이 된다. 아래로 일원화한다:

| 분류 | Rust 배치 | 프론트 배치 | `cargo test`/`npm test` 게이트 |
|---|---|---|---|
| ① 단위(순수 로직) | `src/` 내 `#[cfg(test)] mod tests` | `*.test.ts` 코로케이션(vitest) | ✅ |
| ② 통합(모듈경계 인과, 단언) | `crates/<crate>/tests/*.rs` | vitest(mock transport) | ✅ |
| ③ 실/시각(실프로세스·렌더링) | `tests/*.rs` + `#[ignore]` | CDP(`scripts/cdp.mjs`, shot/layout) | 수동/CI |
| 데모·스파이크 | `examples/` | — | ❌ (게이트 아님) |

**규칙 한 줄: `examples/` 는 데모·스파이크(사용예시·throwaway) 전용. 검증 하네스는 전부 `tests/`(단언 기반).** 그래야 `cargo test --workspace --exclude engram-dashboard` 가 전 층 회귀의 단일 진실원이 된다.

## 1. 층별 현재 인벤토리

### protocol (`crates/engram-dashboard-protocol`)
- **① 단위**: `src/` 내 `#[test]`(codec/discovery/ids 등).
- **② 계약**: `tests/codec_golden.rs`(binary frame wire 표현 박제), `tests/ts_export.rs`(ts-rs 바인딩 export 검증).
- 실행: `cargo test -p engram-dashboard-protocol`.

### core (`crates/engram-dashboard-core`)
- **① 단위**: `src/` 내 `#[test]`(OutputCore seq/replay/finalize, session, transport, backend, platform liveness, persistence 등).
- **② 격리 통합(단언, 실 PTY)**: `tests/headless.rs`(manager 전체, 프론트 없이 spawn→subscribe→write→resize→kill — PTY out 수신·Exiting→Killed 전이·kill 후 list count=0·hang 없음 단언), `tests/transport_smoke.rs`·`tests/session_smoke.rs`(manager 없이 PtyTransport/AgentSession 직접 — shutdown→pump EOF→finish(Killed) 인과·resize cols/rows 반영 단언). 기록형 RecordingSink(받은 OutputFrame 바이트·status 전이를 `Mutex<Vec<..>>`에 push)로 로그 eyeball 대신 단언. 실 셸 spawn이지만 가볍고 전역 경합 없어 default(자동 실행). `examples/spike*.rs`는 throwaway 스파이크 보존.
- 실행: `cargo test -p engram-dashboard-core`.
- 격리 게이트: `rg "^\s*use tauri" crates/engram-dashboard-core/src/` → 0줄(import 라인 앵커 — 게이트 규칙을 자기 인용한 주석의 오탐 방지, ADR-0003 실측 2026-07-13) · `rg "engram_dashboard_protocol" crates/engram-dashboard-core/src/` → 0줄.

### messaging (`crates/engram-dashboard-messaging`) — 2026-07-28 신설(ADR-0110)
- **① 단위**: `src/` 내 단위테스트(mailbox 파킹·TTL, ledger 이력/회신 계약, groups 해석, envelope 렌더·이스케이프, service 3분기/flush/sweep, busy 게이트 상태머신). 전부 clock injection(주입 `now`)이라 실시간 sleep 0.
- **격리 하네스 = crate 경계 그 자체**: 이 crate 는 워크스페이스 crate 무의존(ADR-0110 결정 2)이라 `AgentManager`·PTY·Tauri 없이 단독으로 돈다 — 외부 의존은 포트 trait(`DeliveryPort`·`ControlPlanePort`·`TurnFacts`·`IdleNotifier`·`FlushTrigger`)의 fake 로만 들어온다.
- 실행: `cargo test -p engram-dashboard-messaging`.
- 격리 게이트: `rg "engram_dashboard_(core|daemon|protocol|discovery)" crates/engram-dashboard-messaging/src/` → 0.
- **호스트 어댑터는 daemon 쪽 테스트**: 배달·턴 사실 조회 어댑터(`messaging_host::ManagerDeliveryPort`·`ManagerTurnFacts`)는 daemon crate 단위 테스트가 덮는다. 출력 이벤트→턴 신호 분류는 백엔드 지식이라 코어 `backend/` seam 뒤로 내려갔고(`AgentBackend::turn_classifier`, ADR-0127) core crate 테스트가 덮는다.

### net (`crates/engram-dashboard-net`) — 2026-08-05 신설(ADR-0129 슬라이스 1)
- **① 단위**: `src/` 내 `#[cfg(test)]`(ws 연결 수명·Origin 허용목록·auth 핸드셰이크·연결당 단일 writer·keepalive 판정, portfile IO/stale 판정, instance 단일 인스턴스 가드). 데몬 crate 에서 이사해 온 것들이다.
- **격리 하네스 = crate 경계 그 자체**: 실소켓(127.0.0.1:0) 또는 합성 프레임열로 돌고 에이전트 시스템 실물(`AgentManager`·실 PTY)을 쓰지 않는다. daemon·messaging·discovery 를 못 박는 것은 소스가 이름조차 부르지 않는다는 게이트 1과 직접 의존으로 선언돼 있지 않다는 게이트 3이다. `AgentManager`·`PtyTransport` 는 **core 안**에 있고 core 는 허용된 직접 의존이라 그 둘이 잡지 못한다 — 이 자리는 게이트 2의 core 심볼 allowlist(허용 심볼이 전부 `agent::platform::` 아래의 프로세스 liveness 헬퍼)가 맡는다.
- 실행: `cargo test -p engram-dashboard-net`.
- **격리 게이트 3종** — 기대값·근거의 정본은 `crates/engram-dashboard-net/src/lib.rs` 헤더:
  - 게이트 1(소스 참조): `rg "engram_dashboard_(daemon|messaging|discovery)" crates/engram-dashboard-net/src/` → 0줄.
  - 게이트 2(core 심볼 allowlist — 파일 단위가 아니라 **심볼 단위**): `rg -o --no-filename "engram_dashboard_core::[A-Za-z0-9_:]+" crates/engram-dashboard-net/src/ | sort -u` → 정확히 2줄.
  - 게이트 3(직접 워크스페이스 의존 상한 — **해석된 의존 그래프**): `cargo tree -p engram-dashboard-net --depth 1 --prefix none -e normal,dev,build --target all --all-features | rg "^engram-dashboard" | sort -u` → 정확히 3줄. 매니페스트 텍스트 grep 으로 바꾸지 말 것(rename·`[dependencies.<이름>]` 테이블 형·들여쓴 선언·`[build-dependencies]`·비활성 target·`optional` 이 빠져나간다 — 실측), 플래그도 줄이지 말 것.

### daemon (`crates/engram-dashboard-daemon`)
- **① 단위**: `src/` 내 `#[cfg(test)]` — `connection_core`·`agent_conn`·`status_fanout`·`messaging_host`·`control/*`. 목록의 정본은 `rg -l "#\[cfg\(test\)\]" crates/engram-dashboard-daemon/src/`. ws·portfile·instance 는 net crate 로 이사했다(ADR-0129 슬라이스 1).
- **② 통합**: `crates/engram-dashboard-daemon/tests/`(ws_e2e·control_send·engram_send_cli·mcp_control·mcp_manager_lifecycle 등) — in-process WS E2E는 `ws_e2e.rs`가 데몬 WS 서버를 127.0.0.1:0 + MemProfileStore 로 기동해 tokio-tungstenite 클라로 전 경로(auth/구독/replay/resume/truncated/epoch/backpressure/dispatch 전 command/keepalive/lease/resize 협상)를 검증하고, 나머지 파일들은 제어 채널 듀얼 입구 배달(`control_send.rs`)·engram-send CLI 프로세스 레벨(`engram_send_cli.rs`)·MCP 제어 채널 입구 인증(`mcp_control.rs`)·제어 채널 생명주기(`mcp_manager_lifecycle.rs`)를 각각 검증한다.
- **③ 실프로세스(#[cfg(windows)] + #[ignore])**: `tests/ws_e2e.rs` 하단의 `#[ignore]` 분(② 파일 안에 있다) — 실제 데몬 .exe spawn(데몬 kill→PTY child Job 동반사망 / single-instance mutex / stale discovery 자가덮어쓰기). `ENGRAM_DATA_DIR`·`ENGRAM_INSTANCE_KEY` 로 운영환경 격리.
- 실행: `cargo test -p engram-dashboard-daemon` · 실프로세스 `cargo test -p engram-dashboard-daemon --test ws_e2e -- --ignored --nocapture`.

### src-tauri (`src-tauri`)
- **① 단위**: 18건(discovery DTO 변환, `ensure_with` OS/WMI/clock trait 주입 순수 검증, ComGuard 분류 등).
- thin command wrapper(spawn/kill/write/profile)는 로직이 core 에 있어 여기선 배선만.
- 실행: `cargo test -p engram-dashboard` 은 현재 lib 타깃이 `0xc0000139 STATUS_ENTRYPOINT_NOT_FOUND` 로 죽는다(실측 2026-08-05). 그래서 전 멤버 회귀는 이 패키지를 빼고 `cargo test --workspace --exclude engram-dashboard` 로 돈다 — 루트 bare `cargo test` 도 같은 이유로 금지(§3 치트시트).

### frontend (`src/`)
- **① 로직 단위**: **없음** (JS 테스트 러너 미설치 — vitest/jest 부재). ← **최대 갭**.
- **타입 게이트**: `npx tsc --noEmit` (타입 에러 0).
- **③ 시각/실앱**: `scripts/cdp.mjs`(CDP, Windows WebView2) — `info`/`eval`/`shot`. **현재 로직 검증까지 여기서 떠맡는 중(과용)**.

## 2. 갭 · 개선 항목 (우선순위)

> §0-a 일원화 결정(2026-06-15)에 따른 구체 실행 항목. 우선순위 순.

### HIGH
1. **프론트 로직 단위테스트 도입 (vitest)** — vitest 설치 + `npm test` 스크립트. `src/api/daemonClient.test.ts` 식 **코로케이션**. mock `WebSocket` + mock `@tauri-apps/api/core` invoke 로 브라우저 없이: decodeOutputFrame(바이트/UUID), high-water dedup, **재연결 resume(드롭→재연결→무손실·무중복)**, request_id 매칭, #13133 정리, clientFactory 모드. `embeddedClient`/`decodeBase64`/store 전이도. 재연결 버그류 회귀를 ①에서 잡는 그물 — ②③ 부하·EDR 마찰 감소의 핵심.
2. **core `examples/` 검증 하네스 → `tests/` 이관(§0-a)** — `examples/{headless,transport_smoke,session_smoke}` 의 "로그 eyeball" 을 **단언 기반 통합테스트**(`crates/engram-dashboard-core/tests/`)로 전환: spawn→write→resize→kill 인과, hang 없음, finish(Killed) 종점 등. 그러면 `cargo test` 가 core 격리까지 자동 회귀(현 구멍 메움). `spike*.rs` 는 스파이크라 `examples/` 잔류.
3. **CDP 역할 재정의 + 최소화** — CDP eval 로 로직 검증하던 관행 중단. CDP = 시각(shot)·레이아웃·실앱 최종 스모크 전용. CLAUDE.md 의 "검증은 스샷보다 eval 텍스트 유리" 문구도 이 분리에 맞게 보정 검토(eval 은 실앱 스모크 한정).

### MED
3. **프론트↔wire 타입 드리프트 게이트** — ts-rs `bindings/*.ts` 가 있으나 프론트가 `src/api/types.ts` 로 손-미러. `ts_export` 산출물과 프론트 소비 타입의 drift 검출(빌드 시 diff 비교) 또는 bindings 직접 소비로 전환.
4. **프론트 event 배선 검증** — DaemonClient 의 `StatusChanged`/`AgentListUpdated`/`RestoreResult` 소비 경로(현재 갭). 배선 후 vitest(mock event) + 데몬모드 CDP 스모크(트리 갱신)로 검증.

### LOW
5. **실프로세스 테스트 실행 시점 문서화** — `#[ignore]` 3건은 수동 전용. "데몬 수명주기·Job·discovery 변경 시 반드시 `--ignored` 실행"을 PR 체크리스트화.
6. **CI 부재** — 현재 로컬 수동. `cargo test --workspace --exclude engram-dashboard` + (도입 후)vitest + clippy + tsc 를 한 번에 도는 스크립트/CI 후보.
7. **`net → protocol` 심볼 게이트 부재** — 허용된 그 간선에는 core 쪽 같은 심볼 allowlist 게이트가 없어, 간선을 타고 에이전트 어휘가 늘어도 어느 게이트도 울리지 않는다(ADR-0129 슬라이스 1 note 가 의도적 유예로 기록 — 작업항목 0-4 뒤의 자연스러운 후속).
8. **src-tauri 단위테스트가 로컬 회귀에서 안 돈다** — §1 src-tauri 항목의 단위테스트는 존재하지만 `cargo test -p engram-dashboard`의 lib 타깃이 `0xc0000139 STATUS_ENTRYPOINT_NOT_FOUND`로 죽어(실측 2026-08-05) 실행되지 않는다 — 그래서 `cargo test --workspace --exclude engram-dashboard`가 이 패키지를 뺀다. 사전부터 있던 환경 요인이며 원인 미해결 — 이력은 `docs/process/step-log.md` 494/532/761행 부근.

## 3. 명령 치트시트 (workspace 루트 `I:\Engram\apps\engram-dashboard`)

```bash
# Rust 전 층
cargo test --workspace --exclude engram-dashboard  # 전 멤버 회귀(src-tauri 패키지 `engram-dashboard` 만 제외 — 그 lib 타깃이 0xc0000139 로 죽어 루트 bare cargo test 는 못 쓴다)
cargo test -p engram-dashboard-protocol     # 단위+golden+ts_export
cargo test -p engram-dashboard-core         # 단위+통합(headless/transport_smoke/session_smoke, 실 PTY)
cargo test -p engram-dashboard-messaging    # 메시징 커널 단위 — 워크스페이스 crate 무의존, ADR-0110
cargo test -p engram-dashboard-net          # 네트워크 행 단위 (실소켓·합성 프레임열 — 격리 게이트 3종은 위 §1 net 절)
cargo test -p engram-dashboard-daemon       # src/ 단위 + tests/(ws_e2e 등 — #[ignore] 분은 빠짐)
cargo test -p engram-dashboard-daemon --test ws_e2e -- --ignored --nocapture  # 위 ws_e2e 안의 #[ignore] 실프로세스 분
cargo clippy --workspace --all-targets -- -D warnings

# 프론트
npx tsc --noEmit                            # 타입 게이트
# npm run test                              # (vitest 도입 후 추가 예정 — 현재 없음)

# 격리 게이트(코어가 tauri/protocol 안 물었나)
rg "^\s*use tauri" crates/engram-dashboard-core/src/      # → 0줄 (import 라인 앵커 — 자기 인용 주석 오탐 방지)
rg "engram_dashboard_protocol" crates/engram-dashboard-core/src/   # → 0줄

# ③ 실앱/시각 (CDP — EDR 탐지 대상, 최소 사용)
# WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223" npm run tauri dev
# node scripts/cdp.mjs shot out.png   # 스샷(시각 확인)
# node scripts/cdp.mjs info           # 페이지 목록
```

## 4. 보안(EDR) 주의 — CDP

`scripts/cdp.mjs` 는 WebView2(Edge 기반)에 `--remote-debugging-port` 로 디버그 포트를 열고 외부 `node` 가 붙어 제어한다. 이 패턴은 보안 솔루션이 **"브라우저 프로세스 메모리 접근"** 으로 탐지한다(공격자의 자격증명 탈취 수법과 시그니처 동일). 2026-06-15 실제 탐지됨.
- **원칙**: 로직은 ①(vitest)에서 검증해 CDP 의존을 최소화. CDP 는 시각/레이아웃 확인이 꼭 필요할 때만.
- **정식 사용이 잦아지면**: 보안관제에 개발 예외 등록 요청(대상=`msedgewebview2.exe` + 디버그 플래그 / 주체=`node` `scripts/cdp.mjs` / 루프백 `127.0.0.1`). 단 예외 가부·형태는 보안관제 소관.

## 5. 새 코드 추가 시 체크 (TDD, ADR-0012)
- 순수 로직이면 → ① 단위테스트 **먼저/함께**(Rust `#[test]` 또는 프론트 vitest).
- 모듈 경계·인과면 → ② 격리 하네스(example/smoke bin 또는 `tests/`).
- 외부 의존은 seam(trait/sink)으로 끊어 단독 실행 가능하게(직접 호출로 격리 불가하게 만들면 ADR-0012 위반).
- 실앱/시각 확인이 정말 필요한 부분만 ③(CDP/실프로세스). 코드 통과 ≠ 완료지만, **③ 의존을 키우지 말고 ①②로 최대한 내린다.**
