# ADR-0097: 스폰 에이전트 기본 auto mode(bypassPermissions) 채택 — 헤드리스 워커 권한 현실화

- 상태: 확정 (2026-07-22, 근거: 사용자 결정 + CLI 발신 0/38 실측)
- 관련: Amends ADR-0094 (발신만 pre-authorize·bypassPermissions 거부 → 스폰 기본을 auto mode(bypassPermissions)로 채택(2026-07-22 사용자 결정). grant seam은 미래 공용 제약 레이어용 정책 표면으로 유지) · ADR-0004(백엔드 지식 격리)

## 맥락
ADR-0094는 "발신 입구만 pre-authorize, 전부-허용(bypassPermissions)은 거부"를 택했다 — 왕복(메시지 주고받기) MVP엔 발신 grant만으로 충분하고 인젝션 표면을 최소화한다는 근거였다. 그러나 두 가지가 드러났다:

1. **헤드리스 워커엔 기본 거부가 구조적 벽.** stream-json(`-p`) 스폰엔 승인자(사람)가 없다. pre-approve 안 된 툴 호출(Read/Edit/Bash 등)은 전부 거부로 떨어진다. 에이전트가 "메시지 주고받기"를 넘어 실제 일을 하려면 능력마다 grant 배관을 늘려야 하는데, 이는 사실상 권한 시스템을 도구 단위로 재발명하는 코스다.
2. **grant 배관 자체가 취약(실측).** CLI 발신(engram-send) grant는 절대경로 `Bash(<abs> *)` 패턴이 에이전트 실호출(`$ENGRAM_SEND_EXE ...`)과 문자열 미매칭 → claude 권한 게이트가 **38/38 전부 차단**(2026-07-22 roundtrip 실측). ADR-0098(bare-name+PATH 정렬)로 고쳐 0/38→10/10로 회복했지만, 이는 "grant 문자열 하나만 어긋나도 조용히 전멸"임을 보여줬다.

업계 방향도 "승인 완화 + 안전은 경계(containment)가 담당"(Codex `--sandbox`, Claude Code 샌드박스, 컨테이너 격리)이다. 경계 하나를 감사하는 게 grant N개를 관리하는 것보다 단순·결정적이다.

## 결정
**스폰되는 모든 claude 에이전트를 `--permission-mode bypassPermissions`(auto mode)로 띄운다.** Terminal PTY·StreamJson 두 모드 공통, control endpoint 유무와 무관하게 무조건. (`backend/claude.rs::build_spec` — args 맨 앞 base 플래그.)

- **발신 입구 grant(ADR-0094/0096: MCP send_message + Bash/PowerShell engram-send)는 유지한다.** bypass 하에선 런타임 게이트가 아니지만, ① 미래 공용 제약 레이어가 되살릴 **정책 표면**이고 ② 어느 채널을 여는지의 **문서화**다. 제거하지 않는다.
- **임시 체제.** 대체 = 전 LLM(claude·codex·gemini) 공용 제약 레이어(공용 셋팅 정본 → LLM별 설정 파일로 materialize, 셋+상속 조합 주입). step-log 백로그 "전 LLM 공용 제약 레이어" + 이 ADR가 착수 시 전제.

## 거부한 대안
- **기본 거부 유지 + 능력마다 grant 확대(ADR-0094 연장)** — 헤드리스 승인자 부재로 매 능력이 벽이고, grant 문자열 미매칭이 조용한 전멸을 낳는다(0/38 실측). 도구 단위 권한 재발명 = rot.
- **`--dangerously-skip-permissions`** — bypassPermissions와 실질 동일 효과지만 이름이 "위험" 경고를 달고 세팅 의존(`allowDangerouslySkipPermissions`)이 얽힌다. `--permission-mode bypassPermissions`가 claude가 노출하는 정식 모드값이라 이걸 쓴다.
- **`auto`(모델 분류기 승인) 모드** — claude가 노출하는 다른 모드로, 분류기가 확률적으로 승인/거부한다(이번 CLI 0/38의 "권한 분류기 차단"이 이 계열 동작). 워커가 자기 도구를 확률적으로 못 쓰는 건 헤드리스 신뢰성에 반한다. 결정적 bypass를 택한다.
- **지금 Windows 경계부터 깔고 bypass** — 방향은 옳으나(경계 있어야 "auto + 제약") Windows OS 샌드박스 부재 → WSL2/컨테이너+PTY 스파이크 선행 필요. 그 사이를 막지 않으려 bypass를 먼저 채택하고 경계는 공용 제약 레이어에서 건다(사용자 결정: 지금 auto, 경계 나중).

## 근거
- **실측:** CLI 발신 0/38(default-deny, grant 미매칭·권한 분류기 차단) → ADR-0096 정렬 후 10/10(cli). auto mode 하 라이브 roundtrip 발신 성공 실측. `bypassPermissions`가 claude CLI 정식 모드값이며 헤드리스에서 non-granted Bash를 실제 실행함을 로컬 실측(echo 통과).
- **게이트:** `/review code full` — doc-aware(Claude) FIX 1건(types.rs stale doc) 반영 + blind(codex) PASS. `/qa standard` 전 게이트 green.

## 영향 / 불변식
- **`--permission-mode bypassPermissions` = args 첫 base 플래그**(변경 금지 위치 규칙): variadic `--allowedTools` 그룹은 여전히 **맨 끝**이어야 한다(ADR-0094 positional 흡수 방지). pair는 비-variadic이라 앞에 둔다. 회귀 가드 = `claude_auto_permission_mode_precedes_allowed_tools_*` + 기존 `claude_allowed_tools_group_is_last_*`.
- **측정 seam 함의(중요):** `ENGRAM_DISALLOW_MCP_SEND`/`--disallow-mcp`는 MCP **grant**만 제거한다. bypass 하에선 권한 게이트가 없고 mcp-config 서버 연결은 남으므로 send_message가 여전히 호출 가능 — **auto mode에선 이 seam이 "MCP 부재"를 시뮬레이트하지 못한다.** 순수 CLI-only/채널 강제 측정은 default-deny(ADR-0094 체제)에서만 유효하다. auto mode에서 채널 선택 = 프라이밍 결정(강제 없음). (0/38→10/10 Phase-3 수치는 default-deny 세계 측정 = grant 배관 검증으로 유효.)
- **인젝션 표면 상향:** 팀메이트 메시지가 입력으로 꽂히는 구조에서 bypass 에이전트는 설득당하면 경계 안에서 뭐든 한다. 봉투 검증(발신자 위조 차단)·프라이밍 "자기판단 유지"는 확률적 방어일 뿐 — **안전 상한 = 미래 경계(containment) 품질**. 그 전까지는 신뢰 환경 전용으로 운용.
