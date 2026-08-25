# 로깅 컨벤션 (engram-dashboard)

**상태:** crates(agent/daemon/discovery) de-facto 관행을 명문화. 마스킹은 *목표*가 아니라 **명시적으로 거부된 방향**이다(아래 보안 — ADR-0138). load-bearing 경로 작성·리뷰 시 이 문서가 단일 출처.

> "무엇을·언제·어느 레벨로 로깅하나"(컨벤션)다. "어떻게 켜고 끄나"(인프라)는 `crates/engram-dashboard-agent/src/logging/mod.rs`.

## 인프라 (요약 — 정본은 코드)

- **진입점은 둘이다.** `logging::init_logging()` = stdout 만 · `logging::init_logging_with_file(data_dir, kind)` = stdout **+ 파일**. 둘 다 부팅 1회·멱등이고 **먼저 부른 쪽이 이긴다**. 데몬(`crates/engram-dashboard-daemon/src/lib.rs`)과 앱 셸(`src-tauri/src/lib.rs`)은 후자를 쓴다.
- `set_log_level(level)` 런타임 토글(`EnvFilter` reload).
- 기본 레벨 **warn**(릴리스 평상시 거의 무출력 = 기본 OFF). `RUST_LOG` 우선. 디버깅 = `RUST_LOG=debug`. **단 릴리스 데몬에는 `RUST_LOG`가 닿지 않는다**(부모 환경 미상속) — 그래서 파일 sink 가 있다.
- **파일 sink = `<데이터 폴더>/logs/<종류>-<UTC>-<pid>.log`**(종류 = `daemon` · `app`). **동기 쓰기**(이벤트 한 줄 = write 한 번), ANSI 없음. ★**비동기 writer(`tracing_appender::non_blocking`)를 도입하지 말 것**★ — 데몬은 `std::process::exit`로 끝나 버퍼에 남은 줄, 즉 **기동 실패 직전의 마지막 줄**이 사라진다. (ADR-0138)
- **로그 폴더는 호출자가 넘긴다** — 코어가 데이터 폴더를 해석하면 「코어 격리」(ADR-0003)가 깨진다. 1차 폴더를 못 쓰면 `%TEMP%/engram-dashboard/logs/`로 물러나고, 둘 다 실패하면 파일 sink 없이 뜬다(그 경우의 주인 = 클라이언트 사전 점검, ADR-0135).
- **보존 = 종류별 최신 10개**(이번 실행분을 포함한 상한). 정리 대상은 `<종류>-YYYYMMDD-HHMMSS-<pid>.log` **문법에 맞는 이름만**이다 — 접두사 일치로 바꾸면 다른 종류의 파일과 손으로 둔 파일까지 후보가 된다.
- **파일 첫 줄 = 실행 머리글**(`==== engram <종류> | <UTC> | pid <n> | <exe 경로> ====`). 로그 이벤트가 아니라 이벤트 평면 밖의 한 줄이다 — 기본 레벨이 `warn`이라 정상 기동은 한 줄도 안 남아 **머리글이 없으면 파일이 통째로 빈다**. 이걸 메우려고 기동 알림 레벨을 올리지 말 것.
- 정본: `crates/engram-dashboard-agent/src/logging/mod.rs`. 결정·거부한 대안 = **ADR-0138**.

## 레벨 — 무엇을 어디에 (de-facto)

| 레벨 | 기준 | 실제 예시 |
|---|---|---|
| **error!** | **데이터 위험 또는 복구 불가** — 사람이 반드시 봐야 함(격리 복구되더라도) | 파싱 실패→손상 파일 보존(`persistence.rs:140`), 직렬화 실패, panic(`daemon/lib.rs:91`, reaper 격리복구 `reaper.rs:196`), 인스턴스 가드·data_dir 실패 |
| **warn!** | **비정상이나 안전하게 폴백**(데이터 위험 없음) | resume 실패→fresh fallback(`manager.rs:324`), agents.json **읽기** 실패→빈 목록(`persistence.rs:122`), accept 실패 |
| **info!** | 정상 수명주기 이벤트(운영자 관심). 기본 warn이라 평상시 안 보이나 켜면 흐름이 보임 | 에이전트 spawn(`manager.rs:169`), 복원 시작/결과, 데몬·스레드 시작/종료, 연결 수립(net crate `ws.rs` 의 `handle_connection`) |
| **debug!** | 상세 흐름·진단. 디버깅 때만 | WS upgrade/Origin(net crate `ws.rs` 의 `OriginCheck::on_request`), reaper/thread 종료, 사소한 핸들 정리 실패 |
| **trace!** | 초고빈도 핫패스만. 현재 미사용(0건) | (출력 청크 per-frame 등 — 도입 시 신중) |

읽기 한 줄: **데이터 위험/복구불가(error) → 이상하지만 안전 폴백(warn) → 정상인데 추적 가치(info) → 내부 디테일(debug).** (읽기 실패=warn / 파싱 실패=손상 신호라 error — 분기 예시.)

## 형식

- **메시지 = 한국어 한 줄**(무엇이 일어났나).
- **식별자·수치는 구조화 필드(key=val)** 로 뺀다 — 필터·검색 키가 되므로 메시지에 보간하지 말 것. `%`=Display, `?`=Debug.
  - 예: `tracing::info!(agent = %profile.id, epoch, ?mode, "에이전트 spawn");`
- **에러 디테일(`: {e}`)은 메시지 끝 보간 허용**(de-facto) — "보간 금지"는 *식별자·수치*(agent·epoch·pid·conn) 한정이고 `{e}`는 예외.
- **`[component]` 프리픽스**(`[tray]`/`[layout]`)는 **src-tauri 일부 모듈만** 쓰는 관행, crate 전역 규약 아님. 신규 코드는 프리픽스보다 **필드(`module=`)** 권장(통일은 미결).
- **span/`#[instrument]` 미사용**(현재 0건, flat event). 도입하면 이 문서 갱신.

## 계측 의무 (load-bearing 경로 — 무계측은 결함)

다음은 **반드시** 적정 레벨로 로그를 남긴다(실패를 `let _ =`로 조용히 버리지 말 것 — 최소 debug):

- **연결/세션 수명:** 수립·실패·재연결·종료.
- **동시성 전이:** 상태 변화, 가드 발동(stale 세대 폐기·재시작 등).
- **외부 경계:** spawn, 파일 IO, 네트워크 accept/close.

> S14 `daemon_client`(연결·핸드셰이크·generation 가드)는 이 의무에 맞춰 **계측 완료**(`src-tauri/src/daemon_client/{mod,connection}.rs`) — 연결 시작/수립/종료(info), 접속·Auth·핸드셰이크 실패(warn), stale 가드 발동(debug). 가드 판정 로그는 호출자 레이어에 두고 `lifecycle.rs`는 무계측(이중 로깅 회피, 파일 헤더 참조). 본보기는 `crates/.../ws.rs`(연결/인증 흐름).

## 보안

- **토큰·자격증명·비밀번호를 평문 로깅 금지.** 에러에도 넣지 않는다 — 실측: net crate `ws.rs` 의 `handle_connection`(토큰 불일치 분기, `constant_time_eq` 비교부)이 토큰 값을 로그에서 제외, `daemon_client` 접속 실패 에러는 url만 싣고 token 제외.
- **★로그용 마스킹·레닥션 레이어는 거부됐다(ADR-0138)★** — 토큰은 이미 **같은 폴더에 평문으로** 있다(연결키 파일). 그 사본을 가리는 것은 아무것도 보호하지 않는다. **대신 채택한 규율 = 우리가 쓰는 문장에는 자격증명을 넣지 않는다**(위 첫 항목이 그 규율이다).
- **`mask_secrets` 헬퍼는 남아 있으나 호출처 0**(`agent/src/logging/mod.rs`) = 자동 적용 안 됨. PTY 텍스트 등 민감 가능 출력을 로깅하게 되면 **호출자가 명시 적용**해야 한다(자동 "경유" 아님). 파일 sink 는 이 헬퍼를 경유하지 않는다.

## 안티패턴

- 실패를 무로그로 삼킴(`let _ = x;` 후 침묵) — 최소 debug.
- 식별자·수치를 메시지 문자열에 보간(`format!`) — 필드로 빼라(에러 `{e}`는 예외).
- 토큰·민감정보 평문 로깅.
- 핫패스 무분별 info/debug — trace 또는 샘플링 고려.
