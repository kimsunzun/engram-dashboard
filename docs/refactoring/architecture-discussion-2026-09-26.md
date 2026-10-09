# 아키텍처 논의 — 모듈별 정리

**목적:** transport 를 붙이기 전에 전체 의존관계와 모듈 쪼개기를 먼저 정한다(사용자가 정한 순서, 2026-09-26). 사용자가 모듈 순서대로 묻고, 답에서 나온 사실과 결정 후보를 여기 쌓는다.

- **기준 코드:** master `a226f63`. 사실 항목은 전부 그 시점 코드와 대조했다.
- **결정은 여기서 끝나지 않는다.** 방향이 선 후보는 논의를 마친 뒤 ADR 로 박고 여기서 링크한다(`../decisions/`). 착수 대기 사실은 `../tracking.md` T-47 이 진다.
- 그림은 그리지 않는다(사용자 결정 2026-09-26 — 앞서 만든 `../reference/architecture-map.html` 은 초안으로만 남긴다).

---

## 1. base

### 사실

- **입주자는 둘뿐이다** — `logging` · `platform`(`crates/engram-dashboard-base/src/lib.rs:53-54`).
- **`logging`** — 로그 전역 초기화 · 실행 1회분 파일 로그 · 실행 중 로그 레벨 변경 · 로그 속 비밀값 가리기(`mask_secrets`). 초기화는 데몬·셸이, 가리기는 agent·셸이 부른다.
- **`platform`** — 「이 PID 가 아직 그 프로세스인가」 판정. PID 는 OS 가 재사용하므로 **PID + 프로세스 생성 시각** 쌍으로 식별한다. 자식 PID 목록도 여기 있다. 쓰임은 두 갈래다.
  - **데몬 중복 방지·발견** — 데몬이 `daemon.json` 에 자기 PID·시작 시각을 적고(`crates/engram-dashboard-daemon/src/lib.rs:698`), 뒤에 온 쪽이 그 둘로 살아 있는 데몬인지 죽고 남은 파일인지 가른다(`crates/engram-dashboard-net/src/portfile.rs:86` · `crates/engram-dashboard-discovery/src/lib.rs:1037`).
  - **codex 세션 id 회수** — 잠금 파일 주인이 살아 있나(`crates/engram-dashboard-agent/src/backend/codex/thread_lock.rs:279`), 에이전트 PID 아래 프로세스 트리, 파일을 연 프로세스 대조.
- **Windows 밖은 자리채움이다** — PID 가 0 만 아니면 살아 있다고 답한다(`crates/engram-dashboard-base/src/platform.rs:201-228`).
- **설계 의도 = 어느 crate 든 쓸 수 있는 잎.** 단 지금은 `command`·`messaging` 이 자기 게이트(워크스페이스 의존 0)에 막혀 base 도 못 쓴다 → ~~아래 후보 2 가 푼다~~ ★후보 2 철회(사용자 2026-10-03) — 필요해질 때만 연결한다 · ADR-0267★.
- **agent 에도 `platform` 모듈이 따로 있다** — Job Object 래퍼 · 파일을 연 프로세스 찾기(`file_holders`) · 프로세스 트리(`process_tree`)의 셋이다(`crates/engram-dashboard-agent/src/platform/mod.rs:1-2`). CLAUDE.md 「백엔드 모듈 맵」 agent 항목의 「남은 `platform`은 Job Object 래퍼 하나뿐」은 낡은 서술이다(아래 후보 1 이 착지하면 그 문장째 바뀐다).

### 결정 후보 1 — OS 의존 코드를 독립 `platform` crate 로 분리

**상태: 방향 확정(사용자 2026-09-26) · ADR-0266 · 착수는 아키텍처 논의 뒤.**

- **무엇:** OS 에 따라 달라지는 코드를 워크스페이스 의존 0 인 독립 crate 하나로 모은다. 부르는 쪽은 인터페이스만 부르고 OS 분기(`#[cfg]`)는 그 crate 안에만 있다. 이미 OS 코드에 연결된 곳도 전부 이쪽으로 옮긴다.
- **사용자 근거:** 「인터페이스 호출로 하고 플랫폼적인 건 다 감추고」 · 「어차피 플랫폼 전용 기능은 의존 0」 · 「애초에 초반부터 잡고 갔어야 됐어」.
- **옮길 것(현황 실측 — 테스트 전용 분기 제외):**

  | 무엇 | 지금 위치 |
  |---|---|
  | Windows API 래퍼 — PID 생존·생성 시각·자식 PID · Job Object · 파일을 연 프로세스 찾기 · 프로세스 트리 · WMI 로 터미널 트리 밖에 띄우기 | `base/src/platform.rs` · `agent/src/platform/` · `discovery/src/lib.rs:1119-1320`(`wmi_spawn`·`wmi_create_raw`) → 실측 정정 `1060-1323`(COM 분류부터) |
  | OS 규칙 — `.exe` 붙이기 · 기본 셸 · 홈 디렉터리 · CLI 를 `cmd.exe /c` 로 감싸기 · `PATH` 대소문자 무시 | `discovery/src/lib.rs:981` · `daemon/src/lib.rs:96,146` · `agent/src/manager.rs:75` · `agent/src/backend/claude/mod.rs:1012` · `agent/src/backend/mod.rs:44,136` → 실측 정정 `discovery:987` · `daemon:109` **하나**(CLI — 앱 이름을 가르던 자리는 없었다) · `manager.rs:77` · `claude/mod.rs:1829` · `backend/mod.rs:46-60, 138` |
  | 부르는 쪽에 샌 분기 — Job Object 핸들을 `#[cfg(windows)]` 로 들고 다님 | `agent/src/transport/pty.rs` · `agent/src/transport/stdio.rs` · `agent/src/backend/codex/transport.rs:177` → 실측 정정 `:183` |

  ~~셸(`src-tauri/src`)에는 OS 분기가 없다.~~ → **틀렸다**: 셸 `src-tauri/src/fsutil.rs:107` 에 원자적 쓰기의 운영 분기(`cfg!(windows)`)가 있다 — storage P3 착지 뒤 U-W 가 옮기는 시한부 예외다(ADR-0275 결정 15). PTY 자체는 `portable-pty` 가 이미 감춘다.

  ★위 줄 번호 정정은 TRD 1-3 §2-5(실측 `0ef6292`)의 것이고, 표의 자리는 1-3 U0~U7 로 전부 platform 으로 옮겨져 지금은 어느 쪽 번호도 가리키지 않는다★ — 지금 자리 = platform 각 모듈 헤더 · 남은 OS `cfg` 파일 명단 = `ci.yml` 의 `platform gate 4` 스텝(1-3 U8).
- **남길 것:** 도메인 지식 — 예: claude 설정 폴더는 「홈 디렉터리 찾기」만 옮기고 `.claude` 경로 지식은 backend 에 둔다(CLAUDE.md 「백엔드 확장」). 테스트를 Windows 에서만 돌리는 분기는 테스트 쪽에 남긴다.
- **인터페이스 모양(메인 판단 — 사용자가 위임):** 함수 + 핸들 타입, 컴파일 시점 분기. 트레이트 객체는 쓰지 않는다 — OS 는 실행 중에 안 바뀌어 런타임 분기가 얻는 게 없다. Job Object 는 「프로세스 그룹」 핸들 하나로 감싸 pty·stdio·codex transport 의 분기를 없앤다. 테스트용 가짜가 필요한 곳은 **쓰는 쪽이** 작은 트레이트를 둔다(discovery 의 주입 seam 이 선례).
- **뒤집는 옛 결정:** ADR-0175 결정 1 의 거부한 대안 첫 항목(platform 을 독립 crate 로) · ADR-0218 결정 11(`file_holders`·`process_tree` 는 소비자가 하나라 agent 안에). 둘 다 **소비자 수**가 기준이었고 이 후보는 기준을 **OS 의존 여부**로 바꾼다.
- **열린 것:**
  - ~~옮긴 뒤 base 에는 `logging` 만 남는다 — base 를 그대로 둘지.~~ → **닫힘: `logging` 은 base 에 그대로 둔다. base 의 역할 = 어디서든 쓰는 잎**(사용자 2026-09-26). 새 입주 후보 조사는 아래 「base 입주 후보」.
  - `windows` crate feature 를 기능별 cargo feature 로 나눌지 — `net` 은 PID 판정만 필요한데 Job Object·Restart Manager 바인딩까지 끌려온다.
  - 다른 OS 구현은 이 분리로 생기지 않는다. 빈 곳이 한 자리에 모여 보일 뿐이다.

### base 입주 후보 (조사 2026-09-26)

**기준:** 도메인 지식 0 · 입주자끼리 무참조. 소비자 수보다 **범용성**을 본다 — 「base 는 어디서든 쓰는 잎」(사용자 2026-09-26). OS 전용 코드는 후보 1 로 가므로 뺐다.
**검증:** 경량 워커가 조사했고 메인이 정의 위치만 대조했다. ★동작 차이·비공개 게이트 서술은 워커 보고이고 메인이 재확인하지 않았다★.

| 후보 | 하는 일 | 지금 위치 | 판정 |
|---|---|---|---|
| 원자적 파일 쓰기 | 임시 파일에 쓰고 rename — 반쯤 쓴 파일을 안 남김 | 공개 함수 `src-tauri/src/ui_settings.rs:687` · agent 에 손으로 두 벌(`persistence/mod.rs` · `persistence/presets.rs`) | **가장 유력.** 단 세 벌의 동작이 다르다고 보고됨(임시 파일 이름 · 실패 시 정리 · 폴더 fsync) — 하나로 합칠 동작을 먼저 정해야 한다 |
| 손상 파일 치우기 | 못 읽는 파일을 `<이름>.corrupt-<ms>` 로 옮겨 둠 | agent `persistence/mod.rs` · `persistence/presets.rs`(같은 crate 두 벌) | 범용 · 위 원자적 쓰기와 한 묶음(「JSON 파일 저장」 도우미)이 자연스럽다 |
| UTF-8 경계 자르기 | 글자를 쪼개지 않고 N 바이트로 자름 | `transport/src/ws.rs:147` · `daemon/src/experiment/record.rs:178` · agent `output_core.rs:716`(앞을 자르는 변형) | 들어간다 · 5줄짜리라 값어치는 작다 |
| 경로 표기 정리(`normalize_cwd`) | 감싼 따옴표 한 쌍 제거 · `\`→`/`. 파일시스템을 안 봄 | `agent/src/commands.rs:763` → 셸이 가져다 씀 | 들어갈 수 있다 · 계약이 「사람·LLM 이 친 cwd」로 적혀 있어 **범용 이름으로 바꿔서** 옮겨야 한다 |
| 현재 시각(epoch ms) | 벽시계 밀리초 | agent 안에 세 벌(`profile.rs:20` · `persistence/mod.rs` · `persistence/presets.rs`) | 범용 · agent 안에서 하나로 합치기만 해도 된다 |
| XML 이스케이프 | `&`·`<`·`>`·따옴표 치환 | `messaging/src/envelope.rs:126,135` | 범용(함수 자체엔 우편 지식 없음) · 소비자 하나. ~~`messaging` 이 base 를 쓰려면 아래 후보 2(게이트 완화)가 먼저다~~ ★→ 지금 옮기지 않는다 — 후보 2 철회, messaging 은 base 가 필요해질 때만 의존한다(ADR-0267)★ |
| 바이트 → hex 문자열 | 소문자 hex 인코딩 루프 | daemon 세 곳(`experiment/record.rs:189` · `lib.rs:65` · `control/mod.rs:141`) | 인코딩 루프만 범용 · 토큰 생성기 자체는 공유 금지(ADR-0086) · `sha256_hex` 는 `sha2` 를 끌고 오므로 제외 |
| 글자 수 자르기(`clip`·`truncate`) | 앞 N 글자만 | agent codex `decoder.rs` · `transport.rs`, daemon 인라인 두 곳 | **안 된다** — agent 쪽은 비밀값 가리기를 거쳐서만 부르도록 테스트가 막는 의도된 비공개다. base 로 빼면 그 문이 뚫린다 |
| 데이터 폴더 찾기(`default_data_dir`) | `ENGRAM_DATA_DIR` · `.engram-data` · 릴리즈 배치로 폴더 결정 | `discovery/src/lib.rs:84` | **안 된다** — Engram 고유 지식(ADR-0134)이라 discovery 몫 |
| 토큰 생성 | 32바이트 난수 hex | daemon 두 벌 | **안 된다** — 공유 금지가 명시돼 있다(ADR-0086) |
| 재연결 백오프 | 지수 대기 + 상한 | `src-tauri/src/daemon_client/connection.rs:87` · `transport/src/policy.rs:81`(더 풍부한 판) | **base 아님** — transport 가 이미 가진 정책이다. 셸이 transport 를 붙일 때 그쪽 정책으로 흡수하면 사라진다(부착 때 체크할 것) |
| `dunce::canonicalize` | UNC 접두 없는 정규화 | 라이브러리 직접 호출 | 뺄 것이 없다(한 줄 호출) |

- **못 본 것:** 「비어 있지 않은 환경변수 읽기」 같은 도우미가 여러 crate 에 중복돼 있는지는 확인하지 않았다(대부분 `ENGRAM_*` 도메인 읽기로 보임).
- **곁 사실:** `transport` crate 는 아직 소비자가 0 이다(자기 테스트만 쓴다 — 매니페스트·소스 대조).

### 결정 후보 4 — base 에 범용 헬퍼 네 모듈을 들인다

**정정(2026-10-03) — 아래 원안은 기록으로 남긴다:** ① 원자적 쓰기(`write_atomic`)는 base `file` 이 아니라 platform 으로 간다(ADR-0266 결정 8 — Windows 전용 rename 재시도를 품는다) ② `Clock` 은 「같은 모양」이 아니다 — 네 벌의 모양이 다르고, 합치는 모양은 ADR-0269 결정 3 이다(지금 읽기만 base · 기다리기는 쓰는 쪽 자기 트레이트 · `UsageClock` 은 보류 — 1-1 때 깔끔히 맞으면 합치고 안 맞으면 그때 사용자에게 묻는다, 사용자 2026-10-03) ③ 락 오염 복구 줄의 「(후보 3 과 같은 결)」이 가리키는 후보 3 은 번복됐다(ADR-0268 — 감싸지 않는다). 그 경고 로그는 `tracing::warn!` 을 직접 부른다(ADR-0269 결정 4) ④ messaging 의 XML 이스케이프와 command 하네스(`testing.rs`)의 락 오염 복구 3곳은 옮기지 않는다 — 후보 2 철회(ADR-0267 · ADR-0269 영향). ⑤ 2026-10-04 — sync 는 경고 없이 복구 · text 는 hex_lower 만(자르기는 std) · Clock seam 여섯 · UsageClock 유지 · transport 는 3단계 (ADR-0275).

**상태: 입주 확정(사용자 2026-09-26 — 「파일 텍스트 시간 경로 등 다 옮겨. 딱 적당하네」) · 제공 방식은 메인 제안 · ADR-0269.**

- **제공 방식(메인 제안):** 상태 없는 공개 함수. 호출은 `engram_dashboard_base::file::write_atomic(…)` 꼴. 바꿔 끼울 구현이 없으므로 트레이트를 두지 않는다(예외 = 아래 `Clock`).

  | 모듈 | 함수(이름은 가안) | 흡수 대상 |
  |---|---|---|
  | `file` | `write_atomic(path, bytes)` · `set_aside_corrupt(path, stamp_ms) -> 새 경로` | 셸 `ui_settings.rs:687` · agent `persistence/mod.rs` · `persistence/presets.rs` |
  | `text` | `truncate_bytes(s, max)`(앞을 남김) · `keep_tail_bytes(s, max)`(뒤를 남김) · `hex_lower(bytes)` · `escape_xml_attr` / `escape_xml_text` | transport `ws.rs:147` · daemon `experiment/record.rs:178,189` · daemon `lib.rs:65` · `control/mod.rs:141` · agent `output_core.rs:716` · messaging `envelope.rs:126,135` |
  | `time` | `now_epoch_ms()` | agent `profile.rs:20` · `persistence/mod.rs` · `persistence/presets.rs` |
  | `path` | `normalize_spelling(raw)`(감싼 따옴표 한 쌍 제거 · `\`→`/`) | agent `commands.rs:763`(→ 셸) |

- **지킬 것:**
  - **base 에 무거운 의존을 들이지 않는다** — JSON 직렬화는 호출부에 남기고(base 가 serde 를 안 끌어옴) `file` 은 바이트만 받는다. hex 는 직접 루프(`hex` crate 안 씀). `sha256_hex` 는 `sha2` 때문에 제외.
  - **입주자끼리 무참조(입주 조건 ③)** — 손상 파일 이름의 시각은 `file` 이 `time` 을 부르지 않고 **인자로 받는다**. CI 의 입주자 상호 참조 게이트(`rg "(crate|super)::(logging|platform)"`)에 새 모듈 이름을 더한다.
  - **오류는 `std::io::Result`** — 자체 오류 타입을 만들지 않는다.
  - `file` 은 세 벌의 동작 차이(임시 파일 이름 · 실패 시 정리 · 폴더 fsync)를 하나로 정한 뒤 옮긴다.
- **추가 후보(실측 2026-09-26):**
  - **`Clock` 트레이트가 세 벌이다** — 같은 모양(`fn now(&self) -> Instant`)을 transport(`clock.rs:20`) · daemon(`command_delivery.rs:176`) · discovery(`lib.rs:425`)가 각자 정의한다. **master 흡수(2026-10-02) 뒤 넷째가 생겼다** — daemon `usage_service/clock.rs:9` 의 `UsageClock`(실물 + 수동 가짜 포함). 시간을 가짜로 바꿔 끼우는 테스트용 seam 이다 → **확정: base `time` 으로 합친다. 규칙 = 「시간은 공용이면 무조건 합친다」**(사용자 2026-09-26). 위 「트레이트를 두지 않는다」의 유일한 예외다.
  - **테스트 대기 헬퍼** — 범용인 것은 `wait_until(timeout, cond) -> bool` 하나이고 **같은 시그니처가 9개 파일에 복사돼 있다**(agent 테스트 7 · daemon 테스트 2 — 예: `crates/engram-dashboard-agent/tests/reaper.rs:54` · `crates/engram-dashboard-daemon/tests/control_send.rs:70`). 변형(`FnMut` 판 · `poll_until` · 모듈 안 사본)도 몇 있다. 전부 동기 함수라 tokio 가 필요 없다. `wait_for_output`·`wait_for_text` 같은 나머지는 도메인을 알아서 제자리에 남는다.
    → **메인 판단: base 에 `testing` 기능 플래그 뒤 모듈로 둔다**(가짜 `Clock` 도 같은 자리). 운영 빌드에는 안 실리고 dev-dependency 에서만 켠다. 범용 헬퍼가 std 만 쓰는 함수 하나뿐이라 개발 전용 crate 를 따로 세울 값어치가 없다.
  - **락 오염 복구가 57곳에 손으로 반복된다** — `.lock().unwrap_or_else(|e| e.into_inner())` 54곳 + `PoisonError::into_inner` 판 3곳. agent 가 대부분이고 daemon · 셸 · command 에도 있다. 헬퍼는 없다. → **확정: base `sync` 모듈로 모은다**(사용자 2026-09-26). 한 곳으로 모으면 지금은 조용히 삼키는 오염을 **복구할 때 경고 로그를 남기는** 식의 통제가 생긴다(후보 3 과 같은 결).
  - **약한 것:** 「비어 있지 않은 환경변수 읽기」 — 모양이 비슷한 곳이 daemon 9 · agent 3 · protocol 1 · 셸 1 이나 대부분 `ENGRAM_*` 도메인 읽기라 범용 헬퍼로 뽑을 몫이 작다. discovery 의 `retry_if_vanished`(`lib.rs:218`)는 discovery 전용 정책이다.
- **입주 판단 규칙(사용자 2026-09-26):** 「기준을 미리 잡지 않고 지금 여러 군데 쓰는 걸로 판단」 — 도메인 지식이 없고 **지금 여러 곳에서 쓰이거나 복사돼 있으면** 메인이 판단해 base 로 옮긴다(사용자에게 묻지 않고 보고). ADR-0175 입주 조건 ①(소비자 crate 둘 이상)을 이 규칙이 대신한다 — ②(도메인 지식 0 · 워크스페이스 의존 0) · ③(입주자끼리 무참조)은 그대로.
- **나중(사용자 2026-09-26):** 새 범용 헬퍼가 생기면 메인 세션이 알아서 base 로 옮기도록 CLAUDE.md(또는 다른 자리)의 아키텍처 서술을 갱신해 유도한다.

### 결정 후보 2 — `command`·`messaging` 도 base 를 쓴다

**철회(사용자 2026-10-03 — 「필요할 때만 연결하면 되지」) → ADR-0267 재작성: 지금은 워크스페이스 의존 0 을 유지하고, base 도우미가 실제로 필요해질 때 그 변경이 필요를 적고 연결한다. 아래 원안은 기록으로 남긴다.**

**상태: 방향 확정(사용자 2026-09-26 — 「커맨드·메시지는 로깅 안 함? 당연히 해야지」) · ADR-0267.**

- **바뀌는 것:** 두 crate 의 의존 상한 게이트 기대값이 「자기 자신 1줄」에서 「자기 자신 + base」로 바뀐다(CLAUDE.md 「빌드·검증 명령」 · `ci.yml`). 「워크스페이스 의존 0」 을 성질로 적은 ADR-0155(command) · ADR-0110(messaging)의 그 부분을 뒤집는다.
- **지금 로그 현황(실측):** `messaging` 은 이미 로그를 남긴다 — `tracing` 을 직접 의존해 약 34곳(`service.rs` 등). 로그를 **찍는** 데는 base 가 필요 없고, 찍힌 로그는 그 프로세스(데몬)가 base 로 설치한 구독자를 타고 파일에 간다. `command` 는 로그가 0줄이다(`tracing` 의존도 없음) → 리팩토링 때 추가 대상.
- **그러니 base 를 쓰게 해서 얻는 것:** 로그를 base 경유로 찍는 것(후보 3 — ★2026-10-03 번복, 로그는 `tracing` 직접 · ADR-0268★) · 위 입주 후보(범용 헬퍼)를 두 crate 도 쓸 수 있다.
- **안전성 근거:** base 는 도메인 지식 0 · 워크스페이스 의존 0 을 자기 게이트가 지키므로, 두 crate 가 base 에 기대도 순환이나 도메인 유입 경로가 생기지 않는다.

### 결정 후보 3 — 로그는 base 를 거쳐서만 찍는다 (로그 라이브러리 직접 의존 금지)

**번복(사용자 2026-10-03 — 「로그 감싸지 마」) → ADR-0268 재작성: 감싸지 않는다. 아래 원안은 기록으로 남긴다.**

**상태: 방향 확정(사용자 2026-09-26 — 「로그 라이브러리를 왜 직접 쓰냐고. 나중에 컨트롤이 안 되잖아」) · 인터페이스 모양은 메인 제안 · ADR-0268.**

- **현황(실측):** `tracing` 을 직접 의존하는 crate 가 base 말고 여섯이다 — agent · daemon · discovery · messaging · net · 셸. 로그 매크로 호출은 약 530곳(daemon 193 · agent 129 · 셸 125 · net 38 · messaging 34 · discovery 9 — `tracing::` 경로 포함 대략치). `command` · `protocol` · `transport` 는 0.
- **지금 중앙에서 쥔 것:** 출력 설정 — 레벨 필터 · 파일 · 형식은 base 가 설치하는 구독자 한 곳에서 정한다.
- **못 쥔 것:**
  - **비밀값 가리기가 중앙에서 안 걸린다.** 파일 writer 는 받은 바이트를 그대로 쓴다(`crates/engram-dashboard-base/src/logging/mod.rs:135-141`). 가리기는 부르는 쪽이 각자 `mask_secrets` 를 불러야 하는 선택 사항이라, 직접 찍는 약 530곳 중 하나라도 빠뜨리면 샌다.
  - **로그 라이브러리 교체나 로그 규칙(필드 이름·형식) 강제**가 여섯 crate 를 다 고치는 일이 된다.
- **메인 제안(인터페이스 모양):**
  - base 가 로그 매크로를 **자기 경로로** 제공한다(예: `engram_dashboard_base::log::warn!`). 처음엔 `tracing` 을 얇게 감싸 호출부는 import 경로만 바뀐다. 호출부가 쓰는 표면(레벨 매크로 + `키 = 값` 필드)을 작게 유지하면 나중에 라이브러리를 바꿔도 base 만 고친다.
  - **`tracing` 직접 의존은 base 만 허용** — CI 게이트로 막는다(해석된 의존 그래프에서 base 밖의 `tracing` 직접 의존 = 0).
  - **가리기를 base 의 writer 단계로 옮겨 모든 줄에 기본으로 건다**(선택 → 기본).
- **열린 것:** 스팬(범위 추적) 같은 `tracing` 고유 기능을 쓰는 곳이 있는지는 아직 안 봤다 — 있으면 감쌀 표면이 커진다. 모든 줄에 정규식 가리기를 거는 비용은 재지 않았다(기본 레벨이 warn 이라 작을 가능성은 높다).
- **지금 만들 모양(사용자 2026-10-02 — 「일단은 함수화하고 기본적인 인자만 넣도록」):** base 가 로그 입구를 제공하고 인자는 **위험도 · 카테고리 · 메시지** 셋만 받는다. 카테고리 어휘는 큰 절 A 전까지 자유 문자열(검사 없음). 메시지 형식은 표준(Rust 포매팅 관례)을 따른다(사용자). 메인 구현 판단: 호출부에서는 함수처럼 보이되 안쪽은 매크로로 둔다 — 호출 위치(파일:줄)를 기록에 남기고, 꺼진 위험도의 메시지는 문자열로 만들지 않기 위해서다.
- **인터페이스 모양은 아래 「큰 절 A」가 정해지면 다시 본다** — 카테고리·위험도가 매크로 표면에 들어갈 자리이기 때문이다. 후보 3 이 정하는 것은 「base 를 거친다」까지다.


---

## 2. command

### 사실 (경량 워커 조사 2026-10-02 · 메인이 핵심 위치 4곳 대조)

- **명령 0개인 명령 버스 도구다.** 비테스트 약 2,540줄 — 봉투(`envelope`) · 오류 어휘(`error`) · 전송 구멍(`link`) · 선언 매크로(`macros`) · 명부(`roster`) · 배달 3단계(`route`) · 명세(`spec`) · 명령 표(`table`) · 인자 형 변환(`coerce`) · 테스트 도구(`testing`, 플래그 뒤).
- **통신 규격은 따로다.** `protocol` 이 이 crate 의 봉투·응답·선언·주인 표지를 자기 메시지 칸으로 싣는다(`protocol/src/messages.rs:4`). 실제 전송은 데몬·셸 연결 코드가 한다.
- **요청 번호는 타입 둘 · 값 하나.** `command` 와 `protocol` 에 각자 있고 같은 UUID 를 그대로 옮긴다(`protocol/src/ids.rs:30-40`, ADR-0081 결정 4). 둘인 이유 = `protocol` 은 TS 자동 생성(ts-rs)이 필요하고 도구 crate 는 일부러 안 들였다.
- **조립은 프로세스마다.** 데몬 `command_delivery.rs` · 셸 `lib.rs:132`(+ `daemon_client/inbound.rs`) · 프론트 `src/commands/registry.ts`.
- **데몬 `command_roster.rs` 는 감싼 것이다(중복 구현 아님)** — 도구의 명부에 연결 생존·연결별 송신구를 얹는다. `net` 을 알아서 도구로 못 옮긴다.
- **데몬은 배달 3단계 중 1단계를 우회한다** — 빈 표를 넘겨 2·3단계만 쓰고(`daemon/src/command_delivery.rs:1833`), 1단계(자기 명령 실행)는 자체 포트로 따로 돈다. 3단계가 그대로 도는 곳은 셸뿐이다.

### 후보 (메인 판단 대기 — 워커 판정 + 근거)

| 무엇 | 판정 | 근거 |
|---|---|---|
| `route.rs` 의 패닉 그물 함수들을 따로 떼기 | 쪼갬(낮음) | `route` ↔ `table` 이 서로 부른다(`table.rs:339`). 같은 패닉 잡기가 세 곳에 있다 |
| 「주인에게 넘기기」(2·3단계)를 `route` 와 별도로 공개 | 쪼갬(검토) | 데몬이 빈 표를 넘기는 우회를 없앤다. ADR-0155 결정 3 과 대조 필요 |
| 읽기/쓰기 표식(`Effect`)에 직렬화 추가 | 추가 | 셸이 같은 것을 `ViewEffect` 로 복제했다(`view_commands.rs:107`). 철자는 wire 결정 |
| 실행 중 만든 명세용 카탈로그 항목 생성기 | 추가 | 셸이 같은 모양과 오류 목록을 손으로 짰다(`view_commands.rs:573-633`) |
| 「재시도 안 함」 오류 생성자 · 이미 끝난 응답 헬퍼 | 추가(사소) | 데몬·셸에 사본 |
| 데몬 `command_delivery.rs`(2.4k줄) 를 폴더로 쪼개기 | 쪼갬(데몬 내부) | 네 가지 일 — 요청 자리 표 · 데몬 자체 명령 실행 · 배달 · 제어 경로 버스. 데몬 라운드에서 다룬다 |
| 요청 번호 두 타입 | 유지 | ts-rs 결정과 묶여 있다 |
| 명부·자리 표·인자 검사 묶음을 도구로 | 유지 | 쓰는 곳이 하나거나 의미가 다르다 · 도구에 tokio 를 끌고 온다 |
| 프론트 사본(명세 모양 · enum 철자 맞추기) | 유지(메모) | 프론트 라운드에서 `Effect` 직렬화와 함께 본다 |

- **곁 사실:** 데몬 `command_delivery.rs:176` 의 `Clock` 은 base 로 합칠 시간 인터페이스 목록(위 후보 4)에 이미 들어 있다.

---

## 3. 크레이트 구성 — 셸이 agent 를 의존하는가

### 사실 (코드 대조 2026-10-02)

- 셸은 agent crate 에서 **세 가지만** 쓴다(호스팅 아님): `COMMAND_SPECS`(`src-tauri/src/view_commands.rs:204` — 웹뷰가 못 가져갈 예약 이름) · `llm_creation_refusal`(`src-tauri/src/layout/apply.rs:36` — 슬롯 스폰 전 백엔드 정책 거름) · `normalize_cwd`(`src-tauri/src/layout/commands.rs:35`).
- **이름 충돌은 데몬이 이미 막는다** — 데몬이 스스로 답하는 이름을 클라이언트가 등록하면 `CONFLICT` 로 **등록 패킷 통째로** 거절한다(`crates/engram-dashboard-daemon/src/connection_core.rs:1146`). 셸 예약 목록은 그보다 먼저 거르는 두 번째 그물이고 `mail.*` 을 못 봐 구멍이 있다.
- **셸 명령과 웹뷰 명령은 한 패킷으로 등록된다** — 거절되면 이 셸의 명령이 다음 재연결까지 **0개**가 된다(LLM 이 창·탭·슬롯을 못 만짐). 지금 셸은 그 거절을 `warn` 로그 한 줄로만 남긴다(`src-tauri/src/daemon_client/connection.rs:1968-1975`).
- 백엔드 정책 판정의 정본은 agent 한 곳이고 데몬의 `agent.new` 가 같은 표를 본다(ADR-0219). 셸 쪽은 먼저 거르는 복제다.

### 결정 후보 5 — 셸 → agent 의존을 끊는다

**상태: 방향 확정(사용자 2026-10-02 — 「데몬에 거절만 하면 딱히 복잡하게 할 필요 없을 듯 … 실행해서 안 되면 다시 빌드하면 되니깐」) · ADR-0270.**

- 예약 이름 목록을 걷는다. 벽은 **데몬의 등록 거절 하나**로 둔다. CI 대조 스크립트는 두지 않는다.
- **단 거절을 클라이언트가 OS 메시지 박스로 띄운다**(사용자 2026-10-02 — 「빵 하고 뜨면서 클라에서 뜨면 유저도 알지」). 띄우는 쪽 = 셸(클라). 플랫폼마다 다른 창이므로 OS 분기는 셸이 아니라 OS 의존 코드 자리(후보 1)나 Tauri 의 대화상자 기능 뒤에 둔다 — 구현 때 고른다. **LLM 통지는 두지 않는다** — 원인은 로그에 남고 에이전트가 로그를 뒤지면 나온다(사용자). 지금의 `warn` 로그 한 줄은 그대로 남긴다.
- **원칙(사용자 2026-10-02):** 「클라는 데몬이랑 아예 별도의 개념이라서 agent 를 모르는 게 좋음」 — 셸(클라)은 agent crate 를 의존하지 않는다.
- 백엔드 정책 거름은 데몬에 맡긴다. ★**단 셸 검사를 그냥 지우면 구멍이 난다**★ — 정책 함수를 부르는 곳은 `agent.new` 처리 한 곳뿐이고(`crates/engram-dashboard-agent/src/commands.rs:1077`), 셸의 슬롯 스폰은 그 명령을 거치지 않고 통신 메시지 `SpawnByCwd` 를 바로 보내며(`src-tauri/src/layout/apply.rs:597`) 데몬은 그 경로에서 정책을 안 본다. → **순서: 슬롯 스폰을 데몬의 `agent.new` 명령 경로로 바꾼 뒤 셸 검사를 지운다.** 슬롯 스폰은 「검사 → 데몬 스폰 → 배치」 순이라 데몬이 거절해도 레이아웃이 안 바뀐다(`layout/apply.rs:585-620`). 거부 문구가 데몬을 한 번 다녀오고, `tests/layout_apply.rs` 의 「두 입구가 함께 열리나」 시험과 ADR-0219 의 두 입구 서술을 고친다. → 경로는 ADR-0279 로 바뀌었다(정책 벽 = 데몬 `SpawnByCwd` 처리부 · 슬롯 스폰 경로 그대로).
- `normalize_cwd` 는 base 로(후보 4).
- 얻는 것: 셸 실행 파일이 agent crate 전체(PTY · Windows 프로세스 관리 · 명령 수집 등)를 안고 가지 않는다. 「셸 = 데몬 클라이언트」(ADR-0029)가 의존 그래프에서도 보인다.

---

## 4. discovery

### 사실 (코드 대조 2026-10-02)

- **원래 이유(ADR-0024):** 데몬이 데이터 폴더의 `daemon.json` 에 포트·토큰·PID 를 적고 클라이언트가 같은 파일을 읽어 접속·인증한다 → 양쪽이 **같은 폴더 규칙**을 써야 해서 그 규칙(`default_data_dir`)을 공용 crate 에 두었고, 클라이언트 쪽 데몬 찾기·띄우기·끄기를 함께 넣었다.
- **실제로 양쪽이 다 쓰는 것은 `default_data_dir` 하나뿐이다.** 쓰기 가능 확인 · 설치 위치는 데몬(+ 데몬 crate 안의 `engram` CLI)만, 찾기·띄우기·상태·정지·실행 파일 찾기는 셸만 쓴다.
- **안에 작은 데몬 클라이언트가 있다** — `send_stop` 이 WebSocket 접속·인증·정지 명령을 직접 한다. discovery 가 `net`(인증 메시지)과 `protocol` 명령 메시지를 아는 이유가 전부 이것이다.
- 실행 파일 위치 계산이 두 곳에 갈려 있다 — 데몬(`daemon/src/lib.rs:96,146`) · discovery(`lib.rs:981`). **→ 실측 정정(TRD 2-2 §1-3): 세 곳** — 데몬 `locate_send_exe` · discovery `locate_daemon_exe` · 하네스 `roundtrip_smoke.rs` `sibling_send_exe`. 옛 줄 둘은 낡았다. **처리 2-2 U1**: 셋이 platform `env::sibling_exe` 한 곳을 부른다(ADR-0282).
- T-10(2026-08-26)에서 「discovery 를 없애지 않는다」로 종결한 적이 있다. 아래 후보 6 은 **새 근거**(클라·데몬 분리 원칙 · 원격 대비)로 그것을 다시 연다.

### 결정 후보 6 — discovery 를 뽀개 데몬·셸에 각각 둔다

**상태: 방향 확정(사용자 2026-10-02) · ADR-0271 · T-10 재개.**

- **사용자 근거:** 네트워크 방식이 되면 폴더 규칙은 데몬만 필요하다 · 같은 PC 모드도 원격과 **같은 인증 방식**으로 하면 되고 로직을 갈라 둘 필요가 없다(편의용 자동 인증은 나중) · 「지금 당장은 discovery 뽀개고 데몬 셸 각각 두고 나중에 관련해서 얘기할 때 분리」.
- **나누는 법:**
  - 데몬으로: 데이터 폴더 규칙(`default_data_dir` · `release_data_dir` · 쓰기 가능 확인) · 설치 위치(`find_install_root`) · 실행 파일 위치 계산(두 곳 → 한 곳).
  - 셸로: 데몬 찾기 · 띄우기 · 상태 · 프로세스 끄기 · 실행 파일 찾기. 정지 명령(`send_stop`)은 셸의 데몬 연결 코드로 — 그러면 셸 밖 어디에도 작은 데몬 클라이언트가 안 남는다.
  - WMI 로 띄우는 코드는 `platform`(후보 1).
- **과도기 연결 하나:** 원격 인증 경로가 생기기 전까지 셸은 `daemon.json` 을 계속 읽어야 하므로 **데몬이 그 파일을 쓰는 위치**를 셸도 알아야 한다. 셸은 데몬 crate 를 의존하지 않으므로(클라·데몬 분리) 위치 규칙이 셸 쪽에 한 벌 더 생긴다 → 두 벌이 같은 경로를 내는지 재는 시험으로 묶는다(셸은 데몬을 테스트 전용 의존으로 이미 끌어온다). 원격·같은 PC 인증 통일 때 이 연결은 사라진다.
- **나중(원격 작업 때):** 클라이언트가 「데몬 주소와 토큰을 어디서 얻나」를 인터페이스로 두고, 같은 PC 도 같은 인증 방식으로 통일 · 편의용 자동 인증 검토. 토큰은 서버가 발급하고 클라이언트가 열쇠로 들고 있는 모델 그대로.

---

## 5. 정리 원칙 — 애매한 것은 일단 데몬으로 (사용자 2026-10-02)

- **원칙:** 「애매한 건 데몬으로 몬 다음에 위에 정리되고 다시 데몬 정리를 하는 방향」. 자리가 애매한 코드는 crate 를 새로 세우거나 공용으로 두지 않고 **일단 데몬 안으로 넣는다.** 경계(위)가 다 정리된 뒤 데몬 라운드에서 다시 쪼갠다.
- **순서(메인 판단 — 사용자 위임):** ① 경계 리팩터링(후보 1·2·4·5·6 — 단 discovery 의 정지 명령 클라이언트 이전은 ② 로) → ② transport 를 셸에 붙인다(정지 명령 클라이언트 · 접속 정보 인터페이스 함께) → ③ transport 를 데몬에 붙인다 → ④ 데몬 정리.

## 6. transport · net

### 사실

- `transport` 는 독립 crate · 워크스페이스 의존 0 · 소비자 0. 연결 기계(메시지 단위 · 요청/응답 짝짓기 · 스트림 순번·이어받기·구멍 찾기 · 핸드셰이크 · keepalive · 쓰기 시한 · 재연결 · 상대 N 개)를 갖고, 메시지 어휘는 모른다(ADR-0177).
- ADR-0177 결정 1: **데몬·셸 양쪽에** 각자 전용 폴더를 두고 붙이며, 최종적으로 transport 가 `net` 의 네트워크 담당 자리를 가져간다. 다중 데몬은 상대를 하나 더 올리는 것뿐이라 지금은 현재 구조(데몬 하나·셸 하나)에 붙인다.
- `net` 은 세 가지를 섞는다 — 데몬 전용 서버 연결 코드 · 클라이언트도 쓰는 인증 메시지 타입 · 데몬 전용 일(단일 인스턴스 · `daemon.json` 쓰기 · 접속 허용 판정). 클라이언트가 데몬 쪽 crate 를 알게 되는 것을 기능 플래그로 겨우 막고 있다.

### 결정 후보 7 — transport 로 바꾼 뒤 net 을 걷는다

**상태: 방향 확정(사용자 2026-10-02) · ADR 미작성 — transport TRD 때 · 착수 = 위 순서 ③.**

- 공통 연결 기계 → `transport` · 공통 어휘(인증 메시지 포함) → `protocol` · 데몬 전용 일 → **데몬 안**(위 5 의 원칙 — 데몬 라운드에서 다시 본다).
- 그 결과 `net` 은 crate 로 남을 이유가 거의 없다. 셸은 `net` 을 모르게 된다.
- **검증 상태:** 데몬 쪽에 무엇이 남는지는 ADR-0177 결정 1·3·4 와 `net` 현재 구성으로 추정했다 — 부착 TRD 전이라 확정 아님.
- **열린 질문 — 인증을 따로 떼나(사용자 2026-10-06 「인증 쪽은 모듈이 따로 있어야 되지 않나」) · transport TRD 에서 정한다.** 지금 인증은 세 조각이 `net` 한 곳에 섞여 있다: ① 핸드셰이크 프레임 **모양**(`net/src/auth.rs` `AuthFrame` — 셸도 쓴다) ② 그것을 **판정**하는 일(`net/src/ws.rs` — Origin · 비밀값 대조 · 접속 허용) ③ 비밀값을 **만들고 나누는** 일(데몬이 `daemon.json` 에 쓰고 셸 · CLI 가 읽어 보낸다). 후보 7 의 현재 배치는 ① → `protocol` · ②③ → 데몬 안이고, `transport` 는 인증을 모른다(실측 — `transport/src/` 에 인증 어휘 0). 메인 의견(미결정): ②③ 을 데몬 안에 흩지 말고 **한 모듈(인증 seam — 판정 트레이트 하나 + 현행 구현)** 으로 모은다. crate 로 뗄지는 원격 대비(나중 트랙)에서 클라이언트 쪽에도 판정 · 자격 로직이 생기면 그때 — 지금 클라이언트가 하는 일은 「파일에서 읽어 보내기」뿐이라 crate 의 소비자가 데몬 하나다.

---

## 7. protocol

### 사실

- `protocol` = 셸·데몬이 주고받는 **패킷 목록**(+ 인코딩 · 프로토콜 버전 · 프론트용 TS 바인딩). 경계 = command 에 기대고 데몬·셸·프론트가 쓴다(discovery·net 은 후보 6·7 로 사라짐).
- command 에서 가져다 품는 것 = 명령 버스 양식 넷(봉투 · 답장 · 등록 항목 · 주인 표지 — `crates/engram-dashboard-protocol/src/messages.rs:4`).
- 기능 데이터를 싣는 길이 이미 둘 있다 — ① 명령 버스(모듈이 모양을 정하고 JSON 「속」으로) ② `protocol` 사본 타입(에이전트 목록 등 — 데몬 끝자락에서 옮겨 담음).

### 결정 후보 8 — 패킷 정의는 남의 구조체를 품지 않는다

**상태: 방향 확정(사용자 2026-10-02 — 「패킷이 똥통이 되잖아」 · 「바닥 구조체가 아니라 장기적으론 모든 구조체를 품어야 되는 거잖아」) · ADR-0272 · 착수 = transport 부착 때.**

- **원칙:** `protocol` 은 다른 crate 의 구조체를 품지 않는다 — 바닥 도구(command)도 예외가 아니다. 한 번 허용하면 기능이 늘 때마다 모든 구조체가 흘러든다.
- **기능 데이터:** `protocol` 이 통신용 사본을 갖고 셸·데몬 끝자락(transport 어댑터)에서 옮겨 담는다.
- **확정 모양(사용자 2026-10-02 — 「다른 건 다 패킷 구조체 방식으로 하고 command 만 json 으로」):**
  - **고정 패킷**(에이전트 목록 · 상태 · 구독 · 프로필 등) = `protocol` 에 타입으로 정의(Zed 식 · 커지면 영역별 모듈로 나눔). 변환은 데몬·셸 끝자락(agent 는 `protocol` 을 모른다).
  - **명령 버스** = 패킷의 명령 칸 넷(명령 · 답장 · 등록 · 등록 변경)을 `Box<RawValue>` 로 — 원문 바이트만 나르고 봉투 구조체로 푸는 것은 셸·데몬 끝자락. JSON-RPC 봉투 · Tauri 명령과 같은 모양(명령은 계속 늘어나는 열린 집합). → `protocol` → command 의존이 사라지고 통신선 바이트는 그대로다.
  - **답장 짝 맞추기(메인 판단):** 끝자락 어댑터가 패킷을 받는 즉시 봉투·답장을 구조체로 풀고, 짝 맞추기용 요청 번호는 **푼 구조체에서** 읽는다(transport 의 `reply_tag` 가 어댑터 안이므로 자연스럽다). 번호를 칸 밖으로 빼지 않으므로 바이트·프로토콜 버전이 그대로이고 추가 파싱도 없다. 지금 `protocol` 안에 있는 짝 맞추기 함수(`messages.rs:1048`)는 어댑터로 옮긴다.
  - 비용: 바깥 패킷을 풀 때 봉투를 한 번 훑어 검증하고 끝자락에서 한 번 더 푼다(중간 트리는 안 생김) — 명령 패킷은 드물어 무시할 수준. 근거 = `../research/wire-shared-types-placement-2026-10-02.md`.
- **새로 짤 것:** 「패킷은 풀렸는데 봉투가 깨진」 경우의 처리 · 데몬 중계 경로의 풀기/다시 싸기.
- **근거 조사:** `../research/wire-shared-types-placement-2026-10-02.md` — rust-analyzer(사본 + `to_proto`/`from_proto`) · Cargo `cargo-util-schemas`(행동 없는 공용 스키마) · JSON-RPC 봉투(불투명 칸)가 이 모양이고, 패킷 crate 가 남의 타입을 품는 모양을 권하는 1차 자료는 없다.

---

## 8. 데몬 경계 · engram CLI

### 사실 (코드 대조 2026-10-02)

- `engram.exe`(에이전트가 자기 터미널에서 부르는 제어 CLI)는 **데몬 패키지의 두 번째 bin 타깃**이다(`crates/engram-dashboard-daemon/Cargo.toml:37-39` · 소스 `src/bin/engram.rs` 비테스트 2,791줄). 데몬 lib 를 한 줄도 쓰지 않는다 — 쓰는 것은 agent 의 명령 어휘 상수(`engram.rs:117-121`) · command 의 요청 번호 · discovery 의 설치 위치 셋뿐. 그런데 같은 패키지라 빌드 때 데몬 의존 전체를 끌고 온다(HTTP 를 손으로 짤 만큼 의존 최소화가 의도였다 — 같은 `Cargo.toml` 주석). **→ 처리 2-3(2026-10-08)**: 독립 패키지 `crates/engram-dashboard-cli` 로 옮겼다(U2 `58c80a3` · 의존 = agent · command · serde_json). 그 사이 2-2 U2 가 설치 위치를 데몬 lib 로 옮겨 CLI 가 데몬 lib 를 한 줄(help 본문 경로) 불렀고, 2-3 U1(`2de9756`)이 help 를 데몬으로 옮기며(`/control/help`) 그 줄째 걷었다 — 아래 후보 9 의 「설치 위치 규칙 한 벌 더」는 대상이 없어졌다(ADR-0285 · step-log).
- CLI 어휘 상수(동사 · 플래그 · 실행 파일 이름 · 상태 낱말)는 agent 자신도 쓴다(`manager.rs` · `commands.rs` · 백엔드) — 「`agent.*` 명령의 어휘」라 agent 소유가 맞다.
- 셸 테스트가 데몬 패키지를 테스트 전용 의존으로 끌어온다(`src-tauri/Cargo.toml:104`) — 테스트용 서버 함수(`start_test_server*`, `crates/engram-dashboard-daemon/src/lib.rs:915-929`)가 테스트 표시 없이 공개 API 로 나가 있어서다. **→ 처리 2-4(2026-10-08 · ADR-0286)**: 그 함수들을 데몬 기능 `test-support` 뒤로 옮겼고 셸의 데몬 dev 의존이 그 기능을 켠다(간선은 남는다).

### 결정 후보 9 — engram CLI 를 독립 패키지로

**상태: 방향 확정(사용자 2026-10-02 — 「engram 은 독립 패키지로 빼는 게 맞는데 그냥 shell 처럼 하나 분리」) · ADR-0273.**

- 셸(`src-tauri` → `engram-dashboard.exe`)처럼 **exe 하나를 뽑는 패키지**를 따로 둔다 — 위치 `crates/` 아래, 패키지 이름은 `engram-dashboard-` 접두를 지킨다(CI 의존 상한 게이트가 워크스페이스 멤버를 그 이름 접두로 식별한다 — 다른 이름이면 게이트를 그냥 통과한다). **bin 이름은 `engram` 그대로**(agent `CLI_EXE_NAME` 과 맞물림 — 바꾸면 우편이 조용히 멈춘다).
- 의존 = agent(명령 어휘) · command(요청 번호) · 설치 위치 규칙. 설치 위치 규칙은 discovery 가 사라지면 데몬으로 가므로(후보 6) CLI 쪽에 한 벌 더 두고 같은 경로를 내는지 시험으로 묶는다(데몬·셸의 `daemon.json` 위치와 같은 처리).
- 남은 데몬 경계(미결): 테스트용 서버 함수를 테스트 전용 기능 플래그 뒤로 — 셸 통합 테스트가 실제 데몬을 띄워 쓰므로 셸 → 데몬 테스트 의존 자체는 남는다. **→ 처리 2-4(2026-10-08 · ADR-0286)**: 데몬 기능 `test-support` 뒤로 옮겼다 — 이 항목은 닫혔다.

## 9. 셸 — 나중 (사용자 2026-10-02)

- 「일단 한 곳에 있다가 나중에 분리할 거니. 플러그인도 돼야 하고 좀 천지개벽이 발생될 거라서」 — 셸은 경계 정리(agent·net·discovery 끊기)만 하고, 안쪽 분리는 플러그인 설계와 함께 따로 다룬다(T-27 과 묶임).
- **플러그인으로 하고 싶은 것(사용자 2026-10-10 — 「그냥 한번 얘기한 거야 나중에」 · 결정 아님)** — ① JSON 모드 채팅 스타일 풀 커스터마이징(예: 우편 기본 = 네모 상자 + 우편 아이콘인데 사용자가 HTML 양식으로만 보게 바꿀 수 있게) ② Engram Slot 추가 개발 ③ 로컬 모델 커스텀 추가 — 무엇이 나올지 몰라 지금은 뺀다. 순서 = 셸 설계 때(transport 뒤).
  - 메인이 미리 짚은 것(조사 전 · 지식 기반): ①은 갈아 끼우는 깊이가 셋(테마 값 · HTML 템플릿 · 코드 컴포넌트)이다. ②는 ADR-0060(슬롯 내용 = 고정 목록 · 근거 「플러그인 생태계가 없다」)을 다시 열고, 레이아웃 권위가 백엔드라(ADR-0035 · 0057) 백엔드가 모르는 슬롯 종류를 불투명하게 보관해야 하며, 「LLM-우선 제어」상 플러그인 슬롯도 명령 버스에 자기 명령을 올려야 한다. ★가장 큰 갈림길 = 격리★ — 웹뷰가 셸 · 데몬 IPC 를 쥐고 채팅 내용은 LLM 출력(신뢰 불가)이라, HTML · 코드 커스터마이징은 샌드박스(iframe) 여부부터 정한다. 셸 설계 착수 때 `/research` 로 성숙한 곳(VS Code 노트북 렌더러 · 웹뷰 · JupyterLab MIME 렌더러 · Obsidian 등)부터 본다.

---

## 10. 작업 순서와 진행 방식 (2026-10-02 — 메인 제안 · 사용자 검토)

**원칙:** 아래에서 위로(바닥을 먼저) · 단계마다 빌드 초록 · 넓게 건드리는 작업은 혼자 · transport 는 맨 뒤 별도 단계.

- **0. ADR** — 후보 1~9 박제 → ADR-0266~0273 작성(2026-10-02 · 후보 7 은 transport TRD 때로 미룸). 뒤집히는 것: ADR-0175 결정 1(platform 독립 crate 거부 · 입주 조건 ①) · ADR-0218 결정 11 · ~~ADR-0155/0110 「워크스페이스 의존 0」~~(후보 2 철회로 뒤집지 않는다 · ADR-0267 재작성 2026-10-03) · ADR-0219 두 입구 서술 · ADR-0024 데이터 위치 공유 + T-10 종결 · ADR-0155 결정 3(봉투를 `protocol` 에 싣기).
- **1. 바닥**
  - 1-1 base 공용 함수 + 교체(후보 4) — ~~command·messaging 게이트 완화(후보 2)~~ 빠졌다(후보 2 철회 — 필요해질 때만 연결한다 · 사용자 2026-10-03 · ADR-0267) · ★원자적 쓰기는 여기서 빠진다★(Windows 전용 rename 재시도를 품어 platform 으로 간다 → 1-3 · 메인 판단 2026-10-03 · ADR-0266/0269). 손상 사본 치우기 통일은 1-1 그대로 — ★단 storage P3 착지 뒤★(아래 진행 방식). 나머지(text · time · path · sync · testing)는 먼저 간다.
  - 1-2 → 비워 둔다(옛 로그 전환 — 사이드 작업으로 뺐다가 2026-10-03 에 없어졌다 · ADR-0268. 옛 기록이 1-2 를 가리킨다).
  - 1-3 platform crate(후보 1) — Job Object 를 「프로세스 그룹」 핸들로 · 원자적 쓰기 여러 벌의 동작을 하나로 정해 옮김(★storage P3 착지 뒤 — 나머지 platform 이전은 먼저 간다★) · **kill 인과 불변식이 걸려 full QA**.
- **2. 클라·데몬 떼기**
  - 2-1 셸 → agent 끊기(후보 5) — 선행 1-1 · **① 슬롯 스폰을 `agent.new` 경로로 → ② 셸 검사 제거 순서 엄수** · ③ 등록 거절 메시지 박스 · GUI 실측. → 경로는 ADR-0279 로 바뀌었다(정책 벽 = 데몬 `SpawnByCwd` 처리부 · 슬롯 스폰 경로 그대로). **→ 착지 2026-10-07**(U1 `8b48386` · U2 `16f81be` · U3 `7222b90` — 거절 박스 = ADR-0281 · step-log).
  - 2-2 discovery 나누기(후보 6, 정지 명령 클라이언트 제외) — 선행 1-3 · 데몬 기동 실측. **→ 착지 2026-10-07**(U1 `a0ec7f4` · U2 `3c9bf33` · U3 `b42e139` · U4 `d2f9822` — crate 삭제 · 정지 명령 클라이언트는 3-2 까지 셸 `daemon_client/stop.rs` 임시 거처 · 세부 = ADR-0282 · step-log).
  - 2-3 engram CLI 독립 패키지(후보 9) — 선행 2-2. **→ 착지 2026-10-08**(U1 `2de9756` — help 를 데몬이 낸다(`/control/help`) · CLI 공통 실패 길 · U2 `58c80a3` — 패키지 `engram-dashboard-cli` · U3 — 게이트 · 문서 · 세부 = ADR-0285 · step-log).
  - 2-4 테스트용 서버 함수를 테스트 전용 플래그 뒤로. **→ 착지 2026-10-08**(세부 = ADR-0286 · step-log).
- **3. transport**(별도 TRD · 별도 브랜치) — 3-1 `protocol` 정리(후보 8 · 인증 메시지 이전 · 의존 0 게이트) · 3-2 셸 부착(어댑터 · 정지 명령 클라이언트 · 접속 정보 인터페이스) · 3-3 데몬 부착 + net 걷기(후보 7).
- **4. 안쪽 정리** — 데몬 구조 문제 · command 작은 정리 넷 · agent `transport/` 이름 변경 · agent 쪼갤지. 셸 안쪽은 플러그인과 함께.
- ~~사이드 작업 — 로그 base 경유 전환(후보 3 · 옛 1-2)~~ — 없어졌다(사용자 2026-10-03 「로그 감싸지 마」 · ADR-0268 재작성).
- **나중(별도 트랙)** — 로깅 시스템 설계(큰 절 A) · 원격 대비.

**진행 방식(사용자 2026-10-02):**
- 브랜치 = 0~2단계 하나(`v0.3.3/refactor/<슬러그>`) · 3단계 따로.
- **단위(단계)마다 push** → CI 초록 → master 머지(머지 커밋 · 체크아웃 없는 통합 절차). push·머지는 매번 사용자 확인.
- **핸드오프마다 origin 업데이트를 받아 작업 브랜치에 병합**한다.
- **커밋 뒤 다른 워크트리 동기화는 사용자가 전달**한다(세션이 다른 워크트리를 건드리지 않는다).
- **작업하며 코드베이스를 훑다 보이는 리팩터링 관련 사항은 그때그때 아래 11절에 적립한다**(사용자 2026-10-02 — 「지나가면서 계속 메모해 놔」). 묻지 않고 적고, 결정이 필요한 것만 묶어 올린다.
- **1단계는 TRD 로 시작한다**(사용자 2026-10-03) — 착수 = 다음 세션. 구현 갈림길(하나로 합칠 원자적 쓰기 동작 · 시계 트레이트 세부 등)은 선택지로 사용자에게 올린다.
  - **1단계 TRD 완료(2026-10-03~04)** — 1-1 = `../process/S21-crate-boundaries/trd-1-1-base-helpers.md` · 1-3 = `../process/S21-crate-boundaries/trd-1-3-platform-crate.md`. 갈림길은 사용자 위임(2026-10-04 「알아서 진행해」) → 권고안 채택 = ADR-0275. 다음 = 1-1 U1 · 1-3 은 1-1 머지 뒤.
- **파일 도우미 통일은 storage P3 가 master 에 착지한 뒤에 한다**(사용자 2026-10-03 「알아서」 → 메인이 권고안 적용) — 원자적 쓰기 `write_atomic`(→ platform · 1-3)과 손상 사본 치우기 `set_aside_corrupt` 통일(1-1)이 해당한다. 사유 = 다른 워크트리(wt1)의 storage P3(셸 화면 상태 `shell\state\state.json` · `../process/S21-storage/trd.md`)가 셸 `src-tauri/src/fsutil.rs` · 설정 저장소(`src-tauri/src/settings/`) 자리에 파일 쓰기 코드를 더한다. 1단계의 나머지(text · time · path · sync · testing 도우미 · `write_atomic` 을 뺀 platform 이전)는 먼저 간다.

---

## 11. 지나가며 본 것 — 정리 후보 적립 (계속 쌓는다)

결정 후보로 아직 안 올린 관측. 단계 작업 중 해당 자리를 지나갈 때 처리하거나 4단계 안건으로 넘긴다.

| 무엇 | 위치 | 처리 시점 |
|---|---|---|
| ~~CLAUDE.md 「백엔드 모듈 맵」 agent 항목의 「남은 `platform` 은 Job Object 래퍼 하나뿐」은 낡았다(실제 셋)~~ **→ 처리 1-3 U3**: agent `platform` 모듈이 사라지며 그 문장은 「`platform` 모듈도 없다」로 바뀌었다(표시 = 1-3 U8) | `CLAUDE.md` · ~~`crates/engram-dashboard-agent/src/platform/mod.rs:1-2`~~(삭제됨) | ~~1-3(platform 이전) 때 문장째 고침~~ 처리됨 |
| ~~셸 `Cargo.toml` 주석 「agent 에서 `COMMAND_SPECS` 하나만 남았다」는 낡았다(실제 셋)~~ **→ 처리 2-1 U2**: 셸의 agent 의존을 그 주석째 지웠다(그 전에 주석은 이미 「둘」로 고쳐져 있었다 — TRD 2-1 §1-5) | `src-tauri/Cargo.toml:62` | ~~2-1(셸 → agent 끊기) 때 의존째 사라짐~~ 처리됨 |
| 데몬이 `tracing-subscriber` 를 선언만 하고 쓰지 않는다 | `crates/engram-dashboard-daemon/Cargo.toml` (CLAUDE.md 「의존성」 절 기록) | 4단계(데몬 정리) |
| agent 가 TS 바인딩을 생성·커밋하는데 프론트가 가져다 쓰는 곳이 0 — 그런데 ts-rs 를 운영 의존으로 안고 있다 | `crates/engram-dashboard-agent/bindings/` | 4단계(agent) — 별칭 import 여부부터 확인 |
| agent 안 `transport/`(PTY·stdio)와 transport crate(WebSocket) 이름이 같다 | `crates/engram-dashboard-agent/src/transport/` | 4단계 이름 변경 |
| ~~실행 파일 위치 계산이 두 곳에 갈림~~ **→ 처리 2-2 U1**: 실제로는 세 곳이었다 — 셋 다 platform `env::sibling_exe` 를 부른다(ADR-0282) | ~~`crates/engram-dashboard-daemon/src/lib.rs:96,146` · `crates/engram-dashboard-discovery/src/lib.rs:981`~~ 실제 = 데몬 `src/lib.rs` `locate_send_exe` · discovery `src/lib.rs` `locate_daemon_exe` · 데몬 `src/bin/roundtrip_smoke.rs` `sibling_send_exe` | ~~2-2(discovery 나누기)~~ 처리됨 |
| 셸이 command 의 읽기/쓰기 표식·카탈로그 항목을 손으로 복제 | `src-tauri/src/view_commands.rs:107,573-633` | 4단계(command 정리 3·4) |
| 패킷 `CommandListEntry` 가 명부 항목 `RosterEntry` 와 거의 같다(`available` 은 늘 참) | `crates/engram-dashboard-protocol/src/messages.rs:392` · `crates/engram-dashboard-command/src/roster.rs:10` | 3-1(`protocol` 정리) 때 함께 볼 것 |
| 입구 인자 검사의 순서 함정(`contains` → `check_args` → `call`) — 입구가 둘이 되면 묶음 함수로 | `crates/engram-dashboard-daemon/src/control/commands.rs:172-182` | 입구가 늘 때 |
| `Clock` 시간 인터페이스가 네 벌(transport · daemon `command_delivery` · discovery · daemon `usage_service`) | 후보 4 표 | 1-1 |
| ~~★**후보 6 과 부딪힐 수 있다 — 2-2 착수 전에 사용자와 다시 본다**★~~ **→ 풀림: 사용자 2026-10-02 — `DataLayout` 도 쪼갠다 → ADR-0271 결정 4.** 원 서술: master 의 저장 구조 개편(ADR-0264, 2026-10-02)이 데이터 루트를 컴포넌트별(`daemon\{state,run}` · `shell\{config,state}` · `webview\` · `logs\`)로 나누고 그 경로의 **단일 출처 `DataLayout` 을 discovery 에 새로 두었다**. 후보 6(「discovery 를 뽀개 경로 규칙은 데몬으로, 셸은 각자」)과 「셸·데몬 경로를 한 곳에서 계산」이 갈린다 | `crates/engram-dashboard-discovery/src/layout.rs` · ADR-0264 | ~~2-2 전 사용자 결정~~ 풀림(ADR-0271 결정 4) |
| `use tauri` 격리 게이트(agent · base · transport · platform)의 `^\s*use tauri` 앵커는 `pub use tauri::…` · `use ::tauri::…` 를 못 잡는다 — 해석된 의존 그래프에서 tauri 부재를 재는 게이트(`cargo tree -i tauri`)로 넓힐지 · 출처 = 1-3 U0 codex 리뷰(2026-10-05) | `.github/workflows/ci.yml` 의 그 게이트 넷 · CLAUDE.md 「빌드·검증 명령」 · `.claude/skill-bindings/qa.md` 4 · 4b · 4e | 미정(메인 판단) |
| transport 헤더 게이트 ① 의 「이 crate 의 패키지 이름에서 그 접두를 떼면 게이트가 조용히 눈을 감는다」는 틀렸다 — 자기 이름을 바꾸면 `cargo tree -p` 가 죽거나 자기 줄이 접두를 잃어 그 게이트는 빨개진다. 실제 구멍은 접두 없는 멤버를 의존하는 것과, 이 crate 가 접두를 떼면 **남의** 상한 게이트가 이 crate 로 가는 간선을 못 보는 것이다(platform 헤더의 같은 문장은 1-3 U0 리뷰(2026-10-05)에서 고쳤다) | `crates/engram-dashboard-transport/src/lib.rs:39-40` | 3단계(transport) 때 그 헤더를 지나며 |
| ~~메시징 이름 게이트 정규식에 `net` · `transport` 가 없다 — 「새 워크스페이스 crate 가 생기면 더한다」가 그 둘 때 지켜지지 않았다(messaging 소스에 두 이름 0건 — 더해도 초록 · 실측 2026-10-05)~~ **→ 처리 2-2 U4**: `discovery` 를 빼는 같은 편집에서 두 이름을 더했다(사본 다섯 — TRD 2-2 §8 O2) | `.github/workflows/ci.yml` 메시징 이름 게이트 · CLAUDE.md · `.claude/skill-bindings/qa.md` · `docs/testing-strategy.md:64` · `docs/reference/architecture-overview.md:492` | ~~다음에 그 게이트를 손볼 때(사본 다섯을 한 번에)~~ 처리됨 |
| ~~`.claude/skill-bindings/qa.md` 에 transport 격리 게이트가 하나도 없다 — CI 만 잰다(그 파일 머리말 「CI에 있는데 여기 없는 게이트를 발견하면 그건 드리프트다 — 이 파일을 채운다」와 어긋난다)~~ **→ 처리 1-3 U8**: qa standard 에 「재사용 전송 lib 격리 게이트」 블록(헤더 ①~⑤ = CI `transport gate 1`~`4d`)을 더했다 | `.claude/skill-bindings/qa.md` · `crates/engram-dashboard-transport/src/lib.rs` 「격리 게이트」 | ~~3단계(transport) 전~~ 처리됨 |
| ~~운영 코드에 OS 를 가리지 않고 늘 대소문자를 접는 환경변수 이름 비교가 둘 있다 — codex 락 폴더의 `CODEX_HOME` 찾기 · claude 스폰의 `MAX_THINKING_TOKENS` 찾기. (셋째 = 사용량 조회의 `env_overrides_daemon`(`usage/gate.rs:69`) — 이쪽은 **의도**다: 「다르다」 쪽으로 틀리게 골랐다고 그 자리 문서가 적는다 · 1-3 U8 리뷰가 짚음.) platform `env::env_key_eq` 와 같은 OS 규칙(환경변수 이름의 대소문자)인데 POSIX 에서의 뜻이 다르다(그 함수는 POSIX 에서 정확히 같아야 같다). `cfg` 가 없어 TRD 1-3 §2 의 `cfg` 정규식에도 계획된 §4-4 `cfg` 게이트에도 안 걸린다. `transport/pty.rs` 의 `TERM` · `COLORTERM` 기본값 찾기는 모든 OS 에서 접는 것이 의도라(그 자리 주석 — ADR-0049) 뺀다 · 출처 = 1-3 U2 리뷰(2026-10-05) · **1-3 U8 재고 결과 = OS `cfg` 파일 명단에는 안 든다**(`cfg` 가 없다) — 그 부류(`cfg` 없는 런타임 OS 가름 · 늘 대소문자를 접는 환경변수 이름 비교)는 불변식 게이트의 한계로 platform 헤더 게이트 ④ 에 적었다. 둘을 `env::env_key_eq` 로 바꾸는 것은 POSIX 에서의 뜻을 바꾸는 동작 변경이라 U8(게이트 · 문서) 범위 밖~~ **→ 처리 1-3 작은 수정 커밋**: `CODEX_HOME` · `MAX_THINKING_TOKENS` 찾기를 `env::env_key_eq` 로 바꿨다(Windows 결과 불변 · 그 자리 시험 둘은 기대값을 `env_key_eq` 로 정해 어느 OS 에서든 맞게 했다). `env_overrides_daemon` 은 의도된 보수 판정이라 그대로다. 사용량 조회가 데몬 env 에서 `CLAUDE` 키를 벗기는 `stripped_env_keys`(`backend/claude/usage_probe.rs`)도 늘 대소문자를 접지만 의도라 그대로다 — 벗길 목록 · 남길 목록이라 POSIX 에서는 더 벗기거나 CLI 가 안 읽는 닮은 키를 남길 뿐이다 | `crates/engram-dashboard-agent/src/backend/codex/thread_lock.rs`(`child_lock_dir`) · `crates/engram-dashboard-agent/src/backend/claude/mod.rs`(`build_spec` 의 `MAX_THINKING_TOKENS_KEY`) · `crates/engram-dashboard-agent/src/usage/gate.rs:69`(`env_overrides_daemon` — 의도) | ~~1-3 작은 수정 커밋(사용자 결정 2026-10-06)~~ 처리됨 |
| platform `process::child_pids` 의 doc 「★왜 필요한가★」가 쓰임을 실프로세스 격리 시험 하나로만 정당화한다 — 운영 코드(`process::subtree` — codex 세션 id 회수)도 그것을 부른다. 그 문단을 운영 쓰임까지 담게 고쳐 쓴다 · 출처 = 1-3 U4 리뷰(2026-10-06) | `crates/engram-dashboard-platform/src/process.rs`(`child_pids`) | 다음에 그 함수를 지날 때 |
| ~~net 게이트 2a(platform 심볼 allowlist)는 **개수만** 센다 — 별칭 import(`use engram_dashboard_platform as p;`) · 중괄호(`use …::{spawn::x, group::Y}`)는 0 매치, 모듈 import(`use …::spawn;`)는 1줄이 그 모듈 전부를 덮고, 심볼을 바꿔 끼워도 개수가 같으면 초록이다. net `Cargo.toml` 주석 「심볼 목록 자체를 기대값으로 못 박는다」와도 어긋난다(CI 는 목록이 아니라 개수를 본다). 처방 후보 = 정렬한 출력을 이름 다섯과 그대로 대조 · 맨 crate 이름 매치 수를 따로 잰다 · 출처 = 1-3 U6 리뷰(2026-10-06 — codex · Claude 둘 다 선재로 짚음)~~ **→ 처리 1-3 U8**: 2a 를 검사 둘로 — ① 정렬한 심볼 목록을 다섯 이름과 그대로 대조 ② crate 이름 뒤에 `::이름글자` 가 오지 않는 부름(별칭 · 중괄호 · glob · 끊긴 경로)을 주석 밖에서 0줄로. 사본 다섯(CI · net 헤더 · CLAUDE.md · qa · testing-strategy)을 함께 고쳤고 net `Cargo.toml` 주석은 이제 사실이라 두었다. 남은 한계 = 매니페스트에서 의존 이름을 바꾸면 두 검사가 다 눈멀고 게이트 3 도 받치지 않는다(허용된 간선이라 초록) — 그때 심볼 범위는 리뷰 몫(net 헤더에 적었다) | `.github/workflows/ci.yml` net 2a · `crates/engram-dashboard-net/src/lib.rs` 헤더 · `crates/engram-dashboard-net/Cargo.toml` | ~~1-3 U8(불변식 게이트) 후보~~ 처리됨 |
| ~~통로 stdio · pty 는 띄운 뒤 무리에 넣다 실패하면(`GroupOwner::new()?` · `adopt(pid)?`) 띄운 자식이 무리 밖에 남는다 — codex 통로처럼 가드(`ChildGuard`)를 먼저 세우지 않는다(HEAD 부터 같다) · 출처 = 1-3 U3 리뷰(2026-10-06)~~ **→ 처리 A U7**(ADR-0291 결정 13): 세 통로의 띄우기를 `transport/spawn.rs` 하나로 모았다 — 가드 `ChildGuard` 가 띄운 바로 다음에 서고 그다음 걸음이 무리 넣기(`new_group_with` — 띄운 뒤 첫 실패 가능 걸음)다. 넣기가 실패하면 가드가 직속 자식을 끄고, 넣은 뒤에 실패하면 무리 주인이 먼저 버려져 넣은 자식과 그 뒤에 뜬 후손을 Job 이 끝낸 뒤 가드가 직속 자식을 거둔다(번호로 후손을 찾아 끄지 않는다 — 결정 13 의 U7 개정 · 잠정). 배치는 소스 시험이, 직속 자식 끄기와 넣은 뒤 실패의 손자까지 끝나기는 실프로세스 시험(std · pty)이 잰다. 남은 것 = 무리 넣기 전에 뜬 손자(성공 · 실패 경로 둘 다 · 범위 밖 — TRD A §3-7 · `docs/tracking.md` T-53) | ~~`crates/engram-dashboard-agent/src/transport/{stdio.rs, pty.rs}`(`open`)~~ 실제 = `crates/engram-dashboard-agent/src/transport/spawn.rs` · 그것을 부르는 `transport/{stdio.rs, pty.rs}` · `backend/codex/transport.rs`(`open`) | ~~4단계(agent) 또는 다음에 그 통로를 지날 때~~ 처리됨 |
| `docs/reference/architecture-map-notes.md:390` 의 「base 심볼 allowlist = 정확히 2」는 U1 부터 낡았다(지금 platform 심볼 · 5) — 날짜 박힌 스냅숏 문서 · 출처 = 1-3 U6 리뷰 | `docs/reference/architecture-map-notes.md` | 그 지도를 다시 뽑을 때 |
| ~~`.claude/skill-bindings/qa.md` 에 실 claude `#[ignore]` 시험을 로컬에서 돌리는 명령이 없다(CI `--skip` 목록을 정본으로 가리키기만) — QA 워커가 명령을 추정했다 · 출처 = 1-3 U3 QA(2026-10-06)~~ **→ 처리 1-3 U8**: qa 「CI와의 분담」의 CI 미커버 ② 아래에 로컬 명령을 적었다 — `--skip` 목록(전부 daemon `tests/control_send.rs` · 비-ignore 다섯은 standard 2번이 이미 돈다 · `#[ignore]` 인 c1 은 `--ignored --exact`)과 그 밖의 실 CLI `#[ignore]` 레인(`usage_probe_smoke` · `backend_contract`) | `.claude/skill-bindings/qa.md` | ~~1-3 U8 또는 다음에 바인딩을 손볼 때~~ 처리됨 |
| ~~platform `process::kill_tree` 가 `taskkill` 을 창 숨김 없이 띄운다 — GUI 셸이 데몬 끄기를 부르면 콘솔이 번쩍일 수 있다(U7 은 동작을 그대로 옮겼다 · 옆에 `spawn::hide_console_window` 가 있어 한 줄 거리지만 동작 변경이다 · 그 경로는 fallback `daemon_stop` 뿐이다 — 연결이 없거나 graceful 끄기가 실패했을 때, 그리고 graceful 이 받아들여졌는데도 `daemon_status` 가 살아 있다고 답하거나 그 조회가 실패했을 때 돈다 — 프론트 `daemonControl`) · 출처 = 1-3 U7 리뷰(2026-10-06)~~ **→ 처리 1-3 작은 수정 커밋**: `kill_tree` 의 `taskkill` 에 `spawn::hide_console_window` 를 붙였다 | `crates/engram-dashboard-platform/src/process.rs`(`kill_tree`) | ~~1-3 작은 수정 커밋(사용자 결정 2026-10-06)~~ 처리됨 |
| ~~셸 주석이 「`taskkill /F`」라 적지만 실제 인자는 `/F /T` 다 · 출처 = 1-3 U7 코더~~ **→ 처리 2-2 U3**: 그 주석을 「자식 트리째 강제로 끈다(`/F /T` — platform `process::kill_tree`)」로 고쳤다 | `src-tauri/src/commands/discovery.rs:161` | ~~2-2(discovery 나누기)~~ 처리됨 |
| `docs/reference/architecture-map-notes.md:86` 이 WMI 띄우기 자리를 `discovery/src/lib.rs:932,1105-1153` 로 가리킨다 — 1-3 U7 로 그 코드는 platform `spawn/wmi.rs` 로 갔다(그 전에도 줄이 어긋나 있었다) · 출처 = 1-3 U7 리뷰 | `docs/reference/architecture-map-notes.md` | 그 지도를 다시 뽑을 때 |
| ~~discovery 의 실 WMI `#[ignore]` 시험 둘은 `cargo test -p engram-dashboard-discovery -- --ignored real_wmi` 로 돌리면 `ExeNotFound` 로 진다 — cargo 가 cwd 를 crate 폴더로 두는데 데몬 exe 찾기가 시험 exe 폴더 · cwd/target/debug 를 본다. 저장소 루트를 cwd 로 그 시험 바이너리를 직접 돌리면 통과한다(실측 2026-10-06 · rv 행렬 None 0 · NEW_CONSOLE 0 · DETACHED 0 · NO_WINDOW 21) · 출처 = 1-3 U7 QA~~ **→ 처리 2-2 U3**: 시험이 셸 `lib_unit` 으로 옮겨 오며 운영 `locate_daemon_exe` 대신 시험 exe 위치(`target\<profile>`)에서 데몬 exe 를 찾는 시험 전용 도우미(`test_daemon_exe`)를 쓴다 — cwd 에 기대지 않는다. 실행 명령은 qa 「CI와의 분담」의 셸 실 데몬 레인에 적었다 | ~~`crates/engram-dashboard-discovery/src/lib.rs`(`locate_daemon_exe` · 시험 `real_wmi_*`)~~ 실제 = `src-tauri/src/discovery/tests.rs`(`test_daemon_exe` · `real_wmi_*`) · `.claude/skill-bindings/qa.md` | ~~2-2 또는 다음에 그 시험을 돌릴 때~~ 처리됨 |
| CLAUDE.md 「백엔드 모듈 맵」에 `engram-dashboard-transport` crate 항목이 없다 — 워크스페이스 멤버이고 CI 격리 게이트(`ci.yml` 의 `transport gate 1`~`4d`)와 qa 블록도 있는데 맵에만 이름이 없다(`rg engram-dashboard-transport CLAUDE.md` → 0줄 · 실측 2026-10-06) · 출처 = 2-1 조사(2026-10-06) | `CLAUDE.md` 「백엔드 모듈 맵」 · `crates/engram-dashboard-transport/src/lib.rs`(헤더 — 항목이 가리킬 정본) | 미처리 — 다음에 그 맵을 손볼 때(늦어도 3단계 transport) |
| 프론트 레지스트리의 `agentlist.createCodex`(형제 `createCodexJson`)는 `humanOnly` 가 걷힌 뒤(2026-09-22) LLM 이 `__engramCmd` 로 부를 수 있고, 그 `CreateProfile` 경로는 LLM 백엔드 정책(`LLM_BACKEND_POLICY`)을 안 본다 — 2-1 의 데몬 벽은 `SpawnByCwd` 만 덮는다. 오늘 영향 0(표가 아무 낱말도 안 닫는다). ~~따로, agent 쪽 「정책을 묻는 문 셋」 중 ③(`humanOnly`) 서술은 `humanOnly` 가 2026-09-22 에 걷혔기 때문에 낡았다(우회 경로 때문이 아니다)~~ **→ 「문 ③」 글은 처리 2-1 U1 `8b48386`**: 문 목록을 `agent.new` · 데몬 `SpawnByCwd` · 셸 `gate_backend`(~~U2 가 걷는다~~ 2-1 U2 가 걷었다 — 문 목록도 둘로 고쳤다)로 고쳐 쓰고 `humanOnly` 는 표를 보는 문이 아니라고 적었다 — `CreateProfile` 우회 자체는 미처리 · 출처 = 2-1 TRD 작성(2026-10-06 · `../process/S21-crate-boundaries/trd-2-1-shell-agent-cut.md` §8 O5) | `src/commands/agentCommands.ts`(`agentlist.createCodex` · 머리 주석 `:41`) · `crates/engram-dashboard-agent/src/commands.rs:600-614`(「문 ③」) | 미처리 — 경로는 정책 표가 한 낱말이라도 닫기 전에 본다 · ~~「문 ③」 글은 2-1 U1 이 그 주석을 지날 때~~ 처리됨(`8b48386`) |
| 슬롯에 codex 를 띄우면 새로고침 전까지 탭 제목이 「Claude Code」다 — 데몬 `SpawnByCwd` 처리부가 `Spawned` 만 보내고 `ProfileListUpdated` 를 브로드캐스트하지 않아 프론트 프로필 목록이 비어 있다(표시만 틀림 · 동작 무관) · 출처 = 2-1 U2 QA full(2026-10-06) | `crates/engram-dashboard-daemon/src/connection_core.rs`(`SpawnByCwd` 처리부) | 미처리 — 데몬 정리(4단계) 또는 그 처리부를 지날 때 |
---

## 큰 절 — 나중에 따로 다룬다 (지금은 모으기만)

### A. 로깅 시스템 설계

**상태: 요구만 모음(사용자 2026-09-26) · 설계는 다른 소프트웨어 벤치마킹(`/research`) 뒤.**

- **요구(사용자 원문):**
  - 「기본적으로 카테고리가 있고, 위험도가 있고」
  - 「지금은 UI 켜져 있어야 현황을 파악할 수 있는데 로그만으로 파악할 수 있게」
  - 근거 — 「나중에 다중 데몬 들어가면 결국 장애에 대한 판단을 로깅으로밖에 할 수가 없으니깐. 프론트는 괜찮은데 백엔드는 치명적」
  - 경계 — 「모든 대화 자체를 로깅하는 건 아니고」
- **토대:** 로그는 감싸지 않는다(ADR-0268 재작성 · 사용자 2026-10-03). 그룹 켜고 끄기는 tracing target + base 의 `EnvFilter` 로 한다. **현황(코드 대조 2026-10-03):** 필터 기준은 레벨 하나 — 기동 때 `RUST_LOG` 가 있으면 그것, 없으면 `warn`(`crates/engram-dashboard-base/src/logging/mod.rs` `init_subscriber`) · 실행 중 변경 함수 `set_log_level` 은 있으나 base 밖 호출자가 0 이라 실제로 바꿀 입구가 없다 · 별도 `target:` 은 `agent_stderr` 하나뿐(`agent/src/transport/stdio.rs` · codex transport). 이 설계가 정할 것 = 그룹(카테고리) 어휘 · 실행 중 바꾸는 입구(LLM 제어 표면 — CLAUDE.md 「LLM-우선 제어」).
