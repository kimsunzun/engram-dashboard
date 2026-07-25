# ADR-0106: 스폰 툴 deny 레이어 — 내장 SendMessage 차단(제어 채널 스폰 한정)

- 상태: 확정 (2026-07-26, 근거: roundtrip 라이브 진단 + 리뷰 스코프 지적 — 아래 근거 절)
- 관련: ADR-0094(발신 입구 allowlist pre-authorization — 이 ADR이 **deny 축을 추가**) · ADR-0097(스폰 argv 위치 규칙 — deny쌍이 규칙에 추가됨) · ADR-0099(채널 스위치) · ADR-0092(프라이밍)

## 맥락

스폰된 claude가 메시지를 보낼 때 우리 MCP 툴 `send_message`(소문자, `engram` 서버)가 아니라 **Claude Code 내장 `SendMessage` 툴**(하네스 자체의 에이전트 간 메시징, PascalCase)을 잘못 집는 사례가 라이브에서 재현됐다(2026-07-26 roundtrip 진단 — 내장 툴은 engram 에이전트를 몰라 "No agent named 'X' is reachable" 에러, 폴백 서술이 없으면 발신 실패로 종결). 이름 유사성에 의한 비결정적 오발이라 프라이밍 문구만으론 막을 수 없다.

## 결정

1. **deny 레이어 신설:** claude 스폰 argv에 `--disallowedTools SendMessage`를 주입해 내장 툴을 차단한다. deny 규칙은 bypassPermissions 모드에서도 강제됨(공식 문서 — 프롬프트 생략이지 deny 무시가 아님)을 확인했다.
2. **스코프 = 제어 채널(메시징) 스폰 한정.** ControlEndpoint가 있는 스폰(= 메시징 프라이밍을 받는 에이전트)에만 주입한다. 일반(비메시징) 스폰은 무변경 — 내장 기능 유지 + 미등록 툴 deny 시 CLI 스타트업 경고 비용 회피.
3. **argv 위치 규칙(ADR-0097 확장):** `extra_args` 뒤 · `--allowedTools` 그룹(항상 마지막) 앞에 deny쌍을 둔다. variadic 값 목록은 다음 플래그(`--allowedTools`) 또는 argv 끝으로 종결 — 어느 빌드 경로에서도 뒤 인자를 삼키지 않음을 골든 테스트로 고정.

## 거부한 대안

- **프라이밍 문구 강화만(차단 없음)** — 라이브에서 두 에이전트 모두 오발 후 폴백 서술로 자기교정하는 걸 관측했으나 비결정적(QA 런에선 폴백 없이 발신 실패). 이름 충돌은 구조로 막아야 한다.
- **전 스폰 무조건 차단** — 메시징과 무관한 대화용 에이전트까지 내장 기능을 잃는 사용자 체감 정책 변화 + 내장 툴이 없는 CLI 빌드에서 스폰마다 deny 경고 출력(리뷰 적출). 충돌이 문제 되는 스코프(메시징 스폰)만 막는 게 최소 표면.
- **내장 SendMessage를 그대로 두고 MCP 툴 개명(`engram_send` 등)** — 툴 3개 상한·기존 프라이밍/grant/실측이 `send_message`에 정렬돼 있고(ADR-0094 세 문자열 정렬), 개명은 그 정렬 전부를 다시 실측해야 함. 이름 소유권을 지키는 쪽이 싸다.

## 근거

- 라이브 진단(2026-07-26): roundtrip 재현 런에서 alice·bob 모두 내장 SendMessage 오발 → "No agent named … is reachable"(repo grep 0건 = 외부 문자열) → engram-send 폴백으로 자기교정. QA 런에선 bob이 폴백 없이 실패.
- deny의 실효: Claude Code 권한 문서 — deny 규칙은 모든 모드(bypassPermissions 포함)에서 강제. 규칙 매칭은 정식 이름 exact라 `mcp__engram__send_message`엔 닿지 않음(리뷰 검증).
- 실효 라이브 실측(구현 QA full): 차단 스폰에서 내장 SendMessage 거부 + MCP send_message 정상 + 스타트업 경고 없음 확인 항목.

## 영향 / 불변식

- **deny쌍 위치 불변식:** `extra_args` < `--disallowedTools SendMessage` < `--allowedTools`(항상 마지막, ADR-0097). 재배열 금지 — variadic 삼킴 방지.
- **스코프 불변식:** ControlEndpoint 없는 스폰엔 deny쌍 없음(골든 테스트 고정). 스코프 확대는 사용자 결정.
- 프라이밍(ADR-0092)은 차단 후 실패 양상(권한 거부)에 맞게 문구 정합 + 폴백 트리거는 "모든 발신 실패"로 일반화.
- 코드 앵커: `backend/claude.rs`의 주입 지점에 `// ADR-0106`.
