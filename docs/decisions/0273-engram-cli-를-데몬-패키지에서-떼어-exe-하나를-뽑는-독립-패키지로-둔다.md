# ADR-0273: engram CLI 를 데몬 패키지에서 떼어 exe 하나를 뽑는 독립 패키지로 둔다

- 상태: 확정 (2026-10-02, 근거: 사용자 결정 2026-10-02 (`docs/refactoring/architecture-discussion-2026-09-26.md` 결정 후보 9) + 코드 대조 2026-10-02) · 부분 폐기 by ADR-0285 (결정 4의 설치 위치 규칙과 결정 5) · 부분 폐기 by ADR-0286 (영향의 남은 데몬 경계 항목)
- 관련: ADR-0132(제어 평면 CLI = 단일 실행 파일 `engram` — 패키지 자리는 정하지 않았다) · ADR-0094(bare-name grant · PATH 주입 — 실행 파일 이름 정렬) · ADR-0151(crate 판정 기준 · 「개명 함정」 이름 접두) · ADR-0175 결정 6(lib 무게 — 이 패키지는 bin 이다) · ADR-0271(discovery 나누기 — 설치 위치 규칙이 데몬으로 간다) · `crates/engram-dashboard-daemon/Cargo.toml`(`[[bin]] engram`) · `crates/engram-dashboard-daemon/src/bin/engram.rs` · `crates/engram-dashboard-agent/src/types.rs`(`CLI_EXE_NAME` 등 CLI 어휘) · step-log S21 · Amended by ADR-0285 (결정 4의 설치 위치 규칙과 결정 5) · Amended by ADR-0286 (영향의 남은 데몬 경계 항목)

## 맥락

`engram.exe`(에이전트가 자기 터미널에서 부르는 제어 CLI)는 **데몬 패키지의 두 번째 bin 타깃**이다(`crates/engram-dashboard-daemon/Cargo.toml` 의 `[[bin]] name = "engram"` · 소스 `src/bin/engram.rs` 비테스트 2,791줄 — 메모 실측 master `a226f63`). 데몬 lib 를 한 줄도 쓰지 않는다 — 쓰는 것은 agent 의 명령 어휘 상수(`engram.rs` 의 `use engram_dashboard_agent::types::{…}`) · command 의 요청 번호 · discovery 의 설치 위치(`find_install_root`) 셋뿐이다. 그런데 같은 패키지라 빌드 때 데몬 의존 전체를 끌고 온다. HTTP 를 손으로 짤 만큼 의존 최소화가 의도였다(같은 `Cargo.toml` 주석 — 「의존성 최소화(std TcpStream 손조립 HTTP)」).

CLI 어휘 상수(동사 · 플래그 · 실행 파일 이름 · 상태 낱말)는 agent 자신도 쓴다(`manager.rs` · `commands.rs` · 백엔드) — 「`agent.*` 명령의 어휘」라 agent 소유가 맞다.

사용자(2026-10-02): 「engram 은 독립 패키지로 빼는 게 맞는데 그냥 shell 처럼 하나 분리」.

## 결정

1. **셸(`src-tauri` → `engram-dashboard.exe`)처럼 exe 하나를 뽑는 패키지를 따로 둔다**(사용자 2026-10-02). 위치 = `crates/` 아래.
2. **패키지 이름은 `engram-dashboard-` 접두를 지킨다** — CI 의존 상한 게이트가 워크스페이스 멤버를 그 이름 접두로 식별한다. 다른 이름이면 게이트를 그냥 통과한다(ADR-0151 「개명 함정」).
3. **bin 이름은 `engram` 그대로다** — agent `CLI_EXE_NAME` 과 맞물린다. 바꾸면 우편이 조용히 멈춘다(데몬 `Cargo.toml` 주석 · ADR-0094).
4. **의존 = agent(명령 어휘) · command(요청 번호) · 설치 위치 규칙.** CLI 어휘는 agent 에 남는다(위 「맥락」).
5. **설치 위치 규칙(`find_install_root`)은 CLI 쪽에 한 벌 더 둔다**(메모 결정 후보 9) — ADR-0271 결정 2 가 그 규칙을 데몬으로 보내고, CLI 는 데몬 패키지를 떠나므로 데몬 crate 를 의존하지 않는다. **두 벌이 같은 경로를 내는지 재는 시험으로 묶는다** — 데몬 · 셸의 `daemon.json` 자리와 같은 처리다(ADR-0271 결정 4). 그 시험이 어느 패키지에 사는지는 2-3 착수 때 정한다. 착수 = 작업 순서 2-3(선행 2-2 discovery 나누기).

## 거부한 대안

- **데몬 패키지의 둘째 bin 으로 둔다(현행).** 기각 = 사용자 결정 + 코드: CLI 는 데몬 lib 를 한 줄도 안 쓰는데 같은 패키지라 데몬 의존 전체를 빌드에 끌고 온다(위 「맥락」 — 의존 최소화라는 원래 의도와 어긋난다).
- **bin 이름을 바꾼다.** 기각 = 코드: agent `CLI_EXE_NAME` 과 grant · 프라이밍 · PATH 해석이 그 이름에 정렬돼 있어 어긋나면 우편이 조용히 멈춘다(데몬 `Cargo.toml` 주석 · ADR-0094).
- **`engram-dashboard-` 접두 없는 패키지 이름.** 기각 = 코드: 의존 상한 게이트가 이름 접두로 멤버를 식별해 그 패키지를 조용히 안 본다(ADR-0151).

## 근거

- **사용자 결정 2026-10-02** — 위 인용.
- **코드 대조(2026-10-02)** — 데몬 `Cargo.toml` 의 `[[bin]] engram` · `engram.rs` 의 워크스페이스 import 가 agent · command · discovery 셋뿐 · `Cargo.toml` 주석의 의존 최소화 의도 · bin 이름 = `CLI_EXE_NAME` 정렬 주석.

## 영향 / 불변식

- **새 패키지는 게이트를 세 곳(`ci.yml` · CLAUDE.md 「빌드·검증 명령」 · `.claude/skill-bindings/qa.md`)에 등록한다**(ADR-0175 영향). 테스트 명령에 `-- --test-threads=4` 를 붙일지는 그 패키지의 테스트가 실 자식 프로세스를 띄우는지로 가른다(CLAUDE.md 「병렬은 테스트 바이너리마다 걸린다」).
- ★**의존 최소화는 일부만 얻는다**★ — 떨어지는 것은 데몬 몫(tokio · WebSocket · net · messaging · protocol 등)뿐이다. CLI 가 agent 를 의존하는 한(결정 4) agent 의 운영 의존이 함께 빌드된다 — `portable-pty` · `windows`(Windows 전용 · Job Object · Restart Manager 등) · `ts-rs` · `chrono` · base(→ `tracing-subscriber` · `regex`) 등과 command 를 거친 `inventory`(CLI 는 command 를 직접도 쓴다). 근거 = `crates/engram-dashboard-agent/Cargo.toml` · `crates/engram-dashboard-base/Cargo.toml` 대조(2026-10-02). CLI 어휘 상수를 agent 밖으로 빼면 더 줄지만, 이 결정은 그 어휘를 agent 에 둔다(위 「맥락」).
- **릴리스 빌드가 CLI 를 데몬 패키지에서 집는다** — `scripts/build-release.ps1` 의 `cargo build --release -p engram-dashboard-daemon --bin engram-dashboard-daemon --bin engram`. 같은 변경에서 새 패키지로 바꾼다(그 스크립트의 기대 exe 목록 `engram.exe` 는 그대로).
- **작업은 망가지지 않는 단위로 묶는다**(사용자 2026-10-02 — 「망가지지 않는 단위로 잘 그룹지어서 작업하라」). 새 패키지를 세우고 bin 을 옮기는 단위와 릴리스 스크립트를 바꾸는 단위가 같은 커밋에 들어가야 배포 빌드가 끊기지 않는다(위 릴리스 항목).
- **옛 경로를 가리키는 ADR 이 있다** — `crates/engram-dashboard-daemon/src/bin/engram.rs` 를 관련 포인터 · 본문으로 든다. 명단은 여기 적지 않는다 — **찾는 법 = `rg -l "src/bin/engram.rs" docs/decisions`**(이 ADR 자신도 걸린다). 이 ADR 이후 그 경로는 새 패키지의 같은 파일을 가리킨다.
- **남은 데몬 경계(별건 · 미결)** — 테스트용 서버 함수(`start_test_server*`, `crates/engram-dashboard-daemon/src/lib.rs`)가 테스트 표시 없이 공개 API 로 나가 있다. 테스트 전용 기능 플래그 뒤로 옮기는 것은 작업 순서 2-4 다. 셸 통합 테스트가 실제 데몬을 띄워 쓰므로 셸 → 데몬 테스트 의존 자체는 남는다.
- **코드 앵커 = `// ADR-0273`** — 새 패키지 `Cargo.toml` 의 `[[bin]]` 줄.
