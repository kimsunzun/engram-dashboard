# ADR-0094: S17 에이전트 간 발신 권한 — 런타임 최소권한 pre-authorization (grant seam)

- 상태: 확정 (2026-07-20, 근거: 사용자 결정 + /research medium(cross-family 리뷰) + C0~C3 실측)
- 관련: ADR-0093(왕복 실험·C0~C3) · ADR-0092(수신 계약·프라이밍) · ADR-0086(듀얼 입구·토큰 파생 from) · ADR-0004(백엔드 지식 격리) · `crates/engram-dashboard-core/src/agent/backend/claude.rs`(번역) · `crates/engram-dashboard-core/src/agent/types.rs`(ControlEndpoint) · daemon 컨트롤 채널(grant 소유) · step-log S17

## 맥락
왕복 실험(ADR-0093, C0~C3)에서 **전 케이스 B_SENT=false**. 진단(B의 seed-후 턴 캡처)으로 원인 규명: claude 런타임 툴-권한 게이트가 스폰 에이전트의 발신 툴 호출(`send_message` MCP / `engram-send` CLI)을 차단(승인자 없는 헤드리스). B는 메시지를 정상 수신하고 답할 의사·발신법도 있으나 "사람 승인 필요"로 멈춤. `backend/claude.rs` 스폰 args엔 권한 플래그가 전무했다. `/research`(medium, cross-family codex 리뷰 VERDICT FIX로 세부 정정)로 확증: **Claude Code 권한은 런타임 강제 — 프롬프트·CLAUDE.md로 self-grant 불가**(공식 문서). 발신을 열려면 런타임 pre-authorization이 필수.

## 결정
1. **발신 권한 = 런타임 pre-authorization**(프롬프트 아님). 스폰 시 발신 입구 툴을 allowlist에 명시한다. 프롬프트 pre-auth는 보조(B가 발신을 시도하게)일 뿐 enforcement가 아니다.
2. **최소권한** — 발신 입구만 pre-authorize(`send_message` MCP + `engram-send` CLI). 나머지 툴은 기본 게이트 유지. 전부-허용(`--dangerously-skip-permissions`/bypassPermissions) 아님.
3. **정책 파일 없음** — 발신은 메시지 시스템의 상수(항상 켬)라 릴리즈에서 고칠 변수가 없다. 코드에 박는다(외부화 YAGNI — 능력이 가변이 되면 그때 seam).
4. **grant = 컨트롤 채널이 단일 출처** — `ControlEndpoint.grants: Vec<ToolGrant>`(추상: `Mcp{server,tool}` / `Cli{exe}`)를 컨트롤 채널이 자기 입구 정의 옆에서 채운다. 백엔드는 **형식만 번역**: claude → `--allowedTools mcp__{server}__{tool}` + `Bash({exe} *)`. codex/gemini는 같은 grants를 자기 방언으로(현재 stub·TODO). 툴 이름·플래그 문법은 코드(claude 지식 = claude.rs, ADR-0004)에만.

## 거부한 대안
- **프롬프트-only pre-authorization** — 권한은 런타임 강제라 모델이 self-grant 불가. C1~C3서 프라이밍이 발신법을 알려줘도 B는 게이트에 막혀 "허가 필요"로 멈춤(실측). 프롬프트는 의사결정 입력이지 enforcement가 아님.
- **전부-허용(bypassPermissions / --dangerously-skip-permissions)** — 프롬프트 인젝션 무방비(조사 경고 — 격리 환경 전용). 왕복엔 발신만으로 충분하므로 과도.
- **외부 정책 파일** — 발신이 상수(항상 허용)라 외부화할 변수가 없다. 프라이밍(가변 프롬프트 텍스트)과 달리 릴리즈 편집 수요가 없어 YAGNI.
- **툴 이름을 claude.rs에 하드코딩** — 입구 정의(컨트롤 채널)와 물리적으로 분리돼 이름 변경 시 두 곳 손대야 하고 rot. grant seam으로 단일 출처화(이름은 컨트롤 채널, 형식만 백엔드).

## 근거
- `/research` medium(주계열 3갈래 + grounding + cross-family codex 적대 리뷰): Claude Code 권한 런타임 강제(공식 문서 "permissions는 프롬프트/CLAUDE.md로 안 바뀜") · MCP 툴 `mcp__<server>__<tool>` 네이밍 · `Bash(exe *)` 문법(전체 Bash 아님) · `--allowedTools`가 해당 호출 pre-approve 확인. 리뷰가 정정한 것 = 프레임워크 분류 과도·인용 통계 1건 오기재(방향 결론 불변).
- **우리 실측(ADR-0093 C0~C3)**: 프라이밍 유무·채널 무관하게 B가 런타임 게이트에 막힘 — 런타임 pre-auth가 유일 해결이라는 조사 결론과 독립 교차확증.

## 영향 / 불변식
- `ControlEndpoint`에 `grants` 추가(core type) · daemon 컨트롤 채널이 자기 입구 정의 옆에서 채움 · 각 backend가 자기 CLI 문법으로 번역. **claude만 지금 구현, codex/gemini TODO**(stub — CLI 스파이크 후).
- **최소권한 불변** — 발신 입구만 열고 나머지 툴 게이트 유지. 여기에 툴을 추가하려면 명시적 결정(이 ADR 개정). 전부-허용으로 넓히지 않는다.
- **단일 출처 불변** — 발신 툴 이름(`send_message`·`engram`·`engram-send`)의 정본은 컨트롤 채널 입구 정의. claude.rs는 형식 규칙(`mcp__{s}__{t}`/`Bash({e} *)`)만 안다 — 이름을 재타이핑하지 않는다.
- 검증 = 왕복 재실행(pre-auth 후 발신-프라이밍 케이스 B_SENT=true 실측). 프롬프트 pre-auth 보조가 추가로 필요한지도 그때 실측으로 판정.
