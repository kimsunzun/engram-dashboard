# 핸드오프 — dashboard-Sub: 제어표면/fleet 선행조사 + 문서 네비게이션 규약(미적용) + codex MCP 입주

작성 2026-06-21 (**dashboard-Sub 세션** = dashboard10의 질문을 대신 받아 답하는 *응대/조사 세션*. dashboard10과 **같은 working tree 공유**). 본문(`docs/`·`CLAUDE.md`)이 항상 우선. **master HEAD=`aacb0f2`(dashboard10 코드, 커밋됨). 내 산출=docs 3건 uncommitted(의도적 분리).**

## ★ 이 핸드오프의 목적 ★
이 세션은 코드를 안 짰다. **선행조사·문서구조·환경설정**만 했다. 사용자가 **새로 입장한 Codex + Claude에게 아래 §4 "리뷰 대상"의 의견을 종합**시키려 한다. 리뷰어는 §4를 적대적으로 검토하라(칭찬 말 것).

## 0. 한 줄 요약
백엔드(S10~S13) 종료 후 들어갈 **"제어표면/fleet" 설계**를 선행조사해 `docs/research/control-surface-and-fleet.md`로 박았고, 그 과정에서 **문서 발견 체인이 rot(README 트리 stale, research/ 고아)**임을 발견 → 보류건은 `tracking.md` T-9로, README는 임시 수선. **"의도별 플로우 허브" 재구성안은 미적용(승인 대기)**. codex를 user-scope MCP로 입주(새 세션부터 유효).

## 1. git 상태 (소유권 명확화)
dashboard10 직전 핸드오프(`2026-06-21-embedded제거…`)가 §4-3에서 아래를 **"출처불명, 커밋 금지"**로 적었으나 — **전부 이 dashboard-Sub 세션이 만든 것이다(코드 무관, docs 전용):**
- `docs/research/control-surface-and-fleet.md` (신규) — 선행조사 본문.
- `docs/README.md` (M) — 트리에 S9~S13 채움 + research/ 등재 + step-log 범위(S0~S13). **임시 수선이며 §4-a 재구성안이 이를 대체 예정.**
- `docs/tracking.md` (M) — T-9(풀링 보류) 추가.
- (커밋 여부는 사용자 결정. dashboard10 코드와 분리돼 있음.)

## 2. 무엇을 했나
- **선행조사 3트랙(서브에이전트 fan-out → 결론만 회수):** ① claude 제어표면 4채널 ② fleet(20+) 호스팅 ③ 코드베이스 선행 지점. → `docs/research/control-surface-and-fleet.md`(§1~§6). 신뢰도 표기 [F]/[2]/[?]/[op].
- **tracking.md T-9:** 프로세스 풀링 보류(트리거·막다른 길 포함). 상태=tracking 단일관리, 상세=research §6.
- **codex MCP 입주:** `claude mcp add -s user codex -- codex mcp-server`. `~/.claude.json` 수정. ✔ Connected. **새 세션부터 유효**(진행 중 세션엔 hot-load 안 됨). 되돌리기 `claude mcp remove -s user codex`.

## 3. 핵심 결론 (research note 요약 — 리뷰어가 검증할 사실 기반)
- **메모리 = 프로세스수 × Node 베이스라인.** PTY/headless/SDK 무관하게 인스턴스당 별도 Node면 베이스라인 불변. **Agent SDK `query()`도 내부적으로 claude CLI를 subprocess spawn** → 메모리 절감 아님 [F].
- **메모리 본체 레버 = 유휴 kill + resume**(RAM이 활성 N개만 추적, S9 resume 구조에 네이티브). per-process 트림(`--bare`+headless)은 작음.
- **메모리 vs 속도 = 같은 다이얼 양끝**(warm=메모리비용/kill=속도비용). 둘 다 잡는 **유일 출구 = API transport**(claude.exe 미사용 → 에이전트=in-process 태스크). `ApiTransport` stub이 그 자리.
- **풀링(T-9): headless 메인 전제 하 무의미** — retarget에 cwd 변경 필요한데 headless는 `/cd` 불가. (sid는 블로커 아님 — 풀 슬롯에 engram-통제 sid 부팅 후 채택.)
- **MCP N×M 비용**(서버당 컨텍스트 stub + 프로세스) → fleet worker엔 `--bare`로 게이팅.
- **문서 밀도 원칙**(합의): 의미 동일 시 토큰 적을수록 유리(단 신호 떨구지 말 것). 휘발성 문서=모델 최적화 OK, CLAUDE.md/ADR=사람-감사 유지(지금 CLAUDE.md 손대지 말 것).

## 4. ★리뷰 대상 (codex + claude 의견 종합)★
**a. README "의도별 플로우 허브" 재구성안 (미적용, 승인 대기 — 채팅에 초안 있음).**
   - 제안: README의 step 폴더 열거(S0~S13) 제거(→step-log 단일출처), "하려는 것→어디로" 표 추가, "새 문서는 발견체인에 링크(고아 금지)" 불변규칙 명문화. 근거: README 트리=손으로 미러한 리스트 = 너희 anti-rot 원칙("베끼는 리스트 금지") 위반, 실제로 S9~S13 누락 rot 발생. *쟁점: 열거 제거가 옳은가, 아니면 "추가 시 갱신" 규칙화가 나은가?*
**b. 그 규약의 위치:** docs/README.md(문서-about-문서, governance 린 유지) vs CLAUDE.md. 내 권고=README + CLAUDE.md 포인터 한 줄. *쟁점: 동의?*
**c. research note fleet 결론 검증:** 특히 "API transport가 다이얼 탈출구"·"SDK=CLI subprocess"·"메모리 본체=유휴kill+resume" 주장의 타당성·반례. 메모리 수치는 [2]/[?](미실측) — 실측 전 신뢰 금지.
**d. T-9 풀링 보류 판단:** "headless 메인이면 풀링 무의미" 결론이 성급한가? 폴더별 풀(대안1) 재평가 여지?

## 5. 미적용 / 대기
- **README 재구성안(§4-a):** 미적용. 사용자 승인 후 docs/README.md 재작성 + CLAUDE.md 포인터 1줄.
- **코드 앵커(TODO):** `AgentManager` spawn 경로에 `// see tracking.md T-9` — 백엔드 정착 후(충돌 회피).
- **실측 미실행:** claude cold-start 시간 · `--bare` 절감폭 · headless RSS. (research §5 목록)

## 6. 환경 변화 (주의)
- **codex MCP user-scope 추가** → 이후 *모든* 새 세션(dashboard10 포함, 재시작 시)에 codex 도구 stub 로드(컨텍스트 소폭↑). 의도된 것.
- working tree는 dashboard10과 공유 — 내 docs 변경을 dashboard10 코드 커밋과 혼동 말 것(이미 분리됨).

## 7. 종료 체크리스트
1. 새 결정 ADR → **없음**(조사/문서 작업, 결정은 사용자 보류). 후보 ADR은 research §4에 적재.
2. 보류 항목 → tracking.md **T-9** 추가 ✅
3. README 인덱스 → 임시 수선(S9~S13·research/) ✅ / 재구성안은 §4-a 대기
4. step-log → **이 세션은 미기재**(코드 흐름 아님). 필요 판단 시 추가.
5. push 안 함. docs 3건 커밋 여부 사용자 결정.
