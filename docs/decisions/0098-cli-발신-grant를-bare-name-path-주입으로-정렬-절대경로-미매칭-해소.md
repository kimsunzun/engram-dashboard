# ADR-0098: CLI 발신 grant를 bare-name + PATH 주입으로 정렬 — 절대경로 미매칭 해소

- 상태: 확정 (2026-07-22, 근거: CLI 발신 0/38→10/10 실측)
- 관련: Amends ADR-0094 (CLI 발신 grant 번역을 절대경로 Bash({exe} *)에서 bare-name Bash/PowerShell({exe}:*) + PATH 주입으로 정렬(claude 권한 매처 미매칭 0/38 해소·배포 이식성)) · ADR-0004(백엔드 지식 격리)

## 맥락
ADR-0094는 CLI 발신 입구(engram-send) grant를 `Cli{exe = 절대경로}` → claude `Bash({exe} *)`(space-star, 절대경로)로 번역했고, "claude Bash 권한 매처가 절대경로·백슬래시·space-star를 어떻게 정규화하는지 **미검증**(best-effort, 실제 에이전트 왕복으로 후속 검증)"이라고 코드 주석에 명시해 뒀다.

그 후속 검증(roundtrip 실측, 2026-07-22)에서 **CLI 발신 38/38 전부 차단**이 확인됐다. 원인 = 세 문자열 불일치:
- grant 패턴 = `Bash(C:\...\engram-send.exe *)`(절대경로 space-star)
- 실제 호출 = `$ENGRAM_SEND_EXE --to ...`(프라이밍이 가르친 env 변수 문자열)
- claude 권한 매처는 이 둘을 프리픽스 문자열로 못 맞춰 헤드리스에서 자동 차단.

에이전트 로그가 직접 확증: "자동 권한 분류기가 차단 — /update-config로 Bash 권한에 engram-send 규칙 추가하라". MCP 경로(mcp__engram__send_message)는 정확 매칭돼 10/10 되던 것과 대비.

## 결정
**세 문자열을 bare 호출명 `engram-send`로 정렬한다:**

1. **grant 값 = bare `"engram-send"`**(`control/mod.rs::build_grants`) — 절대경로를 grant에 안 박는다(배포 이식성: 어느 머신·설치 경로든 동일).
2. **번역 = `Bash({exe}:*)` + `PowerShell({exe}:*)`**(`claude.rs::grants_to_allowed_tools`) — colon-star는 Claude Code 권한 문법의 프리픽스 와일드카드. **두 셸 모두** 커버(Windows에서 에이전트가 PowerShell 툴을 잡아 발신을 시도하는 실측 실패모드 대응). 여전히 발신 입구 한정(ADR-0094 최소권한 불변).
3. **PATH 주입**(`claude.rs::build_spec`) — engram-send는 PATH에 없는 내부 형제 바이너리라, 데몬이 런타임에 찾은 exe 부모 디렉토리를 스폰 env PATH **맨 앞**에 병합(shadowing 방어). 프로필 PATH 존중(마지막 case-equivalent 항목이 승자 + dedupe — env 순차 적용 last-wins). join 실패·비-UTF8은 **loud skip**(warn, lossy 변환 금지 — 오늘 동작 유지 + 불일치 관측화). `ENGRAM_SEND_EXE` env는 하위호환으로 유지.
4. **프라이밍 = bare `engram-send --to <name> --body "..."`**(`agent-priming-routing-v3-en-cli.md`) — env 변수 indirection 제거, grant와 같은 문자열.

## 거부한 대안
- **절대경로 grant 유지 + 매처 정규화 기대(ADR-0094 원안)** — 실측 0/38로 반증. 절대경로 백슬래시·공백·.exe 정규화가 claude 매처와 어긋난다.
- **blanket `Bash(*)` 허용** — ADR-0094 최소권한 위반. 발신 외 임의 셸을 연다.
- **claude에 절대경로를 프라이밍으로 주입** — 프라이밍은 정적 파일이라 런타임 절대경로를 모르고, 경로가 grant·프라이밍·설정에 박혀 배포 이식성이 깨진다.
- **불변 스냅샷 주입 등 배달 재설계** — 과잉. bare-name+PATH가 최소 변경으로 정렬을 달성.

## 근거
- **실측:** default-deny 체제에서 CLI 발신 0/38(차단) → 이 정렬 후 cli-sonnet 10/10, cli-haiku 8/10, cli-xml 9/10(entrance=cli). both-mcp 회귀 10/10 유지.
- **게이트:** `/review code full` — doc-aware(Claude) PASS + blind(codex) FIX 2라운드(프로필 PATH 존중·loud skip·lossy 금지·PowerShell 커버·중복 PATH dedupe) 전부 CLOSED. `/qa standard` green.
- **wire 무영향:** `ToolGrant`/`ControlEndpoint`는 in-proc(serde 없음, protocol crate 밖) — PROTOCOL_VERSION 범프 불요.

## 영향 / 불변식
- **grant 이름 단일 출처 = 컨트롤 채널**(`build_grants`), 번역 문법만 claude.rs(ADR-0004). 프라이밍 텍스트의 `engram-send`는 의도된 교차-아티팩트 중복이며 **roundtrip 하네스가 그 문자열 일치의 강제 기구**다(내용 기반 CLI-지시 판정).
- **PATH 주입 = superset**(prepend, 기존 항목 미제거 — ADR-0086 env 상속). sibling dir 반드시 첫 항목(shadowing 방어).
- 회귀 가드: `claude_control_endpoint_injects_path_*` · `claude_duplicate_*_path_*` · `grants_to_allowed_tools_*`(Bash+PowerShell 2패턴) · `claude_allowed_tools_group_is_last_*`.
- 후속(ADR-0097): 이 grant는 auto mode(bypassPermissions) 하에선 런타임 게이트가 아니라 **미래 공용 제약 레이어용 정책 표면**으로 남는다.
