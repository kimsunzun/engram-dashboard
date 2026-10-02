# 아키텍처 논의 — 모듈별 정리

**목적:** transport 를 붙이기 전에 전체 의존관계와 모듈 쪼개기를 먼저 정한다(사용자가 정한 순서, 2026-09-26). 사용자가 모듈 순서대로 묻고, 답에서 나온 사실과 결정 후보를 여기 쌓는다.

- **기준 코드:** master `a226f63`. 사실 항목은 전부 그 시점 코드와 대조했다.
- **결정은 여기서 끝나지 않는다.** 방향이 선 후보는 논의를 마친 뒤 ADR 로 박고 여기서 링크한다(`../decisions/`). 착수 대기 사실은 `../tracking.md` T-46 이 진다.
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
- **설계 의도 = 어느 crate 든 쓸 수 있는 잎.** 단 지금은 `command`·`messaging` 이 자기 게이트(워크스페이스 의존 0)에 막혀 base 도 못 쓴다 → 아래 후보 2 가 푼다.
- **agent 에도 `platform` 모듈이 따로 있다** — Job Object 래퍼 · 파일을 연 프로세스 찾기(`file_holders`) · 프로세스 트리(`process_tree`)의 셋이다(`crates/engram-dashboard-agent/src/platform/mod.rs:1-2`). CLAUDE.md 「백엔드 모듈 맵」 agent 항목의 「남은 `platform`은 Job Object 래퍼 하나뿐」은 낡은 서술이다(아래 후보 1 이 착지하면 그 문장째 바뀐다).

### 결정 후보 1 — OS 의존 코드를 독립 `platform` crate 로 분리

**상태: 방향 확정(사용자 2026-09-26) · ADR 미작성 · 착수는 아키텍처 논의 뒤.**

- **무엇:** OS 에 따라 달라지는 코드를 워크스페이스 의존 0 인 독립 crate 하나로 모은다. 부르는 쪽은 인터페이스만 부르고 OS 분기(`#[cfg]`)는 그 crate 안에만 있다. 이미 OS 코드에 연결된 곳도 전부 이쪽으로 옮긴다.
- **사용자 근거:** 「인터페이스 호출로 하고 플랫폼적인 건 다 감추고」 · 「어차피 플랫폼 전용 기능은 의존 0」 · 「애초에 초반부터 잡고 갔어야 됐어」.
- **옮길 것(현황 실측 — 테스트 전용 분기 제외):**

  | 무엇 | 지금 위치 |
  |---|---|
  | Windows API 래퍼 — PID 생존·생성 시각·자식 PID · Job Object · 파일을 연 프로세스 찾기 · 프로세스 트리 · WMI 로 터미널 트리 밖에 띄우기 | `base/src/platform.rs` · `agent/src/platform/` · `discovery/src/lib.rs:1119-1320`(`wmi_spawn`·`wmi_create_raw`) |
  | OS 규칙 — `.exe` 붙이기 · 기본 셸 · 홈 디렉터리 · CLI 를 `cmd.exe /c` 로 감싸기 · `PATH` 대소문자 무시 | `discovery/src/lib.rs:981` · `daemon/src/lib.rs:96,146` · `agent/src/manager.rs:75` · `agent/src/backend/claude/mod.rs:1012` · `agent/src/backend/mod.rs:44,136` |
  | 부르는 쪽에 샌 분기 — Job Object 핸들을 `#[cfg(windows)]` 로 들고 다님 | `agent/src/transport/pty.rs` · `agent/src/transport/stdio.rs` · `agent/src/backend/codex/transport.rs:177` |

  셸(`src-tauri/src`)에는 OS 분기가 없다. PTY 자체는 `portable-pty` 가 이미 감춘다.
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
| XML 이스케이프 | `&`·`<`·`>`·따옴표 치환 | `messaging/src/envelope.rs:126,135` | 범용(함수 자체엔 우편 지식 없음) · 소비자 하나. `messaging` 이 base 를 쓰려면 아래 후보 2(게이트 완화)가 먼저다 |
| 바이트 → hex 문자열 | 소문자 hex 인코딩 루프 | daemon 세 곳(`experiment/record.rs:189` · `lib.rs:65` · `control/mod.rs:141`) | 인코딩 루프만 범용 · 토큰 생성기 자체는 공유 금지(ADR-0086) · `sha256_hex` 는 `sha2` 를 끌고 오므로 제외 |
| 글자 수 자르기(`clip`·`truncate`) | 앞 N 글자만 | agent codex `decoder.rs` · `transport.rs`, daemon 인라인 두 곳 | **안 된다** — agent 쪽은 비밀값 가리기를 거쳐서만 부르도록 테스트가 막는 의도된 비공개다. base 로 빼면 그 문이 뚫린다 |
| 데이터 폴더 찾기(`default_data_dir`) | `ENGRAM_DATA_DIR` · `.engram-data` · 릴리즈 배치로 폴더 결정 | `discovery/src/lib.rs:84` | **안 된다** — Engram 고유 지식(ADR-0134)이라 discovery 몫 |
| 토큰 생성 | 32바이트 난수 hex | daemon 두 벌 | **안 된다** — 공유 금지가 명시돼 있다(ADR-0086) |
| 재연결 백오프 | 지수 대기 + 상한 | `src-tauri/src/daemon_client/connection.rs:87` · `transport/src/policy.rs:81`(더 풍부한 판) | **base 아님** — transport 가 이미 가진 정책이다. 셸이 transport 를 붙일 때 그쪽 정책으로 흡수하면 사라진다(부착 때 체크할 것) |
| `dunce::canonicalize` | UNC 접두 없는 정규화 | 라이브러리 직접 호출 | 뺄 것이 없다(한 줄 호출) |

- **못 본 것:** 「비어 있지 않은 환경변수 읽기」 같은 도우미가 여러 crate 에 중복돼 있는지는 확인하지 않았다(대부분 `ENGRAM_*` 도메인 읽기로 보임).
- **곁 사실:** `transport` crate 는 아직 소비자가 0 이다(자기 테스트만 쓴다 — 매니페스트·소스 대조).

### 결정 후보 4 — base 에 범용 헬퍼 네 모듈을 들인다

**상태: 입주 확정(사용자 2026-09-26 — 「파일 텍스트 시간 경로 등 다 옮겨. 딱 적당하네」) · 제공 방식은 메인 제안 · ADR 미작성.**

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

**상태: 방향 확정(사용자 2026-09-26 — 「커맨드·메시지는 로깅 안 함? 당연히 해야지」) · ADR 미작성.**

- **바뀌는 것:** 두 crate 의 의존 상한 게이트 기대값이 「자기 자신 1줄」에서 「자기 자신 + base」로 바뀐다(CLAUDE.md 「빌드·검증 명령」 · `ci.yml`). 「워크스페이스 의존 0」 을 성질로 적은 ADR-0155(command) · ADR-0110(messaging)의 그 부분을 뒤집는다.
- **지금 로그 현황(실측):** `messaging` 은 이미 로그를 남긴다 — `tracing` 을 직접 의존해 약 34곳(`service.rs` 등). 로그를 **찍는** 데는 base 가 필요 없고, 찍힌 로그는 그 프로세스(데몬)가 base 로 설치한 구독자를 타고 파일에 간다. `command` 는 로그가 0줄이다(`tracing` 의존도 없음) → 리팩토링 때 추가 대상.
- **그러니 base 를 쓰게 해서 얻는 것:** 로그를 base 경유로 찍는 것(후보 3) · 위 입주 후보(범용 헬퍼)를 두 crate 도 쓸 수 있다.
- **안전성 근거:** base 는 도메인 지식 0 · 워크스페이스 의존 0 을 자기 게이트가 지키므로, 두 crate 가 base 에 기대도 순환이나 도메인 유입 경로가 생기지 않는다.

### 결정 후보 3 — 로그는 base 를 거쳐서만 찍는다 (로그 라이브러리 직접 의존 금지)

**상태: 방향 확정(사용자 2026-09-26 — 「로그 라이브러리를 왜 직접 쓰냐고. 나중에 컨트롤이 안 되잖아」) · 인터페이스 모양은 메인 제안 · ADR 미작성.**

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

**상태: 방향 확정(사용자 2026-10-02 — 「데몬에 거절만 하면 딱히 복잡하게 할 필요 없을 듯 … 실행해서 안 되면 다시 빌드하면 되니깐」) · ADR 미작성.**

- 예약 이름 목록을 걷는다. 벽은 **데몬의 등록 거절 하나**로 둔다. CI 대조 스크립트는 두지 않는다.
- **단 거절을 클라이언트가 OS 메시지 박스로 띄운다**(사용자 2026-10-02 — 「빵 하고 뜨면서 클라에서 뜨면 유저도 알지」). 띄우는 쪽 = 셸(클라). 플랫폼마다 다른 창이므로 OS 분기는 셸이 아니라 OS 의존 코드 자리(후보 1)나 Tauri 의 대화상자 기능 뒤에 둔다 — 구현 때 고른다. **LLM 통지는 두지 않는다** — 원인은 로그에 남고 에이전트가 로그를 뒤지면 나온다(사용자). 지금의 `warn` 로그 한 줄은 그대로 남긴다.
- **원칙(사용자 2026-10-02):** 「클라는 데몬이랑 아예 별도의 개념이라서 agent 를 모르는 게 좋음」 — 셸(클라)은 agent crate 를 의존하지 않는다.
- 백엔드 정책 거름은 데몬에 맡긴다. ★**단 셸 검사를 그냥 지우면 구멍이 난다**★ — 정책 함수를 부르는 곳은 `agent.new` 처리 한 곳뿐이고(`crates/engram-dashboard-agent/src/commands.rs:1077`), 셸의 슬롯 스폰은 그 명령을 거치지 않고 통신 메시지 `SpawnByCwd` 를 바로 보내며(`src-tauri/src/layout/apply.rs:597`) 데몬은 그 경로에서 정책을 안 본다. → **순서: 슬롯 스폰을 데몬의 `agent.new` 명령 경로로 바꾼 뒤 셸 검사를 지운다.** 슬롯 스폰은 「검사 → 데몬 스폰 → 배치」 순이라 데몬이 거절해도 레이아웃이 안 바뀐다(`layout/apply.rs:585-620`). 거부 문구가 데몬을 한 번 다녀오고, `tests/layout_apply.rs` 의 「두 입구가 함께 열리나」 시험과 ADR-0219 의 두 입구 서술을 고친다.
- `normalize_cwd` 는 base 로(후보 4).
- 얻는 것: 셸 실행 파일이 agent crate 전체(PTY · Windows 프로세스 관리 · 명령 수집 등)를 안고 가지 않는다. 「셸 = 데몬 클라이언트」(ADR-0029)가 의존 그래프에서도 보인다.

---

## 4. discovery

### 사실 (코드 대조 2026-10-02)

- **원래 이유(ADR-0024):** 데몬이 데이터 폴더의 `daemon.json` 에 포트·토큰·PID 를 적고 클라이언트가 같은 파일을 읽어 접속·인증한다 → 양쪽이 **같은 폴더 규칙**을 써야 해서 그 규칙(`default_data_dir`)을 공용 crate 에 두었고, 클라이언트 쪽 데몬 찾기·띄우기·끄기를 함께 넣었다.
- **실제로 양쪽이 다 쓰는 것은 `default_data_dir` 하나뿐이다.** 쓰기 가능 확인 · 설치 위치는 데몬(+ 데몬 crate 안의 `engram` CLI)만, 찾기·띄우기·상태·정지·실행 파일 찾기는 셸만 쓴다.
- **안에 작은 데몬 클라이언트가 있다** — `send_stop` 이 WebSocket 접속·인증·정지 명령을 직접 한다. discovery 가 `net`(인증 메시지)과 `protocol` 명령 메시지를 아는 이유가 전부 이것이다.
- 실행 파일 위치 계산이 두 곳에 갈려 있다 — 데몬(`daemon/src/lib.rs:96,146`) · discovery(`lib.rs:981`).
- T-10(2026-08-26)에서 「discovery 를 없애지 않는다」로 종결한 적이 있다. 아래 후보 6 은 **새 근거**(클라·데몬 분리 원칙 · 원격 대비)로 그것을 다시 연다.

### 결정 후보 6 — discovery 를 뽀개 데몬·셸에 각각 둔다

**상태: 방향 확정(사용자 2026-10-02) · ADR 미작성 · T-10 재개.**

- **사용자 근거:** 네트워크 방식이 되면 폴더 규칙은 데몬만 필요하다 · 같은 PC 모드도 원격과 **같은 인증 방식**으로 하면 되고 로직을 갈라 둘 필요가 없다(편의용 자동 인증은 나중) · 「지금 당장은 discovery 뽀개고 데몬 셸 각각 두고 나중에 관련해서 얘기할 때 분리」.
- **나누는 법:**
  - 데몬으로: 데이터 폴더 규칙(`default_data_dir` · `release_data_dir` · 쓰기 가능 확인) · 설치 위치(`find_install_root`) · 실행 파일 위치 계산(두 곳 → 한 곳).
  - 셸로: 데몬 찾기 · 띄우기 · 상태 · 프로세스 끄기 · 실행 파일 찾기. 정지 명령(`send_stop`)은 셸의 데몬 연결 코드로 — 그러면 셸 밖 어디에도 작은 데몬 클라이언트가 안 남는다.
  - WMI 로 띄우는 코드는 `platform`(후보 1).
- **과도기 연결 하나:** 원격 인증 경로가 생기기 전까지 셸은 `daemon.json` 을 계속 읽어야 하므로 **데몬이 그 파일을 쓰는 위치**를 셸도 알아야 한다. 셸은 데몬 crate 를 의존하지 않으므로(클라·데몬 분리) 위치 규칙이 셸 쪽에 한 벌 더 생긴다 → 두 벌이 같은 경로를 내는지 재는 시험으로 묶는다(셸은 데몬을 테스트 전용 의존으로 이미 끌어온다). 원격·같은 PC 인증 통일 때 이 연결은 사라진다.
- **나중(원격 작업 때):** 클라이언트가 「데몬 주소와 토큰을 어디서 얻나」를 인터페이스로 두고, 같은 PC 도 같은 인증 방식으로 통일 · 편의용 자동 인증 검토. 토큰은 서버가 발급하고 클라이언트가 열쇠로 들고 있는 모델 그대로.
---

## 큰 절 — 나중에 따로 다룬다 (지금은 모으기만)

### A. 로깅 시스템 설계

**상태: 요구만 모음(사용자 2026-09-26) · 설계는 다른 소프트웨어 벤치마킹(`/research`) 뒤.**

- **요구(사용자 원문):**
  - 「기본적으로 카테고리가 있고, 위험도가 있고」
  - 「지금은 UI 켜져 있어야 현황을 파악할 수 있는데 로그만으로 파악할 수 있게」
  - 근거 — 「나중에 다중 데몬 들어가면 결국 장애에 대한 판단을 로깅으로밖에 할 수가 없으니깐. 프론트는 괜찮은데 백엔드는 치명적」
  - 경계 — 「모든 대화 자체를 로깅하는 건 아니고」
- **토대:** 후보 3(로그는 base 경유 · 가리기 중앙화)이 이 설계가 올라설 자리다.
