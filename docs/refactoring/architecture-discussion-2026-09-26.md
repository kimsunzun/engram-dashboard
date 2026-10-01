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
  - **`Clock` 트레이트가 세 벌이다** — 같은 모양(`fn now(&self) -> Instant`)을 transport(`clock.rs:20`) · daemon(`command_delivery.rs:176`) · discovery(`lib.rs:425`)가 각자 정의한다. 시간을 가짜로 바꿔 끼우는 테스트용 seam 이다 → **확정: base `time` 으로 합친다. 규칙 = 「시간은 공용이면 무조건 합친다」**(사용자 2026-09-26). 위 「트레이트를 두지 않는다」의 유일한 예외다.
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
- **인터페이스 모양은 아래 「큰 절 A」가 정해지면 다시 본다** — 카테고리·위험도가 매크로 표면에 들어갈 자리이기 때문이다. 후보 3 이 정하는 것은 「base 를 거친다」까지다.

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
