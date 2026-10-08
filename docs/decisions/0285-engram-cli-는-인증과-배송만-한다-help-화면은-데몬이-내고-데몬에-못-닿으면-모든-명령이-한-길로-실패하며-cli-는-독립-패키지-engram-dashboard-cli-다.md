# ADR-0285: engram CLI 는 인증과 배송만 한다 — help 화면은 데몬이 내고 데몬에 못 닿으면 모든 명령이 한 길로 실패하며 CLI 는 독립 패키지 engram-dashboard-cli 다

- 상태: 확정 (2026-10-08, 근거: 사용자 결정 2026-10-08 (CLI 의 역할 = 인증 + 배송 · help 는 데몬이 낸다 · 못 닿으면 모든 명령이 한 길로 실패한다) + 사용자 위임 2026-10-07 아래의 메인 결정(그 밖 전부) · 2-3 착지 U1 `2de9756` · U2 `58c80a3` · 정본 설계 = `docs/process/S21-crate-boundaries/trd-2-3-cli-package.md` §7)
- 관련: Amends ADR-0212 (결정 3과 4 및 거부한 대안 셋과 영향의 잃은 보증) · Amends ADR-0211 (영향의 help 무자격증명 무데몬 성질) · Amends ADR-0220 (결정 2의 목차 화면 자리와 영향 첫 항목) · Amends ADR-0232 (md 일괄 제외 기각의 include_str 근거) · Amends ADR-0132 (조각 1의 데몬 없는 help 근거) · Amends ADR-0273 (결정 4의 설치 위치 규칙과 결정 5) · Amends ADR-0271 (영향의 CLI 사본 항목과 거부한 대안 첫 항목의 사본 낱말) · Amends ADR-0282 (영향의 CLI 임시 간선 항목) · Amends ADR-0100 (영향의 조용한 저하 서술 중 help 본문 파일) · ADR-0156 · ADR-0081(데몬은 클라이언트 help 블롭 · UI payload 를 해석하지 않는다 — 이 결정은 위반이 아니다, 아래 「영향」) · ADR-0157(모르는 필드 관용 — 이 라우트는 거절한다) · ADR-0092(경로 해석 모양) · ADR-0133(우편 거절은 데몬) · ADR-0094(bin 이름 = `CLI_EXE_NAME`) · ADR-0175 결정 6(lib 무게 — bin 전용) · `crates/engram-dashboard-cli/` · `crates/engram-dashboard-daemon/src/control/help.rs` · `crates/engram-dashboard-agent/src/types.rs`(help 계약 상수) · step-log S21

## 맥락

ADR-0273 은 engram CLI(`engram.exe` — 스폰된 에이전트가 자기 셸에서 부르는 제어 평면 CLI)를 데몬 패키지에서 떼어 exe 하나를 뽑는 독립 패키지로 두기로 했다(작업 순서 2-3 · 이 단계의 헌장). 그 결정 4 · 5 는 CLI 가 설치 위치 규칙(`find_install_root`)을 계속 쓴다고 보고, 데몬 crate 를 떠나는 CLI 쪽에 그 규칙의 사본을 두고 같은 경로 시험으로 묶기로 했다. 2-2 는 그때까지 CLI → 데몬 lib 임시 간선을 남겼다(ADR-0282 「영향」).

CLI 가 그 규칙을 부른 자리는 하나였다 — `engram help` 가 본문 파일 `prompts/engram-help.md` 를 스스로 찾아 읽고 렌더했다(ADR-0212 — 외부 파일 + `include_str!` 내장 사본 폴백 · 결정 3 「데몬도 자격증명도 안 탄다」). 2-3 TRD 3판은 그 규칙을 CLI 쪽 어디에 둘지(base · 걸음만 base · CLI 사본) 셋을 저울질했다.

사용자(2026-10-08): **CLI 의 역할은 인증 + 배송뿐이다.** CLI 는 에이전트가 데몬에 닿을 다른 길이 없어서 생긴 입구다(MCP 대용) — 내용(화면)을 쥘 자리가 아니다. 그래서 help 화면은 데몬이 내고, 데몬에 못 닿거나 자격증명이 없으면 help 도 다른 명령처럼 실패한다(help 전용 폴백 · 내장 사본 없음). 이 이사는 2-3 안에서 패키지 이사 **전에** 한다 — CLI 가 이미 얇은 채로 데몬 패키지를 떠나게.

그 결과 CLI 에는 설치 위치 규칙을 부를 자리가 없어져 위 저울질(TRD 3판의 D4)이 대상째 접혔다.

## 결정

(1 · 2 의 골격 = 사용자 결정 2026-10-08. 그 안의 세부(계약 · 원천 · 반려 코드 · 출력 채널 · stdin 순서)와 3 ~ 6 = 사용자 위임 2026-10-07 아래의 메인 결정 · qa 바인딩 편집 = 사용자 승인 2026-10-08 · 이름은 착지 코드의 것)

1. **CLI 의 역할 = 인증 + 배송 — help 화면은 데몬이 낸다.** 데몬에 제어 라우트 `/control/help` 를 더하고, 본문 파일 읽기 · 구획 파싱 · 필수 구획 검사 · CRLF 접기 · 화면 낱말과 별칭 · 렌더를 데몬 `control/help.rs` 로 옮긴다. CLI `help` 는 화면 낱말 하나를 실어 보내고 받은 화면을 지금과 같은 `println!` 으로 찍는다 — 낱말을 검사하지 않는다. CLI 에 남는 것은 형태 규칙뿐이다(help 토큰의 자리 · 단독 호출 규칙 · help 토큰이나 `-` 로 시작하는 낱말은 화면 낱말이 아니다 → 로컬 `BAD_ARGS`).
   - **계약 = agent `types.rs` 의 상수 한 벌** — `CLI_HELP_ROUTE`(`/control/help`) · `CLI_HELP_TOPIC_KEY`(`topic`) · `CLI_HELP_SCREEN_KEY`(`screen`). 무-세션 POST · 형제 라우트와 같은 bearer · 우편 아님(우편 막힌 자격증명도 우편 화면을 읽어야 한다) · 응답은 늘 200 + JSON(성공 `{"screen": …}` · 반려 `{"status":"error","code","hint"}`). ★`topic` 은 필수(null = 목차)이고 모르는 키는 거절한다★(중복 키도 거절한다 — U1 착지) — 철자 어긋남이 조용히 목차로 접히지 않게. ADR-0157 의 모르는 필드 관용은 빌드 경계를 건너는 배선 몫이고, 여기는 ADR-0100 이 CLI 와 데몬을 한 빌드로 내보내므로 어긋남을 잡는 것이 목적이다. 새 CLI 가 옛 데몬에 모르는 키를 보내면 `INVALID_ARGUMENT` 를 받는다 — 아래 2 의 판 어긋남 처리와 같은 결이다. 기존 다섯 제어 라우트는 손으로 맞춘 채 둔다(이 결정은 새 라우트만 묶는다).
   - **원천 = 조립부 주입.** 데몬 `lib.rs` 가 프라이밍과 같은 자리에서 `HelpSource::from_install_root()` 를 지어 넘긴다. 기존 다섯 인자 서버 함수는 서명 그대로 두고 이름을 `start_mcp_server_without_help` 로 바꾸고(운영 호출자가 그 판을 부르면 리뷰에서 눈에 띄게 — 그 판의 원천은 모든 요청에 `INTERNAL` 로 답하고 기동 때 warn 한 줄을 남긴다), 원천을 받는 주입판 `start_mcp_server_with_help` 를 더한다. 주입판 호출 자리는 넷이다 — `lib.rs` · `tests/mail_gate.rs` 픽스처 · 프라이밍 하네스 bin `roundtrip_smoke` · `priming_smoke`(실 primed 에이전트가 프라이밍이 가르친 대로 `engram help` 를 칠 수 있다). 나머지 21 자리는 이름만 바뀐다.
   - **경로 모양은 그대로 데몬이 진다** — ADR-0212 결정 3 의 모양(환경변수 override 먼저 · install-root 걸음 + 고정 상대경로 `prompts/engram-help.md` · override 실패는 고정 경로로 안 내려간다 · 절대경로만). `ENGRAM_HELP_FILE` 은 이제 데몬 프로세스의 env 다.
   - **요청마다 읽는다** — 데몬은 오래 산다. 기동 때 한 번만 읽으면 ADR-0212 의 목적(「재빌드 없이 반영」)이 「재시작해야 반영」으로 퇴행한다. 비용 = 호출마다 본문 파일 읽기 하나. 기동 때는 해석 경로를 한 번 읽어 보고 정상이면 `info`, 못 읽거나 구획이 빠지거나 경로를 못 지으면 `warn` 을 남긴다(진단이지 판정이 아니다).
   - **화면은 호출자로 고르지 않는다** — 데몬은 호출자가 누구인지(어느 에이전트 · 우편 가부) 알지만 help 는 그것을 쓰지 않는다. ADR-0220 결정 4 가 걷은 호출자별 필터를 이 라우트가 되살리지 않는다.
   - **반려 코드** — 모르는 화면 낱말 = `INVALID_ARGUMENT`(본문을 읽기 전에 판정한다 — 어휘는 코드다) · 본문을 못 씀(파일 없음 · 필수 구획 빠짐 · 원천 없음) = `INTERNAL`. 반쪽 화면도 낡은 사본도 내지 않는다(반쪽 화면 금지는 ADR-0212 결정 4 그대로).

2. **데몬을 부르는 모든 명령이 한 길로 실패한다 — help 포함 · exit 1.** 실패 셋이 같은 출력 함수를 지난다.
   - **자격증명 없음** — `NO_TOKEN` · `NO_CONTROL_URL`. help 갈래를 자격증명 검사 뒤로 옮겼다.
   - **데몬에 못 닿음** — `CONNECT_FAILED` · `INCOMPLETE_RESPONSE`. 손으로 세 번 적혀 있던 꼴을 공통 함수 `fail_unreached` 하나로 모았다(다섯째 호출자를 손으로 적지 않게).
   - ★**데몬이 그 라우트를 모름(404 — 판 어긋남) = `PROTOCOL_MISMATCH`**★ — command 어휘(`ErrorCode::ProtocolMismatch`)를 빌린다. CLI 가 `UNKNOWN_COMMAND` 를 데몬 어휘에서 빌린 것과 같은 규율이고(같은 사실 = 같은 코드), 오타와 판 어긋남을 한 코드로 섞지 않는다.
   - help 전용 폴백도 내장 사본(`include_str!`)도 없다. 인자 형태 오류(`BAD_ARGS`)는 데몬이 필요 없어 자격증명 앞에서 로컬로 끝난다.
   - ★**출력 채널 = 지금의 공통 봉투(stdout 의 JSON 한 줄 + exit code) — 메인 결정이고, 사용자 지시 문구와 다르다.**★ 사용자 지시 문구는 「stderr 로 보고」였다. 채널을 옮기면 모든 명령의 실패 출력 · CLI 헤더 계약(「CLI 가 스스로 내는 반려는 항상 stdout 봉투 · 기계 판정은 exit code」) · 그 봉투를 재는 시험이 함께 바뀌어 이 단계의 범위를 넘는다. 「한 길 · 0 이 아닌 종료 · help 폴백 없음」은 그대로 선다. stderr 는 지금처럼 진단 줄에만 쓴다. 다음 세션이 「사용자는 stderr 라 했다」로 되돌린다면 그 대가가 위 셋이다.
   - **stdin 순서** — `mail send --body-stdin` 은 stdin 을 자격증명 **뒤에** 읽는다(파싱 → 자격증명 → stdin → 네트워크). 자격증명 없는 호출이 닫히지 않은 stdin 에 매달려 공통 길에 닿지 못하던 것을 막는다. 대가 = 오류 우선순위가 바뀐다 — 자격증명 없음 + 빈 stdin 은 `BAD_ARGS` 가 아니라 `NO_TOKEN` 이다.
   - **알려진 한계 둘** — ① 옛 데몬 + 새 CLI 에서 우편 막힌 자격증명(오늘의 MCP 갈래 전부)은 help 에 `MAIL_NOT_ALLOWED` 를 받는다. 옛 데몬이 명단 밖 제어 경로를 우편 게이트에 접는 fail-closed 다(우편 열린 자격증명은 404 → `PROTOCOL_MISMATCH`). CLI 는 그것을 판 어긋남으로 고쳐 읽지 않는다(「거부한 대안」). 그 창은 개발 중에만 선다 — 배포판은 데몬 · CLI 가 한 폴더 · 한 빌드다(ADR-0100). 서는 조건 = 오래 떠 있는 데몬 옆에서 engram.exe 만 다시 지었을 때. ② `CARGO_TARGET_DIR` 가 체크아웃 밖인 개발 빌드는 help 가 전부 `INTERNAL` 이다 — 데몬의 설치 위치가 그 target 폴더로 떨어지고 거기 `prompts/` 가 없다. 프라이밍이 같은 조건에서 이미 꺼지는 것과 같은 갈래이고, 기동 warn 이 그 상태를 로그에 남긴다.
   - ★**착지 조건**★ — codex app-server 에이전트(workspace-write 샌드박스)의 셸에서 루프백 HTTP 가 데몬에 닿아야 한다. 막혔다면 이 결정은 착지하지 않았다 — 그때 이 결정은 MCP 로 우편을 쓰는 codex 에이전트에게서 우편 계약 화면(`engram help mail` — MCP 도구 설명문 둘이 가리키는 자리)을 빼앗는다. 결과 = 닿았다(「근거」).

3. **패키지 모양** — 이름 `engram-dashboard-cli`(폴더 `crates/engram-dashboard-cli`) · bin 전용(lib 타깃 없음) · 파일 자리 `src/bin/engram.rs` 그대로(명시 `[[bin]] name = "engram"`) · `tests/engram_cli.rs` 도 같은 패키지로 간다(`CARGO_BIN_EXE_engram` 은 같은 패키지 bin 만 본다 · 실 exe 를 띄우므로 로컬 명령에 `-- --test-threads=4`) · 의존 = agent(명령 어휘 · help 계약 상수) · command(요청 번호) · serde_json · dev 의존 없음. 데몬 · base 직접 · tokio 0. 버전 = 제품 버전(아래 5).

4. **빌드 입구**
   - **릴리스 스크립트(`scripts/build-release.ps1`) = 데몬 · CLI 두 호출, 각자의 `Invoke-Step`.** 한 블록에 둘을 넣으면 그 함수가 블록 끝의 종료코드를 한 번만 봐 앞 호출의 실패를 뒤 호출의 성공이 덮고, 존재 검사가 이전 빌드의 낡은 exe 를 받아들인다. 대가 = 두 호출은 serde_json 기능 집합이 갈려 agent · command 가 CLI 호출에서 다시 지어진다(릴리스 프로필이라 릴리스 시간이 는다 — 폭은 미검).
   - ★**릴리스 런처 `scripts/rebuild-run-release.bat` 도 두 호출이다 — TRD 설계(한 호출)에서 U2 리뷰 중에 바꿨다.**★ 이 런처가 쓰는 `target\release\` 는 릴리스 스크립트가 조립하는 폴더이고, 두 빌드 그래프(합친 `-p 데몬 -p CLI` · CLI 단독)가 같은 해시 없는 산출 파일 `target\release\engram.exe` 를 쓴다. 나중의 CLI 단독 빌드가 cargo 가 「최신」으로 판정한 단위를 그 파일로 다시 올려 쓰는지는 재지 않았다 — 두 호출로 가르면 그 물음이 대상째 사라진다(합친 빌드의 산출이 그 폴더에 놓일 일이 없다). 리뷰어 둘의 의견이 갈린 것을 이 분리로 닫았다. 각 호출이 자기 실패 검사를 갖는다. debug 런처(`rebuild-run-debug.bat`)는 한 호출(`-p engram-dashboard-daemon -p engram-dashboard-cli`)이다.
   - **`roundtrip_smoke` 하네스의 안내문 빌드 명령 = `cargo build -p engram-dashboard-cli`.**
   - ★**릴리스 스크립트 1회 + 릴리스 데몬 단독 기동 = master 머지 전 필수.**★ 그 스크립트와 버전 게이트는 CI 에서 태그 push 때만 돈다 — 거기서 처음 깨지면 그 태그를 버린다(태그는 다시 못 쓴다).
   - **qa 바인딩의 빌드 명령 편집 = 사용자 승인(2026-10-08).**

5. **게이트**(세 곳 — `ci.yml` · CLAUDE.md 「빌드·검증 명령」 · qa 바인딩. 정본 = `ci.yml` 의 해당 스텝)
   - **cli 의존 상한 = 이름 집합 일치(agent · cli · command).** 줄 수를 세지 않는다 — 수만 세면 command 자리에 데몬이 들어와도 셋이라 이 단계의 요점(데몬 0)을 못 본다.
   - **시험 기능 운영 그래프 게이트(base · platform) 두 고리에 cli 를 더한다** — 운영 바이너리를 내는 패키지가 셋(데몬 · 셸 · cli)이 된다.
   - **platform 게이트 ⑦ 에 cli 0줄 한 줄** — CLI 는 동기 crate 이고, tokio 를 끌고 오지 않는 것이 데몬을 떠난 이유다. agent 줄이 이미 덮는 것 위에 CLI 자신의 직접 서드파티를 덮는다.
   - **버전 게이트에 cli 매니페스트를 넣는다** — 코드는 지금 버전을 읽지 않지만 단독 배송물(engram.exe)이 있어 「일부러 뺀 것」의 라이브러리 기준에 들지 않는다. 대가 = 릴리스마다 고칠 매니페스트가 하나 늘고, 빠뜨리면 태그를 push 한 뒤에야 드러나 그 태그를 버린다.
   - **단독 기립 스텝** — cli 를 `-p` 로 따로 지어 시험한다. ① 데몬과의 기능 합집합 없이 서는가(이 패키지가 따로 선 이유) ② 파일명 가드가 든 `engram_cli` 타깃이 사라지면 「no test target」 으로 죽는다(타깃 부재를 침묵이 아니라 실패로).

6. **「새 crate 는 이름 알파벳에 더한다」의 bin 전용 예외** — 메시징 격리 정규식(`ci.yml` · qa 바인딩 · CLAUDE.md 세 자리)에 cli 를 더하지 않는다. lib 타깃이 없어 아무도 `engram_dashboard_cli::…` 로 부를 수 없고, 그 정규식이 막는 것(메시징이 다른 crate 를 부르는 것)이 성립할 자리가 없다. 그런 멤버에 lib 를 세우는 날 이름을 더한다. 예외를 택한 이유 = 「이 멤버를 부를 수 있다」는 오독을 알파벳이 만들지 않게 하는 메인 판단이다(기각한 쪽의 근거 강도는 「거부한 대안」).

## 거부한 대안

(사용자 · 메인이 준 것만 옮겼다 — 출처 = TRD 2-3 §7 과 그것이 가리키는 §2-1 · §2-9 · §2-10)

- **CLI 가 help 본문을 계속 렌더한다(결정 1).** 기각 = 사용자 결정 — CLI 가 내용을 쥔다(역할 = 인증 + 배송 위반).
- **데몬이 꺼지면 CLI 가 최소 내장 화면을 낸다(결정 2).** 기각 = 사용자 결정 — 같은 이유이고, 다른 명령이 다 실패하는 상태에서 help 만 사는 것은 쓸모가 없다(프라이밍이 「`engram` 명령이 계속 실패하면 우회하지 말고 주인에게 알린다」를 이미 가르친다). ★단 이 기각은 codex 측정에 매였다★ — codex 샌드박스가 루프백을 막았다면 help 만 로컬로 살아 있던 셈이라 결론이 달랐다. 측정 결과는 닿음이다(「근거」). ADR-0212 「거부한 대안」의 「요란하게 죽어도 결과는 같다 — 표면 없이 떠난다」는 help 가 다른 명령과 따로 살 수 있던 때의 논리다.
- **화면 낱말 목록을 CLI 에 남겨 반려 문구에 싣는다(결정 1).** 기각 = 어휘가 데몬과 CLI 두 벌이 된다. CLI 의 반려 문구는 목록 대신 `engram help` 를 가리킨다.
- **서버가 안에서 설치 위치로 help 원천을 짓는다(결정 1).** 기각 = 시험이 시험 exe 의 자리 · `CARGO_TARGET_DIR` 에 매인다. 조립부 주입이면 시험이 알려진 본문을 넣는다.
- **서버 함수의 인자를 늘린다(결정 1).** 택하지 않았다 — 호출 자리 25 전부가 원천을 지어 넘겨야 한다. 개명은 21 자리를 건드리지만 글자 치환뿐이고, help 를 안 부르는 호출자가 원천을 지을 일이 없다. 택한 안의 대가 = 운영 조립이 실수로 `start_mcp_server_without_help` 를 부르면 help 가 늘 `INTERNAL` 이다 — 그 자리는 `lib.rs` 하나이고 이름 · 기동 warn · 바이트 대조가 잡는다.
- **help 에서 온 `MAIL_NOT_ALLOWED` 를 CLI 가 판 어긋남으로 고쳐 읽는다(결정 2 의 알려진 한계 ①).** 기각 = 데몬 반려의 뜻을 CLI 가 고쳐 읽는 것이다 — 의미 판정은 데몬 단독이다.
- **404 를 `UNKNOWN_COMMAND` 로 낸다(결정 2).** 기각 = 「표에 그 이름이 없다(오타)」와 「데몬 빌드가 이 입구를 모른다」를 한 코드로 섞으면 호출자가 멀쩡한 이름을 고치기 시작한다.
- **실패를 stderr 로 보고한다 — 사용자 지시 문구 그대로(결정 2).** 이 단계에서 택하지 않았다(메인 결정) — 이유와 되돌릴 때의 대가는 결정 2 의 출력 채널 항목.
- **패키지 이름 `engram-dashboard-engram`(결정 3).** 기각 = 제품 이름 「Engram Dashboard」와 읽힘이 겹쳐 「engram 패키지」가 무엇을 가리키는지 흐려진다. 기각 근거 강도 = 서술뿐(정량 · 실측 · 코드 근거가 없다 — 약함).
- **소스를 `src/main.rs` 로 옮긴다(결정 3).** 기각 = 경로가 바뀌어 옛 ADR 포인터의 뒤꼬리가 깨진다(ADR-0273 「영향」은 그 경로가 새 패키지의 같은 파일을 가리킨다고 적었다).
- **lib + bin(결정 3).** 기각 = lib 를 부를 소비자가 없다. 한 파일짜리 lib 는 ADR-0175 결정 6 에 걸린다.
- **릴리스 빌드를 한 호출(`-p 데몬 -p CLI` + `--bin` 둘)로 한다(결정 4).** 기각 = 두 패키지에 `--bin` 이 걸리는 꼴의 cargo 동작을 확인하지 않았고(미검), CLI 가 데몬의 기능 집합으로 지어진다. 두 호출의 릴리스 시간이 문제가 되면 다시 본다(TRD §8 O10).
- **메시징 정규식에 cli 를 더한다 — 「새 crate 는 이름을 더한다」 규칙 그대로(결정 6).** 택하지 않았다. ★기각 근거 강도 = 보통★ — 더해도 해는 없고 규칙이 단순해지는 이점이 있다. 예외를 택한 것은 오독 방지라는 메인 판단이다.

## 근거

- **사용자 결정 2026-10-08** — CLI 의 역할은 인증 + 배송뿐이다(에이전트가 데몬에 닿을 다른 길이 없어 MCP 대용으로 생긴 입구다). help 는 데몬이 낸다 · 데몬 못 닿음 · 자격증명 없음은 모든 명령 공통 실패 · help 전용 폴백과 내장 사본 없음 · 2-3 안에서 이사 전에. qa 바인딩 편집 승인도 같은 날이다.
- **TRD 2-3 4판** — `/review trd` 1라운드(codex blind FIX 3 · Claude doc-aware FIX 11 — BLOCK 없음 · 반영)와 4판 확인 리뷰(Claude doc-aware FIX 경미 4 · 반영). 리뷰를 반영한 메인 결정 = 원천 주입(서버 안에서 짓지 않는다) · 계약 상수와 모르는 키 거절 · stdin 순서 · 404 코드 · 출력 채널.
- **착지 U1 `2de9756`**(help 이사 + 공통 실패 길 · 커밋 하나) — `/review code full`(codex blind + Claude doc-aware) PASS · `/qa full` PASS. 워크스페이스 회귀 = 결과 줄 59 · 4259 통과 · 0 실패 · 31 무시. help 아홉 호출(화면 다섯 + 별칭 하나 + 다른 철자 셋)이 U1 전 기준 CLI 와 stdout 바이트로 같고 stderr 는 비었고 exit 0. 데몬 로그에 help 원천 info 가 있고 warn 이 없다. 자격증명 없는 help = `NO_TOKEN` · exit 1. ★codex app-server 에이전트가 자기 샌드박스 셸에서 `engram agent list` 와 `engram help mail` 을 성공했다 — 루프백이 막히지 않아 착지 조건(결정 2)을 채웠다.★ 스폰한 claude 에이전트의 자격으로 부른 `agent list`(TRD §5-1 4)의 결과는 U1 커밋 본문에 기록 없음.
- **착지 U2 `58c80a3`**(원자 이사) — `/review code full` PASS(FIX 2라운드 — 주석 · 릴리스 런처) · `/qa full` PASS. 워크스페이스 회귀 = 59 · 4252 · 0 · 31 로 전후 같다(U1 뒤 4259 와 U2 기준선 사이에는 master 흡수 머지 `5697997` 이 있다 — 그 차이의 내역은 여기서 재지 않았다). cli bin 단위 114 + `engram_cli` 43 · 의존 상한 = agent · cli · command. help 아홉이 U1 exe 와 바이트로 같다(같은 자격증명 · 같은 데몬). debug GUI(TRD §5-2) — 데몬 로그에 info 「제어 평면 CLI 위치 확정 path=…\target\debug\engram.exe」가 있고(새 패키지 exe 를 형제로 찾았다) 「형제 exe 를 못 찾음」 warn 이 없다 · 새 exe 의 `engram agent list` → JSON · exit 0 · 스폰한 에이전트가 목록에 있다 · qa 바인딩 F 절의 자격증명 절차가 실 engram.exe 와 처음 돌았고 첫 시도에 됐다. 릴리스 스크립트 1회 PASS — 데몬 · CLI 가 각자의 `Invoke-Step` 으로 찍혔고 CLI 단계가 다시 컴파일했으며 배송된 `engram.exe` 의 해시가 target 의 것과 같다. 릴리스 데몬 단독 기동(스크래치 `ENGRAM_DATA_DIR`)이 `<OutDir>\prompts\engram-help.md` 를 읽었고 warn 이 없다.
- **U3(게이트 · 문서 · 이 ADR)의 실측은 여기 없다** — 이 ADR 을 쓴 시점에 U3 는 진행 중이었다.
- ★**번호를 0284 → 0285 로 바꿨다(2026-10-08)**★ — 다른 브랜치의 ADR 이 0284 를 먼저 썼다. TRD 2-3 · 핸드오프 · U1 · U2 커밋 본문이 「ADR-0284」라 부른 2-3 의 새 ADR 은 이 ADR 이다(그 번호의 실제 ADR 은 다른 결정이다).

## 영향 / 불변식

- **CLI 는 데몬 lib 를 한 줄도 부르지 않고 설치 위치 규칙도 갖지 않는다.** ADR-0282 「영향」의 CLI → 데몬 lib 임시 간선은 U1 이 help 를 데몬으로 옮기며 간선째 없앴다(사본으로 바꾸지 않았다). ADR-0273 결정 4 의 의존에서 「설치 위치 규칙」이 빠지고, 결정 5(CLI 쪽 사본 + 같은 경로 시험)는 대상이 없다. ADR-0271 「영향」의 「CLI 쪽 사본과 같은 경로 시험은 ADR-0273 이 진다」는 그 짐이 없어졌고, 「거부한 대안」 첫 항목의 「ADR-0273 뒤에는 설치 위치를 자기 사본으로 가져」는 「CLI 는 사본도 갖지 않는다」로 읽는다. 데몬 `find_install_root` 와 ADR-0282 결정 3(셸 사본을 같은 경로 시험으로 묶는다)은 그대로다.
- **ADR-0212 를 고친다.** 결정 3 「cwd 를 안 보고 데몬도 자격증명도 안 탄다」는 뒤집힌다 — 경로 해석 모양은 그대로 데몬이 지고, CLI 의 help 는 자격증명을 타고 데몬에 간다. 결정 4 「파일이 없거나 깨졌으면 내장 사본으로 답한다」도 뒤집힌다 — 사본 없음 · 본문을 못 쓰면 반려(반쪽 화면 금지는 그대로). 같은 ADR 「거부한 대안」의 「파일이 없으면 시끄럽게 죽는다」 · 「내장 사본을 안 두고 파일만 쓴다」는 이 결정이 받아들인다(결정 2 · 「거부한 대안」 둘째) · 「exe walk-up 을 CLI 안에 다시 구현한다」는 대상이 없어진다(CLI 가 경로를 안 본다).
- ★**ADR-0212 「잃은 보증 하나」의 새 집**★ — 그 항목은 하위 항목마다 페이지가 있다는 컴파일러 강제를 잃은 대신 「필요한 절 명단」 검사와 내장 사본 테스트가 그 자리를 대신한다고 적었다. 내장 사본 테스트는 사라졌고 그 자리를 데몬 `control/help.rs` 의 두 시험이 진다 — 배포되는 파일(저장소 `prompts/engram-help.md`)이 필수 구획을 다 싣는다(`the_shipped_help_file_carries_every_section_the_screens_need`) · 모든 화면이 그 파일만으로 선다(`every_screen_renders_from_the_shipped_help_file`)(옛 `the_embedded_copy_carries_every_section_the_screens_need` · `every_screen_still_renders_from_the_embedded_copy_alone` 의 새 뜻). 필요한 절 명단 검사는 로더에 그대로 산다. 배송된 폴더에 그 파일이 있는가는 릴리스 tripwire(`scripts/build-release.ps1`)가 그대로 진다. ★셋 중 하나라도 지우면 없는 구획을 가리키는 화면 낱말이 운영에서 `INTERNAL` 로만 드러난다.★
- **ADR-0211 「영향」의 「`engram help` 는 자격증명도 데몬도 없이 답한다 — 기존 성질」은 뒤집힌다.**
- **ADR-0220 을 고친다.** 결정 2 의 「목차 화면 자체는 남긴다(프라이밍을 못 본 사람이 셸에서 치는 자리)」 — 화면은 남지만 자격증명 없는 사람 셸에서는 help 가 실패한다. 그 사람은 `prompts/engram-help.md` 를 직접 읽는다(이 결정이 받아들인 값). 「영향」 첫 항목의 「하나라도 빠지면 파일이 통째로 거부되고 내장 사본이 나간다」는 「help 가 반려로 답한다」로 읽는다. 고정 시험 둘(`the_mail_screen_teaches_the_reply_contract` · `the_priming_pointer_names_help_entries_that_actually_render`)의 집은 `bin/engram.rs` 에서 데몬 `control/help.rs` 로 옮겼다(프라이밍 포인터는 CLI 쪽에 파싱 반이 남는다 — `the_priming_pointer_lines_parse_as_help_requests`). 새 계열을 늘리면 데몬 `HelpTopic` · 필수 구획과 파일 구획을 함께 늘린다.
- **ADR-0232 — 결론은 그대로, 근거만 바뀐다.** 「`**/*.md` 일괄 제외」를 기각한 근거 중 「`prompts/engram-help.md` 는 `include_str!` 로 바이너리에 구워진다」는 U1 이 그 `include_str!` 를 걷어 거짓이 됐다. 그 경로를 필터에서 빼지 않는다는 결론은 선다 — 데몬이 실행 중에 그 파일을 읽고 · 데몬 시험이 그 파일을 읽어 필수 구획 · 우편 계약 · 프라이밍 포인터를 재며 · 배포판 manifest 에 든다. 그 파일만 바꾼 push 에 게이트가 없으면 구획 하나를 빠뜨린 편집이 CI 를 지나 운영 help 를 `INTERNAL` 로 만든다(전에는 사본이 덮었다 — 지금은 더 비싸다).
- **ADR-0132** — 조각 ① 의 근거(「표면을 배우는 자리가 이미 스폰돼 있어야 하면 발견이 아니다」 — 옛 `engram.rs` 의 help 구획 주석이 인용했다)는 이 결정이 대신한다. 결정 4(발견은 그룹 help)는 그대로다 — 화면을 내는 쪽만 바뀐다.
- **ADR-0100** — `prompts/engram-help.md` 가 빠진 배포판은 조용하던 것(사본이 덮었다)이 요란해진다(help 가 `INTERNAL` 로 반려한다). tripwire 는 그대로이고, 그 요란함이 배포 전에 나게 하는 장치로 남는다.
- **ADR-0156 · ADR-0081 과의 경계 — 위반이 아니다.** ADR-0156 은 데몬 코드가 클라이언트가 등록한 명령의 `help` 블롭을 파싱 · 검증 · 분기하면 위반이라 했고, ADR-0081 은 데몬이 UI payload 를 파싱하지 않는다는 것이다. 데몬 `control/help.rs` 는 둘 다 하지 않는다 — 읽는 것은 데몬 자신의 배포 파일(`prompts/engram-help.md`)이고, 하는 일은 `## <id>` 구획을 골라 그대로 내는 것뿐이다. 명부의 `help` 블롭 · UI 명령 · payload 는 손대지 않는다. 화면 낱말 `window` · `settings` 는 화면 이름이지 UI 명령 이름이 아니다(창 명령의 이름 · 인자는 그 화면 본문에 산문으로만 있고, 데몬은 그 글을 해석하지 않는다).
- **옛 포인터 읽기**(옛 ADR 본문은 고치지 않는다):
  1. `crates/engram-dashboard-daemon/src/bin/engram.rs` → `crates/engram-dashboard-cli/src/bin/engram.rs`(ADR-0273 「영향」이 이미 적은 규칙). ★단 help 화면 렌더 · 로드와 그 시험은 데몬 `crates/engram-dashboard-daemon/src/control/help.rs` 로 갔다★ — ADR-0212 · ADR-0220 · ADR-0232 가 `engram.rs` 로 가리킨 help 구획 · 고정 시험은 거기서 찾는다(`include_str!` 는 없어졌다). 찾는 법 = `rg -l "src/bin/engram.rs" docs/decisions`.
  2. `crates/engram-dashboard-daemon/tests/engram_cli.rs` → `crates/engram-dashboard-cli/tests/engram_cli.rs`. 찾는 법 = `git grep -l engram_cli -- docs/decisions`.
  3. 데몬 `Cargo.toml` 의 `[[bin]] engram` → `crates/engram-dashboard-cli/Cargo.toml` 의 `[[bin]]`.
- **코드 앵커 — 찾는 법 = `rg "ADR-0285"`.** 자리 = agent `types.rs` 의 help 계약 상수 · 데몬 `control/help.rs` · 주입판 서버 함수 · 데몬 `lib.rs` 의 원천 주입 줄 · CLI `engram.rs` 의 공통 실패 함수 · help 배송 갈래 · stdin 순서 자리 · `ci.yml` 의 새 스텝 둘(U3). `// ADR-0212` 는 데몬 `control/help.rs` 의 경로 규칙 자리에 있고, `# ADR-0273` 은 cli `Cargo.toml` 의 `[[bin]]` 에 있다.
