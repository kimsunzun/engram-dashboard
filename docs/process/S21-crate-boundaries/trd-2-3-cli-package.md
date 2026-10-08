# TRD — 경계 리팩터링 2-3: engram CLI 를 데몬 패키지에서 떼어 독립 패키지로 둔다 (S21)

> 상태: **4판(리뷰 1라운드 반영) — 사용자 결정(2026-10-08: CLI = 인증 + 배송만, help 는 데몬이 낸다) 반영 · 재리뷰 전.** 코드는 아직 한 줄도 바뀌지 않았다.
> **4판에서 바뀐 것(2026-10-08):** ① **D10 신설 — help 를 데몬이 낸다.** 본문 파일 읽기 · 구획 파싱 · 필수 구획 검사 · CRLF 접기 · 어휘(화면 낱말 · 별칭)가 데몬 `control/help.rs`(가안)로 가고, CLI `help` 는 제어 라우트 `/control/help`(가안)에 요청을 보내 받은 화면을 찍는다. ② **D11 신설 — 데몬에 못 닿거나 자격증명이 없으면 데몬을 부르는 모든 명령이 한 길로 실패한다(help 포함).** help 전용 폴백은 없고 `include_str!` 내장 사본은 걷는다(ADR-0212 결정 3 · 4 개정). ③ **D4 가 접혔다** — CLI 가 설치 위치 규칙을 아예 안 쓰므로 A · A′ · B 선택이 사라졌다. 걸음 통합(옛 A′)과 데이터 루트 시험 빈틈(옛 C1)은 2-3 단위가 아니라 §8 후속이다. ④ 단위를 U1(help 이사 + D11 — 데몬 패키지 안) → U2(원자 이사) → U3(게이트 · 문서 · ADR)로 다시 짰다. ⑤ qa 바인딩 편집(옛 O3)은 **사용자 승인됨(2026-10-08)**. ⑥ 사용자에게 올릴 설계 갈림은 없다 — D11 의 출력 채널은 리뷰 1라운드 때 메인 결정으로 닫았다(지금 봉투 그대로 — §2-10).
> **4판 리뷰 1라운드 반영(2026-10-08):** codex blind(Designer) **FIX 3** · Claude doc-aware(Architect-breaker) **FIX 11**(F10 = C2 · F11 = C3 — 겹친다) — 서로 맞고 BLOCK 없음. 바뀐 것: C1 `--body-stdin` 이 자격증명보다 먼저 stdin 을 읽던 순서를 뒤집는다(오류 우선순위 변경 · 열린 stdin 시험 — §2-10) · C2 help 원천을 서버 안에서 짓지 않고 조립부(`lib.rs`)에서 주입 — 기존 `start_mcp_server` 는 그대로 두고 주입판을 더해 바뀌는 호출 자리 2 · 픽스처는 알려진 본문으로 성공 화면을 단언(§2-9 ②) · C3 바이트 기준을 기준 커밋에서 다시 짓고(수정 시각 확인) bash/node 로 아홉 호출의 exit · stdout · stderr 를 뜬다 · qa F 절 자격증명 절차는 실 engram.exe 와 처음 돈다(§3-5) · F1 help 토큰과 `-` 로 시작하는 낱말은 화면 낱말이 아니다(로컬 `BAD_ARGS`) · F2 파서 단언이 섞인 화면 시험 둘을 가른다(수 재계산 — 통과 +9) · F3 라우트 · 키 이름을 agent `types.rs` 상수로 · `topic` 필수(null = 목차) · 모르는 키 거절 · 스텁이 요청 바디를 글자로 단언 · F4 U1 QA 에 codex app-server 에이전트 안에서 `engram help` · `engram agent list` — 루프백이 막히면 착지 전 멈추고 사용자에게(§5-1 5) · F5 `CARGO_TARGET_DIR` 가 체크아웃 밖이면 help 가 `INTERNAL` · 판 어긋남 — 404 는 D11 공통 길의 `PROTOCOL_MISMATCH` 봉투, 우편 막힌 자격증명은 옛 데몬이 `MAIL_NOT_ALLOWED` 로 답한다(알려진 한계) · F6 help 원천이 기동 때 해석 경로를 info / warn 으로 · 릴리스 데몬 단독 기동 확인을 머지 전 필수로(스크래치 데이터 폴더) + 릴리스 모양 임시 폴더 단위 시험(§5-3 ②) · F7 `dev = uuid` 제거 · §5-3 「체크아웃 밖」은 데몬 측정에만 · F8 §7 에 ADR-0232 · ADR-0212 「잃은 보증」 · ADR-0156(+ ADR-0081) · F9 U1 은 커밋 하나 · O11 을 메인 결정으로 닫았다(D11 채널 = 지금 봉투 그대로).
> **리뷰 2라운드 반영(2026-10-08 · 3판 대상):** codex blind(Designer) **FIX** · Claude doc-aware(Architect-breaker) **FIX** — 둘 다 경미하고 서로 맞는다. 바뀐 것: ① 데이터 루트 조인 시험의 「체크아웃 밖이면 건너뜀」 — `CI` 가 있으면 실패 · 「안에 있나」 판정 전에 `current_exe` 와 기대 루트를 같은 `dunce::canonicalize` 로 고른다 · 로컬 확인은 `-- --show-output` ② 「표지 없음」 갈래는 임시 폴더가 아니라 판정을 인자로 받는 내부 함수 + 가짜 판정으로 잰다 ③ §5-3 의 `-OutDir` 는 모든 체크아웃 밖 · `launch\빌드.bat` 의 `launch\release` 는 체크아웃 안이라 폴백을 못 잰다 · `build-release.ps1:16-17` 주석은 U2 가 고친다 ④ base 폴백 함수 이름 ⑤ §7 2 에 ADR-0271 결정 2 ⑥ §3-4 수용 기준 2 의 「지금 결과」 서술 정정. **4판에서:** ①②④⑤ 는 D4 와 함께 2-3 밖으로 갔다(§8 O12 · O13) · ③⑥ 은 그대로 산다.
> **리뷰 1라운드 반영(2026-10-08 · 3판 대상):** `/review trd` — codex blind(Designer) **FIX** · Claude doc-aware(Architect-breaker) **FIX**. 바뀐 것: D4 를 셋으로 다시 세움(C2) · 걸음 API 이름(C3) · 데이터 루트 회귀 시험 필수화(C1) · 릴리스 빌드 두 호출을 각자의 `Invoke-Step` 으로(F1) · 릴리스 스크립트 1회를 머지 전 필수로(F2) · `roundtrip_smoke.rs` 안내문(F3) · 데몬 `--test-threads=4` 근거 정정(F4) · ADR 이 고칠 문장 목록(F5) · 포인터 수용 rg 를 짧은 꼴까지(F6) · 버전 게이트 `:1420` 과 실패 비용(F7) · 이름 알파벳의 bin 전용 예외(F8) · 자잘한 줄 번호 · 낱말(F9) · 두 호출의 릴리스 시간 비용(C3). **4판에서:** C1 · C2 와 C3 의 API 이름은 D4 와 함께 빠졌고(§8 O12 · O13), F1~F9 와 C3 의 serde_json 기능 차는 그대로 산다.
> **결정 출처:** **사용자 결정(2026-10-08)** — D10(help 를 데몬이 낸다 · 2-3 안에서 이사 전에) · D11(데몬 못 닿음 · 자격증명 없음은 모든 명령 공통 실패 · help 전용 폴백 없음 · 내장 사본 없음) · O3 승인(qa 바인딩 편집). **메인(오케스트레이터) 결정** — D1~D3 · D5~D9 · D4(D10 의 결과로 접힘) · D11 의 출력 채널(지금 봉투 그대로)과 404 의 코드(리뷰 1라운드). 그 밖의 내부 배치(라우트 · 함수 · 오류 코드 이름 · 시험 자리 · 단위 안 순서 · 게이트 꼴)는 이 TRD 의 「제안」이다(이름은 전부 가안).
> **범위 = 작업 순서 2-3 전부**(`docs/refactoring/architecture-discussion-2026-09-26.md` §8 결정 후보 9 · §10 2-3) — ADR-0273 결정 1~4 와 「영향」을 코드로 옮기고, 그 앞에 help 를 데몬으로 옮긴다(사용자 결정 2026-10-08 — CLI 가 데몬 패키지를 이미 얇은 채로 떠나게). 선행 2-2(discovery 나누기)는 착지해 master 에 머지됐다(ADR-0282).
> **배치 근거:** `docs/README.md` 「새 내용을 어디에 넣나」의 「새 기능 **설계 착수** → `process/SN-name/`」. 형제(1-1 · 1-3 · 2-1 · 2-2 TRD)가 이 폴더에 있다.
> **표기:** 「실측」 = 기준 커밋 `02ab36c`(브랜치 `v0.3.3/refactor/crate-boundaries` 의 HEAD — 코드 트리는 master `79b8d09` 흡수 머지 `d9baa2d` 와 같다. 그 뒤 세 커밋은 문서뿐)에서 잰 것과 그 명령 · 「결정」 = 위 결정 · 「ADR」 = 확정 ADR 본문 · 「제안」 = 이 TRD 의 안 · 「예상」 = 코드를 바꾸기 전이라 잴 수 없는 수치. 줄 번호는 따로 적지 않으면 `crates/engram-dashboard-daemon/src/bin/engram.rs` 다.
> 앵커: **ADR-0273**(이 단계의 헌장) · **ADR-0212**(help 본문 외부 파일 + 내장 사본 — 결정 3 · 4 를 고친다) · ADR-0220(help 다섯 화면 · 우편 가시성 필터 제거) · ADR-0211(help 가 계약을 진다) · ADR-0092(프라이밍 경로 모양) · ADR-0100(배포 동거 · manifest) · ADR-0132(CLI 표면) · ADR-0133(우편 거절은 데몬) · ADR-0282(「영향」 — CLI → 데몬 lib 임시 간선) · ADR-0271 · ADR-0270 · ADR-0175 결정 6 · ADR-0094 · step-log S21.

---

## 0. 결론 (먼저)

```
① U1 — help 를 데몬이 낸다(D10). 데몬에 제어 라우트 `/control/help`(가안 · POST · 같은 bearer · 우편 아님)를 더하고
     본문 파일 읽기 · 구획 파싱 · 필수 구획 검사 · CRLF 접기 · 화면 낱말 · 별칭을 데몬 `control/help.rs`(가안)로 옮긴다.
     라우트 경로와 바디 · 응답 키는 agent `types.rs` 의 상수 한 벌이다(`topic` 필수 · null = 목차 · 모르는 키 거절).
     help 원천은 조립부(`lib.rs`)가 주입한다 — 시험은 알려진 본문을 넣는다. 파일은 요청마다 읽는다(재시작 없이 반영).
     CLI `help` 는 낱말 하나를 실어 보내고 받은 화면을 지금과 같은 꼴로 찍는다. 화면은 호출자와 무관하게 같다
     (ADR-0220 결정 4 가 호출자별 필터를 이미 걷었다 — 데몬은 인증만 하고 누구인지로 화면을 고르지 않는다).
② U1 — D11: 데몬에 못 닿거나 · 자격증명이 없거나 · 데몬이 그 라우트를 모르면(404 — 판 어긋남) 데몬을 부르는 모든
     명령이 한 길로 실패한다(help 포함 · exit 1). help 전용 폴백은 없고 `include_str!` 사본은 걷는다(ADR-0212 결정 3 · 4
     개정). 출력은 지금의 공통 봉투(stdout JSON + exit code) 그대로다(메인 결정). `mail send --body-stdin` 은 stdin 을
     자격증명 뒤에 읽는다 — 자격증명 없는 호출이 열린 stdin 에 매달리지 않는다(오류 우선순위가 바뀐다).
③ U1 이 끝나면 CLI 는 데몬 lib 를 한 줄도 부르지 않는다 — 데몬 lib 를 부르던 유일한 줄 `:338`(help 경로)이 사라진다.
     그래서 D4(설치 위치 규칙의 자리)가 접힌다: CLI 에는 설치 위치 규칙도 base 경로 코드도 필요 없다.
④ U2 — 새 패키지 `crates/engram-dashboard-cli`(bin 전용 · bin 이름 `engram` · 소스 자리 `src/bin/engram.rs` 그대로)로
     원자 이사. 의존 = agent(명령 어휘 · help 계약 상수) · command(요청 번호) · serde_json · dev 의존 없음 — 데몬 · base
     직접 · tokio 0. engram.exe 의 빌드 그래프가 약 194 → 약 92 패키지(예상). `engram_cli.rs` 는 bin 을 따라 새 패키지로(D5).
⑤ U2 — 빌드 입구 넷(릴리스 스크립트 · rebuild 런처 둘 · `roundtrip_smoke.rs` 안내문)을 같은 커밋에서 고친다. 런처 둘은
     안 고치면 engram.exe 를 조용히 안 짓는다. 릴리스 빌드는 두 호출 · 각자의 `Invoke-Step`. qa 바인딩의 빌드 명령도
     같은 단위에서 고친다 — 사용자 승인됨(2026-10-08)(D6).
⑥ U3 — 게이트: cli 의존 상한(이름 집합 일치) · test-support 운영 그래프 고리 둘에 cli · platform 게이트 ⑦ 에 cli 0줄 ·
     버전 게이트에 cli 매니페스트 · CI 단독 기립 스텝 · 이름 알파벳의 bin 전용 예외(D7) · ADR-0284(D9).
⑦ 단위마다 커밋 하나 · 커밋마다 빌드 · 회귀가 초록. U1 도 커밋 하나다(파서 · 어휘 두 벌이 커밋에 남지 않게). U2 는 쪼갤 수
     없다(같은 bin 이름 둘이면 cargo 가 막지 않고 같은 exe 를 서로 덮어쓴다). 결과 줄 · 무시 수 그대로 · 통과 약 +9(예상).
     릴리스 스크립트 1회와 릴리스 데몬 단독 기동 확인(help 본문 경로 — 사용자 데이터 무관)은 머지 전 필수(F2 · F6).
⑧ ★착지 조건 하나★ — U1 QA 에서 codex app-server 에이전트(workspace-write 샌드박스) 안에서 `engram help` · `engram agent
     list` 가 데몬에 닿는지 잰다. 루프백이 막히면 멈추고 사용자에게 올린다(그때 D10 은 MCP 로 우편을 쓰는 codex 에이전트에게서
     우편 계약 화면을 빼앗는다 — §2-10).
⑨ 에이전트가 보는 변화: 데몬이 떠 있으면 help 아홉 호출의 exit · stdout · stderr 가 바이트로 같아야 한다(§3-5). 바뀌는 것 =
     데몬 없이 · 자격증명 없이 help 가 실패한다 · 모르는 화면 낱말의 반려 `BAD_ARGS` → `INVALID_ARGUMENT`(가안) ·
     `help <낱말> <더>` 의 반려 문구 · 본문 파일이 없거나 깨지면 낡은 사본 대신 반려 · 404 가 빈 줄 대신 `PROTOCOL_MISMATCH`
     봉투 · `CARGO_TARGET_DIR` 가 체크아웃 밖이면 help 가 `INTERNAL` · 옛 데몬 + 새 CLI 의 우편 막힌 자격증명은 help 에
     `MAIL_NOT_ALLOWED` 를 받는다(알려진 한계 — §2-10) · 자격증명 없고 stdin 이 빈 `--body-stdin` 은 `NO_TOKEN`.
```

**결정 출처:** D10 · D11 · O3 = 사용자 결정(2026-10-08). D11 의 출력 채널(지금 봉투 그대로) · 404 의 코드(`PROTOCOL_MISMATCH` 가안) · 나머지 = 메인 결정(머리말). **사용자에게 올릴 것은 없다** — 단 ⑧ 의 codex 측정이 루프백 차단을 보이면 착지 전에 올린다.

**지금 하지 않는 것:** `scripts/engram.mjs`(T-23 · ADR-0132 — D8) · CLI 어휘 상수를 agent 밖으로 빼기(ADR-0273 「영향」) · 테스트용 서버 함수 정리와 셸 → (dev) 데몬 간선(2-4) · CLI 표면 · help 화면 내용 변경 · 데몬 · 셸 걸음 두 벌 통합과 데이터 루트 시험 빈틈(§8 O12 · O13) · D11 의 채널을 stderr 로 옮기는 것(메인 결정 — 지금 봉투 그대로 · §2-10) · 기존 다섯 제어 라우트의 경로를 agent 상수로 옮기는 것(§8 O15) · 데몬의 쓰이지 않는 `tracing-subscriber` 선언(별건 — `docs/todo/doc-drift.md:16`).

---

## 1. 현황 실측 (`02ab36c`)

### 1-1. bin 과 그 의존

| 무엇 | 자리 | 비고 |
|---|---|---|
| bin 소스 | `engram.rs` 5,508줄 — 시험 모듈 `:2806`(`#[cfg(test)]`) 아래 124건 · `#[ignore]` 0 | 시험 앞 2,805줄 |
| 워크스페이스 import | `:117-121` `engram_dashboard_agent::types::{…}`(CLI 어휘 상수 — 열둘 전부 비시험 코드가 쓴다 · 실측 awk) · `:177` `engram_dashboard_command::RequestId` · `:338` `engram_dashboard_daemon::data_dir::find_install_root`(앵커 `:337` · `help_file_path` `:336-340` 안) | ★데몬 lib 를 쓰는 유일한 줄 `:338` 은 help 본문 경로만을 위한 것이다★(ADR-0282 「영향」의 임시 간선) — help 가 데몬으로 가면 간선째 사라진다 |
| 서드파티 | `serde_json`(운영) · `uuid`(시험 `:4482` 하나) · 나머지는 std(`TcpStream` 손조립 HTTP — `:113-115`) | protocol · net · platform · tokio 0. OS `cfg` · `std::os` 0(`cfg(` 는 `:2806` 하나) |
| help 본문 | `:231` `include_str!("../../../../prompts/engram-help.md")` · 시험 도우미 `help_repo_root`(`:4339` — `CARGO_MANIFEST_DIR` 의 두 단계 위) | U1 이 `include_str!` 를 걷는다. `help_repo_root` 는 CLI 시험에 남는다(§2-9 ⑦) |
| 선언 | 데몬 `Cargo.toml:33-39`(`[[bin]] name = "engram"` · 주석 `:33-36`) · 패키지 버전 `:3` = `0.3.2` | 이 bin 만을 위한 데몬 의존 줄은 없다 — `serde_json` · `uuid` 는 데몬 자신도 쓴다 |
| 실 exe 시험 | 데몬 `tests/engram_cli.rs` 1,784줄 · 41건 · `#[ignore]` 0 — `env!("CARGO_BIN_EXE_engram")`(`:122` 등) · 파일명 가드 `the_built_binary_file_name_matches_the_shared_constant`(`:604-616`) · import = agent(`:13`) · serde_json · std. ★데몬을 띄우지 않는다★ — std `TcpListener` 스텁(`:15-95`)이 고정 응답을 낸다(`:6`) | `CARGO_BIN_EXE_<name>` 은 같은 패키지의 bin 만 보인다 → bin 을 따라가야 한다 |

`cargo metadata` — 데몬 타깃 = lib · bin 여섯(`engram` · `engram-dashboard-daemon` · `test-harness` 전용 넷) · 시험 일곱(`engram_cli` 포함). `CARGO_BIN_EXE_engram` 을 쓰는 시험은 `engram_cli.rs` 뿐이다(`rg CARGO_BIN_EXE crates src-tauri` — 그 밖은 `ws_e2e.rs:2329` 의 데몬 exe 하나).

### 1-2. `engram help` 가 지금 도는 법

| 단계 | 자리 | 동작 |
|---|---|---|
| 분기 | `run` `:399-438` · help 갈래 `:407-412` | 파싱이 `ParsedCommand::Help(topic)` 이면 **자격증명 검사(`:428-434`) 앞에서** `println!("{}", render_help(topic))` · exit 0. 주석 `:407` 「help 는 크레덴셜도 데몬도 없이 답한다」 · 헤더 `:22-23` 같은 말 |
| 어휘 | `HelpTopic` `:556-605`(Root · Mail · Agent · Window · Settings) · `SECTION_ROOT` `:235` · `HELP_TOPIC_WINDOW/SETTINGS` `:242-243` · 별칭 `HELP_TOPIC_ALIASES` `:251`(`theme` → Settings) | 화면 하나 = 구획 하나 = 낱말 하나. mail · agent 낱말은 agent 상수 `CLI_GROUP_MAIL/AGENT`, 나머지 둘은 CLI 지역 상수 |
| 파싱 규칙(형태) | `parse_command` `:750-779` · `is_help_token` `:874-876`(`help` · `--help` · `-h`) · `reject_help_with_extra_args` `:881-894`(반려 문구가 `HelpTopic::ALL` 의 낱말 목록을 싣는다) · `parse_help_topic` `:896-908`(모르는 낱말 → `unknown help topic: …` BAD_ARGS) | help 는 키워드 자리에서만 · 단독 호출일 때만. 모두 네트워크 전이다 |
| 경로 | `help_file_path` `:336-340` = 데몬 `find_install_root()` + `REL_HELP_FILE`(`:207` `prompts/engram-help.md`) · 절대일 때만 | 프라이밍과 같은 앵커(ADR-0092 모양) |
| override | `ENV_HELP_FILE` `:213` = `ENGRAM_HELP_FILE`(이 CLI 프로세스의 env) · 비어 있지 않으면 고정 경로보다 먼저 · 실패해도 고정 경로로 안 내려간다(`:209-212`) | 저장소 안에 이 변수를 설정하는 곳은 없다(`git grep ENGRAM_HELP_FILE` → `engram.rs` 뿐) |
| 로드 | `load_help_text` `:348-373` · 프로세스당 한 번(`help_text` `:376-379` `OnceLock`) | 읽기 실패 · 구획 누락이면 `warn_help_fallback`(`:344-346` — **stderr 한 줄**, stdout 무오염 · exit 0 그대로) 뒤 `HELP_EMBEDDED`(`:231`) 로 |
| 파싱 | `HelpText` `:253-309` · `section_marker` `:311-321`(줄 전체가 `## <id>` · id 는 소문자 · 점 · 밑줄) · CRLF 만 접는다(`:266-268`) · 첫 구획 앞 머리글은 버린다 | 반쪽 로드 없음 — 필수 구획(`required_section_ids` `:324-328` = 다섯) 하나라도 없으면 표째 버린다 |
| 렌더 | `render_help_from` `:388-391` = 구획 하나 + `{tool}`(`HELP_TOOL_SLOT` `:188`) → `CLI_EXE_NAME` | 출력 = `println!` — 구획 본문 뒤에 줄바꿈 하나가 더 붙는다 |

- **★호출자별 화면 필터는 지금 없다★** — 우편 가시성 표식(`ENGRAM_MAIL`)과 `MailSurface` 는 ADR-0220 결정 4(2026-09-23)가 주입 · 독해 양쪽에서 걷었다. 루트 · 에이전트 화면을 가르는 것도 없다 — `Root` 는 목차 화면의 이름일 뿐이다. 「거절당할 자격증명으로도 같은 화면」을 잡는 시험이 `engram_cli.rs` `the_mail_screen_opens_whatever_the_environment_carries`(`:1712-1752`)다. 그래서 help 를 데몬으로 옮겨도 **옮길 필터가 없다** — 데몬은 인증만 하고 화면은 누구에게나 같다(§2-9 ③).
- 본문 파일은 지금 19,891 바이트다(`wc -c prompts/engram-help.md` — ADR-0220 근거의 「8.7KB」는 그 시점 값). 구획 = `root` `:15` · `mail` `:24` · `agent` `:46` · `window` `:72` · `settings` `:146` · `theme` `:187`(옛 바이너리 호환 — 이 바이너리는 내지 않는다). 머리글 `:1-13` 은 파서가 버리는 편집자용 글이다.

### 1-3. 하위 명령별 — 로컬로 끝나나 데몬을 부르나 · 실패 경로 (D11 의 재료)

| 하위 명령 | 지금 | 데몬 라우트 |
|---|---|---|
| `help [낱말]` · `<계열> --help` · 인자 없음 | 로컬 렌더(자격증명 · 데몬 없이) | 없음 |
| `commands [이름]` | 자격증명 뒤 · 목록 받아 렌더(`run_catalog` `:1818-1843`) | `POST /control/commands` |
| `<전체.이름> --k v` · `<전체.이름> --help` | 목록 받아 스키마로 인자 옮김 → 호출(`run_invoke` `:1849-`) · `--help` 는 목록만 받아 상세 화면 | `/control/commands` → `/control/call` |
| `agent …` | 자격증명 뒤 `run_legacy` `:476-499` | `POST /control/agent` |
| `mail send/status/pending` | 같은 `run_legacy`(본문 stdin 읽기는 자격증명 **앞** — `:420-425`) | `/control/send` · `/control/messages` |
| 인자 형태 오류(어느 명령이든) | `print_error("BAD_ARGS", …)` · exit 1 — 자격증명 앞 · 네트워크 0 | 없음 |

- **자격증명 없음 — 이미 한 자리다:** `read_credentials` `:457-472`(`NO_TOKEN` · `NO_CONTROL_URL`)를 `run` `:428-434` 이 한 번 부르고 `print_error(code, hint)` · exit 1. 데몬을 부르는 세 갈래(옛 계열 · 발견 · 호출)가 모두 그 뒤에 선다 — help 만 그 앞이다.
- **순서 하나(C1):** `mail send --body-stdin` 은 stdin 을 자격증명보다 먼저 끝까지 읽는다(`:418-425` → `:428-434` · `read_stdin_to_string` `:1320-1326` = `read_to_end`). stdin 이 닫히지 않으면 자격증명 없는 호출이 실패하지 못하고 멈춘다.
- **데몬에 못 닿음 — 같은 꼴, 세 자리:** `post_json` `:2634-` 이 `SendError`(`Connect` — 연결 · 쓰기 · 읽기 IO · 침묵 한도 초과 · URL 파싱 / `Incomplete` — Content-Length 보다 짧은 body)를 돌려주고, 받는 쪽이 각자 `print_error(e.code(), &e.to_string())` + exit 1 을 적는다: `run_legacy` `:493-496` · `fetch_catalog` `:1759-1763` · `run_invoke` `:1897-1900`. 코드 = `CONNECT_FAILED` · `INCOMPLETE_RESPONSE`.
- **다른 점 하나(지금도 있다):** `fetch_catalog` 가 호출(`<전체.이름> …`)의 첫 왕복으로 불렸으면 `note_catalog_failure` `:1809-1816` 가 stderr 에 「위 답은 목록 조회의 것이고 그 명령은 불리지 않았다」 한 줄을 더한다 — 어느 요청이 실패했는지 밝히는 진단이다.
- **출력 채널:** `print_error` `:2801-2804` = stdout 에 `{"status":"error","code":…,"hint":…}` 한 줄. 헤더 계약 `:72-78` — 「이 CLI 가 **스스로** 내는 반려(BAD_ARGS · NO_TOKEN · UNKNOWN_COMMAND · 전송 실패)만 항상 봉투 JSON 이고 … 기계 판정은 stdout 형태가 아니라 **exit code** 로 한다」. stderr 는 진단 줄(형태가 깨진 성공 · 목록 행 버림 · 위 메모)에만 쓴다. 시험이 stdout 봉투를 잰다 — `engram_mail_send_transport_error_exits_one_with_error_json`(`engram_cli.rs:243-259`) · `NO_TOKEN` 대조군(`:713-718` · `:1644-1648`).
- **데몬이 닿았지만 거절 · 비-2xx** 는 D11 대상이 아니다 — 받은 body 를 그대로 찍고 exit 1(데몬의 답이다 · 3분법 `:79-86`). 단 404 는 D11 로 간다(§2-10 — 지금은 빈 body 를 빈 줄로 찍고 exit 1 이다).

### 1-4. 데몬 쪽 — 제어 라우트 · 인증 · 프라이밍 로더

- **라우트 선언:** `control/mcp_server.rs` — 경로 상수 `:51-97`(`/control/send` · `/messages` · `/agent` · `/commands` · `/call` · 네임스페이스 `/control`) · 닫힌 명단 `ControlRoute` `:108-143`(`ALL` `:118` = 5) · 우편 분류 `is_mail` `:150-162`(exhaustive — `_` 금지) · 명단 밖 `/control…` 은 우편으로 접혀 거절(`mail_gated_path` `:178-189` · fail-closed) · 라우터는 명단을 돌며 얹는다(`:1302-1327` — 빌더 체인 금지) · 명단 시험 `:1380-1400`(`ALL.len() == 5` `:1394` · 우편 둘).
- **모양 규율:** 전부 무-세션 **POST**(무-세션 GET 은 미들웨어가 400 — `:57-61`) · 항상 200 + JSON(성공 · 반려 모두 — 반려 = `ControlQueryResult::Error` → `{"status":"error","code","hint"}` · `control/ingress.rs:440-455`) · 오류 코드 어휘 = command `ErrorCode`(`crates/engram-dashboard-command/src/error.rs:65-85` — `INVALID_ARGUMENT` · `UNKNOWN_COMMAND` · `INTERNAL` …). 데몬은 `BAD_ARGS` 를 쓰지 않는다(그건 CLI 고유 코드).
- **인증:** `bearer_auth` `:648-790` 이 라우터 전체를 감싼다 — `Authorization: Bearer <token>` → `registry.validate` → 없으면 401. 통과하면 요청 extensions 에 `BoundIdentity{agent_id, epoch}`(`:775`)와 `TokenBinding`(`mail_allowed` 포함 · `:779`)을 싣는다. **데몬은 호출자를 안다**(어느 에이전트 · 우편 가부) — 단 help 에는 그것이 필요 없다(1-2 — 필터 없음). 우편 라우트 거절도 이 미들웨어 한 곳이다(`:681-696`).
- **404 가 나는 자리(F5):** 라우터 fallback(명단 밖 경로 — 단 `/control…` 아래 명단 밖 경로는 우편 막힌 자격증명이면 `mail_gated_path` 가 먼저 200 + `MAIL_NOT_ALLOWED`(`mail_not_allowed` `:830-838`)로 답하고, 열린 자격증명만 404 다 — 시험 `tests/mail_gate.rs:400-427`) · `bearer_auth` 의 세션 고아(`:759` — `Mcp-Session-Id` 를 실은 요청만 · CLI 는 안 싣는다). 둘 다 본문이 비어 있다(`not_found()` `:815-820` · axum 기본 fallback). 핸들러는 늘 200 이다.
- **자격증명이 닿는 범위:** 스폰 때 control endpoint 가 있는 모든 에이전트(claude · codex)에 `ENGRAM_TOKEN` · `ENGRAM_CONTROL_URL` · PATH 가 깔린다(`crates/engram-dashboard-agent/src/backend/mod.rs:71-` `inject_cli_entrance` · claude `backend/claude/mod.rs:258-259` · codex `backend/codex/mod.rs:973-974`). 프라이밍(`prompts/agent-priming.md`)이 `engram help <계열>` 을 가리키는 상대 = 바로 그 스폰들이다 — **help 를 읽는 에이전트는 자격증명을 이미 든다.**
- **프라이밍 로더:** `control/priming.rs` `FilePrimingProvider` `:239-337` — base = `from_install_root()`(`:274-277` — `data_dir::find_install_root()`, 못 얻으면 `.` → 절대화 실패 → `None`) · `ENGRAM_PRIMING_FILE`(`:244`) 가 고정 경로를 이기고 실패해도 고정으로 안 내려간다 · 절대경로 · cmd 메타문자 · 존재 검사(`resolve_checked` `:290-320`) · **내용은 읽지 않는다**(경로만 claude 에 넘긴다). 운영 배선 = `lib.rs:631`. 시험 도우미 `repo_root()` `:494-`.
- **MCP 쪽에 help 본문은 없다** — 도구 설명문 둘이 `engram help mail` 을 가리키기만 한다(`mcp_server.rs:419` · `:511` · 시험 `:1700`).
- **통합 시험 픽스처:** `tests/mail_gate.rs` — 실 서버를 프로세스 안에 띄우고(`start_mcp_server`) 우편 막힌 자격증명(`mcp_token`)과 열린 자격증명(`cli_token`)을 함께 쥔다(`:68-70` · `:218-219`). 발견 · 호출 라우트가 두 자격증명 모두에 열려 있음을 `the_same_credential_passes_on_the_catalog_routes`(`:298-326`)가 잰다. `start_mcp_server` 를 부르는 곳은 25 군데다(`git grep -n "start_mcp_server("` — 시험 · 하네스 bin 포함).

### 1-5. help 를 재는 시험 (지금)

- **CLI 단위(`engram.rs`) — help 에 닿는 것 19건:** 화면 렌더 · 파일 로드 쪽 13 = `each_group_word_renders_its_own_screen` `:4169` · `the_old_theme_word_reaches_the_settings_screen` `:4201` · `the_help_file_still_satisfies_the_pre_rename_binary_and_this_one_ignores_the_extra` `:4225` · `the_mail_screen_teaches_the_reply_contract` `:4264` · `the_embedded_copy_carries_every_section_the_screens_need` `:4353` · `the_runtime_path_and_the_embedded_copy_name_the_same_file` `:4366` · `the_fixed_help_path_resolves_absolute_to_a_real_file` `:4378` · `a_help_file_missing_one_section_is_rejected_whole` `:4391` · `every_screen_still_renders_from_the_embedded_copy_alone` `:4406` · `a_screen_never_renders_empty_even_with_no_sections_at_all` `:4426` · `a_section_marker_must_be_a_whole_line` `:4442` · `section_bodies_keep_their_whitespace_but_fold_crlf` `:4465` · `the_env_override_wins_and_its_failure_falls_back_to_the_embedded_copy` `:4477`. 파싱과 렌더가 섞인 것 4 = `agent_exit_code_requires_every_list_field_the_help_screen_promises` `:3947`(exit code 판정기와 agent 화면을 한 시험이 묶는다) · `help_lists_groups_and_each_group_documents_its_verbs` `:4060` · `the_priming_pointer_names_help_entries_that_actually_render` `:4301` · `the_static_help_points_at_the_catalog_verb_by_its_real_spelling` `:5004`(CLI 지역 상수 `CLI_VERB_COMMANDS` 와 root 화면을 묶는다). 파싱만 2 = `a_help_token_is_not_a_help_topic` `:4037` · `unknown_group_and_unknown_verb_point_at_help` `:4052`(`help wat` 이 파싱 오류라는 단언을 포함).
- **CLI 프로세스(`engram_cli.rs`) — help 에 닿는 것 8건, 전부 「자격증명 · 데몬 없이 help 가 답한다」에 기댄다:** `engram_agent_argument_errors_never_touch_the_network` `:524`(hint 가 제안한 help 명령을 자격증명 없이 돌려 exit 0) · `engram_agent_help_is_discoverable_the_same_way_as_the_mail_group` `:565` · `engram_help_lists_groups_and_group_help_documents_its_verbs` `:619`(`UNREACHABLE_URL` 로 · `help mail send` 반려가 네 낱말을 싣는지) · `engram_help_answers_before_any_credential_check_and_prints_plain_text` `:678`(성질 pin) · `conventional_help_spellings_render_the_same_screens` `:724`(`help inbox` → exit 1 포함) · `engram_unknown_group_and_bare_flags_are_argument_errors_without_touching_the_network` `:765`(`help wat` 을 네트워크 0 목록에 둔다 · 제안 help 를 자격증명 없이 돌린다) · `the_static_help_points_at_the_catalog_without_fetching_it` `:1633` · `the_mail_screen_opens_whatever_the_environment_carries` `:1712`. 도우미 `run_cli_without_credentials` `:659-670` · `run_cli_bare` `:1692-1703`.
- **고정 위치를 ADR 이 적는다:** ADR-0220 「영향」 — `the_mail_screen_teaches_the_reply_contract` · `the_priming_pointer_names_a_help_entry_that_actually_renders`(실제 이름은 `…names_help_entries_that_actually_render` — 1-11) 둘 다 「`crates/engram-dashboard-daemon/src/bin/engram.rs`」. 짧은 꼴 포인터 셋이 그 화면 시험을 가리킨다 — `control/priming.rs:705` · `control/mcp_server.rs:1636` · `:1697`(`bin/engram.rs`) · 그리고 `control/catalog.rs:421`(「그 화면은 정적이라(bin/engram.rs `render_help`)」).
- **배포:** `scripts/build-release.ps1:50` `$ExpectedPrompts = @('agent-priming.md', 'engram-help.md')` — 복사(`:194`) · 빠짐 · 남음 양방향 tripwire(`:215-226`). 그 위 주석 `:18-21` 「★이 파일이 빠져도 help 는 죽지 않는다★ — include_str! 사본」. `ci.yml:32-34` 경로 필터 주석이 그 `include_str!`(`engram.rs:231`)을 근거로 든다.

### 1-6. 에이전트가 읽는 글 — 「help 는 데몬 없이 된다」고 약속하나

- **`prompts/agent-priming.md`(1,287 바이트):** 그런 약속이 없다. 「아래가 필요해지면 그 줄의 명령을 셸에서 실행한다」 + `engram help <계열>` 네 줄 · 「`engram` 명령이나 도구가 거절당하거나 계속 실패하면 우회하지 말고 주인에게 알린다」 — D11 과 맞는다. **고칠 줄 0.**
- **`prompts/engram-help.md` 화면(구획 `:15-`):** 데몬 · 자격증명 · 오류 채널에 대한 말이 없다(`grep -n -i "NO_TOKEN\|CONNECT_FAILED\|stdout\|stderr\|데몬\|daemon"` — 데몬은 「중개 데몬」 낱말로 `:3`(프라이밍) · `:45` 에만). **고칠 줄 0.**
- **같은 파일의 편집자용 머리글(화면 밖):** `:3` 「고치면 다음 `engram help` 호출이 바뀐 글을 낸다 — 다시 빌드하지 않는다」(D10 에서도 참 — 요청마다 읽는다) · `:8` 「하나라도 빠뜨리면 이 파일이 통째로 거부되고 **내장 사본이 대신 나간다**」 · `:9` 「빠지면 옛 바이너리가 새 파일을 거부하고 **자기 내장 사본**(옛 내용)을 낸다」 → **`:8` 은 U1 에서 고친다**(사본이 없다 — help 가 오류로 답한다). `:9` 는 이 저장소 밖에 남은 옛 engram.exe(파일을 직접 읽던 판)에 대한 말이라 참으로 남는다 — 「옛 바이너리」를 「파일을 직접 읽던 2-3 전 engram.exe」로 좁혀 적는다.
- **코드 주석 · 문서(에이전트가 안 읽는다):** `engram.rs:22-23` · `:198-199` · `:407`(데몬 없이 답한다) · `engram_cli.rs:675-677` · `:1630-1631` · `build-release.ps1:18-21` · `ci.yml:32-33` — U1 이 고친다(§6 1). ADR-0211 「영향」 「`engram help` 는 자격증명도 데몬도 없이 답한다」 · ADR-0212 결정 3 · 4 · ADR-0220 「영향」 첫 항목은 ADR-0284 가 링크로 고친다(§7).

### 1-7. 설치 위치 · 체크아웃 루트 찾기 — U1 뒤 CLI 는 이것을 부르지 않는다

| 자리 | 내용 |
|---|---|
| 데몬 `data_dir.rs:162-168` `find_install_root` | `current_exe` → 걸음 → 못 찾으면 exe 폴더 · exe 도 못 얻으면 `None`. 데몬 상태를 읽지 않는 exe 경로의 순수 함수다. 부르는 곳 = `control/priming.rs:275` · CLI `:338`(U1 이 걷는다) · U1 뒤 데몬 help(§2-9) |
| 데몬 `data_dir.rs:170-196` `find_workspace_root` · `is_workspace_root` | 비공개. 부르는 곳 = `default_data_dir` 디버그 분기(`:72-87` · 호출 `:78`) · `find_install_root`(`:164`). 표지 시험 넷(`:415-461` · `:473-495`) |
| 셸 `src-tauri/src/discovery/layout.rs:99-127` | 같은 두 함수의 사본. 부르는 곳 = `default_data_dir` 디버그 분기(`:51`) 하나. 같은 경로 시험(`:231-279`)이 루트 결과만 데몬과 견준다(ADR-0282 결정 3) |

- 두 벌의 본문은 글자까지 같다(실측 `diff` — 차이는 셸의 `cfg_attr` 한 줄). 이 두 벌은 ADR-0282 결정 3 이 정한 그대로 2-3 이 건드리지 않는다 — 합치는 안(옛 A′)은 §8 O13 후속이다.
- **빈틈(옛 C1 — 2-3 단위가 아니다):** 디버그 데이터 루트를 재는 시험은 끝이 `.engram-dev` 인지만 본다 — 데몬 `data_dir_empty_env_falls_through_to_default`(`:284-301`) · `default_data_dir_debug_is_local_data_dir`(`:303-317`) · 셸 같은 경로 시험의 전제(`:242-250`). 걸음이 실패해 exe 폴더 폴백으로 떨어져도 셋 다 초록이다. 3판은 D4 가 이 분기를 지나서 U1 에서 닫으려 했다 — 4판은 그 분기를 안 지나므로 §8 O12 로 넘긴다.
- 저장소는 target 폴더를 체크아웃 밖으로 돌리는 경우를 안다(`CARGO_TARGET_DIR` — `scripts/build-client-shell.mjs:48` · `build-release.ps1:79`). 그때는 걸음이 표지를 못 만나 운영 동작부터 exe 폴더 폴백이다 — §2-9 의 시험 설계가 이 경우를 피한다(⑦).

### 1-8. 빌드 입구 — 「데몬 패키지를 지으면 CLI 도 지어진다」에 기대는 자리

| 자리 | 지금 | 2-3 뒤 그대로 두면 |
|---|---|---|
| `scripts/build-release.ps1:133-135` | 한 `Invoke-Step` 안 한 호출 `-p engram-dashboard-daemon --bin engram-dashboard-daemon --bin engram` | 빌드 실패(그 패키지에 bin `engram` 이 없다) — 요란하다 |
| `build-release.ps1:61-66` `Invoke-Step` | 블록 진입 전 `$LASTEXITCODE` 를 0 으로 두고 ★블록이 끝난 뒤 한 번만★ 본다 | 한 블록에 cargo 호출 둘을 넣으면 앞 호출의 실패를 뒤 호출의 성공이 덮는다 — 그 뒤 존재 검사(`:139-142`)는 이전 빌드의 낡은 exe 를 받아들인다(F1) |
| `ci.yml:1281-1283`(태그 릴리스만 · `if: github.ref_type == 'tag'`) | 위 스크립트를 부른다(`:1471-1477`) | 같다 — 이 스크립트와 버전 게이트는 태그 push 에서만 돈다(F2) |
| `scripts/rebuild-run-debug.bat:93` · `scripts/rebuild-run-release.bat:111` | `cargo build [--release] -p engram-dashboard-daemon` | ★조용하다★ — engram.exe 를 안 짓는다. 데몬은 남아 있는 옛 exe 를 형제로 집거나, 없으면 warn 한 줄 뒤 CLI 입구 없이 뜬다(데몬 `lib.rs:103-117` · fail-open) |
| 데몬 `src/bin/roundtrip_smoke.rs:66`(doc) · `:424`(SETUP-SKIP 안내 문자열 — 실행 중 찍힌다) | `cargo build -p engram-dashboard-daemon --features test-harness --bin engram` | 그 명령을 따라 치면 「no bin target named `engram`」로 진다(F3) |
| qa 바인딩 `.claude/skill-bindings/qa.md:320` · `:355` · `:360` | GUI 실측 전 `cargo build -p engram-dashboard-daemon` | 조용하다(런처와 같다) |
| `src/util/launcherWiring.test.ts:31` | 정규식 `-p\s+engram-dashboard(?![-\w])` 가 런처 · qa 바인딩의 생 셸 빌드를 막는다 | `-p engram-dashboard-cli` 는 뒤에 `-` 가 와 걸리지 않는다(코드 읽기) |

`build-release.ps1:49` 의 기대 exe 목록(`engram.exe`)과 `scripts/rebuild-run-debug-log.bat`(debug 런처를 부를 뿐 — `:23`) · `launch/빌드.bat`(그 스크립트를 부를 뿐 — `:4`)은 빌드 명령을 고칠 것이 없다. 단 `launch/빌드.bat:4` 의 조립 폴더 `"%~dp0release"` = `<저장소>\launch\release` 는 체크아웃 안이다 — 거기 놓인 릴리스 데몬의 설치 위치는 저장소 루트가 된다(지금 동작 · `build-release.ps1:16-17` 주석과 어긋난다 — §5-3). `rg -n "engram-dashboard-daemon[^\n]*--bin engram($|[^-\w])"` 의 지금 결과가 정확히 위 세 줄(`build-release.ps1:135` · `roundtrip_smoke.rs:66` · `:424`)이다.

### 1-9. 데몬이 CLI 를 찾는 법 — 바뀌지 않는다

- 데몬 `locate_send_exe`(`lib.rs:103-117`)와 하네스 `sibling_send_exe`(`src/bin/roundtrip_smoke.rs:902-903`)는 `platform::env::sibling_exe(CLI_EXE_NAME)` — 현재 exe 폴더 + 이름이다(ADR-0282 결정 1). 워크스페이스는 target 폴더 하나를 나눠 쓰므로 새 패키지의 `engram.exe` 도 `target\<profile>\` 에 떨어져 형제 관계가 그대로다(cargo 기본 동작 — §9).
- ★단 「데몬을 지으면 CLI 도 지어진다」가 사라진다★ — 데몬 실 레인 `cargo test -p engram-dashboard-daemon --test ws_e2e -- --ignored` 는 engram.exe 를 짓지 않게 된다. 2-2 U1 은 그 레인이 띄운 데몬의 로그에 「형제 exe 를 못 찾음」 warn 이 없는지를 봤다(2-2 TRD §3-2) — 그 확인을 다시 쓰려면 CLI 를 먼저 짓는다.

### 1-10. 게이트 · 바인딩 · 문서가 CLI 자리를 부르는 곳

- **`ci.yml`** — `:32-34`(경로 필터 주석 — `prompts/engram-help.md` 가 `include_str!` 로 구워진다 · 포인터 `engram.rs:231`) · `:520-523`(메시징 이름 정규식 주석 「새 워크스페이스 crate 가 생길 때마다 여기 이름을 더하고」) · `:675` · `:767`(test-support 운영 그래프 고리 `for pkg in engram-dashboard-daemon engram-dashboard`) · `:1208-1229`(platform 게이트 ⑦ — 대상 agent · net) · `:1355-1362`(버전 게이트 주석 「5건」 · 셋째 = 데몬 `Cargo.toml` 「daemon.exe·engram.exe 를 만드는 crate」) · `:1411`(그 검사 줄) · `:1420`(`PASS — 제품 버전 선언 5건`). platform 게이트 ④ · ⑥ 은 `crates` · `src-tauri` 전체를 훑고 명단에 CLI 파일이 없다 — 두 파일에 OS `cfg` · `std::os` 가 없으므로(1-1) 명단은 그대로다(`roundtrip_smoke.rs` 는 명단에 있지만 안내문 편집은 `cfg` 를 건드리지 않는다). 새 `control/help.rs` 도 OS `cfg` 가 없다(§2-9). 게이트 ⑤ 는 `--workspace` 라 새 멤버를 저절로 본다. CI 의 crate 별 시험 스텝(base · platform · agent · command · protocol · messaging · net · transport)에 데몬 스텝은 없다(워크스페이스 회귀가 덮는다).
- **버전** — 라이브러리 crate 는 `0.1.0` 이고 버전 게이트 주석이 그것을 「단독 배송물이 없는 내부 semver」라 뺀다(`:1373-1375`). 데몬 · 셸은 제품 버전 `0.3.2`. ★CLI 는 `CARGO_PKG_VERSION` 을 읽지 않는다★(`rg CARGO_PKG_VERSION crates src-tauri/src` → agent 셋 · 데몬 `agent_conn.rs:290` 뿐).
- **serde_json 기능(C3)** — 데몬 그래프는 serde_json 에 `default` · `std` · `alloc` · `raw_value` 를 켜고, agent · command 그래프는 `default` · `std` 만 켠다(실측 `cargo tree --locked -p <p> -e normal,features --target all -i serde_json`). 한 cargo 호출 안에서는 고른 패키지들의 기능이 합쳐지고 따로 부르면 갈린다.
- **데몬 시험의 프로세스 띄우기(F4)** — `--test-threads=4` 를 데몬에 붙이는 근거는 CLI 스위트만이 아니다. 기본 실행(`#[ignore]` 아님) 시험이 PTY 로 실 셸을 띄운다 — `tests/control_agent.rs:406` · `:824` · `:1139` · `tests/mcp_manager_lifecycle.rs:236` · `:324`(`platform::shell::default_shell()`) · `src/connection_core.rs:2890`(`cmd.exe /c exit`) · `src/messaging_host.rs:1137`. 그 위에 `ws_e2e.rs` 의 `#[ignore]` 실 데몬 exe 분이 있다.
- **CLAUDE.md** — `:207`(`inventory` 가 「데몬·셸 릴리즈 바이너리」에 링크된다 — engram.exe 가 빠져 있다) · `:222`(CI 에만 있는 async 반입 게이트) · `:231`(「병렬은 테스트 바이너리마다 걸린다」의 「`-p engram-dashboard-daemon`도 해당 — 프로세스 레벨 CLI 스위트와 실 `.exe` spawn `#[ignore]` 분을 갖는다」) · `:255`(메시징 정규식) · `:256-261`(의존 상한 · 시험 기능 게이트 줄) · 「백엔드 모듈 맵」에 CLI 항목 없음. (3판이 고치려던 `:158` base `path` 서술은 4판에서 그대로다.)
- **qa 바인딩** — `:79-84`(게이트 ⑦ 로컬 블록) · `:138`(`--test-threads=4` 를 붙이는 crate `agent`·`daemon`·`platform`) · `:253`(메시징 정규식 「새 워크스페이스 crate가 생기면 여기에 더하고」) · `:254-261`(의존 상한 「4종」) · `:262-275`(test-support 두 블록) · `:320` · `:355` · `:360`(빌드) · `:442` · `:445`(F 절 — CLI 자리 서술).
- **문서 · 주석** — `docs/testing-strategy.md:93` · `:95` · `:159`(데몬 절의 `engram_cli`) · `docs/reference/architecture-overview.md:169`(실행 산출) · `:173-197`(crate 계층 그래프) · `:384`(`crates/engram-dashboard-daemon/src/bin/engram.rs` 헤더) · 짧은 꼴 포인터(F6) — agent `src/types.rs:474`(daemon `tests/engram_cli.rs`) · `:558`(daemon `bin/engram.rs` 의 드리프트 시험 — CLI 에 남는다) · 데몬 `control/mcp_server.rs:208`(CLI 가 자기 사본을 든다 — CLI 몫) · ★`control/priming.rs:705` · `control/mcp_server.rs:1636` · `:1697` · `control/catalog.rs:421` 은 help 화면 시험 · 렌더를 가리키므로 U1 에서 데몬 `control/help.rs` 로 바뀐다(cli 경로가 아니다)★ · 그 밖 `build-release.ps1:11` · `:127-132` · `rebuild-run-release.bat:5-8` · `scripts/README.md:12`.
- **ADR** — `src/bin/engram.rs` 를 부르는 ADR 10 개(`git grep -l "src/bin/engram.rs" -- docs/decisions` — ADR-0273 「영향」의 읽기 규칙이 덮는다) · `engram_cli` 를 부르는 ADR 2 개(ADR-0220 · ADR-0232 — 그 규칙 밖이다) · help 화면 시험 위치를 적는 ADR-0220 「영향」(그 시험은 cli 가 아니라 데몬으로 간다 — §7 6).

### 1-11. 기록과 어긋난 사실

| 기록 | 실제(실측 `02ab36c`) |
|---|---|
| ADR-0273 「맥락」 — 데몬 lib 를 한 줄도 쓰지 않는다 · 쓰는 것은 agent · command · discovery | discovery 는 2-2 에 사라졌고 지금은 데몬 lib 한 줄(`:338` — help 경로)이다 — ADR-0282 「영향」이 이미 적었다 |
| 메모 §8 「사실」 — 비테스트 2,791줄(`a226f63`) | 지금 5,508줄 중 시험 모듈 앞 2,805줄 |
| 2-3 착수 지시의 전제 — help 에 루트 · 에이전트 화면을 가르는 우편 가시성 필터가 있다 | 없다 — ADR-0220 결정 4(2026-09-23)가 걷었다(1-2). 옮길 필터가 없으므로 데몬은 호출자를 화면 선택에 쓰지 않는다 |
| ADR-0220 근거 — help 본문 온디맨드 8.7KB | 지금 19,891 바이트(구획이 늘었다 — `window` · `settings`) |
| ADR-0220 「영향」 — 고정 = `the_priming_pointer_names_a_help_entry_that_actually_renders` | 실제 이름 `the_priming_pointer_names_help_entries_that_actually_render`(`:4301`) |
| ADR-0273 결정 5 · ADR-0271 「영향」 — CLI 쪽에 설치 위치 규칙을 「한 벌 더」 | D10 뒤 CLI 는 설치 위치 규칙을 아예 쓰지 않는다 — 결정 5 는 대상이 없어진다(§7 2) |
| 메모 §8 「사실」 — 셸이 데몬을 dev 의존으로 끌어온다 `src-tauri/Cargo.toml:104` | 지금 `:118`(줄이 밀렸다 — 내용은 맞다) |
| `scripts/build-release.ps1:11` — 파일명 가드 = 「daemon tests/engram_cli.rs」 | 2-3 뒤 cli 패키지(U2 가 고친다) |
| `build-release.ps1:18-21` — engram.exe 가 본문을 같은 앵커로 읽고 · 빠져도 include_str! 사본으로 죽지 않는다 | U1 뒤 둘 다 거짓 — 데몬이 읽고 사본은 없다(U1 이 고친다) |
| CLAUDE.md `:231` — 데몬의 `--test-threads=4` 근거 = 「프로세스 레벨 CLI 스위트와 실 `.exe` spawn `#[ignore]` 분」 | 기본 실행의 실 PTY 셸 스폰도 있다(1-10 F4) — CLI 스위트가 나가도 데몬은 그 플래그를 그대로 진다 |

---

## 2. 결정 · 설계

| | 결정 | 한 줄 이유 |
|---|---|---|
| **D1** | 패키지 이름 `engram-dashboard-cli` · 폴더 `crates/engram-dashboard-cli` | 접두는 ADR-0273 결정 2(이름 접두로 멤버를 세는 게이트). `-cli` 는 역할 낱말이다. `-engram` 은 제품 이름과 읽힘이 겹친다(기각) |
| **D2** | bin 전용 패키지 · 파일은 `src/bin/engram.rs` 그대로 · `[[bin]]` 명시(앵커 `# ADR-0273`) | 옛 ADR 포인터의 뒤꼬리 · 시험 도우미 `help_repo_root` 의 깊이가 그대로다. `src/main.rs`(경로를 고친다) · lib + bin(lib 소비자 0 · ADR-0175 결정 6) 기각 |
| **D3** | 운영 의존 = agent · command · serde_json · dev 의존 없음 | 소스가 실제로 쓰는 것만(1-1) — 데몬 · base 직접 · protocol · net · platform 직접 0. uuid 는 U1 이 데몬으로 옮기는 시험 하나(`:4482`)만 썼다(F7) |
| **D4** | ★접힘★ — CLI 는 설치 위치 규칙을 갖지 않는다(D10 의 결과) | help 경로가 CLI 를 떠나면 그 규칙을 부를 일이 없다 · 데몬 · 셸 걸음 두 벌은 ADR-0282 결정 3 그대로(§8 O13) |
| **D5** | `engram_cli.rs` → 새 패키지 `tests/` · 로컬 명령에 `-- --test-threads=4` | `CARGO_BIN_EXE_engram` 이 같은 패키지 bin 만 본다 · 41건이 실 exe 를 띄운다(ADR-0273 「영향」의 판정) |
| **D6** | 빌드 입구 넷은 옮기는 커밋에서 고친다 · 릴리스 빌드는 두 호출을 각자의 `Invoke-Step` 으로 · 릴리스 스크립트 1회는 머지 전 필수 · qa 바인딩 편집 = **사용자 승인됨(2026-10-08)** | 런처 둘은 안 고치면 engram.exe 가 조용히 낡는다(1-8) · 한 블록 두 호출은 앞 실패를 덮는다(F1) · 그 스크립트는 CI 에서 태그 때만 돈다(F2) |
| **D7** | CI · CLAUDE.md · 바인딩 세 곳 등록 — 의존 상한(이름 집합) · test-support 고리 둘 · 게이트 ⑦ cli 줄 · 버전 게이트 편입 · 단독 기립 스텝 · 주석 · 이름 알파벳의 bin 전용 예외 | ADR-0273 「영향」 — 새 패키지는 게이트를 세 곳에 등록한다 |
| **D8** | `scripts/engram.mjs` 는 범위 밖 | T-23 · ADR-0132 의 스로어웨이 스파이크 — CLI 와 코드를 나누지 않는다 |
| **D9** | 새 ADR-0284(커밋 직전 번호 재확인) · 옛 포인터 읽기 규칙을 짧게 · 고쳐지는 옛 문장 목록(§7) | ADR-0273 의 읽기 규칙은 `engram.rs` 하나만 덮는다 · help 화면 시험은 cli 가 아니라 데몬으로 간다 |
| **D10** | ★**사용자 결정**★ — help 를 데몬이 낸다(라우트 · 로더 · 어휘가 데몬으로 · CLI 는 낱말을 실어 보내고 받은 화면을 찍는다) · 계약 상수는 agent `types.rs` · 원천은 조립부 주입(메인 결정 — 리뷰 1라운드) · 2-3 안에서 이사 전에(U1) | CLI 의 역할 = 인증 + 배송뿐(MCP 대용으로 생긴 입구다) · CLI 가 얇은 채로 데몬 패키지를 떠난다 |
| **D11** | ★**사용자 결정**★ — 데몬에 못 닿거나 자격증명이 없으면 데몬을 부르는 모든 명령이 한 길로 실패한다(help 포함 · exit 1) · help 전용 폴백 없음 · 내장 사본 없음. 메인 결정(리뷰 1라운드) — 출력은 지금 봉투(stdout JSON + exit code) 그대로 · 404 도 같은 길(`PROTOCOL_MISMATCH`) · `--body-stdin` 은 자격증명 뒤에 읽는다 | 그 상태에서는 다른 명령도 다 실패한다 · help 만 살아도 할 수 있는 것이 없다(단 codex 샌드박스 측정 조건 — §2-10) · 프라이밍이 「계속 실패하면 주인에게 알린다」를 이미 가르친다 |

### 2-1. D1 · D2 — 패키지 모양

- **매니페스트(제안):**

  ```toml
  [package]
  name = "engram-dashboard-cli"
  version = "0.3.2"        # 옮기는 순간의 데몬 `version` 값 그대로 — 버전 게이트가 잰다(D7 5)
  edition = "2021"
  description = "Engram Dashboard 제어 평면 CLI(engram.exe) — 스폰된 에이전트가 데몬 제어 라우트에 손조립 HTTP 로 붙는 인증 + 배송 클라이언트. bin 전용(ADR-0273 · ADR-0284)."

  # ADR-0273
  # (데몬 Cargo.toml:33-36 의 주석을 옮긴다 — bin 이름 = agent `CLI_EXE_NAME` · 바꾸면 우편이 조용히 멈춘다 · ADR-0094)
  [[bin]]
  name = "engram"
  path = "src/bin/engram.rs"

  [dependencies]
  engram-dashboard-agent = { path = "../engram-dashboard-agent" }
  engram-dashboard-command = { path = "../engram-dashboard-command" }
  serde_json = "1"
  # dev 의존 없음 — CLI 시험이 쓰던 uuid 는 U1 이 데몬으로 옮기는 시험 하나(`:4482`)뿐이었다(F7)
  ```

- **깊이가 그대로인 것:** `help_repo_root`(`:4339`) — `CARGO_MANIFEST_DIR` 두 단계 위 = 저장소 루트 그대로(`crates/engram-dashboard-cli`). U1 뒤 CLI 시험 셋이 이것으로 프라이밍 · 본문 파일 원문을 읽는다(§2-9 ⑦). `include_str!` 는 U1 이 걷어 U2 에는 없다.
- **자동 발견과 겹치지 않는다** — 명시 `[[bin]]` 이 `src/bin/engram.rs` 를 점유하면 같은 파일로 타깃이 되살아나지 않는다(`engram_cli.rs:598-603` 의 실측 주석 · 지금 데몬이 같은 꼴이다).
- **bin 전용이라 Doc-tests 줄이 없다**(cargo 는 lib 타깃에만 doctest 를 돈다 — 가능성 높음 · §9).
- **데몬 쪽:** `Cargo.toml:33-39`(주석 + `[[bin]]`) 를 지운다. 데몬의 다른 의존은 그대로다 — `serde_json` · `uuid` 는 데몬 자신이 쓴다. 루트 `Cargo.toml` 멤버에 한 줄 · 머리 주석 「멤버 10」 → 11.
- **거부한 대안(메인 결정):**
  - `crates/engram-dashboard-engram` — 제품 이름 「Engram Dashboard」와 읽힘이 겹쳐 「engram 패키지」가 무엇을 가리키는지 흐려진다.
  - `src/main.rs` 로 옮긴다 — 경로가 바뀌어 옛 ADR 포인터의 뒤꼬리가 깨진다(ADR-0273 「영향」은 그 경로가 새 패키지의 같은 파일을 가리킨다고 적었다).
  - lib + bin — lib 를 부를 소비자가 없다. 한 파일짜리 lib 는 ADR-0175 결정 6 에 걸린다.

### 2-2. D3 — 의존과 그래프 무게

- **운영 직접 의존:** agent · command · serde_json. ★dev 의존 없음(F7)★ — CLI 시험의 uuid 는 `:4482`(env override 시험의 임시 폴더 이름) 하나였고 그 시험은 U1 이 데몬으로 옮긴다(데몬은 uuid `v4` 를 운영 의존으로 이미 진다 — `Cargo.toml:134`). agent 는 U1 뒤에도 필요하다 — `:117-121` 의 상수 열둘을 전부 비시험 코드가 쓰고(파서 · 판정기 · 반려 문구) U1 이 help 계약 상수 셋을 더한다. command 는 `RequestId`(`:177`). base 는 agent 를 거쳐 그래프에 있을 뿐 직접 부르지 않는다.
- **빌드 그래프(실측 + 예상):** 데몬의 정상 그래프는 194 패키지다(실측 `cargo tree --locked -p engram-dashboard-daemon -e normal --prefix none --target all` 의 이름 · 판 중복 제거). CLI 는 agent · command 의 정상 그래프 합집합(agent 가 base 를 문다) + serde_json 계열 = 약 92(예상 — 구성요소를 각각 잰 합집합이다 · 새 패키지가 없어 직접 못 잰다). 빠지는 것 = tokio · axum · hyper · rmcp · tower · tokio-tungstenite · net · messaging · protocol 등. 남는 것은 ADR-0273 「영향」이 적은 그대로다(agent 의 portable-pty · windows · ts-rs · chrono · base 의 tracing-subscriber · regex · command 의 inventory).
- **async 런타임 0:** agent · command 각각의 `-e normal` 그래프에 `^(tokio|mio|tokio-tungstenite|futures-util) ` 가 0줄이다(실측) — CLI 그래프도 0 이어야 하고 D7 의 게이트 ⑦ 줄이 그것을 잰다.
- **Cargo.lock:** 새 `[[package]]` 는 멤버 자신 하나 · 서드파티 0(예상 — 쓰는 서드파티가 전부 이미 lock 에 있다).

### 2-3. D5 — `engram_cli.rs`

- 글자 그대로 `git mv` 한다(히스토리를 잇는다). import 는 agent · serde_json · std 뿐이라 새 패키지의 운영 의존으로 선다. U1 이 help 시험 8건을 고치고 둘을 더한 상태(43건)로 옮긴다(§2-9 ⑦).
- 파일명 가드(`:604-616`)는 그대로 값어치를 갖는다 — 상수만 개명하면 이 단언이, `[[bin]] name` 만 개명하면 컴파일이 잡는다(그 주석 `:598-603`). 그 가드가 든 타깃이 조용히 사라지지 않게 CI 스텝이 이름으로 집는다(D7 1).

### 2-4. D4 — 접혔다 (D10 의 결과)

- **무엇이 바뀌었나:** 3판의 D4 는 「CLI 가 help 본문을 찾으려고 쓰는 설치 위치 규칙을 어디 두나」였다(A′ base · A 걸음만 base · B CLI 사본). D10 이 help 본문 읽기를 데몬으로 옮기므로 **CLI 에는 설치 위치 규칙을 부를 자리가 없다.** CLI 는 base 경로 코드도, 데몬 `data_dir` 도, 같은 경로 시험도 갖지 않는다.
- **데몬 · 셸 쪽은 그대로다:** 데몬 `find_install_root`(프라이밍 · 이제 help 도 부른다)와 데몬 · 셸 걸음 두 벌은 ADR-0282 결정 3 이 정한 대로 둔다. 2-3 은 2-2 착지 코드(데몬 `data_dir.rs` · 셸 `layout.rs`)를 다시 열지 않는다.
- **넘긴 것:** 걸음 두 벌을 base 로 합치는 안(옛 A′)은 §8 O13 · 디버그 데이터 루트 시험의 빈틈(옛 C1)은 §8 O12 — 둘 다 2-3 단위가 아니다.
- **ADR 영향:** ADR-0273 결정 5(CLI 쪽 사본 + 같은 경로 시험)는 대상이 사라진다 · 결정 4(의존 = … · 설치 위치 규칙)에서 그 항목이 빠진다 · ADR-0282 「영향」의 임시 간선은 사본으로 바뀌는 것이 아니라 U1 에서 없어진다(§7 2).

### 2-5. D6 — 빌드 입구 · 바인딩

- **`build-release.ps1:133-135` → 두 호출, 각자의 `Invoke-Step`(F1 · 제안):**

  ```powershell
  Invoke-Step 'cargo build daemon (engram-dashboard-daemon)' {
      & cargo build --release --manifest-path (Join-Path $ProjectRoot 'Cargo.toml') -p engram-dashboard-daemon --bin engram-dashboard-daemon
  }
  Invoke-Step 'cargo build cli (engram-dashboard-cli)' {
      & cargo build --release --manifest-path (Join-Path $ProjectRoot 'Cargo.toml') -p engram-dashboard-cli --bin engram
  }
  ```

  ★한 블록에 둘을 넣지 않는다★ — `Invoke-Step`(`:61-66`)은 블록이 끝난 뒤 `$LASTEXITCODE` 를 한 번만 보므로 앞 호출의 실패를 뒤 호출의 성공이 덮고, 존재 검사(`:139-142`)는 이전 빌드의 낡은 exe 를 받아들인다. **두 호출인 이유:** 한 호출(`-p A -p B --bin x --bin y`)은 `--bin` 이 고른 두 패키지 모두에 걸리는 꼴이라 cargo 동작을 확인해야 하고(안 돌렸다 · §9), CLI 가 데몬의 기능 집합으로 지어진다. **비용(C3):** 두 호출이면 serde_json 이 기능 집합 둘로 따로 지어지고 그것을 쓰는 agent · command 도 CLI 호출에서 다시 지어진다 — 릴리스 프로필(`lto` · `codegen-units = 1`)이라 릴리스 시간이 는다(얼마인지는 미검 · §8 O10). 같은 편집: `:127-132` 주석(「데몬+CLI 는 … 별개 crate 의 두 [[bin]]」 → 「별개 패키지 둘」) · 헤더 `:11`(가드 시험 자리 — 전체 경로로). `--all-targets` 금지 주석은 그대로 둔다.
- **런처 둘:** `rebuild-run-debug.bat:93` → `cargo build -p engram-dashboard-daemon -p engram-dashboard-cli` · `rebuild-run-release.bat:111` → 같은 꼴 + `--release` · 두 파일의 echo 문구 · `rebuild-run-release.bat:5-8` 주석(「`tauri build` 가 데몬을 안 짓는다」에 CLI 를 더한다). ★두 파일은 CRLF 다(`.gitattributes` 의 `*.bat text eol=crlf`)★ — 편집 뒤 `file` 로 본다. `launcherWiring.test.ts` 는 그대로 초록이어야 한다(1-8).
- **`roundtrip_smoke.rs` 안내문(F3):** `:66`(doc) · `:424`(SETUP-SKIP 문자열)의 `cargo build -p engram-dashboard-daemon --features test-harness --bin engram` → `cargo build -p engram-dashboard-cli`(CLI 에는 `test-harness` 기능이 없다 · 같은 profile 이라 하네스 exe 옆에 놓인다). `:67` 의 `cargo run -p engram-dashboard-daemon --features test-harness --bin roundtrip-smoke` 는 그대로다.
- **★릴리스 스크립트 1회는 2-3 의 master 머지 전 필수(F2)★** — 이 스크립트와 버전 게이트는 CI 에서 태그 push 때만 돈다(`ci.yml:1281-1283`). 태그는 다시 쓸 수 없으므로(CLAUDE.md 「태그」) 거기서 처음 깨지면 그 태그가 버려진다. U2 의 QA 가 §5-3 을 돈다.
- **qa 바인딩 편집 — ★사용자 승인됨(2026-10-08)★**(추가 승인 없이 적용한다):
  - **U2 와 함께(빌드 명령):** `:320` · `:355` · `:360` 의 `cargo build -p engram-dashboard-daemon` → `cargo build -p engram-dashboard-daemon -p engram-dashboard-cli`(문구 「백엔드/데몬을 고쳤으면」 → 「백엔드 · 데몬 · CLI 를 고쳤으면」). U2 의 QA 전에 들어간다 — 안 고치면 U2 의 GUI 실측이 옛 engram.exe 를 잰다.
  - **`:66` 은 그대로** — 셸 실 데몬 레인(`stop_smoke` · `real_wmi`)은 CLI 를 쓰지 않는다(데몬은 CLI 없이도 뜬다 · fail-open).
  - **U3(게이트 · 서술):** `:138` `agent`·`daemon`·`platform` → + `cli`(실 exe 41건) · `:253` 메시징 정규식 「새 워크스페이스 crate가 생기면 여기에 더하고」 뒤에 bin 전용 예외(D7 8 의 문구) · `:254-261` 의존 상한 「4종」 → 「5종」 + cli 줄(기대 = 이름 집합 — D7 2) · `:262-275` test-support 두 블록에 `-p engram-dashboard-cli` 0줄 줄 하나씩 · `:79-84` 게이트 ⑦ 블록에 cli 0줄 줄 · `:442` `crates/engram-dashboard-daemon/src/bin/engram.rs` → `crates/engram-dashboard-cli/src/bin/engram.rs` · `:445` 「데몬 패키지의 `[[bin]] engram` 이라 데몬 exe 와 같은 target 폴더에」 → 「CLI 패키지(`engram-dashboard-cli`)의 `[[bin]] engram` — 워크스페이스가 target 폴더 하나를 나눠 써 데몬 exe 옆에」.

### 2-6. D7 — 게이트 등록

1. **CI 단독 기립 스텝**(backend 잡 · agent 스텝 뒤 · 제안): `cargo test --locked -p engram-dashboard-cli --bins --test engram_cli`. 값어치 둘 — ① `-p` 로 따로 지어 데몬과의 기능 합집합 없이 서는지(이 패키지의 존재 이유) ② 파일명 가드가 든 `engram_cli` 타깃이 사라지면 「no test target」로 죽는다(셸 `--test lib_unit` 과 같은 값어치 — ADR-0094 가드가 조용히 증발하지 않는다). CI 는 `--test-threads` 를 쓰지 않는다(CLAUDE.md 「빌드·검증 명령」).
2. **의존 상한**(gates 잡 · 가안 이름 `cli gate 1: direct workspace deps — exactly agent · cli · command`) — 꼴은 net 게이트 3(`cargo tree --locked -p … --depth 1 --prefix none -e normal,dev,build --target all --all-features`)을 따르되 **판정은 줄 수가 아니라 이름 집합 일치**다(첫 칸을 정렬해 heredoc 과 견준다). 수만 세면 command 자리에 데몬이 들어와도 셋이라 이 단계의 요점(데몬 0)을 못 본다(§8 O6). 기대 = agent · cli · command.
3. **test-support 운영 그래프 고리 둘**(`ci.yml:675` · `:767`) — `for pkg in engram-dashboard-daemon engram-dashboard engram-dashboard-cli` · 스텝 이름 「(daemon · shell · cli = 0, agent dev >= 1)」 · 주석 「운영 바이너리를 내는 둘」 → 셋. CLI 그래프에 base · platform 이 agent 를 거쳐 있으므로 `-i` 가 rc 101 로 죽지 않는다(그래프 논리 — U3 에서 실측).
4. **platform 게이트 ⑦** — `async_ingress zero -p engram-dashboard-cli` 한 줄(`ci.yml:1227` 다음). CLI 는 동기 crate 이고 데몬을 떠나는 이유가 tokio 를 끌고 오지 않는 것이라 그 사실을 기계로 잰다 — agent 줄이 이미 덮는 것(agent · base · command · platform)에 더해 CLI 자신의 직접 서드파티(지금 serde_json)를 덮는다. 같은 편집: 스텝 이름 · platform `src/lib.rs:166-180` 헤더의 「대상 셋」 → 넷 · qa `:79-84` · CLAUDE.md `:222`(§8 O7).
5. **버전 게이트(F7)** — `check crates/engram-dashboard-cli/Cargo.toml …` 한 줄(`:1411` 다음) · 주석 「5건」 → 「6건」(`:1355`) · `:1420` 의 `PASS — 제품 버전 선언 5건` → 6건 · 셋째 「daemon.exe·engram.exe 를 만드는 crate」(`:1362`) → 「daemon.exe 를 만드는 crate」 + 새 항목 「cli — engram.exe 를 만드는 패키지. 코드는 지금 버전을 읽지 않지만 단독 배송물이 있어 「일부러 뺀 것」의 라이브러리 기준(`:1373-1375`)에 들지 않는다 — 나중에 `engram --version` 같은 것이 생기면 거짓 버전을 막는다」. 대가 = 릴리스마다 고칠 매니페스트가 하나 늘고, 빠뜨리면 ★태그를 push 한 뒤에야★ 드러나 그 태그를 버린다(§8 O4). 그래서 U3 수용 기준이 그 함수를 로컬에서 한 번 돌린다.
6. **주석** — `ci.yml:32-34` 는 U1 이 고친다(§6 1 — `include_str!` 가 사라진다). U3 은 그 결과 포인터를 다시 본다.
7. **CLAUDE.md** — 「빌드·검증 명령」에 `cargo test -p engram-dashboard-cli -- --test-threads=4`(근거 = 「병렬은 테스트 바이너리마다 걸린다」 · 실 exe 41건) · 의존 상한 줄 · 시험 기능 게이트 두 줄(`:259` · `:261`)의 「같은 꼴 `-p engram-dashboard`(셸)」 옆에 cli · `:231` 병렬 항목의 괄호(F4) — 「프로세스 레벨 CLI 스위트」는 cli 몫으로 옮겨 적고, 데몬의 근거는 「기본 실행의 실 PTY 셸 스폰 + `#[ignore]` 실 데몬 exe 시험」으로 고친다(1-10) · `:222`(⑦ 대상) · `:207`(`inventory` — 「데몬·셸 릴리즈 바이너리」 → + CLI) · `:255` 메시징 정규식 줄에 bin 전용 예외(아래 8) · 「백엔드 모듈 맵」 cli 항목(가안):
   > **cli** — 제어 평면 CLI `engram.exe` 를 뽑는 bin 전용 패키지(ADR-0273 · ADR-0284). 스폰된 에이전트가 데몬 제어 라우트에 손조립 HTTP 로 붙는다. ★역할은 인증 + 배송뿐이다★ — 화면(help)도 데몬이 내고(`/control/help`), 데몬에 못 닿거나 · 자격증명이 없거나 · 데몬이 라우트를 모르면 모든 명령이 한 길로 실패한다. 의존 = agent(명령 어휘 · help 계약 상수) · command(요청 번호) · serde_json — 데몬 · tokio 없음(게이트 = cli 의존 상한 · platform 게이트 ⑦ 의 cli 줄). bin 이름 = agent `CLI_EXE_NAME`(바꾸면 우편이 조용히 멈춘다 · ADR-0094).
8. **이름 알파벳 규칙의 bin 전용 예외(F8)** — 메시징 정규식(`ci.yml:520-523` 주석 · qa `:253` · CLAUDE.md `:255`)은 「새 워크스페이스 crate 가 생기면 이름을 더한다」고 적는다. cli 는 **더하지 않는다** — lib 타깃이 없어 아무도 `engram_dashboard_cli::…` 로 부를 수 없고, 그 정규식이 막는 것(메시징이 다른 crate 를 부르는 것)이 성립할 자리가 없다. 문구 제안(세 곳 같게): 「단 lib 타깃이 없는 bin 전용 멤버(`engram-dashboard-cli`)는 더하지 않는다 — 부를 수 있는 crate 가 아니다(ADR-0284). 그런 멤버에 lib 를 세우는 날 여기 이름을 더한다.」 net 게이트 1 · 2b 의 이름 목록은 「모든 crate」 규칙이 아니라 고칠 것이 없다.
- **고치지 않는 게이트:** platform 게이트 ④ · ⑥ 명단(1-10) · ⑤(저절로) · 셸 게이트 1(셸 그래프 불변) · base 게이트 ①②③(base 무변경) · net · transport 게이트.

### 2-7. D8 — `scripts/engram.mjs`

머리(`:2`)가 스스로 「THROWAWAY 스파이크 … 롤백 예정」이라 적는 Node 스크립트다. 데몬 WS 로 붙고 `daemon.json` 후보를 자기 코드로 찾는다(`:13-`) — engram CLI(HTTP 제어 라우트)와 코드도 경로 규칙도 나누지 않는다. 그 자리는 T-23 · ADR-0132 몫이라 2-3 이 건드릴 줄이 없다.

### 2-8. D9 — 새 ADR

§7. ADR-0273 「영향」의 읽기 규칙(「그 경로는 새 패키지의 같은 파일을 가리킨다」)은 `src/bin/engram.rs` 하나만 덮는다. `engram_cli.rs`(ADR-0220 · ADR-0232 가 부른다) · 데몬 `[[bin]] engram` 을 짧게 더하고, ★help 화면 시험은 cli 가 아니라 데몬 `control/help.rs` 로 갔다★는 것(ADR-0220 「영향」이 `bin/engram.rs` 로 적은 두 고정)을 따로 적는다 — 「같은 파일을 가리킨다」 규칙만 따르면 그 둘을 cli 에서 찾게 된다.

### 2-9. D10 — help 를 데몬이 낸다 (★사용자 결정 2026-10-08★)

**① 계약 — 경로와 키 이름은 agent `types.rs` 의 상수 한 벌(F3 · 이름 가안):** CLI 와 데몬이 둘 다 agent 를 의존하므로 `CLI_GROUP_MAIL`(`types.rs:501`) · `CLI_GROUP_AGENT`(`:576`) 옆에 `CLI_HELP_ROUTE = "/control/help"` · `CLI_HELP_TOPIC_KEY = "topic"` · `CLI_HELP_SCREEN_KEY = "screen"` 을 둔다. 데몬 `CONTROL_HELP_PATH` 와 CLI 의 요청 조립 · 응답 읽기가 그 상수를 쓴다 — 한쪽만 고친 편집은 컴파일 단계에서 두 쪽이 함께 움직이거나, 리터럴을 다시 적으면 아래 스텁 시험이 잡는다. ★기존 다섯 라우트는 그대로 손으로 맞춘다★(`engram.rs:158` 「경로 지식은 CLI 소유, 데몬측 상수와 손으로 맞춘다」 · `mcp_server.rs:51-89`) — 이 단계는 새 라우트 하나만 공유 상수로 묶는다(다섯을 옮기는 것은 §8 O15).
- **요청:** 무-세션 POST(형제 규율 — 무-세션 GET 은 미들웨어가 400). 바디 = `{"topic": null}`(목차) 또는 `{"topic": "<낱말>"}`. ★`topic` 키는 필수다★ — 빈 바디를 목차로 읽으면 CLI 가 키 이름을 틀려도 목차가 나와 어긋남이 안 보인다. ★모르는 키는 거절한다★ — 철자가 틀린 키가 조용히 무시되지 않게. 바디가 JSON 객체가 아니거나 · `topic` 이 없거나 · 문자열도 null 도 아니거나 · 모르는 키가 있으면 `INVALID_ARGUMENT`(catalog 의 「바디가 계약을 못 지킨 경우」와 같은 꼴).
- **응답:** 항상 200 + JSON — 성공 `{"screen": "<렌더된 화면>"}` · 반려 `{"status":"error","code","hint"}`(`ControlQueryResult` · `ingress.rs:440-455`).
- **라우트:** `ControlRoute::Help`(`ALL` 5 → 6) · `is_mail` = **false**(제어 평면 — 발견과 같은 줄. 우편 막힌 자격증명도 우편 화면을 읽어야 한다 · ADR-0220 결정 4) · 라우터 명단 순회에 한 arm · 핸들러 `control_help_handler`(가안 — `control_commands_handler` `:1070-1092` 꼴: `Option<Extension<BoundIdentity>>` 없으면 401 · 파일 읽기는 `spawn_blocking` 안) · 명단 시험 `:1394` 의 `5` → `6`(우편 수 2 그대로). 인증 = 형제와 같은 `bearer_auth` — 이 라우트를 위한 인증 코드는 0줄이다. 명단에 들어가면 `is_mail` exhaustive match 가 분류를 컴파일 단계에서 강제한다(ADR-0133).

**② 데몬 모듈 `control/help.rs`(가안 · 프라이밍 옆) — CLI 에서 옮겨 오는 것:** 어휘(`HelpTopic` 다섯 · `SECTION_ROOT` · `HELP_TOPIC_WINDOW/SETTINGS` · 별칭 `theme` → settings · 낱말 → 화면 · mail · agent 낱말은 agent `CLI_GROUP_MAIL/AGENT` 그대로) · 구획 파싱(`HelpText::parse` · `section_marker` · 줄 전체 규칙 · 첫 구획 앞 버림 · ★CRLF 접기★) · 필수 구획 검사(`required_section_ids` · 반쪽 로드 금지) · 렌더(구획 하나 + `{tool}` → `CLI_EXE_NAME` — 데몬은 agent 를 의존하므로 같은 상수다) · 모르는 낱말 반려 문구(`unknown help topic: … — run \`engram help\` to list groups, or \`engram help mail\`` 그대로 · 호출자 글은 catalog 의 `preview` 처럼 다듬어 싣는다). **옮기지 않는 것:** `include_str!` 사본 · `warn_help_fallback` · `OnceLock` 캐시.
- **원천 seam(가안 `HelpSource`):** base 폴더를 받아 짓는다(`HelpSource::new(base)`). 경로 규칙은 프라이밍과 같은 모양 — 고정 상대경로 `prompts/engram-help.md` · `ENGRAM_HELP_FILE`(이제 **데몬 프로세스의 env**)이 비어 있지 않으면 먼저 · override 실패는 고정 경로로 안 내려간다 · 절대경로만. 운영 생성자 `HelpSource::from_install_root()` = `data_dir::find_install_root()` 를 base 로(못 얻으면 경로 없음 → 반려).
- **★주입은 조립부에서(C2 · F10)★** — 서버가 안에서 설치 위치로 원천을 짓지 않는다. 운영 = `lib.rs` 가 프라이밍(`lib.rs:630-631`)과 같은 자리에서 `HelpSource::from_install_root()` 를 지어 넘긴다. 시험 = 알려진 본문을 담은 원천을 넘긴다 — 시험 exe 의 자리 · `CARGO_TARGET_DIR` 와 무관해진다.
  - **넘기는 길(최소 변경):** 기존 `start_mcp_server`(다섯 인자 · `mcp_server.rs:1242`)는 서명 그대로 두고, 원천을 하나 더 받는 주입판(가안 `start_mcp_server_with_help`)을 더한다. 다섯 인자판은 「구성되지 않음」 원천을 넘기는 한 줄 래퍼다 — 그 원천은 모든 요청에 `INTERNAL`(`help source not configured on this server`)로 답하고 기동 때 warn 한 줄을 남긴다(조용히 비지 않는다). **바뀌는 호출 자리 = 2**(`lib.rs:618` 운영 조립 · `tests/mail_gate.rs:179` 픽스처) · **그대로 23**(시험 · 하네스 bin 셋 — help 를 부르지 않는다 · `git grep -n "start_mcp_server("` 25 중).
  - **택하지 않은 안:** 인자를 늘린다 — 호출 자리 25 전부를 기계적으로 고친다. 같은 효과에 변경이 열두 배다. 대가(택한 안의) = 운영 조립이 실수로 다섯 인자판을 부르면 help 가 늘 `INTERNAL` 이다 — 그 자리는 `lib.rs` 하나이고 기동 warn · §3-5 바이트 대조가 잡는다.
- **기동 때 해석 경로를 한 번 남긴다(F6):** 원천을 지을 때 본문 파일을 한 번 읽어 본다 — 읽히고 필수 구획이 다 있으면 `info!`(경로 — 정상 수명주기 · `docs/reference/logging-conventions.md:24`), 못 읽거나 구획이 빠지거나 경로를 못 지으면 `warn!`(경로 · 사유 — 비정상이나 데이터 위험 없음 · 같은 문서 `:23`). 기본 레벨이 warn 이라 정상이면 아무것도 안 남고, 잘못이면 `RUST_LOG` 없이도 남는다(릴리스 데몬에는 `RUST_LOG` 가 닿지 않는다 — 같은 문서 `:11`). 이것은 진단이지 판정이 아니다 — 요청은 그때마다 다시 읽는다.
- **★요청마다 읽는다(캐시하지 않는다)★** — 지금은 CLI 프로세스가 호출마다 새로 떠서 파일을 고치면 다음 호출이 바로 바뀐다(ADR-0212 의 목적 · 본문 머리글 `:3`). 데몬은 오래 살므로 기동 때 한 번 읽으면 데몬을 다시 띄워야 바뀐다 — 그 퇴행을 막는다. 비용 = 호출마다 약 20KB 읽기 하나.
- **실패 = 반려, 사본 없음(D11 의 데몬 쪽):** 파일을 못 읽거나 필수 구획이 빠지면 반쪽 화면도 낡은 사본도 내지 않고 `{"status":"error","code":"INTERNAL","hint":"help text unavailable — <사유>; report this to the owner"}`(가안) + 데몬 로그 `warn!`. 모르는 낱말 = `INVALID_ARGUMENT`(가안 — 데몬 어휘 `ErrorCode` · 지금 CLI 의 `BAD_ARGS` 는 CLI 고유 코드라 데몬이 내지 않는다). ★모르는 낱말 판정은 파일을 읽기 전이다★ — 어휘는 코드다.

**③ 호출자는 화면을 고르지 않는다** — 데몬은 호출자가 누구인지(어느 에이전트 · 우편 가부) 알지만(1-4) help 는 그것을 쓰지 않는다. 걷힌 필터(ADR-0220 결정 4)를 이 라우트로 되살리지 않는다: 우편 막힌 자격증명과 열린 자격증명이 같은 화면을 받는다(시험 — ⑦). 그래서 CLI 가 실어 보내는 것은 「어느 화면」 낱말 하나뿐이다 — 그것은 배송할 내용이지 CLI 의 판단이 아니다.

**④ CLI `help` = 배송:**
- **CLI 에 남는 것(형태):** help 토큰 자리 규칙(`is_help_token` · 키워드 자리에서만 · 값 자리에선 값) · 단독 호출 규칙(`help <낱말> <더>` · `<계열> --help <더>` → 로컬 `BAD_ARGS` · 네트워크 0) · `<계열> --help` → 그 계열 낱말(`mail` · `agent` — 파서가 이미 아는 계열) · 인자 없음 = 목차. 헤더의 규율 그대로다 — 「CLI 는 형태만 본다 · 의미 검증은 데몬 단독」(`:103-105`).
- **★help 토큰은 화면 낱말이 아니다(F1 · 형태 규칙)★:** `help` 다음 한 자리에 help 토큰(`help` · `--help` · `-h`)이나 `-` 로 시작하는 낱말이 오면 로컬 `BAD_ARGS` · 네트워크 0 — `engram help --help` · `engram --help help` · `engram help -h` · `engram help --to`. 지금은 낱말 표가 이것을 「모르는 낱말」로 걸렀다(`parse_help_topic` `:896-908`). 낱말 검사가 데몬으로 가면 이 규칙이 없을 때 `--help` 가 화면 낱말로 왕복한다. `-` 규칙은 `parse_catalog`(`:806` — 명령 이름은 대시로 시작하지 않는다)와 같은 판단이다.
- **바뀌는 것:** `ParsedCommand::Help(HelpTopic)` → `Help(Option<String>)`(가안 — 낱말을 검사하지 않고 싣는다) · help 갈래가 `run` 의 자격증명 검사 **뒤**로(`Plan::Help` 가안) · 요청 = `POST <CLI_HELP_ROUTE>` · 바디 = `{"topic":null}` / `{"topic":"<낱말>"}`(agent 상수 키 · 순수 함수 가안 `help_request_body`) · 응답 판정(가안 `exit_code_for_help_response` — 순수 함수):
  - 2xx · 봉투에 문자열 `screen` → `println!("{screen}")` · **exit 0**(지금과 같은 `println!` — 바이트 대조의 근거).
  - 2xx · 검증된 반려 봉투(`is_validated_error_shape`) 또는 404 아닌 비-2xx → 받은 body 를 그대로 · **exit 1**(다른 명령과 같다).
  - 2xx 인데 둘 다 아님 → body + stderr 사유 한 줄 · **exit 2**(3분법 `:79-86`).
  - 전송 실패 · 404 · 자격증명 없음 → D11.
- **반려 문구 하나가 짧아진다:** `reject_help_with_extra_args`(`:881-894`)는 지금 화면 낱말 목록(`HelpTopic::ALL`)을 싣는다. 어휘가 데몬으로 가므로 CLI 문구는 목록 대신 「`engram help` 를 단독으로 쳐 목록을 본다」로 가리킨다 — CLI 에 낱말 목록 사본을 남기지 않는다(남기면 데몬 어휘와 두 벌이 된다).
- **CLI 에서 사라지는 것:** `:186-391` 의 help 구획 전부(`HELP_TOOL_SLOT` · `REL_HELP_FILE` · `ENV_HELP_FILE` · `SECTION_OPEN` · `HELP_EMBEDDED` · `SECTION_ROOT` · `HELP_TOPIC_*` · 별칭 · `HelpText` · `section_marker` · `required_section_ids` · `help_file_path` · `warn_help_fallback` · `load_help_text` · `help_text` · `render_help*`) · `HelpTopic`(`:549-605`) · `parse_help_topic` 의 낱말 검사. **결과: CLI → 데몬 lib 간선(`:338`)이 없어진다** — U1 수용 기준 `rg engram_dashboard_daemon crates/engram-dashboard-daemon/src/bin/engram.rs` → 0.

**⑤ CLI 가 지금 데몬에 보내는 것과의 관계:** 이 요청도 다른 요청과 같은 손조립 POST · 같은 Bearer · 같은 침묵 한도(`TIMEOUT`)다. 새 의존 · 새 HTTP 코드 0.

**⑥ 배포 · 게이트:** `prompts/engram-help.md` 는 그대로 배포된다(`build-release.ps1:50` · tripwire 그대로). ★사본이 없으므로 이 파일이 빠진 배포판은 help 가 반려로 답한다★ — 지금은 조용했고(사본) 그 침묵을 tripwire 가 잡았다. 이제는 요란하고, tripwire 는 그 요란함이 배포 전에 나게 하는 장치로 남는다. `ci.yml:32-34` 경로 필터 주석의 근거가 「`include_str!` 로 구워진다」에서 「데몬이 실행 중에 읽고 배포판에 실린다 · 데몬 시험이 그 파일을 읽는다」로 바뀐다(필터 목록은 그대로 — §7 2 ADR-0232).
- **★`CARGO_TARGET_DIR` 가 체크아웃 밖인 개발 빌드(F5 ①)★** — 데몬의 설치 위치가 걸음 실패로 exe 폴더(그 target 폴더)로 떨어지고 거기 `prompts/` 가 없다 → help 가 전부 `INTERNAL` 이다. 지금은 CLI 가 내장 사본으로 그 경우를 덮었다(stderr 한 줄). 프라이밍은 같은 조건에서 이미 조용히 꺼진다(`priming.rs:314-319` warn — 1-4). 기동 warn(②)이 그 상태를 로그에 남긴다. 저장소 스크립트가 그 변수를 다루는 자리 = `scripts/build-client-shell.mjs:48` · `build-release.ps1:79`(1-7) — 릴리스 조립은 exe 를 `prompts/` 와 한 폴더로 모으므로 영향이 없다.

**⑦ 시험 이사(예상 수 — §3-6):**
- **데몬 `control/help.rs` 로 옮긴다(10):** `the_help_file_still_satisfies_the_pre_rename_binary_and_this_one_ignores_the_extra` · `the_mail_screen_teaches_the_reply_contract`(ADR-0220 고정) · `the_embedded_copy_carries_every_section_the_screens_need` · `every_screen_still_renders_from_the_embedded_copy_alone`(이 둘은 「배포되는 파일(저장소 `prompts/engram-help.md` — `repo_root()`)이 필수 구획을 다 싣고 모든 화면이 그것만으로 선다」로 — ADR-0212 「잃은 보증」의 새 집 · §7 2) · `the_fixed_help_path_resolves_absolute_to_a_real_file`(base = 저장소 루트로 준 `HelpSource` 가 실재하는 절대경로를 낸다) · `a_help_file_missing_one_section_is_rejected_whole`(「사본으로 간다」 → 「반려 `INTERNAL`」) · `a_screen_never_renders_empty_even_with_no_sections_at_all` · `a_section_marker_must_be_a_whole_line` · `section_bodies_keep_their_whitespace_but_fold_crlf` · `the_env_override_wins_and_its_failure_falls_back_to_the_embedded_copy`(「사본으로 간다」 → 「반려이고 고정 경로로도 안 간다」 + ★파일을 바꾼 뒤 같은 원천의 다음 렌더가 바뀐다★ — 요청마다 읽는다를 박는 단언 · 임시 폴더 이름에 uuid — 데몬은 uuid `v4` 를 운영 의존으로 이미 진다(`Cargo.toml:134`)). env 를 만지는 시험은 데몬 쪽 env 잠금 아래 둔다(프라이밍 시험과 같은 규율 — `priming.rs:367-`).
- **지운다(1):** `the_runtime_path_and_the_embedded_copy_name_the_same_file` — 대조할 사본이 없다.
- **갈라진다(4 → CLI 반은 그 자리에 남고 데몬 반이 새로 선다 · F2):**
  - `help_lists_groups_and_each_group_documents_its_verbs`(`:4060`) — CLI = 파싱 · 형태 / 데몬 = 화면이 `CLI_AGENT_VERBS` · `CLI_AGENT_FLAGS` 를 싣고 없는 동사를 안 싣는다(가안 `the_agent_screen_documents_every_verb_and_flag`).
  - `the_priming_pointer_names_help_entries_that_actually_render`(`:4301`) — 데몬 = 프라이밍의 `engram help <낱말>` 줄마다 그 낱말이 화면으로 렌더된다(ADR-0220 고정의 새 집) / CLI = 그 줄들이 `Help(Some(낱말))` 로 파싱된다.
  - `each_group_word_renders_its_own_screen`(`:4169`) — CLI = `help <낱말>` 넷이 `Help(Some(낱말))` 로 파싱된다 / 데몬 = 네 화면이 서로 다르고 목차와 다르며 머리가 자기 계열이다.
  - `the_old_theme_word_reaches_the_settings_screen`(`:4201`) — CLI = `help theme` · `--help theme` 이 `Help(Some("theme"))` · ★`help theme extra` 는 로컬 `BAD_ARGS`(단독 호출 규칙 — 그대로 남긴다)★ / 데몬 = 별칭이 settings 화면을 내고 계열 낱말과 겹치지 않는다.
- **CLI 에 남되 고친다(4):** `agent_exit_code_requires_every_list_field_the_help_screen_promises`(`:3947`) · `the_static_help_points_at_the_catalog_verb_by_its_real_spelling`(`:5004`) — CLI 쪽 사실(판정기의 필드 · `CLI_VERB_COMMANDS`)과 화면을 한 시험이 묶는 값어치를 지키려고 렌더 대신 **본문 파일 원문**(`help_repo_root` · `## agent` / `## root` 구획 줄부터 다음 `## ` 앞까지)과 견준다 · `unknown_group_and_unknown_verb_point_at_help`(`:4052` — `help wat` 은 이제 파싱이 성공한다 · 그 단언은 데몬의 「모르는 낱말 = `INVALID_ARGUMENT`」로) · `a_help_token_is_not_a_help_topic`(`:4037-4049` — F1 규칙: 한 칸짜리 `help --help` · `help -h` · `--help help` · `help --to` 가 오류 · 지금 목록에 한 칸짜리를 더한다).
- **새로(예상):** CLI 단위 +1(`exit_code_for_help_response` 네 갈래 + `help_request_body` 의 글자 — `{"topic":null}` · `{"topic":"mail"}`) · 데몬 단위 +2(요청 바디 해석 — null · 낱말 · 키 없음 · 모르는 키 · 비문자열 · 모르는 낱말 / ★릴리스 모양 임시 폴더(F6)★ — `<tmp>\prompts\engram-help.md` 하나만 둔 폴더를 base 로 준 원천이 화면을 내고, `prompts\` 가 없는 폴더면 `INTERNAL` 이다) · 데몬 통합 +1(`tests/mail_gate.rs` — 픽스처가 알려진 다섯 구획 본문(임시 폴더)을 담은 원천을 주입하고, 우편 막힌 `mcp_token` 과 열린 `cli_token` 이 `/control/help` 에서 각각 ★그 본문의 기대 화면★을 받는다 · 우편 거절이 아니다 · 시험 exe 자리와 무관 — C2) · CLI 프로세스 +2(C1 열린 stdin · F5 404 — §2-10).
- **`engram_cli.rs` 고침(8 + F1):** 1-5 의 여덟. 「자격증명 없이 help 성공」 성질 pin(`:678`)은 **뒤집혀** D11 pin 이 된다 — 자격증명 없이 help 가 `mail pending` 과 같은 봉투(`NO_TOKEN`) · 같은 exit 1 로 끝나고, 닿지 않는 URL 이면 `agent list` 와 같은 `CONNECT_FAILED` 봉투로 끝난다. 나머지는 캡처 스텁(`spawn_capturing_stub` · `spawn_scripted_stub`)으로 — ★요청 바디를 글자로 단언한다(F3)★(`help` · 인자 없음 → `{"topic":null}` · `help mail` = `mail --help` = `mail -h` → `{"topic":"mail"}`) · 요청 줄이 `POST /control/help` 다 · 스텁의 `screen` 이 그대로(+ 줄바꿈) 찍히고 exit 0 · `help mail send` 는 네트워크 0 의 `BAD_ARGS` · hint 가 제안한 help 명령은 스텁 앞에서 exit 0 · help 는 `/control/commands` 를 안 부른다(`:1633` 의 새 뜻) · 우편 화면 요청은 env 와 무관하게 같은 바디다(`:1712` 의 새 뜻 — 화면이 같다는 것은 데몬 통합 시험이 잰다). `:765` 의 「네트워크 0」 목록에서 `help wat` 을 뺀다(이제 왕복한다 — 스텁 시험이 덮는다) · `help --help` · `--help help`(`:782-784`)는 F1 규칙으로 그 목록에 남는다(로컬 `BAD_ARGS`) · `help mail inbox` · `help --to bob` 은 단독 호출 규칙으로 남는다. `conventional_help_spellings_render_the_same_screens`(`:724`)의 `help inbox` → exit 1 은 스텁의 `INVALID_ARGUMENT` 봉투를 그대로 찍고 exit 1 로.

### 2-10. D11 — 데몬에 못 닿거나 자격증명이 없으면 모든 명령이 한 길로 실패한다 (★사용자 결정 2026-10-08★)

- **범위:** 파싱 뒤 데몬을 부르는 모든 명령 — `help`(D10 뒤) · `commands` · `<전체.이름> …` · `agent …` · `mail …`. 인자 형태 오류(`BAD_ARGS`)는 데몬이 필요 없으므로 자격증명 앞에서 끝난다 — `help mail send` · `help --help` 도 그렇다.
- **실패 셋(전부 exit 1 · 같은 출력 함수):**
  1. **자격증명 없음** — `NO_TOKEN` · `NO_CONTROL_URL`. 이미 한 자리다(`run` `:428-434` → `read_credentials` `:457-472`) — help 갈래를 그 뒤로 옮기는 것뿐이다.
  2. **데몬에 못 닿음** — `CONNECT_FAILED`(연결 · 쓰기 · 읽기 · 침묵 한도 · URL) · `INCOMPLETE_RESPONSE`. 같은 꼴이 세 자리에 손으로 적혀 있다(`:493-496` · `:1759-1763` · `:1897-1900`) — 한 함수(가안 `fail_unreached(&SendError) -> i32` — `print_error(e.code(), &e.to_string())` + `EXIT_FAILED`)로 모으고 help 도 그것을 부른다. 다섯째 호출자를 손으로 적지 않게 하는 것이 요점이다.
  3. **★데몬이 그 라우트를 모른다 — 404(F5)★** — 코드 `PROTOCOL_MISMATCH`(가안) · hint 「the daemon does not serve <route> — it is likely a different build than this engram (version mismatch); report this to the owner, retrying will not change it」. 자리 = `post_json` 이 404 를 `SendError` 의 새 변형(가안 `NoRoute { route }`)으로 돌려준다 — 그러면 위 공통 함수가 같은 자리에서 찍고, 비-2xx 를 따로 보는 갈래(`fetch_catalog` · 판정기들)는 404 를 만나지 않는다.
     - **404 가 이 뜻인 근거(1-4):** 데몬이 404 를 내는 자리는 라우터 fallback(명단 밖 경로 — 우편 열린 자격증명만)과 `bearer_auth` 의 세션 고아(`mcp_server.rs:759` — `Mcp-Session-Id` 를 실은 요청만 · CLI 는 안 싣는다) 둘뿐이고 둘 다 본문이 비어 있다(`not_found()` `:815-820` · axum 기본 fallback). 지금 CLI 는 그 빈 body 를 빈 줄로 찍고 exit 1 이라 호출자가 아무것도 못 배운다 — 봉투로 바꿔도 잃는 것이 없다.
     - **코드 이름:** command `ErrorCode::ProtocolMismatch`(`crates/engram-dashboard-command/src/error.rs:80` — 「전송·라우팅 계층 + 공통 오류 코드」 어휘 · retry Never)를 빌린다. 이유 = CLI 가 `UNKNOWN_COMMAND` 를 데몬 어휘에서 빌린 것과 같은 규율(같은 사실 = 같은 코드 · `engram.rs:180-184`) — 두 쪽이 서로 다른 판을 말한다는 사실의 낱말이 이미 있다. `UNKNOWN_COMMAND` 는 택하지 않았다 — 「표에 그 이름이 없다(오타)」와 「데몬 빌드가 이 입구를 모른다」가 같은 코드면 호출자가 멀쩡한 이름을 고치기 시작한다. 새 CLI 고유 코드(`ROUTE_NOT_FOUND` 등)도 택하지 않았다 — 어휘에 같은 뜻의 낱말이 있다. 지금 그 코드를 내는 생산자는 없다(`git grep ProtocolMismatch` → `macros.rs:336` 의 매크로 표뿐).
- **★`mail send --body-stdin` 은 stdin 을 자격증명 뒤에 읽는다(C1)★** — 지금은 `run` 이 stdin 을 먼저 다 읽고(`:419` `materialize_body(m, read_stdin_to_string)` · `read_stdin_to_string` `:1320-1326` = `read_to_end`) 자격증명을 본다(`:428-434`). stdin 이 열린 채 닫히지 않으면(에이전트 셸 · 파이프) 자격증명 없는 호출이 실패하지 못하고 멈춘다 — 공통 길에 닿지도 못한다. **새 순서 = 파싱(형태) → 자격증명 → stdin 읽기(본문 형태 — 빈 stdin 등) → 네트워크.** 「본문 오류는 네트워크 전에 끝난다」(`:418`)는 그대로다. ★오류 우선순위가 바뀐다★ — 자격증명이 없고 stdin 이 비었으면 지금은 `BAD_ARGS`(빈 stdin)이고 바뀐 뒤에는 `NO_TOKEN` 이다(둘 다 exit 1). 그 조합을 재는 기존 시험은 없다(stdin 시험은 모두 자격증명을 준다 — `engram_cli.rs:322-` · `:349-` · `materialize_body` 단위 시험은 순수 함수). 시험 +1(`engram_cli.rs` — 자격증명 없이 `mail send --to bob --body-stdin` 을 stdin 을 열어 둔 채(쓰지도 닫지도 않고) 띄워 시한(가안 5초 · `try_wait` 로 본다) 안에 `NO_TOKEN` · exit 1 로 끝나는지 · 넘기면 죽이고 실패).
- **명시적으로 남는 다른 점 하나:** 호출의 첫 왕복(목록 조회)이 실패하면 공통 실패 뒤에 `note_catalog_failure`(`:1809-1816`)가 stderr 에 「그 명령은 불리지 않았다」 한 줄을 더한다 — 공통 길을 바꾸는 것이 아니라 그 위에 얹는 진단이라 그대로 둔다(404 에도 같이 붙는다).
- **★알려진 한계 — 옛 데몬 + 새 CLI 에서 우편 막힌 자격증명(F5 ②)★:** 옛 데몬은 `/control/help` 를 명단 밖 제어 경로로 보고 우편 게이트에 접는다(`mail_gated_path` `:178-189` · 시험 `tests/mail_gate.rs:400-427`). 우편 막힌 자격증명(오늘의 MCP 갈래 전부)은 404 가 아니라 200 + `MAIL_NOT_ALLOWED` 봉투(「This credential is not allowed to use mail」 — `mail_not_allowed` `:830-838`)를 받고, CLI 는 그것을 데몬의 반려로 찍고 exit 1 이다 — 사유가 틀린 말로 보인다. 우편 열린 자격증명만 404 → 위 3 의 `PROTOCOL_MISMATCH` 다. CLI 가 「help 에서 온 `MAIL_NOT_ALLOWED` 는 판 어긋남」으로 바꿔 읽는 안은 택하지 않았다 — 데몬 반려의 뜻을 CLI 가 고쳐 읽는 것이고(의미 판정은 데몬 단독 — `engram.rs:103-105`), 그 창은 개발 중에만 선다: 배포판은 데몬 · CLI 가 한 폴더 · 한 빌드다(ADR-0100). 서는 조건 = 오래 떠 있는 데몬 옆의 engram.exe 만 다시 지었을 때(실행 중인 데몬 exe 는 잠겨 못 덮는다). U1 은 같은 패키지라 `cargo build -p engram-dashboard-daemon` 이 둘을 함께 짓고, U2 뒤에는 런처 · qa 빌드 명령이 둘을 함께 짓는다(D6).
- **help 전용 폴백 없음 · 내장 사본 없음:** 데몬이 꺼져 있으면 help 도 다른 명령처럼 실패한다. 근거 = 사용자 결정 — 그 상태에서는 다른 모든 명령이 실패하므로 help 만 살아도 할 수 있는 것이 없고, 프라이밍이 「`engram` 명령이 계속 실패하면 우회하지 말고 주인에게 알린다」를 이미 가르친다(1-6). ADR-0212 「거부한 대안」의 「요란하게 죽어도 결과는 같다 — 표면 없이 떠난다」는 help 가 다른 명령과 따로 살 수 있던 때의 논리다.
  - **★단 이 근거는 codex 측정에 매인다(F4)★** — codex app-server 에이전트는 workspace-write 샌드박스에서 셸을 돈다(`crates/engram-dashboard-agent/src/backend/codex/mod.rs:139` `SandboxMode::WorkspaceWrite`). 그 샌드박스가 루프백 HTTP 를 막는지는 이 저장소에 실측이 없다(`git grep` — 기록 0). 막는다면 codex 에이전트에게 CLI 의 데몬 명령은 지금도 전부 실패하고 help 만 로컬로 살아 있던 셈이다 — 그때 D10 은 MCP 로 우편을 쓰는 codex 에이전트에게서 우편 계약 화면(`engram help mail` — MCP 도구 설명문 둘이 가리키는 자리 · `mcp_server.rs:419` · `:511`)을 빼앗는다. 그래서 U1 QA 가 그것을 재고(§5-1 5), 막히면 착지 전에 멈추고 사용자에게 올린다.
- **출력 채널 = 지금의 공통 봉투(메인 결정 · 리뷰 1라운드 때 닫음):** stdout 에 `{"status":"error","code","hint"}` 한 줄 + exit 1. 근거 = ① 헤더 계약 `:72-78`(「CLI 가 스스로 내는 반려는 항상 stdout 봉투 · 기계 판정은 exit code」)을 지금 모든 명령이 따른다 ② 시험이 그 봉투를 잰다(`engram_cli.rs:243-259` · `:713-718` · `:1644-1648`) ③ stderr 로 옮기면 help 와 무관한 모든 명령의 실패 출력이 바뀐다 — 이 단계의 범위 밖이다. 사용자 지시 문구의 「stderr 로 보고」와 다르다는 것을 ADR-0284 에 적는다 — 「한 길 · 0 이 아닌 종료 · help 폴백 없음」은 그대로 선다. stderr 는 지금처럼 진단 줄(형태가 깨진 성공 · 목록 행 버림 · 위 메모)에만 쓴다.

---

## 3. 작업 단위

**원칙(사용자 2026-10-02 — 「망가지지 않는 단위로」 · 「임시 땜빵 금지」):** 단위마다 커밋 하나 · 커밋마다 워크스페이스가 빌드되고 회귀가 초록이다. help 가 데몬으로 먼저 가고(U1) CLI 가 얇아진 채 이사한다(U2) — 그래서 CLI 가 데몬을 잠시 의존하거나 사본을 잠시 갖는 중간 상태도, 파서 · 어휘가 두 벌인 커밋도 없다.

### 3-1. 단위 표

| 단위 | 범위 | 건드리는 파일 | 리뷰 · QA |
|---|---|---|---|
| **U1** | ★커밋 하나★ D10 · D11 — help 계약 상수 · 데몬 help 라우트 · 원천 주입 · 로더 · 어휘 · CLI help 배송 · 공통 실패 길(404 포함) · stdin 순서 · 시험(데몬 패키지 안 — 이사 없음) | agent `src/types.rs`(상수 셋 · `CLI_GROUP_*` 옆) · 데몬 `src/control/help.rs`(신설) · `src/control/mod.rs`(모듈 한 줄) · `src/control/mcp_server.rs`(경로 상수 · `ControlRoute` · `ALL` · `path` · `is_mail` · 라우터 arm · 핸들러 · 주입판 서버 함수 · 명단 시험 `:1394` · 포인터 `:1636` · `:1697`) · `src/lib.rs`(원천 주입 `:618` 부근) · `src/control/priming.rs:705`(포인터) · `src/control/catalog.rs:421`(포인터) · `src/bin/engram.rs`(help 구획 삭제 · 배송 · D11 · stdin 순서 · 헤더 `:13-24` · `:69-86` · 시험) · `tests/engram_cli.rs`(고침 8 + F1 · 새로 2) · `tests/mail_gate.rs`(픽스처 주입 · +1) · `prompts/engram-help.md:8` · `:9`(머리글 — 화면 밖) · `scripts/build-release.ps1:18-21`(주석) · `.github/workflows/ci.yml:32-34`(주석) | `/implement standard` · `/review code full` · `/qa full`(§5-1 — 바이트 대조 · codex 측정에 실 데몬 + 자격증명이 필요하다) |
| **U2** | ★원자★ D1 · D2 · D3 · D5 · D6 — 이사 · 빌드 입구 넷 · qa 빌드 명령 | 새 `crates/engram-dashboard-cli/Cargo.toml` · `git mv` 둘(`src/bin/engram.rs` · `tests/engram_cli.rs`) · 데몬 `Cargo.toml:33-39` · 데몬 `src/bin/roundtrip_smoke.rs:66` · `:424` · 루트 `Cargo.toml`(멤버 · 머리) · `Cargo.lock` · `build-release.ps1`(`:11` · `:16-17` · `:127-135`) · `rebuild-run-debug.bat:92-93` · `rebuild-run-release.bat`(`:5-8` · `:110-111`) · `scripts/README.md:12` · qa `:320` · `:355` · `:360`(사용자 승인됨 2026-10-08) | `/implement standard` · `/review code full` · `/qa full`(§5-2 · §5-3 필수) |
| **U3** | D7 · D9 · 문서 | `ci.yml`(새 스텝 둘 · `:520-523` · `:675` · `:767` · ⑦ · 버전 게이트 `:1355-1420`) · platform `src/lib.rs:166-180` · CLAUDE.md · qa 나머지(사용자 승인됨 2026-10-08) · `docs/testing-strategy.md` · `docs/reference/architecture-overview.md` · 코드 주석(§6 3) · ADR-0284 | `/implement standard` · `/review code full`(+ doc 렌즈) · `/qa standard` + 새 게이트 로컬 1회 |

**순서 U1 → U2 → U3.** U1 이 먼저인 이유 — U2 의 CLI 가 데몬 lib 를 부르지 않아야 원자 이사가 데몬 의존 없이 선다. **파일 겹침:** `engram.rs` · `engram_cli.rs`(U1 · U2) · `ci.yml`(U1 주석 · U3) · `build-release.ps1`(U1 주석 · U2) · qa 바인딩(U2 · U3) · CLAUDE.md(U3) → **한 코더씩 직렬**. **U1 은 크지만 커밋 하나다(F9)** — 데몬 신설 모듈과 CLI 삭제 · 시험 이사가 같은 커밋에 들어가야 파서 · 어휘 두 벌이 커밋에 남지 않는다. 코더 지시서에는 §3-2 의 작업 순서(작업 트리가 중간에 멈춰도 빌드가 서는 순서)를 넣고, 커밋은 끝에서 한 번만 한다. **critical 이 아닌 이유:** kill 인과 · finalize · 락 순서 · replay 어느 것도 지나지 않는다 — 새 라우트는 기존 미들웨어 · 명단 · 핸들러 꼴을 그대로 쓴다.

### 3-2. U1 — help 를 데몬으로 · 공통 실패 길 (커밋 하나)

- **작업 순서(커밋하지 않는 하위 단계 — 어디서 멈춰도 작업 트리의 빌드가 서게):** ① agent `types.rs` 상수 셋(아무도 안 부른다) ② 데몬 `control/help.rs` — 원천 · 어휘 · 파싱 · 필수 구획 · 렌더 · 요청 바디 해석 · 기동 로그 + 옮겨 올 시험 10 · 데몬 반 4 · 새 단위 2(이 시점에는 CLI 에도 같은 시험이 남아 있다) ③ `mcp_server.rs` 라우트 · 핸들러 · 주입판 서버 함수 · 명단 시험 ④ `lib.rs` 주입 ⑤ `tests/mail_gate.rs` 픽스처 주입 · +1 ⑥ `engram.rs` — `Plan::Help` · 배송 · F1 형태 규칙 · `exit_code_for_help_response` · `help_request_body` · `fail_unreached` 로 세 자리 모으기 · 404 변형 · stdin 순서 · help 구획 · `HelpTopic` 삭제 · 반려 문구 · 헤더 · 시험(옮긴 10 · 지운 1 삭제 · 고친 4 · 갈라진 4 의 CLI 반 · 새로 1) ⑦ `engram_cli.rs`(고침 8 + F1 · 새로 2) ⑧ 주석 · 머리글(§6 1). ①~⑤ 는 더하기만이라 거기서 멈춰도 빌드가 선다 — ★단 그 상태를 커밋하지 않는다★(그 시점은 파서 · 어휘가 두 벌이다).
- **수용 기준:** ① `rg -n "engram_dashboard_daemon" crates/engram-dashboard-daemon/src/bin/engram.rs` → 0(CLI → 데몬 lib 간선 소멸) ② `rg -n "include_str!" crates/engram-dashboard-daemon/src/bin/engram.rs crates/engram-dashboard-daemon/src/control/help.rs` → 0(사본 없음) ③ `rg -n "fn (render_help|load_help_text|help_file_path|section_marker)\b" crates/engram-dashboard-daemon/src/bin/engram.rs` → 0 · 데몬 `control/help.rs` 에 파싱 · 렌더 함수가 있다 ④ `rg -n "print_error\(e\.code\(\)" crates/engram-dashboard-daemon/src/bin/engram.rs` → 정확히 1(공통 함수 안 · D11) ⑤ `rg -n "CLI_HELP_(ROUTE|TOPIC_KEY|SCREEN_KEY)" crates` → agent 정의 셋 + 데몬 · CLI 사용 · `rg -n '"/control/help"' crates` → 0(경로 리터럴을 다시 적지 않았다 — 키 이름 글자는 요청 바디를 글자로 단언하는 시험에만 남는다) ⑥ `rg -n "start_mcp_server_with_help\(" crates` → 정의 + 호출 둘(`lib.rs` · `mail_gate.rs`) ⑦ `cargo test -p engram-dashboard-daemon -- --test-threads=4` 초록 — 그중 `--bin engram` **114** · `--test engram_cli` **43** · 데몬 lib 의 help 시험 **16** · `mail_gate` **12**(예상) ⑧ §3-5 바이트 대조 아홉 PASS(§5-1 4) ⑨ 데몬 로그에 help 원천 warn 이 없다(본문 파일을 찾았다) ⑩ ★codex 측정 PASS(§5-1 5) — 아니면 착지하지 않고 사용자에게 올린다★.
- **게이트:** `cargo fmt --check` · `cargo test -p engram-dashboard-agent -- --test-threads=4`(`types.rs` 상수 · 포인터) · `cargo test -p engram-dashboard-daemon -- --test-threads=4` · `cargo build` · `npm test`(무관 — 그대로 초록) · CI · `/qa full`(§5-1).
- **위험:** ① 구획 파싱을 글자 그대로 옮기지 않으면 화면 바이트가 갈린다 — 시험 10 과 §3-5 대조가 잡는다 ② 캐시를 두면 파일 수정이 데몬 재시작 전까지 안 보인다 — 옮겨 오는 override 시험의 「파일을 바꾼 뒤 다음 렌더가 바뀐다」 단언 ③ `is_mail` 을 true 로 두면 우편 막힌 에이전트(MCP 갈래 전부)가 help 를 못 읽는다 — `mail_gate` +1 이 잡는다 ④ 전송 실패 자리 하나를 공통 함수로 안 옮기면 출력이 갈린다 — 수용 기준 ④ ⑤ 같은 철자들이 다른 요청을 보내면 화면이 갈린다 — 캡처 스텁이 바디를 글자로 잡는다 ⑥ 운영 조립이 다섯 인자판을 부르면 help 가 늘 `INTERNAL` — 수용 기준 ⑥ · 기동 warn ⑦ stdin 순서를 안 바꾸면 자격증명 없는 `--body-stdin` 이 열린 stdin 에 매달린다 — C1 시험(시한 5초) ⑧ codex 샌드박스가 루프백을 막으면 D10 이 codex 에이전트의 우편 계약 화면을 없앤다 — 수용 기준 ⑩.

### 3-3. U2 — 이사(원자)

- **왜 쪼갤 수 없나:** 두 패키지가 같은 bin 이름 `engram` 을 가지면 cargo 는 막지 않고 산출 파일 충돌을 경고한다 — 같은 `target\<profile>\engram.exe` 를 서로 덮어써 어느 쪽이 남는지가 빌드 순서에 달린다(F9). 릴리스 스크립트는 데몬의 `--bin engram` 이 사라지는 커밋부터 깨지므로 같은 커밋이어야 한다(ADR-0273 「영향」).
- **단위 안 순서(한 커밋):** ① 새 `Cargo.toml` + 루트 멤버 ② `git mv` 두 파일 ③ 데몬 `Cargo.toml:33-39` 삭제 ④ 빌드 입구 넷(`build-release.ps1` 두 `Invoke-Step` · 런처 둘 · `roundtrip_smoke.rs:66` · `:424`) · `scripts/README.md:12` ⑤ `Cargo.lock` ⑥ qa 빌드 명령 셋(사용자 승인됨 2026-10-08). `engram.rs` 본문은 고치지 않는다 — U1 이 이미 데몬 lib 를 떼어 두었다.
- **수용 기준:** ① `rg engram_dashboard_daemon crates/engram-dashboard-cli` → 0 ② `cargo tree --locked -p engram-dashboard-cli --depth 1 --prefix none -e normal,dev,build --target all --all-features` 의 워크스페이스 줄 = agent · cli · command ③ `cargo test -p engram-dashboard-cli -- --test-threads=4` = bin 단위 **114** · `engram_cli` **43** 전부 초록(예상 수) ④ `cargo test -p engram-dashboard-daemon -- --test-threads=4` 초록 ⑤ `git diff --stat -M` 에 rename 둘 ⑥ `npm test`(launcherWiring 포함) 초록 ⑦ platform 게이트 ④ · ⑤ · ⑥ PASS(명단 불변) · 셸 게이트 1 PASS ⑧ `rg -n "engram-dashboard-daemon[^\n]*--bin engram($|[^-\w])" crates src-tauri scripts .github CLAUDE.md .claude/skill-bindings docs/testing-strategy.md` → 0(F3 — 끝을 `\b` 로 적지 않는다: `--bin engram-dashboard-daemon` 의 `-` 앞도 단어 경계라 그 줄이 걸린다) ⑨ ★§5-3 릴리스 스크립트 1회 + 릴리스 데몬 단독 기동 확인 PASS★(F2 · F6 — 머지 전 필수) ⑩ §3-5 바이트 대조 아홉을 새 exe 로 다시 PASS(§5-2).
- **게이트:** `cargo fmt --check` · `cargo build` · `cargo test --workspace -- --test-threads=4`(§3-6 대조) · `npm test` · CI(`--locked`) · `/qa full`(§5-2 · §5-3).
- **위험:** `.bat` CRLF · 빌드 입구를 하나 빠뜨리면 그 입구만 조용히 낡은 exe 를 쓴다(1-8) · 릴리스 두 호출을 한 블록에 두면 앞 실패가 묻힌다(F1) · `help_repo_root` 의 깊이(같다 — 시험이 잰다).

### 3-4. U3 — 게이트 · 문서

- **단위 안 순서:** ① CI 새 스텝 둘(단독 기립 · 의존 상한) ② 고리 둘 · 게이트 ⑦ · 버전 게이트 ③ `ci.yml:520-523` 주석 · platform 헤더 ④ CLAUDE.md · qa 나머지 ⑤ 문서 · 코드 주석 · ADR-0284 · step-log. 전부 더하는 편집이라 어느 칸에서 멈춰도 빌드가 선다.
- **수용 기준:**
  1. 새 게이트를 로컬에서 한 번 — 의존 상한 집합 일치 · 고리 셋 0줄(짝 그대로 1줄 이상) · ⑦ cli 0줄(짝 그대로) · ★버전 게이트의 `cargo_version` · `check` 를 지금 버전으로 돌려 6건 PASS★(태그에서만 도는 게이트라 여기서 미리 잰다 — F7).
  2. **옛 자리 포인터 0(F6 — 짧은 꼴까지):** `rg -nP "engram-dashboard-daemon/(src/bin/engram\.rs|tests/engram_cli\.rs)|(?<!engram-dashboard-cli/src/)\bbin/engram\.rs|(?<!engram-dashboard-cli/)\btests/engram_cli\.rs" crates src-tauri scripts .github CLAUDE.md .claude/skill-bindings docs/testing-strategy.md docs/reference/architecture-overview.md -g '!crates/engram-dashboard-cli/**'` → 남는 것은 CLAUDE.md `:229`(수치 이력 속 「`bin/engram.rs` 의 훅 구획」 — 역사 서술이라 그대로 둔다) 하나뿐. 지금(`02ab36c`) 이 명령의 결과 = 1-10 의 짧은 꼴 · 전체 꼴 포인터 목록 + CLAUDE.md `:229` + U2 가 없애는 둘(데몬 `Cargo.toml:39` 의 `path = "src/bin/engram.rs"` — 줄째 지운다 · `tests/engram_cli.rs:599` — 파일째 cli 패키지로 가 검색 범위에서 빠진다)이다(실측 — 3판 `d421fcf` 기준 · 그 뒤 코드 무변경). ★U1 이 help 화면 포인터 넷(`priming.rs:705` · `mcp_server.rs:1636` · `:1697` · `catalog.rs:421`)을 데몬 `control/help.rs` 로 이미 바꾸므로 U3 이 고칠 짧은 꼴은 `mcp_server.rs:208` · agent `types.rs:474` · `:558` 셋이다★. ★고칠 때는 「cli `src/bin/engram.rs`」 같은 짧은 꼴을 쓰지 않고 `engram-dashboard-cli/src/bin/engram.rs` · `engram-dashboard-cli/tests/engram_cli.rs` 로 적는다★ — 짧은 꼴이면 이 명령이 다시 잡는다. 덧붙여 `rg -n "engram_cli" docs/testing-strategy.md` 가 새 cli 절에만 있다(데몬 절 `:93` 의 목록 꼴은 위 정규식이 못 본다). ★ADR · 과정 기록 · 핸드오프 · 날짜 박힌 스냅숏은 고치지 않는다★.
  3. `rg -n "daemon.exe·engram.exe|선언 5건|무엇을 보나 \(5건\)" .github` → 0.
- **위험:** 버전 게이트는 태그에서만 돈다 — 수용 기준 1 이 로컬로 대신 잰다 · 의존 상한을 수로 적으면 요점을 잃는다(D7 2).

### 3-5. help 바이트 대조 (U1 · U2 의 QA — ADR-0212 가 한 것과 같은 꼴 · C3 · F11)

- **아홉 호출:** `engram`(인자 없음) · `engram help` · `engram help mail` · `engram mail --help` · `engram help agent` · `engram agent --help` · `engram help window` · `engram help settings` · `engram help theme` — 화면 다섯 + 별칭 하나 + 다른 철자 셋.
- **★잴 것 셋 — exit code · stdout · stderr★**, 같은 바이트 정확 도구로 앞뒤를 뜬다: bash 의 파일 재지정(`"$EXE" "$@" >"$D/out.$i" 2>"$D/err.$i"; echo $? >"$D/code.$i"`) 또는 node `child_process.spawnSync`(버퍼 그대로 기록). ★PowerShell 의 `>` 는 쓰지 않는다★ — 인코딩 · 줄끝을 바꿔 바이트 대조가 거짓이 된다. 비교 = `cmp`(stdout · stderr) + 코드 문자열 비교. 기대 = stdout 같음 · stderr 앞뒤 모두 빈 파일 · exit 앞뒤 모두 0.
- **전(기준):** U1 편집 **전에** 기준 커밋(코드 트리 = `02ab36c`)에서 기준 CLI 를 다시 짓는다 — `cargo build -p engram-dashboard-daemon --bin engram`(분리 실행) → `target\debug\engram.exe` 의 수정 시각이 그 빌드 시작보다 뒤인지 본다(옛 exe 를 재지 않는다) → 그 exe 를 스크래치 폴더로 복사해 둔다(U1 빌드가 같은 자리를 덮어쓴다 · 다시 떠야 할 때 쓴다) → 자격증명 없이(`env -u ENGRAM_TOKEN -u ENGRAM_CONTROL_URL`) 아홉을 떠 둔다(데몬 불필요 · 데이터 폴더 무관).
- **후:** U1 뒤 · U2 뒤 각각 — 새 exe 의 수정 시각을 같은 방법으로 보고, 앱을 띄워 데몬이 서면 스폰한 에이전트의 자격증명(qa F 절 — 토큰 · URL)으로 같은 아홉을 같은 도구로 뜬다 → 위 비교. 아홉 모두 PASS 여야 한다.
- **★qa F 절의 자격증명 절차는 실 engram.exe 와 처음 돈다★** — 그 절은 「두 값을 뽑는 부분은 가짜 파일로만 확인했다 · 실 `engram.exe` 와 함께 돈 적은 없다 — 2026-10-06」라고 스스로 적는다(`.claude/skill-bindings/qa.md:445`). 첫 실행에서 토큰 · URL 추출이 틀리면 대조를 멈추고 그 절차를 메인에 올린다(바인딩 문구를 고치는 일이다). §9 미검.
- **본문 파일을 그 사이에 바꾸지 않는다** — U1 이 고치는 머리글 `:8-9` 는 첫 구획 앞이라 파서가 버린다(화면 밖 · 1-2). 다른 커밋이 구획 본문을 고쳤다면 기준을 다시 뜬다.
- **바이트가 같지 않아도 되는 것(의도된 변화 — 대조 대상 아님):** 모르는 낱말(`help inbox`)의 반려 — 코드 `BAD_ARGS` → `INVALID_ARGUMENT`(가안) · `help mail send` 의 반려 문구(낱말 목록 → `engram help` 안내) · 자격증명 없는 help(화면 → `NO_TOKEN` 봉투 · exit 1).

### 3-6. 단위마다 검증 — 회귀 수 대조

1-3 TRD §5-2 의 명령 그대로다(`cargo test --workspace -- --test-threads=4` 를 앞뒤로 돌려 결과 줄 수 · 통과 · 실패 · 무시를 견준다 · 줄 수가 줄면 타깃 소실 — 초록이어도 멈춘다). 아래는 전부 예상이다.

| 단위 | 결과 줄 | 통과 | 무시 |
|---|---|---|---|
| U1 | 그대로(새 시험 파일 0 — `help.rs` 시험은 데몬 lib 단위 바이너리 안 · `mail_gate.rs` · `engram_cli.rs` 는 있는 파일) | **+9** — CLI bin 단위 124 → **114**(옮김 −10 · 지움 −1 · 새로 +1 · 갈라진 넷과 고친 넷은 수 그대로) · 데몬 lib 단위 **+16**(옮김 10 · 갈라 온 반 4 · 새로 2) · `mail_gate` 11 → **12** · `engram_cli` 41 → **43**(고침 8 + F1 · 새로 2 — C1 · F5) | 그대로 |
| U2 | 그대로 — 데몬 −2(`unittests src/bin/engram.rs` · `tests/engram_cli.rs`) · cli +2(같은 둘 · bin 전용이라 Doc-tests 줄 없음) | 그대로 — 157(114 + 43)이 자리만 옮긴다 | 그대로(두 파일 다 `#[ignore]` 0) |
| U3 | 그대로 | 그대로 | 그대로 |
| 합 | 0 | +9 | 0 |

**기준선은 U1 착수 직전에 잰다** — 마지막 기록(CLAUDE.md 「빌드·검증 명령」 = 결과 줄 59 · 4229 통과 · 0 실패 · 31 무시 · `79b8d09` 흡수 머지 직전)을 이 TRD 는 돌려 보지 않았다. agent 단위 수는 그대로다(`types.rs` 에 상수만 더한다). 셸 lib_unit(819)은 그대로다 — 셸은 건드리지 않는다.

### 3-7. U1 착수 체크리스트

0. **전제 둘.** ① 새 ADR(§7 — 채번 · 링크 = `/adr` · 커밋 직전 번호 재확인). ② 되돌릴 지점 — 이 TRD 를 로컬 커밋해 출발점을 만든다(코드 트리 = `02ab36c`).
1. **출발 수치** — §3-6 명령(분리 실행 — qa 바인딩 「분리 실행」).
2. **바이트 기준** — §3-5 「전」(기준 CLI 재빌드 · 수정 시각 · 복사 · bash/node 로 아홉)을 U1 코드 편집 전에 뜬다.
3. **손댈 파일(정확히):** §3-1 의 U1 줄.
4. **순서:** §3-2 의 작업 순서 ①~⑧ — 커밋은 끝에서 한 번.
5. **게이트 · 수치:** §3-2 게이트 → §3-6 대조(결과 줄 그대로 · 통과 +9).
6. `/review code full` → `/qa full`(§5-1 — codex 측정이 막히면 여기서 멈추고 사용자에게) → 게이트 초록 뒤 커밋(`S21: refactor(daemon): …` — 스텝 번호는 step-log 를 잇는다 · 끝에 Co-Authored-By 트레일러).

---

## 4. 의존 그래프 — 2-3 뒤

| crate | 직접 워크스페이스 의존(normal · build) 지금 | 2-3 뒤 |
|---|---|---|
| cli(신설) | — | agent · command |
| daemon | agent · base · command · messaging · net · platform · protocol | 그대로 — bin `engram` 만 나가고 `control/help.rs` 가 든다 |
| 셸(`engram-dashboard`) | base · command · net · platform · protocol(+ dev daemon · platform `test-support`) | 그대로 |
| base | 없음 | 없음(무변경) |

- **engram.exe 를 짓는 그래프** — 데몬 패키지 전체(정상 194 · 실측) → agent · command · serde_json(약 92 · 예상).
- **사라지는 간선** — CLI → 데몬 lib(2-2 U2 가 만든 임시 간선 · ADR-0282 「영향」). U1 에서 사라지고 U2 이후 다시 서지 않는다 — 사본도 dev 간선도 없다.
- **그대로인 게이트** — 셸 게이트 1(그 짝 「데몬 → agent ≥ 1」도 그대로 선다) · base 게이트 ①②③ · platform 게이트 ①~⑥ · net · 메시징(이름 알파벳 포함 — D7 8) · transport 게이트. **바뀌는 게이트** — §2-6.

---

## 5. GUI 실측

절차(기동 인자 · 환경변수 · PID · teardown)는 `/qa` 바인딩 §full 이 갖는다. 여기는 무엇을 볼지만 적는다.

### 5-0. 전제 (경고 — 사용자 확인)

- **데이터 폴더** — 셸이 띄운 데몬은 WMI 라 늘 `<저장소>\.engram-dev` 를 쓴다. 그 명부의 자동 복원 프로필 · 살아 있는 개발 데몬에 대한 확인은 2-2 TRD §5-0 과 같다(§8 O8).
- **★데몬과 CLI 를 함께 짓는다★** — U1 까지는 `cargo build -p engram-dashboard-daemon`(한 패키지) · U2 부터 `cargo build -p engram-dashboard-daemon -p engram-dashboard-cli`(qa 빌드 명령 — 사용자 승인됨 2026-10-08 · 분리 실행). `node scripts/build-client-shell.mjs` 는 둘 다 짓지 않는다.
- **★바이트 기준(§3-5 「전」)은 U1 편집 전에 뜬다★** — 기준 CLI 를 기준 커밋에서 다시 짓고 복사해 둔다. U1 빌드가 같은 exe 를 덮어쓴다.
- **codex 에이전트를 띄운다(§5-1 5)** — 격리 cwd(스크래치 폴더 안) · 일회용 프로필 · 끝나면 끄고 지운다(qa 바인딩 E 절 · B 절 · F 절 4). 사용자 확인 뒤.
- **릴리스(§5-3)** — 조립 폴더는 모든 체크아웃 밖(데몬의 설치 위치 걸음을 재기 때문 — §5-3) · `target\release` 의 데몬 · CLI exe 를 쓰고 있는 릴리스 앱이 없어야 한다(있으면 링크가 os error 5 로 진다 — qa 바인딩 「공유 데몬 바이너리 락」). 그 앱을 끄는 것은 사용자 확인 뒤다.

### 5-1. U1 뒤 (debug)

1. `target\debug\engram.exe` 가 이번 빌드 산출인지(수정 시각) — 옛 exe 를 재지 않는다.
2. **비GUI(데이터 폴더를 안 건드린다) — D11:** 자격증명 없이 `engram.exe help` → stdout 이 `engram.exe mail pending` 과 같은 `NO_TOKEN` 봉투 · 둘 다 exit 1. 가짜 토큰 + `ENGRAM_CONTROL_URL=http://127.0.0.1:0` 으로 `help` · `agent list` → 같은 `CONNECT_FAILED` 봉투 · exit 1. 자격증명 없이 `mail send --to x --body-stdin` 을 stdin 을 연 채 → 곧바로 `NO_TOKEN` · exit 1(C1 — 매달리지 않는다).
3. **앱 기동** → 셸이 데몬을 띄운다 → 데몬 로그(`.engram-dev\logs\daemon-*-<pid>.log`)에 「제어 평면 CLI 형제 exe 를 못 찾음」(데몬 `lib.rs:114`)도, help 원천 warn(§2-9 ②)도 없다.
4. **스폰한 claude 에이전트의 자격으로 CLI 를 부른다** — qa 바인딩 F 절 그대로(터미널 모드 claude 를 프롬프트 없이 띄우고 그 화신의 MCP 설정 파일에서 토큰 · URL 을 빌린다 · ★그 절차의 첫 실 실행이다★ — §3-5) → ★§3-5 바이트 대조 아홉 PASS★ · `engram.exe agent list` → 응답 JSON · exit 0 · 그 에이전트가 목록에 있다.
5. **★codex app-server 에이전트 안에서(F4 · 착지 조건)★** — codex 에이전트를 JSON 모드로 띄워(qa 바인딩 E 절 — `agent.spawnInto` 의 `backend: 'codex'`) 그 에이전트에게 자기 셸에서 `engram help` 와 `engram agent list` 를 실행하고 stdout 과 exit code 를 그대로 보고하게 한다(토큰은 그 에이전트의 env 에 있다 — 빌리지 않는다). **PASS** = `engram help` 가 목차 화면 · exit 0 이고 `agent list` 가 JSON · exit 0. **멈춤** = `CONNECT_FAILED` 류 · 샌드박스 거절 · 승인 요청이 서서 거절됨 등 루프백 HTTP 가 막혔다는 신호 → U1 을 착지하지 않고 사용자에게 올린다(§2-10 — D10 이 그 에이전트의 우편 계약 화면을 없앤다). 무엇을 보고했는지와 에이전트 출력 원문을 보고에 붙인다.
6. **(선택) 파일 수정이 재시작 없이 보이나** — 본문 파일을 건드리는 측정이라 하지 않는다. 시험(§3-2 위험 ②)이 잰다.
7. **정리** — 두 에이전트를 끄고 프로필을 지운다(F 절 4).

### 5-2. U2 뒤 (debug)

5-1 의 1 · 3 · 4 를 새 패키지 exe 로 다시 — `target\debug\engram.exe` 가 새 패키지 빌드 산출인지(수정 시각 · `cargo build -p engram-dashboard-cli` 뒤) · 데몬이 그것을 형제로 찾는다(3) · 바이트 대조 아홉 PASS(4). codex 측정(5)은 U1 결과로 갈음한다(U2 는 CLI 코드를 바꾸지 않는다).

### 5-3. ★U2 — 릴리스 1회 (master 머지 전 필수 · F2 · F6)★

**① 릴리스 스크립트:** `pwsh scripts/build-release.ps1 -OutDir <모든 체크아웃 밖의 새 폴더>`(예: `%TEMP%\engram-release-check-<시각>` · 두 `Invoke-Step` 이 된 빌드 단계 그대로 · 수 분 — 분리 실행) → 스크립트 PASS · 폴더에 exe 셋 + `prompts/` 두 파일 · 두 빌드 단계가 각자의 줄로 찍힌다 · 그 폴더의 `engram.exe help` 가 자격증명 없이 `NO_TOKEN` 봉투 · exit 1(D11 — 릴리스 CLI 도 같은 길 · 이 CLI 확인은 폴더 자리와 무관하다).

**② ★릴리스 데몬 단독 기동 — help 본문 경로(F6 · 필수 · 사용자 데이터 무관)★:** 그 폴더의 `engram-dashboard-daemon.exe` 를 혼자 띄운다 — 환경변수 `ENGRAM_DATA_DIR=<또 하나의 스크래치 폴더>`(데이터 · 로그 · `daemon.json` 이 전부 거기 간다 — 셸 `stop_smoke` 가 실 데몬을 같은 법으로 격리한다 · CLAUDE.md 「빌드·검증 명령」 그 줄) · 터미널 프로세스 트리 밖 · 출력은 파일로(qa 바인딩 「분리 실행」 — 환경변수를 넘길 수 없으면 메인에 올린다). 셸 · 앱은 띄우지 않는다.
- **판정(게이트):** `<스크래치>\logs\daemon-*-<pid>.log` 에 실행 머리글(`==== engram …`)이 있고 ★help 원천 warn 이 없다★(기동 때 `<설치 위치>\prompts\engram-help.md` 를 읽어 본 결과 — §2-9 ② · warn 은 기본 레벨에서 늘 남는다). (덧붙임) `RUST_LOG=info` 를 넘길 수 있으면 info 줄의 경로가 `<OutDir>\prompts\engram-help.md` 인지까지 본다.
- **이것이 재는 것:** 실제 릴리스 exe 에서 데몬의 설치 위치 걸음이 표지를 못 만나 exe 폴더로 떨어지고 그 옆 `prompts/` 를 집는 갈래 — 시험이 못 타는 유일한 갈래다(시험은 늘 저장소 안에서 돈다). HTTP 로 help 를 부르는 것은 하지 않는다 — 토큰은 스폰한 에이전트에게만 발급되고, 라우트 · 렌더는 §5-1 이 debug 로 이미 잰다.
- **끝:** 띄운 그 PID 를 끈다(`taskkill /PID <pid> /T /F` — 이 측정이 띄운 프로세스다 · graceful 끄기는 알려진 실패라 쓰지 않는다 — `docs/tracking.md` T-52). 두 스크래치 폴더를 지운다(사용자 확인 없이 지워도 되는 것은 이 측정이 만든 폴더뿐이다).
- **CI 쪽 짝:** 데몬 단위 시험 「릴리스 모양 임시 폴더」(§2-9 ⑦ — `<tmp>\prompts\engram-help.md` 만 둔 폴더를 base 로 준 원천)가 원천의 읽기를 매 push 에서 잰다. 걸음 자체(exe → 설치 위치)는 그 시험이 못 탄다 — 그래서 ②가 머지 게이트다.

- **★`-OutDir` 는 모든 체크아웃 밖이어야 한다 — ② 때문이다★** — 걸음은 빌드 모드와 무관하게 늘 걷는다. 폴더가 어느 체크아웃 아래 있으면 걸음이 그 `.git` 을 만나 설치 위치 = 체크아웃 루트가 되고, 데몬은 배포 폴더가 아니라 그 체크아웃의 `prompts/` 를 읽는다 — 폴백 갈래를 한 번도 안 타고 초록이 된다. 먼저 확인한다: 그 폴더와 모든 조상에 `.git` 도 `[workspace]` 를 담은 `Cargo.toml` 도 없다(사용자 임시 폴더가 체크아웃 아래면 다른 자리를 고른다). ①의 CLI 확인만이라면 자리는 상관없다.
- **`launch\빌드.bat` 로 재지 않는다** — 그 런처는 `-OutDir "%~dp0release"`(`launch/빌드.bat:4`) = `<저장소>\launch\release` 로 조립한다. 체크아웃 안이라 위 이유로 폴백을 못 잰다. 같은 사실 때문에 `build-release.ps1:16-17` 의 주석 「릴리즈 폴더엔 .git·[workspace] 마커가 없으므로 install_root = exe 디렉토리」는 그 폴더에서 거짓이다(그 폴더에서 띄운 릴리스 앱의 프라이밍 · help 는 저장소의 `prompts/` 를 읽는다 — 지금도 그렇다 · 동작은 이 단계가 바꾸지 않는다). U2 가 그 주석을 고친다(같은 파일을 이미 고친다 — §6 2). CI 는 이 스크립트를 태그 push 때만 돌리고 태그는 다시 쓸 수 없으므로(`ci.yml:1281-1283` · CLAUDE.md 「태그」) 여기서 먼저 재지 않으면 처음 깨지는 자리가 배포다.

---

## 6. 문서 후속 (이 TRD 는 고치지 않는다 — 각 단위가)

1. **U1** — `engram.rs` 헤더(`:13-24` 「발견은 help 로만 … help 는 크레덴셜 · 데몬 없이 답한다」 → 「help 화면은 데몬이 낸다(`/control/help` — agent `CLI_HELP_ROUTE`) · CLI 는 낱말을 싣는다 · 자격증명 · 데몬이 없거나 데몬이 라우트를 모르면 다른 명령과 같은 길로 실패한다」 · `:69-86` 동작 · 출력 계약에 help 갈래 · 공통 실패 함수 · 404 의 `PROTOCOL_MISMATCH` · stdin 은 자격증명 뒤) · agent `types.rs` 상수 셋의 doc(계약의 두 쪽 — 데몬 라우트와 CLI · `topic` 필수 · 모르는 키 거절 · 기존 다섯 라우트는 손으로 맞춘다는 차이) · 데몬 `control/help.rs` 모듈 머리(ADR-0092 경로 모양 · 요청마다 읽는다 · 사본 없음 · 호출자로 화면을 고르지 않는다 — ADR-0220 결정 4 · 데몬은 본문을 구획으로만 고른다 — ADR-0156 · ADR-0081 과의 경계 · `// ADR-0284`) · `mcp_server.rs` 경로 상수 doc(형제 규율 · 우편 아님의 이유) · 주입판 서버 함수 doc(다섯 인자판은 「구성되지 않음」 원천 — 운영 조립은 주입판을 부른다) · `lib.rs` 조립 자리 주석(프라이밍과 같은 자리에서 원천을 짓는다) · 포인터 넷(`priming.rs:705` · `mcp_server.rs:1636` · `:1697` · `catalog.rs:421` → 데몬 `control/help.rs` — `catalog.rs:421` 의 「정적 화면이라 계열 이름 둘만 낸다」 논지는 그대로 참이다) · `prompts/engram-help.md:8`(「내장 사본이 대신 나간다」 → 「help 가 반려로 답한다 — 사본은 없다」) · `:9`(「옛 바이너리」 → 「이 파일을 직접 읽던 2-3 전 engram.exe」) · `build-release.ps1:18-21`(읽는 쪽 = 데몬 · 사본 없음 · 빠지면 help 가 반려 · tripwire 가 배송을 지킨다) · `ci.yml:32-34`(경로 필터 근거 — 데몬이 실행 중에 읽고 배포판에 실린다 · 데몬 시험이 그 파일을 읽는다 · 포인터를 `scripts/build-release.ps1:50` 과 데몬 `control/help.rs` 로) · `engram_cli.rs` 의 성질 주석(`:6` · `:675-677` · `:1630-1631`).
2. **U2** — cli `Cargo.toml` 주석(데몬 `:33-36` 을 옮긴다) · `roundtrip_smoke.rs:66` · `:424` · `build-release.ps1:11` · `:16-17`(「릴리즈 폴더엔 마커가 없으므로」 → 「체크아웃 밖에 조립한 릴리즈 폴더면 마커가 없어 install_root = exe 디렉토리 · `launch\release` 처럼 체크아웃 안이면 체크아웃 루트다」 — §5-3) · `:127-132` · `rebuild-run-release.bat:5-8` · `scripts/README.md:12`(「데몬까지 재빌드」 → 「데몬 · CLI 까지」) · 루트 `Cargo.toml` 머리(멤버 수 · cli 한 줄).
3. **U3** — CLAUDE.md(§2-6 7 · 8) · `docs/testing-strategy.md`(데몬 절 `:93` · `:95` · `:159` 에서 `engram_cli` 를 빼고 — 데몬의 `--test-threads=4` 근거는 「기본 실행의 실 PTY 셸 스폰 + `#[ignore]` 실 데몬 exe」로(F4) — 데몬 절에 help 라우트 · 원천 시험 한 줄 · 새 cli 절: ① bin 단위 ② 프로세스 레벨 `engram_cli`(실 exe · 스텁) · 실행 `cargo test -p engram-dashboard-cli -- --test-threads=4`) · `docs/reference/architecture-overview.md:169`(실행 산출 = 데몬 exe + cli 패키지의 `engram`) · `:173-197`(그래프에 cli 노드 · → agent · command) · `:384` · 짧은 꼴 포인터 셋(F6 — `mcp_server.rs:208` · agent `src/types.rs:474` · `:558` → `engram-dashboard-cli/src/bin/engram.rs` · `engram-dashboard-cli/tests/engram_cli.rs` 꼴로) · `ci.yml:520-523` · `:1355-1420` · platform `src/lib.rs:166-180`.
4. **오케스트레이터** — step-log 착지 항목 · 메모 §8 「사실」 첫 줄 · §10 2-3 착지 표시 · 옛 ADR 문장은 본문을 고치지 않고 ADR-0284 가 링크로 적는다(§7).
5. **날짜 박힌 스냅숏** — `docs/reference/architecture-map-notes.md` · `architecture-map-data/*.json` 은 그 지도를 다시 뽑을 때.

---

## 7. ADR-0284 에 박을 것

> 채번 · 링크 · 도장 = `/adr`(번호는 커밋 직전에 다시 본다 — 2026-10-08 기준 0284 는 로컬 · 원격 추적 브랜치 어디에도 없고, 0283 은 이 브랜치에 있다(master 흡수 머지로 들어왔다)). 거부한 대안은 메인 · ADR · 사용자가 준 것만 옮긴다(CLAUDE.md 「결정 날조 금지」).

1. **CLI 의 역할 = 인증 + 배송(D10 · D11 — 사용자 결정 2026-10-08).** CLI 는 에이전트가 데몬에 닿을 다른 길이 없어서 생긴 입구다(MCP 대용). 화면(help)은 데몬이 낸다 · 데몬에 못 닿거나 · 자격증명이 없거나 · 데몬이 라우트를 모르면(404) 모든 명령이 한 길로 실패한다 · help 전용 폴백과 내장 사본은 없다.
   - **계약:** 라우트 경로 · 바디 · 응답 키는 agent `types.rs` 상수 한 벌(`topic` 필수 — null = 목차 · 모르는 키 거절 — 철자 어긋남이 조용히 목차로 접히지 않게) · 무-세션 POST · 같은 bearer · 우편 아님 · 200 + JSON. 기존 다섯 라우트는 손으로 맞춘 채로 둔다(이 결정은 새 라우트만 묶는다).
   - **원천:** 조립부(`lib.rs`) 주입 · 기존 서버 함수는 서명 그대로 두고 주입판을 더한다(바뀌는 호출 자리 2 — 인자를 늘리는 안은 25 자리라 택하지 않았다) · 요청마다 읽는다(데몬은 오래 산다 — ADR-0212 의 「재빌드 없이」를 「재시작 없이」까지 지킨다) · 기동 때 해석 경로를 info / warn 으로 남긴다.
   - **화면:** 호출자로 고르지 않는다(ADR-0220 결정 4 를 라우트가 되살리지 않는다).
   - **오류 코드:** 모르는 낱말 `INVALID_ARGUMENT` · 본문 없음 `INTERNAL` · 404 = `PROTOCOL_MISMATCH`(command 어휘를 빌린 이유 = `UNKNOWN_COMMAND` 를 빌린 것과 같은 규율 · 오타와 판 어긋남을 한 코드로 섞지 않는다 — §2-10) — 가안이 확정되면.
   - **D11 의 출력 채널 = 지금의 공통 봉투(stdout JSON + exit code) — 메인 결정.** 사용자 지시 문구는 「stderr 로 보고」였다 — 채널을 옮기면 모든 명령의 실패 출력 · 헤더 계약 · 시험 셋이 바뀌어 이 단계 범위를 넘는다. 「한 길 · 0 이 아닌 종료 · help 폴백 없음」은 그대로 선다. 이것을 ADR 본문에 그대로 적는다(다음 세션이 「사용자는 stderr 라 했다」로 되돌리지 않게, 또는 되돌릴 때 그 대가를 알게).
   - **stdin 순서(C1):** `--body-stdin` 은 자격증명 뒤에 읽는다 · 오류 우선순위 변경(자격증명 없음 + 빈 stdin = `NO_TOKEN`).
   - **알려진 한계(F5):** 옛 데몬 + 새 CLI 에서 우편 막힌 자격증명은 help 에 `MAIL_NOT_ALLOWED` 를 받는다(옛 데몬의 fail-closed 접기) — CLI 가 고쳐 읽지 않는 이유 · 그 창이 개발 중에만 서는 이유(§2-10). `CARGO_TARGET_DIR` 가 체크아웃 밖인 개발 빌드는 help 가 `INTERNAL`(프라이밍이 같은 조건에서 꺼지는 것과 같은 갈래).
   - **착지 조건(F4):** codex app-server 에이전트의 샌드박스에서 루프백 HTTP 가 되는지 실측한 결과(§5-1 5) — 막혔다면 이 결정은 착지하지 않았다.
   - **거부한 대안(사용자 · 메인):** CLI 가 본문을 계속 렌더한다(CLI 가 내용을 쥔다 — 역할 위반) · 데몬이 꺼지면 CLI 가 최소 내장 화면을 낸다(같은 이유 · 다른 명령이 다 실패하는 상태에서 help 만 사는 것은 쓸모가 없다 — 단 F4 측정 조건) · 화면 낱말 목록을 CLI 에 남겨 반려 문구에 싣는다(어휘 두 벌) · 서버가 안에서 설치 위치로 원천을 짓는다(시험이 시험 exe 자리에 매인다 — C2) · help 에서 온 `MAIL_NOT_ALLOWED` 를 CLI 가 판 어긋남으로 고쳐 읽는다(의미 판정은 데몬 단독).
2. **고쳐지는 옛 문장(본문은 고치지 않고 링크 · 도장 = `/adr`):**
   - ADR-0212 **결정 3** 「cwd 를 안 보고 데몬도 자격증명도 안 탄다」 → **뒤집힌다**: 경로 해석 모양(override 먼저 · install-root 걸음 + 고정 상대경로 · override 실패는 고정으로 안 감)은 그대로 데몬이 진다. 읽는 쪽이 데몬이고, CLI 의 help 는 자격증명을 타고 데몬에 간다.
   - ADR-0212 **결정 4** 「파일이 없거나 깨졌으면 내장 사본으로 답한다」 → **뒤집힌다**: 사본 없음 · 본문을 못 쓰면 반려(반쪽 화면 금지는 그대로). 같은 ADR 「거부한 대안」의 「파일이 없으면 시끄럽게 죽는다 — 기각」 · 「내장 사본을 안 두고 파일만 쓴다 — 기각」은 이 결정이 받아들인다(근거 = 위 1) · 「exe walk-up 을 CLI 안에 다시 구현한다 — 기각」은 대상이 없어진다(CLI 가 경로를 안 본다).
   - ADR-0212 **「영향」 「잃은 보증 하나」(F8)** — 그 항목은 「하위 항목마다 페이지가 있다」는 컴파일러 강제를 잃은 대신 **「필요한 절 명단」 검사와 내장 사본 테스트**가 그 자리를 대신한다고 적었다. 내장 사본 테스트는 사라진다 → 그 자리를 **데몬의 두 시험**이 진다: 「배포되는 파일(저장소 `prompts/engram-help.md`)이 필수 구획을 다 싣는다」 · 「모든 화면이 그 파일만으로 선다」(옮긴 `the_embedded_copy_carries_every_section_the_screens_need` · `every_screen_still_renders_from_the_embedded_copy_alone` 의 새 뜻 — §2-9 ⑦). 필요한 절 명단 검사는 로더에 그대로 산다. 배송된 폴더에 그 파일이 있는가는 릴리스 tripwire(`build-release.ps1:215-226`)가 그대로 진다. 셋 중 하나라도 지우면 없는 구획을 가리키는 화면 낱말이 운영에서 `INTERNAL` 로만 드러난다.
   - ADR-0211 **「영향」** 「`engram help` 는 자격증명도 데몬도 없이 답한다 — 기존 성질」 → **뒤집힌다**.
   - ADR-0220 **결정 2** 의 「목차 화면 자체는 남긴다(프라이밍을 못 본 사람이 셸에서 치는 자리)」 → 화면은 남되, 자격증명 없는 사람 셸에서는 help 가 실패한다(그 사람은 `prompts/engram-help.md` 를 직접 읽는다) — 이 결정이 받아들인 값으로 적는다.
   - ADR-0220 **「영향」** 첫 항목 「하나라도 빠지면 파일이 통째로 거부되고 내장 사본이 나간다」 → 「help 가 반려로 답한다」 · 고정 시험 둘의 집(`the_mail_screen_teaches_the_reply_contract` · `the_priming_pointer_names_help_entries_that_actually_render`)이 `bin/engram.rs` 에서 데몬 `control/help.rs` 로 옮겼다(프라이밍 포인터는 CLI 쪽에 파싱 반이 남는다). 새 계열을 늘리면 데몬 `HelpTopic` · 필수 구획과 파일 구획을 함께 늘린다.
   - **ADR-0232(F8)** — 「`**/*.md` 일괄 제외」를 기각한 근거가 `prompts/engram-help.md` 는 `include_str!` 로 바이너리에 구워진다(`engram.rs:231`)였다. U1 이 그 `include_str!` 를 걷는다 → ★결론(그 경로를 필터에서 빼지 않는다)은 그대로 서고 근거만 바뀐다★: 데몬이 실행 중에 그 파일을 읽고 · 데몬 시험이 그 파일을 읽어 필수 구획 · 우편 계약 · 프라이밍 포인터를 재며 · 배포판 manifest 에 든다(`build-release.ps1:50`). 그 파일만 바꾼 push 에 게이트가 없으면 구획 하나를 빠뜨린 편집이 CI 를 지나 운영 help 를 `INTERNAL` 로 만든다(전에는 사본이 덮었다 — 지금은 더 비싸다). `ci.yml:32-34` 주석은 U1 이 고친다.
   - **ADR-0156 · ADR-0081 과의 경계(F8 — 위반 아님을 적는다)** — ADR-0156 은 「데몬 코드가 클라이언트가 등록한 명령의 `help` 블롭을 파싱 · 검증 · 분기하면 위반」이고, ADR-0081 은 데몬이 UI payload 를 파싱하지 않는다는 것이다. `control/help.rs` 는 둘 다 하지 않는다 — 읽는 것은 **데몬 자신의 배포 파일**(`prompts/engram-help.md`)이고, 하는 일은 `## <id>` 구획을 골라 그대로 내는 것뿐이다. 명부의 `help` 블롭 · UI 명령 · payload 는 손대지 않는다. 화면 낱말 `window` · `settings` 는 화면 이름이지 UI 명령 이름이 아니다(창 명령의 이름 · 인자는 그 화면 본문 글 속에 산문으로만 있다 — 데몬은 그 글을 해석하지 않는다).
   - ADR-0132 조각 ①의 근거(「표면을 배우는 자리가 이미 스폰돼 있어야 하면 발견이 아니다」 — `engram.rs:198-199` 가 인용)는 이 결정이 대신한다. ADR-0132 **결정 4**(발견은 그룹 help)는 그대로다 — 화면을 내는 쪽만 바뀐다.
   - ADR-0273 **결정 4** 「의존 = agent · command · 설치 위치 규칙」 → 설치 위치 규칙이 빠진다 · **결정 5**(CLI 쪽 사본 + 같은 경로 시험) → **대상이 없어진다**(CLI 가 설치 위치를 아예 안 쓴다).
   - ADR-0271 **「영향」** 「CLI 쪽 사본과 같은 경로 시험은 ADR-0273 이 진다」 → 그 짐이 없어진다. **「거부한 대안」 첫 항목**의 「ADR-0273 뒤에는 설치 위치를 자기 사본으로 가져」 → CLI 는 사본도 갖지 않는다(낱말만 바로잡는다).
   - ADR-0282 **「영향」** 「engram CLI → 데몬 lib 임시 간선(U2 ~ 2-3) … 2-3 이 이 간선을 CLI 쪽 사본으로 바꾼다」 → 「2-3 U1 이 help 를 데몬으로 옮기며 간선째 없앤다(사본으로 바꾸지 않는다)」. 「`find_install_root` 는 데몬 `data_dir` 에 그대로다」 · 결정 3(셸 사본은 같은 경로 시험)은 그대로다.
   - ADR-0100 · `build-release.ps1` 주석 — `prompts/engram-help.md` 부재가 조용하던 것이 요란해진다(tripwire 는 그대로).
3. **패키지 모양(D1 · D2 · D3)** — 이름 `engram-dashboard-cli` · bin 전용 · 파일 자리 그대로 · 의존 = agent · command · serde_json · dev 의존 없음. 거부한 대안 = `-engram` 이름 · `src/main.rs` · lib + bin(§2-1).
4. **빌드 입구(D6)** — 릴리스 두 호출 · 각자의 `Invoke-Step`(한 블록이면 앞 실패가 묻힌다) · 비용(serde_json 기능 집합 차 → agent · command 재빌드) · 런처 둘 · `roundtrip_smoke.rs` 안내문 · 릴리스 스크립트 1회 + 릴리스 데몬 단독 기동 = 머지 전 필수 · 거부한 대안 = 한 호출(동작 미검 · 기능 합집합) · qa 바인딩은 사용자 승인으로(2026-10-08).
5. **게이트(D7)** — 의존 상한은 이름 집합 일치(수 세기를 버린 이유) · test-support 고리 셋 · 게이트 ⑦ 의 cli 줄 · 버전 게이트 편입(라이브러리 기준과의 구별 · 실패 비용 = 버려지는 태그) · 단독 기립 스텝(타깃 부재를 실패로).
6. **「새 crate 는 이름 알파벳에 더한다」의 bin 전용 예외(F8 — 3판)** — 메시징 정규식(ci.yml `:520-523` · qa `:253` · CLAUDE.md `:255`)에 cli 를 더하지 않는다 · 이유 = lib 타깃이 없어 부를 수 있는 crate 가 아니다 · lib 를 세우는 날 더한다 · 세 자리의 문구는 §2-6 8. **기각 근거 자평: 보통** — 더해도 해는 없고 규칙이 단순해지는 이점이 있다. 예외를 택한 것은 「이 멤버를 부를 수 있다」는 오독을 알파벳이 만들지 않게 하려는 메인 판단이다.
7. **옛 포인터 읽기 규칙(D9 — 짧게 · 찾는 법과 함께):** ① `crates/engram-dashboard-daemon/src/bin/engram.rs` → `crates/engram-dashboard-cli/src/bin/engram.rs`(ADR-0273 「영향」이 이미 적었다 — 확인만) ★단 help 화면 렌더 · 로드와 그 시험은 데몬 `crates/engram-dashboard-daemon/src/control/help.rs` 로 갔다★(ADR-0212 · ADR-0220 · ADR-0232 가 `engram.rs` 로 가리킨 help 구획 · `include_str!` · 고정 시험) ② `crates/engram-dashboard-daemon/tests/engram_cli.rs` → `crates/engram-dashboard-cli/tests/engram_cli.rs`(`git grep -l engram_cli -- docs/decisions`) ③ 데몬 `Cargo.toml` 의 `[[bin]] engram` → cli `Cargo.toml` 의 `[[bin]]`.
8. **착지 실측** — 회귀 수 · 바이트 대조 아홉(exit · stdout · stderr) · codex 측정 · GUI · 릴리스 1회 · 릴리스 데몬 단독 기동(착지 때 채운다).
- **코드 앵커** — `# ADR-0273`: cli `Cargo.toml` 의 `[[bin]]`(ADR-0273 「영향」이 정했다). `// ADR-0284`: agent `types.rs` 의 help 계약 상수 · 데몬 `control/help.rs` 모듈 머리 · 주입판 서버 함수 · `lib.rs` 의 원천 주입 줄 · `engram.rs` 의 공통 실패 함수 · help 배송 갈래 · stdin 순서 자리 · `ci.yml` 새 스텝 둘. `// ADR-0212` 는 데몬 `control/help.rs` 의 경로 규칙 자리로 옮겨 단다.

---

## 8. 열린 것

**사용자에게 올릴 것은 없다.** 단 U1 QA 의 codex 측정(§5-1 5)이 루프백 차단을 보이면 착지 전에 올린다. O8 은 U1 · U2 QA 전에 받을 확인이다. 나머지는 메인 결정이고 리뷰가 다르게 보면 바꾼다.

- **O1 · O2 · O5 — 닫힘(4판).** D4 가 접혀 A′ / A / B 선택 · 폴백의 입주 조건 판단 · base 모듈 자리가 대상을 잃었다.
- **O3 — qa 바인딩 편집: 사용자 승인됨(2026-10-08).** 빌드 명령 셋(`:320` · `:355` · `:360`)은 U2 의 QA 전에 · 나머지(`:79-84` · `:138` · `:253` · `:254-275` · `:442` · `:445`)는 U3.
- **O4 — 버전 = 제품 버전 + 버전 게이트 편입(메인 결정).** 대안 = `0.1.0` 으로 두고 게이트에서 뺀다(라이브러리처럼). 단독 배송물이 있다는 게이트 자신의 기준으로 편입했다. 대가 = 릴리스마다 매니페스트 하나가 늘고, 빠뜨리면 실패는 태그를 push 한 뒤에야 드러나 그 태그를 버린다. 막는 것 = U3 수용 기준 1 의 로컬 1회와 릴리스 절차의 버전 올림 커밋.
- **O6 — 의존 상한 판정 = 이름 집합(메인 결정).** 기존 net 게이트 3 의 꼴(줄 수)과 갈린다 — 줄 수로는 데몬이 들어온 것을 못 본다.
- **O7 — CLI 의 async 0 을 platform 게이트 ⑦ 에 한 줄로 둔다(메인 결정).** 대안 = 따로 `cli gate 2`. 같은 함수 · 같은 판정이라 한 스텝에 둔다 — 그 대신 platform 헤더의 「대상」 서술이 platform 소비자가 아닌 것(CLI 자신의 직접 서드파티)까지 품는다.
- **O8 — GUI · 릴리스 실측의 확인**(2-2 TRD §8 O3 와 같다 · §5-0) — U1 의 §5-1 3 · 4 · 5(codex 에이전트를 띄운다) · U2 의 §5-2 와 §5-3 앞에서 묻는다. §5-3 ②(릴리스 데몬 단독 기동)는 스크래치 데이터 폴더라 사용자 데이터를 건드리지 않지만, `target\release` 를 쓰는 릴리스 앱을 꺼야 하면 그것은 묻는다.
- **O9 — ADR 번호.** 0284 는 2026-10-08 기준 다음 빈 번호다. 다른 작업이 먼저 쓰면 밀린다 — 커밋 직전 재확인.
- **O10 — 릴리스 두 호출의 시간(메인 결정 = 두 호출).** serde_json 기능 집합이 갈려 agent · command 가 다시 지어진다(C3). 릴리스 시간이 문제가 되면 한 호출로 합치는 쪽(CLI 가 데몬 기능 집합으로 지어진다 · `--bin` 동작 확인 필요)을 다시 본다.
- **O11 — 닫힘(메인 결정 · 4판 리뷰 1라운드).** D11 의 출력 채널 = 지금의 공통 봉투(stdout JSON + exit code). 사용자 문구 「stderr」와의 차이와 그 이유는 §2-10 · §7 1 에 적었다.
- **O12 — 디버그 데이터 루트 시험의 빈틈(후속 · 2-3 단위 아님 · 옛 C1).** 1-7 끝. 걸음이 실패해 exe 폴더 폴백으로 떨어져도 시험 셋이 초록이다. 3판의 설계(기대 루트를 `CARGO_MANIFEST_DIR` 에서 따로 내 `<체크아웃 루트>\.engram-dev` 와 정확 비교 · `dunce::canonicalize` 로 철자 고르기 · 체크아웃 밖이면 `CI` 에서 실패 · 로컬은 `--show-output` 으로 건너뜀 확인)는 그대로 쓸 수 있다 — 3판 리뷰 2라운드 ①이 그 설계를 다듬었다. 데이터 루트 분기를 지나는 다음 작업(예: O13)과 함께 닫는다.
- **O13 — 데몬 · 셸 걸음 두 벌을 base 로 합치기(후속 · 2-3 단위 아님 · 옛 A′).** 지금 글자까지 같은 두 벌을 ADR-0282 결정 3 의 같은 경로 시험이 묶는다(표지 하나를 빠뜨린 사본은 못 잡는다 — 그 결정이 적은 빈틈). 합치면 그 빈틈이 대상째 사라지지만 2-2 착지 코드를 다시 열고 ADR-0269 입주 판정 · ADR-0264 경계를 다시 재야 한다(3판 §2-4 A′ 평가가 남아 있다 — git 이력의 3판). 2-3 에서는 CLI 가 걸음을 안 쓰므로 이 정리를 끌어올 이유가 없다.
- **O14 — 데몬 help 의 확장 여지(메모 · 결정 아님).** 데몬이 화면을 내게 되면 화면에 런타임 사실(예: 지금 붙은 클라이언트의 명령)을 섞고 싶어질 수 있다 — `catalog.rs:421` 이 그 방향을 이미 거절한다(「정적 계열 화면」과 「런타임 목록」은 다른 표면이다 · ADR-0156). 이 단계는 바이트 동일 이사만 하고 화면 내용을 바꾸지 않는다.
- **O15 — 기존 다섯 제어 라우트의 경로를 agent 상수로(후속 · 2-3 단위 아님).** help 라우트만 공유 상수로 묶고(F3) 나머지 다섯은 손으로 맞춘 채 둔다 — 한 저장소 안에 두 규율이 섞인다. 다섯을 옮기면 `engram.rs:158` · `:689` 의 「경로 지식은 CLI 소유」 규율과 `mcp_server.rs:51-89` 의 상수 doc 을 함께 고쳐야 해서 이 단계 범위를 넘는다.

---

## 9. 미검

- **기준선 회귀 수** — 이 판에서 돌리지 않았다(§3-6 · U1 착수 직전).
- **시험 이동 수(CLI −10 · 데몬 +16 · `mail_gate` +1 · `engram_cli` +2 · 합 +9)** — 시험 이름을 읽어 분류한 추정이다. 갈라지는 넷 · 새로 쓰는 다섯의 꼴은 구현이 정한다.
- **바이트 동일** — 옮기는 코드가 같다는 독해와 `println!` 을 그대로 쓴다는 설계다. U1 · U2 의 §3-5 대조가 잰다(안 돌렸다).
- **★qa F 절 자격증명 절차와 실 engram.exe 의 첫 실행★** — 그 절은 가짜 파일로만 확인됐다(`.claude/skill-bindings/qa.md:445`). §3-5 · §5-1 4 가 처음 돌린다.
- **★codex workspace-write 샌드박스의 루프백 HTTP★** — 이 저장소에 실측이 없다(`git grep` 0). §5-1 5 가 처음 잰다 — 착지 조건이다.
- **릴리스 데몬을 셸 없이 단독으로 띄우는 길** — `ENGRAM_DATA_DIR` 를 넘기는 분리 실행 경로가 있는지, 단독 기동한 릴리스 데몬이 기동 확인(help 원천 로그)까지 가는지는 안 돌렸다. 셸 `stop_smoke` 가 debug 데몬을 같은 법으로 띄운다는 것이 근거의 전부다.
- **주입판 서버 함수의 호출 자리 2 · 그대로 23** — `git grep -n "start_mcp_server("` 의 25 자리와 하네스 bin 셋이 help 를 부르지 않는다는 grep 독해다.
- **데몬이 404 를 내는 자리 = 라우터 fallback · 세션 고아 둘뿐 · 둘 다 빈 본문** — `mcp_server.rs` 를 읽은 결과다(`not_found()` 호출 1곳 `:759` · 핸들러는 늘 200). 미들웨어 · 서드파티 층(rmcp nest `/mcp`)은 CLI 가 부르지 않는다.
- **옛 데몬 + 새 CLI 의 응답(우편 막힌 자격증명 = `MAIL_NOT_ALLOWED` · 열린 자격증명 = 404)** — `mail_gated_path` 와 그 시험(`tests/mail_gate.rs:400-427`)의 독해다. 실제 옛 데몬으로 돌려 보지 않았다.
- **`PROTOCOL_MISMATCH` 를 내는 생산자가 지금 없다** — `git grep ProtocolMismatch` 독해. 그 코드를 받는 쪽이 모르는 코드로 낮추지 않는다는 것은 command 어휘 등재(`error.rs:80`)가 근거다.
- **C1 시험의 시한(5초)** — 가안이다. 보안 소프트웨어가 프로세스 생성을 느리게 하는 PC 에서 짧을 수 있다(CLAUDE.md 「빌드·검증 명령」의 병렬 항목).
- **`mail_gate` 픽스처 주입 뒤 기존 열한 시험이 그대로 초록** — 픽스처가 원천 하나를 더 쥘 뿐이라는 독해다.
- **D11 공통 함수로 모은 뒤 출력이 바이트로 같다** — 세 자리가 같은 두 줄(`print_error(e.code(), &e.to_string())` + `EXIT_FAILED`)이라는 독해다(1-3). 시험 `engram_cli.rs:243-259` 가 잰다.
- **CLI 빌드 그래프 약 92** — agent · command 의 그래프를 따로 잰 합집합 추정이다(새 패키지가 없다).
- **Cargo.lock 변화 = 멤버 자신 하나 · 서드파티 0** — 쓰는 서드파티가 lock 에 있다는 독해이고 돌려 보지 않았다.
- **bin 전용 패키지에 Doc-tests 줄이 없다** — cargo 동작 지식(가능성 높음). 틀리면 결과 줄이 +1 이다(회귀가 아니다).
- **target 폴더 공유로 형제 관계가 그대로** — 워크스페이스 기본 동작(확실에 가깝다 · §5-2 가 잰다).
- **한 호출 `-p A -p B --bin x --bin y` 의 동작 · 두 호출의 릴리스 시간 증가폭** — 안 돌렸다(두 호출로 피한다 · O10).
- **`-p engram-dashboard-cli` 단독 빌드가 기능 합집합 없이 서는지** — U2 수용 기준 · CI 단독 기립 스텝이 잰다.
- **`launcherWiring.test.ts` 정규식이 `-cli` 를 형제로 본다** — 코드 읽기(U2 의 `npm test` 가 잰다).
- **CLI 그래프에서 `-i engram-dashboard-base` · `-i engram-dashboard-platform` 이 rc 0** — 그래프 논리(U3 실측).
- **`help_repo_root` 깊이** — 코드 읽기(U2 시험이 잰다).
- **에이전트 대면 글에 「help 는 데몬 없이 된다」는 약속이 없다** — `prompts/agent-priming.md` · `prompts/engram-help.md` 를 grep 하고 읽은 결과다(1-6). MCP 도구 설명문 둘은 `engram help mail` 을 가리키기만 한다.
- **GUI 전 단계 · 릴리스 1회**(§5).
- **에이전트 · 사람이 체감하는 변화(코드 읽기)** — 데몬이 떠 있으면 help 아홉 호출의 exit · stdout · stderr 는 바이트로 같다(§3-5 가 잴 것). 바뀌는 것:
  1. 데몬 없이 · 자격증명 없이 help 가 실패한다(D11 — 사람 셸에서 치던 `engram help` 포함 · 그 사람은 `prompts/engram-help.md` 를 직접 읽는다).
  2. 모르는 화면 낱말의 반려 코드 `BAD_ARGS` → `INVALID_ARGUMENT`(가안) · 왕복 뒤에 온다. help 토큰 · `-` 로 시작하는 낱말은 그대로 로컬 `BAD_ARGS` 다(F1).
  3. `help <낱말> <더>` 반려 문구가 낱말 목록 대신 `engram help` 를 가리킨다.
  4. 본문 파일이 없거나 깨지면 낡은 사본 대신 반려(`INTERNAL`)다.
  5. ★`CARGO_TARGET_DIR` 를 체크아웃 밖으로 둔 개발 빌드는 help 가 전부 `INTERNAL` 이다(F5 ①)★ — 데몬 설치 위치가 그 target 폴더로 떨어지고 거기 `prompts/` 가 없다. 지금은 내장 사본이 덮었다. 프라이밍은 같은 조건에서 이미 꺼진다.
  6. ★판 어긋남(F5 ②)★ — 옛 데몬 옆에서 새 engram.exe 를 부르면 우편 막힌 자격증명(오늘의 MCP 갈래 전부)은 help 에 `MAIL_NOT_ALLOWED` 봉투 · exit 1 을, 우편 열린 자격증명은 `PROTOCOL_MISMATCH` 봉투 · exit 1 을 받는다. 반대 조합(새 데몬 + 옛 engram.exe)은 옛 CLI 가 로컬 렌더 · 사본으로 답한다(데몬을 안 부른다).
  7. 어느 명령이든 데몬이 그 라우트를 모르면(404) 빈 줄 대신 `PROTOCOL_MISMATCH` 봉투다(D11).
  8. ★자격증명이 없고 stdin 이 빈 `mail send --body-stdin` 은 `BAD_ARGS` 대신 `NO_TOKEN` 이다(C1)★ — 그리고 stdin 이 열린 채여도 매달리지 않는다.
  9. `ENGRAM_HELP_FILE` 은 이제 데몬 프로세스의 env 다(저장소 안에 설정하는 곳은 없다).
  개발자 쪽: 10. `cargo build -p engram-dashboard-daemon` 만으로는 engram.exe 가 더 지어지지 않는다(D6) 11. `roundtrip-smoke` 하네스의 SETUP-SKIP 안내문 빌드 명령이 바뀐다(`roundtrip_smoke.rs:424` · F3) 12. 자격증명 없이 `engram.exe help` 로 화면을 확인할 수 없다 — qa F 절로 자격증명을 빌린다.
