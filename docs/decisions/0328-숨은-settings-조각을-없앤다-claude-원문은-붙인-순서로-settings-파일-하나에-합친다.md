# ADR-0328: 숨은 settings 조각을 없앤다 — claude 원문은 붙인 순서로 `--settings` 파일 하나에 합친다

- 상태: 제안 (2026-10-31, 근거: 사용자 결정 T10(2026-10-18 「프리셋으로 해줘. 사실 감추는 설정이 있으면 안되지.」 · 「이어붙이기 진행」))
- 관련: Amends ADR-0109 (결정 9의 settings 조각 부분) (아래 「바뀌는 조항」) · ADR-0099(mcp-config 수명주기) · ADR-0327 · ADR-0316 · PRD R32 · TRD §1-2 · §1-5 표 · §3 합침 규칙 · §7 19

## 맥락
지금 제어 채널은 스폰마다 `{"allowedMcpServers":[{"serverName":"engram"}]}` 단일 키 조각 파일을 `--settings` 로 싣는다(ADR-0109 결정 9 · `control/mcp_config.rs`). S25 는 프리셋의 claude 원문 설정도 `--settings` 로 싣는다.

## 결정
1. 숨은 settings 조각이 없다. `allowedMcpServers:[{serverName:"engram"}]` 와 `DISABLE_AUTOUPDATER` 는 기본 제공 「기본」 의 claude 원문(`engram:/settings/default`)에 보이게 든다.
2. 붙은 프리셋(「기본」 포함)의 claude 원문을 붙인 순서로 휘발 폴더 `settings.json` 하나에 합쳐 `--settings` 하나로 싣는다. 합침 = 값은 나중 것 · 객체는 재귀 · 목록은 이어 붙이고 같은 값은 하나 · 키마다 다른 규칙 없음 · Engram 우선 규칙 없음. 합치는 쪽은 backend 설정 검사 포트(ADR-0334).
3. 실 재료면 제어 채널 조각(`ControlEndpoint.settings_file`)을 싣지 않는다. `NoopLaunchKits` 는 지금 그대로 싣는다(argv 바이트 같음).
4. `--mcp-config` 는 범위 밖이다(실행 인자 그대로 · 프리셋 아님).

**바뀌는 조항 (ADR-0109 결정 9):**
- 바뀜: 「조각은 `allowedMcpServers` 단일 키 유지(잡동사니화 금지 — 회귀 테스트로 고정)」 → 조각이 없고 그 키는 「기본」 원문으로 옮긴다. 「mcp-config 디렉토리 · 스윕 · revoke 수명주기에 동승」 → 합친 파일은 실행 재료 휘발 폴더(`daemon\run\launch\` — TRD §1-4)에 산다.
- 그대로: 「`--settings` = 파일 경로 주입(인라인 JSON 아님)」 과 그 사유(`cmd /c` 따옴표 붕괴).
- **원천에 없음(확인 필요)**: 「쓰기 실패는 warn+계속」 이 새 판에서 어떻게 되는지.
- Noop 길은 걷힐 때까지 결정 9 그대로 돈다.

## 거부한 대안
- **숨겨 주입하는 조각** — 사용자 「사실 감추는 설정이 있으면 안되지」(T10).
- **`allowedMcpServers` 는 Engram 값이 권위**(옛 추천 — 프리셋의 그 키는 버리고 경고) — 사용자가 숨은 설정 없음을 골랐다(T10).
- **목록 갈아 끼우기** — 사용자 「이어붙이기 진행」. 귀결: 뒤 프리셋이 `engram` 을 못 뺀다 — 받아들였다.
- **두 번째 `--settings`** — 동작이 문서에 없고 재지 않았다.
- **원문을 안 싣기**(프리셋의 claude 원문을 싣지 않기 — 2026-10-15 TRD 첫 판부터 이름만 적힌 갈래) — 근거(메인 2026-10-31 — 보고): 설정 에셋은 백엔드별 원문 칸이고 claude 칸은 `--settings` 로 넘긴다(PRD R5 — 사용자 2026-10-18) — 싣지 않으면 claude 에이전트에겐 설정 에셋이 하는 일이 없다. 또 T10 이 Engram 통로 키를 「기본」 의 claude 원문으로 옮겼으므로 원문을 안 실으면 그 키가 빠지거나 숨은 조각으로 돌아가야 한다(T10 — 사용자 「사실 감추는 설정이 있으면 안되지」).

## 근거
사용자 결정 T10(2026-10-18) · TRD §2-2 T10.

## 영향 / 불변식
- 「기본」 을 떼면 Engram 메시지 · 제어 통로가 빠질 수 있다(사용자 전역 설정이 `allowedMcpServers: []` 로 막을 때) — 떼기 확인 문구가 그것을 말한다(R32 · `DetachDefaultConfirm`).
- 합친 claude settings 도 검사한다 — 오류는 그것을 들인 나중 에셋 줄에 붙는다(M14).
- ADR-0109 의 앵커 `mcp_config.rs`(settings 조각)는 Noop 길에만 남는다.
