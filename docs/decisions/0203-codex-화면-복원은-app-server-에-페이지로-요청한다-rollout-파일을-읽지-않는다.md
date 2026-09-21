# ADR-0203: codex 화면 복원은 app-server 에 페이지로 요청한다 — rollout 파일을 읽지 않는다

- 상태: 확정 (2026-09-15, 근거: 사용자 결정 + 실 codex app-server 0.154.0 실측 + rollout 901개 전수 스캔) · 부분 폐기 by ADR-0204 (복원 이벤트가 붙는 자리)
- 관련: ADR-0079(데몬이 claude `.jsonl` 을 읽어 버퍼에 seed) · ADR-0008(추적 파일로 기능 확장 금지) · ADR-0172(스크롤백 읽기와 기능 잠그기는 등급이 다르다) · ADR-0004(백엔드 지식 격리) · `backend/mod.rs::AgentBackend::resume_transcript_events` · Amended by ADR-0204 (복원 이벤트가 붙는 자리)

## 맥락

이어받기가 성공해도 codex 슬롯은 **빈 화면으로 시작한다.** codex 는 복원 이벤트를 우리에게 주지 않고, 우리도 읽는 것이 없다.

claude 는 같은 문제를 ADR-0079 로 풀었다 — `~/.claude/projects/<슬러그>/<sid>.jsonl` 을 우리가 직접 읽어 버퍼에 seed 한다. 사용자의 1차 방침은 「유지보수 비용이 없으면 claude 와 맞춘다」였고, 그래서 codex 도 rollout 파일을 직접 읽는 쪽으로 기울어 있었다.

**그 전제를 재 봤더니 깨졌다.** claude 가 싼 이유는 파일 줄 모양이 라이브 스트림 줄 모양과 **같아서** 파서를 새로 안 짰기 때문이다. codex 는 그렇지 않다.

## 결정

**codex 이력은 app-server 에 요청해서 받는다.** rollout 파일을 읽지 않는다.

- `thread/resume` 은 `excludeTurns: true` 로 보내고, 이력은 **페이지 단위로** 따로 받는다(`thread/items/list` — 커서·limit·방향). resume 응답이 되돌려 주는 역방향 커서가 진입점이다.
- 받은 항목은 **우리가 이미 파싱하는 타입 그대로**다. 봉투만 벗기면 기존 번역을 재사용한다.
- 붙는 자리는 claude 와 같다 — 백엔드가 복원 이벤트를 내주고 manager 가 자식 띄우기 **전에** seed 한다(ADR-0079 의 순서 불변식 유지).

## 거부한 대안

- **rollout 파일을 우리가 직접 읽는다(claude 와 같은 모양)** — 사용자의 1차 선택이었고 **실측으로 기각**됐다. rollout 줄은 우리 codex 디코더 입력과 **겹치는 것이 하나도 없다**: 봉투가 JSON-RPC 가 아니고(최상위 `method` 자체가 없어 분류기가 전량 탈락시킨다), 알림 이름이 snake_case 대 slash/camel, item 종류가 PascalCase 대 camelCase. 세 층이 각각 독립으로 어긋난다. 재사용되는 것은 말단 문자열 헬퍼와 출력 타입뿐이라 **새 파서 + 새 어휘표**가 통째로 생긴다. 게다가 그 파일은 계약된 wire 가 아니라 **벤더 내부 저장 포맷**이라 바뀌면 조용히 깨진다.
- **이력을 한 번에 전부 받는다(hydration)** — 벤더가 응답에 `deprecationNotice` 를 실어 직접 말린다: "Full-history hydration is deprecated for paginated threads; use `excludeTurns: true`, then page". 그리고 한 줄이 **최대 1.96MB**(실측)라 우리 줄 상한 4MiB 에 여유가 두 배뿐이다.
- **그대로 둔다(빈 화면)** — 이어받기의 값어치 절반이 화면에 있다. 사용자가 명시적으로 기각.

## 근거

실 `codex app-server` 0.154.0 을 띄워 JSON-RPC 로 직접 재 봤다(2026-09-15):

- **`excludeTurns` 는 존중된다.** 생략·`false` 는 바이트 단위로 동일한 응답(1,259,774B, 120 items)을 주고, `true` 는 6,486B 에 `turns: []` 다. 우리 코드가 그것을 믿고 짠 것은 맞았다.
- ★**항목 타입이 라이브와 같다**★ — 벤더 스키마에서 `item/completed` 알림·resume 응답·페이지 응답의 항목 정의 해시가 **전부 동일**(19 variants). 다른 것은 봉투뿐이다.
- **페이지 수단이 둘 있다**(`thread/items/list`·`thread/turns/list`, 불투명 커서, `nextCursor: null` 로 정상 종료 확인).
- 이력은 **전부 응답 안에** 있다 — resume 뒤 `item/*` 알림은 0건.
- 크기: resume 페이로드는 디스크 rollout 의 0.25~0.4배. ★**페이징으로도 줄 크기가 안 잡힌다**★ — `limit` 은 개수를 자르지 개별 바이트를 자르지 않아서, `limit:1` 에서도 최대 페이지가 437KB 다(`commandExecution.aggregatedOutput` 하나가 그만큼).

ADR-0008 저촉 여부도 기록으로 확인했다 — 그 금지의 대상은 `sessions/<pid>.json` 한 파일이고 취지는 「복원 정확성을 벤더 파일에 걸지 말라」다. transcript 읽기는 ADR-0079 가 정면으로 채택했고, 등급 차이는 ADR-0172 가 평문으로 적어 뒀다("스크롤백 되살리기에 읽는 것과 기능을 잠그는 것은 등급이 다르다"). 즉 **직접 읽기도 금지는 아니었고**, 이 결정은 금지가 아니라 **비용**으로 갈렸다.

## 영향 / 불변식

- 백엔드 지식은 구현체 안에 남는다(ADR-0004) — manager 는 호출만 하고 페이지·커서·항목 모양을 모른다.
- ★**화면 버퍼가 진짜 천장이다**★ — 링은 2MB/4096건이라 그보다 오래된 것은 받아도 안 실린다. 끝에서부터 그만큼만 가져오면 되고, 줄 상한을 올릴 이유가 아니다.
- **서브에이전트 스레드는 resume 자체가 거절된다**(실측: 902개 중 93개). 이력 요청도 같은 제약을 받는다.
- `docs/process/S21-codex-backend/trd-phase2a.md` 가 벤더 파일 읽기 셋(`state_5.sqlite`·rollout JSONL·`session_index.jsonl`)을 한 줄로 묶어 금지처럼 적어 둔 것은 **등급을 안 가른 서술**이다. 이 ADR 이 그 자리의 정본이다.
- 이 결정은 **아직 구현되지 않았다** — 번호를 먼저 박은 것이고(워크트리 다중 운용), 구현은 별도 스텝이다.
