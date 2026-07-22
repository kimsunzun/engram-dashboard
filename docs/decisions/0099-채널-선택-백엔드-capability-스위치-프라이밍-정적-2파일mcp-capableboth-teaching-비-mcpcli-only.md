# ADR-0099: 채널 선택 = 백엔드 capability 스위치 + 프라이밍 정적 2파일(MCP-capable=both-teaching / 비-MCP=CLI-only)

- 상태: 확정 (2026-07-22, 근거: 사용자 결정 + 채널 지연·프라이밍 결정력 실측 — 아래 근거)
- 관련: ADR-0030/0002(capability 매트릭스) · ADR-0004(백엔드 지식 격리) · ADR-0086(듀얼입구·단일 wrap) · ADR-0097(auto mode — 채널 선택=프라이밍) · ADR-0098(bare-name grant) · ADR-0093(실험 케이스 매트릭스 — "문구 확정 시 정리" 예고) · `crates/engram-dashboard-daemon/src/control/mod.rs::build_grants` · `crates/engram-dashboard-daemon/src/control/priming.rs` · `crates/engram-dashboard-core/src/agent/backend/claude.rs` · step-log S17

## 맥락
채널 정책은 확정됐으나("MCP 있으면 MCP, 없으면 CLI 폴백" — 사용자, 2026-07-22) 그걸 **어떻게 강제하느냐**가 미정이었다. 현 배관 실측:
- auto mode(ADR-0097, bypassPermissions) 하에서 grant는 NO-OP 게이트 — 유일한 실질 강제력은 **물리 provisioning**(안 깐 채널은 못 쓴다)이고, 깐 것들 사이의 선택은 **프라이밍이 결정**한다(실측: MCP가 보여도 CLI 프라이밍이면 CLI 사용).
- MCP 부착(mcp-config)은 prod에서 무조건이었고, 프라이밍은 고정 단일 파일(`FilePrimingProvider` → `prompts/agent-priming.md`)로 **변형 선택 배관이 없었다**. 실험 변형(v3-en-cli/-both)은 파일만 존재·미배선.
- 비-MCP 백엔드(codex/gemini 등 미래)엔 MCP 입구 자체가 성립하지 않는데, 단일 프라이밍은 send_message를 가르친다 — 지시-도구 불일치는 실측상 발신 freeze를 유발한다.

## 결정
**백엔드 capability 하나가 물리 배선·프라이밍·grant를 전부 구동한다:**

1. **capability 선언 = 백엔드 지식**(ADR-0004): "MCP config를 받을 수 있는 백엔드인가"는 `backend/`(claude=true)가 선언하고, `DaemonControlChannel::provision`·`build_grants`·프라이밍 선택이 소비한다.
2. **MCP-capable 스폰**: mcp-config 부착 + engram-send 주입(PATH·env) 유지 + 프라이밍 **A(both-teaching)** + grants `[Mcp, Cli]`.
3. **비-MCP 스폰**: mcp-config **미부착(물리)** + engram-send만 + 프라이밍 **B(CLI-only)** + grants `[Cli]`.
4. **프라이밍 = 정적 파일 2개** — `prompts/agent-priming.md`(A = v3-en-both 승격: send_message 주력 + engram-send 폴백) · `prompts/agent-priming-cli.md`(B = v3-en-cli 승격: engram-send만, send_message 단어 자체 부재). 런타임 조립·치환 없음 — 스폰 시 둘 중 한 경로를 `--append-system-prompt-file`로 전달.
5. **실험 변형 정리**: `prompts/experiments/` 프라이밍 변형 삭제(git 이력 보존, ADR-0093이 예고한 정리 실행) + roundtrip_smoke의 C1~C3 파일 별칭 제거(`--priming <직접 경로>`는 유지).

## 거부한 대안
- **hard 단일채널(MCP-capable엔 MCP만 — engram-send PATH·env 미주입)** — airtight한 결정성은 얻지만 in-spawn 폴백을 잃는다. auto mode 자체가 신뢰환경 전제의 임시 체제(ADR-0097)라 airtight의 실익이 없고, both-teaching이 MCP 준수를 해치지 않음이 실측돼(10/10) soft로 충분하다.
- **동적 용어집/템플릿 치환(단일 파일 + 채널 단어만 교체)** — 파일이 11~14줄이고 갈리는 건 "Replying to teammates" 한 섹션뿐인 규모에서, 렌더→임시파일 materialize 단계(mcp-config 쓰기 경로의 수명·청소·실패모드 복잡성 복제)만 추가된다. **재검토 트리거**: 변형 축이 2개 이상으로 확장(언어×채널×백엔드)되거나 3번째 변형이 필요해질 때.
- **스위치 제거·CLI 보편화(MCP 입구 폐기)** — 두 입구가 데몬 한 곳에 수렴하므로 배달은 동일하나, standing 연결(메시지당 프로세스 스폰 0)의 MCP 이점을 포기하고 확정 정책("MCP 있으면 MCP")에 반한다.

## 근거
전부 실측(2026-07-22, step-log S17 "채널 스위치 설계 착수" 항목):
- **CLI 지연은 결정 변수 아님**: `engram-send.exe` 스폰+시작 warm ~33ms(PowerShell 직접)/~110–130ms(bash 내), Bash 툴 셸 스폰 합산 전체 체인 추정 ~200–500ms. 툴 실행은 추론과 직렬(가산)이지만 발신당 추론 수 초 대비 몇 % 수준.
- **프라이밍의 채널 결정력(auto mode)**: auto-cli 8/8 cli · auto-mcp 3/3 mcp · auto-both 3/3 mcp — MCP가 노출돼 있어도 프라이밍이 가리키는 채널을 따랐다.
- **both-teaching 무회귀**: both+MCP 10/10(default-deny 체제).
- **지시-도구 불일치 freeze**: MCP 노출 + CLI-only 지시 = ~6/7 미발신 → 안 깐 채널은 프롬프트에서 완전히 삭제해야 한다.
- **수렴 배관**: 두 입구 모두 데몬 `/control/send` 단일 핸들러·wrap 1회(ADR-0086 재확인) — 채널 선택은 가용성 문제일 뿐 배달 정확성과 무관해, soft 스위치의 실패 비용이 낮다.

## 영향 / 불변식
- **프라이밍-배선 정합 불변식**: 스폰에 물리적으로 깐 채널 집합 = 프라이밍이 가르치는 채널 집합. 어기면 발신 freeze(6/7 미발신 실측)가 재발한다. 새 백엔드·새 채널 추가 시 이 쌍을 함께 움직일 것.
- **grants는 채널별로만 방출** — 지금은 bypass라 NO-OP지만, 미래 공용 제약 레이어(step-log 백로그, auto mode의 정식 대체)가 붙으면 하드 게이트로 재활성된다. 그때 프라이밍 파일 외엔 추가 변경이 없도록 이 정렬을 유지한다.
- 코드 앵커 대상: `backend/claude.rs`(capability 선언·mcp-config 조건 분기) · `control/mod.rs::build_grants`(채널별 방출) · `control/priming.rs`(변형 선택) — 구현 시 `// ADR-0099` 앵커.
