# ADR-0085: CLI 백엔드 제어 채널 = in-band 출력 마커(M3) — engram-ctl 폐기

- 상태: **폐기 (Superseded by ADR-0086)** — 두 독립 조사(메인 /research 팬아웃 + 사용자 별도 Codex 조사)가 수렴: 자유 텍스트 마커 stdout 스크랩을 주채널로 채택한 사례 0건 + 후속 스파이크에서 컴플라이언스 66%(따옴표 body 깨짐·nonce 오기재). 제어 채널은 듀얼 typed 입구(MCP+CLI) + SQLite 메일박스로 전환. ~~확정 (2026-07-15, 근거: 사용자 결정(보안·속도) + claude 2.1.170 스파이크 실측(마커 38/38) + 라이브/리플레이 경계 코드 확인 · Supersedes ADR-0080)~~
- 관련: Supersedes ADR-0080(engram-ctl ingress 폐기) · CLAUDE.md §5(LLM-우선 제어)·아키텍처 §2(capability matrix) · ADR-0002(capability = transport ⊕ backend)·ADR-0081(UI opaque-relay 존속)·ADR-0079(resume seed 경계)·ADR-0003(core 격리) · 결정 노트 `docs/process/S17-llm-control-surface/control-channel-deliberation-m3.md` · step-log S17

## 맥락
에이전트 A→B 메시지(및 향후 UI·관찰 제어)의 **채널**을 정해야 한다. ADR-0080은 이 채널의 ingress를 `child claude의 Bash 도구 → 컴파일 Rust engram-ctl → 데몬 WS(토큰 auth)`로 잡았다(제안 상태, 미확정). 사용자가 두 가지를 제기했다:

1. **보안** — 에이전트 env에 토큰·포트를 쥐여주면 prompt-injection된 child가 그 자격증명으로 형제 에이전트를 조작(kill·write·UI)할 수 있다(공격면 확대). in-band면 에이전트에 아무 자격증명도 안 준다.
2. **속도** — 운영이 **항상 stream-json 모드 + 고빈도 메시징**인데, 호출마다 프로세스 스폰 + WS 핸드셰이크는 결정적 손해다.

핵심 관찰: **CLI(Claude Code) 백엔드에서 데몬은 이미 에이전트의 stdout을 소유**한다(출력 파이프의 단일 소비자). 그러면 에이전트가 출력 텍스트에 제어 마커를 찍고 데몬이 그걸 줍는 게 가장 짧은 경로다 — 토큰 0, 프로세스 스폰 0.

## 결정
**CLI(Claude Code) 백엔드의 제어 채널 = in-band 출력 마커(M3).** 에이전트가 stream-json 출력에 단일라인 센티넬 마커를 찍으면 → 데몬이 이미 소유한 출력 스트림에서 검출 → 데몬이 mail처럼 배달(대상 stdin write). engram-ctl CLI·토큰·WS ingress는 폐기한다.

**백엔드별 구현 분담(capability matrix, ADR-0002)** — "에이전트가 제어 신호를 낸다"는 같은 능력을 백엔드마다 다르게 구현:
- **CLI(Claude Code) = M3 출력 마커** (데몬이 stdout 소유 → 마커 줍기).
- **API/SDK = 직접 tool 콜백** (우리가 추론 루프를 소유 → tool_use 직접 수신 = 진짜 인프로세스 콜백).
- **M4(Bash sentinel + hook) = 문서화된 폴백** (터미널 모드·json 포맷 붕괴 헤지). 지금 구현하지 않는다.

**M3 배달 모델 = 단방향 fire-and-forget** — A→데몬만. 배달·보관은 데몬 소관이고 발신 에이전트에 반환값은 없다. 받은편지함·ACK·영속은 별도 후속(비목표).

**세부 설계(마커 프로토콜 스키마·펜스스킵 파서·`OutputEvent::ControlSignal`·데몬 라우팅·표시 억제)는 후속 설계·ADR로 넘긴다.** 이 ADR은 채널 기제(M3)와 engram-ctl 폐기만 박제한다.

## 거부한 대안
- **engram-ctl CLI (ADR-0080, M1)** — 에이전트 env에 토큰 노출(injection된 child의 형제 조작 공격면) + 호출마다 프로세스 스폰 + WS 핸드셰이크(고빈도 손해). CLI 백엔드에선 데몬이 이미 stdout을 소유하므로 별도 WS ingress는 우회로다.
- **MCP over stdio/HTTP (M2)** — 툴 스키마가 에이전트 컨텍스트에 상주(~0.5~1.5k 토큰/에이전트) + 엔드포인트 인증·권한 게이트. ADR-0080이 이미 "로컬 단일 앱엔 ceremony 과함"으로 보류한 방향. 명령 카탈로그가 커지거나 "넓은 Bash 대신 좁은 툴 권한"이 중요해지면 재검토(보류이지 영구 거부 아님).
- **hook-as-primary (M4)** — hook은 인프로세스 콜백이 아니라 claude가 띄운 스크립트가 밖으로 릴레이하는 **간접** 방식 + 데몬 인그레스 신설 필요 + 메시지마다 툴콜 비용. 백업(문서화 폴백)으로만 남긴다.
- **파일 drop (M5)** — 견고하나 매 호출 프로세스 스폰(고빈도 손해).
- **펜스블록 마커** — 실측에서 에이전트가 포맷을 *설명*할 때 코드펜스 예시 안에 마커를 찍었다(4/5). 펜스블록은 실물 마커와 예시 마커를 구분 못 해 오히려 나쁘다 → **단일라인 센티넬 + 펜스스킵 파서** 채택.

## 근거
- **실측(claude 2.1.170, throwaway 스파이크 `.spike-control/` — git 미추적, 삭제됨):** 엄격 단일라인 지시 시 마커 라인 **38/38 완벽**(유효 JSON·수신자/본문·이스케이프 정확). 유일 함정 = 포맷을 *설명*할 때 4/5로 펜스 예시 안에 마커를 찍음 → 펜스스킵 파서로 방어.
- **라이브/리플레이 경계 코드 확인:** resume seed는 `output_core.seed()`로 fanout 없이 적재, 라이브만 `emit()` 경유 → 마커 파서는 리플레이/transcript seed를 절대 안 봄(부작용 재실행 없음). ADR-0079 정합.
- **보안·속도(사용자 결정):** 토큰 0·스폰 0으로 공격면↓ + 고빈도 메시징 무오버헤드. in-band 방식이 engram-ctl 대비 두 축 모두 월등.
- OSS·grounding 근거는 ADR-0080에 상세(그 조사는 여전히 유효 — engram-ctl ingress 결정만 뒤집힘).

## 영향 / 불변식
- **폐기 범위 = engram-ctl ingress 한정 (오독 주의).** ADR-0080이 정의한 **UI opaque-relay·권위 2도메인(백엔드=데몬 직행 / UI=앱 ViewManager)은 ADR-0081(확정)로 존속**한다. M3는 제어 채널의 *ingress 기제*만 바꾼다 — 데몬이 신호를 받은 뒤의 라우팅·권위 분리는 유지된다. "ADR-0080 폐기"를 "UI relay도 죽었다"로 읽지 말 것.
- **core 격리(ADR-0003):** 마커 검출은 core(디코더)에서 **이벤트 방출까지만**, 라우팅은 데몬. core에 tauri/데몬 import 금지.
- **마커 파싱은 라이브 stdout에만.** 리플레이/transcript seed 경로엔 걸지 않는다(부작용 재실행 방지 — seed는 애초에 fanout 안 함, ADR-0079).
- **펜스스킵 필수.** 코드펜스 안 라인은 무시한다(실측 false-trigger 방어). 마커 = 단일라인 센티넬(펜스블록 금지).
- **engram-ctl 재생성 금지.** 크레이트·스파이크는 삭제됨. CLI 제어 = M3, API 제어 = 직접 콜백.
- **후속(미확정, 이 ADR 밖):** 마커 프로토콜 스키마·상한 · `OutputEvent::ControlSignal{target, message}` 신설 · 데몬 mail 세부(배달 실패/대상 부재/발신자 표기) · PRD/TRD §6(engram-ctl 전제) 갱신 + Unit 재설계.
