# Engram Dashboard

Tauri v2 + React 19 + Rust(portable-pty) 기반 **Claude 에이전트 관리 네이티브 대시보드**.
여러 claude(추후 codex·API) 에이전트를 PTY로 띄우고, xterm 터미널·트리·diff로 한 화면에서 관리한다.

이 파일은 이 폴더에서 claude를 실행할 때의 프로젝트 컨텍스트다. 작업 전 아래 **아키텍처 원칙(불변)**을 반드시 깐다.

## 진행 상태는 이 파일이 아니라 docs에서 본다

이 파일은 **기조(불변 원칙)** 만 담는다. 상태·타임라인·결정은 docs에서 추적한다:
- **상태/구조 허브:** `docs/README.md`
- **문서 시스템(플로우↔문서 매핑·자동화 맵):** `docs/handbook/documentation-system.md`
- **타임라인(언제/무엇):** `docs/process/step-log.md`
- **결정·거부한 대안(왜):** `docs/decisions/`
- **새 문서 = 발견 체인에 연결(고아 금지).** 종류·배치 규약은 `docs/README.md`.
- **세션 핸드오프(/continue 인계):** 정본 = **continue 스킬** — 저장 위치·파일명·형식 전부 스킬이 정의(변경 가능 영역이라 경로 하드코딩 금지).

검증 흐름(코딩 → 리뷰 → QA 게이트)의 강제 규약은 아래 **구현 실행 규약**.

---

## ★ 개발 스텝 (매크로 흐름) ★

> 새 기능은 위에서 아래로 좁힌다: **(컨설)→PRD → TRD → 모듈 경계(DDD) → 구현+TDD.** 굵은 설계는 메인이 임의 확정하지 않는다 — 컨설로 선택지를 깔고 **결정권은 사용자**. ADR(결정)·step-log(흐름)는 과정 내내 기록한다.

1. **PRD — 무엇/왜.** 굵은 설계는 **`/research`(OSS 서베이·옵션셋, 설계-결정 모드) + `/review prd`(opus + Codex 적대검증)** 로 **옵션셋 + 트레이드오프 + 놓친 대안**을 만들어 사용자에게 제시하고 **사용자가 고른다**(임의 채택 금지) → 요구사항 고정. (`docs/process/.../spec/`)
2. **TRD — 어떻게.** 세부 구현·인터페이스 확정. 구현 갈림길(저장 위치·네이밍·기본값 등)도 **사용자 선택**. 굵은 결정은 이 단계부터 ADR로 박는다(끝나고가 아니라 결정 즉시).
3. **모듈 경계 (DDD).** seam으로 분할 — 각 모듈이 외부 의존(Tauri/네트워크/실제 프로세스)을 끊고 **단독 검증 가능**하게. (ADR-0012)
4. **구현 + TDD.** 사용자가 PRD·TRD 선택을 마친 뒤에만 코드 진입. 테스트를 먼저(또는 함께) 쓴다 — 테스트는 *명세한 동작*의 회귀 안전망·환각 거름망이지 "완전성 보장"이 아니다(경계를 잘 그어야 강해진다). 코드 변경은 아래 **구현 실행 규약**(코더→리뷰어→QA 서브에이전트)으로.

**순서 불변(섞지 말 것):** 컨설/선택지 → **사용자 결정(PRD)** → TRD → (구현 갈림길도 **사용자 결정**) → 코더·리뷰어·QA. 사용자 선택 전에 설계 확정·구현 진입 금지.

**기록 분리(섞지 말 것):** ADR = *왜*(결정 + 거부한 대안, `docs/decisions/`) · step-log = *언제/무엇*(흐름, `docs/process/step-log.md`).

---

## 사용자 협업·브리핑 방식

> "결정권은 사용자" 기조를 *실천하는 법* — 기조 자체(순서 불변·굵은 설계 분리·ADR 강제)는 완화 아님.

- **결정은 "동작·정책" 언어로 번역해 제시.** 사용자가 체감하는 동작·정책·데이터 위치 = **사용자 결정**. 안 드러나는 순수 내부 구현(라이브러리·상수·코드 배치)은 메인이 정하되 **반드시 보고** — 결정 떠넘기기도, 무보고 독단도 아님.
- **2층 브리핑:** ① 개념 흐름(쉬운 말) → ② 시나리오/엣지 체크(BDD식) → ③ 용어 넣은 풀 브리핑 → ④ 수용 기준. 각 선택엔 *거부한 대안 한 줄* → ADR. (용어는 첫 등장 때 한 줄 풀이, 이후 그냥 씀)
- **PRD/TRD 묶기:** 작은 기능은 한 문서로 묶어도 됨. 단 **굵은 설계는 분리·ADR 강제.**
- **서브에이전트 브리핑(학습용):** 코더·리뷰어·QA를 돌린 뒤 **스폰 방식(타입·모델·역할)·지시·회수물**을 간단히 브리핑한다(에이전트당 1~2줄 + 핵심 회수물 — 결과만 던지지 않는다).

### 용어 추상화 — 미시 숨김·거시 노출·풀이는 정확히 (항상)

어기면 사용자가 코드 지도를 못 그리거나, 어설픈 의역이 딴 개념과 헷갈려 대화가 꼬인다.

- **미시 코드명(변수·함수명)은 나열 금지** — 역할·동작으로 풀어 말한다.
- **거시 아키텍처명(`AgentManager`·`ProtocolClient`·`TerminalSlot`·`TauriTransport` 등)은 빼지 말고 의도적으로 계속 흘린다** — 반복 노출로 코드베이스 지도를 학습시키는 장치(미시=숨김, 거시=노출, **정반대**).
- **풀어 쓸 땐 정확한 경계로** — 의역이 다른 거시 개념(웹뷰/클라이언트/에이전트)과 충돌하면 역효과. 모호하면 거시 이름을 박아 고정하고 거기에 풀이를 매단다.
- **혼동 쌍은 명시적으로 구분해 부른다(아래 고정 용어).** 일반 기술 용어(직렬화·락·경쟁 조건 등)는 그대로 쓰되 생소할 것만 단서 한 조각. 비유는 보조, 주력은 한 단계 추상화한 직접 설명.

**혼동 쌍 — 고정 용어 (혼동 생길 때마다 누적):**
- **에이전트(claude 프로세스) 재시작** ≠ **클라이언트(src-tauri 셸) 재시작** ≠ **데몬 재시작**. 맨 "재시작" 금지.
- **웹뷰(창 = WebView2)** ≠ **프론트 컴포넌트(웹뷰 안 React 부품)** ≠ **슬롯(레이아웃 한 칸)**.

<examples>
<bad>
"터미널 화면부품이 등록을 취소한다" → TerminalSlot인지 웹뷰인지 슬롯인지 불명 (의역이 다른 개념과 충돌)
"앱 재시작하면…" → 에이전트/클라이언트/데몬 중 뭔지 불명
</bad>
<good>
"`TerminalSlot`(웹뷰 안에서 도는 터미널 컴포넌트)이 출력 구독을 해제한다"
"에이전트(claude 프로세스)를 재시작하면(= epoch 교체) …"
</good>
</examples>

---

## ★ 구현 실행 규약 (강제 — 비자명한 코드 변경마다) ★

> **메인 세션 = 오케스트레이터. 비자명한 코드 변경을 메인 스레드에서 직접 짜지 않는다** — 코더·리뷰어·QA를 서브에이전트로 분리 스폰한다. 권장이 아니라 강제고, "진행 쭉해" 같은 자율 모드에서도 동일하다(처리량을 이유로 생략 금지).

- **코더 = opus(복잡)/sonnet(단순) 스폰.** 메인은 설계·지시·취합만 — 직접 구현 편집 = 규약 위반.
- **리뷰어 = `/review` 스킬(opus + Codex 2인 적대, 다른 family).** 단계 인자(prd/trd/code/doc)가 전용 Advocate/Adversary 렌즈를 박는다(즉석 발명 금지). Codex=`mcp__codex__codex`. 리뷰 스킵 절대 금지. 판정 PASS/FIX/BLOCK — **불일치는 메인 임의 판정 금지, 사용자에게.** 역할표·모델매핑·공통 규약의 실행 정본 = **review 스킬**(스킬 개편 시 그쪽이 이김), 결정 근거 = ADR-0031.
- **QA = `/qa` 스킬(build/test + GUI 실측 `scripts/cdp.mjs`).** 테스트·tsc 통과 ≠ 완료 — 실제 화면 동작 확인 전엔 미완.
- **TDD + 모듈 격리 — 강제.** 기능 단위로 테스트를 먼저(또는 함께) 쓰고, 모든 모듈은 외부 의존(Tauri/네트워크/실제 프로세스)을 seam으로 끊어 **단독 실행 격리 하네스**를 갖춘다 — 코어=Noop/테스트 sink headless, transport/session=smoke bin, 데몬=integration harness bin. 테스트는 누적해 `cargo test`(루트) 한 번에 전 모듈 회귀. (ADR-0012)
- **테스트 가능성 분리 검토 — 사용자 결정.** 중요한 로직이 외부 의존(Tauri/환경)에 묶여 단위테스트가 막히면 순수 로직의 seam 분리를 **항상 검토**하되, 분리 여부는 사용자 판단(구현 갈림길 = 사용자 결정). (ADR-0012)
- **예외(인라인 허용):** 1~2줄 사소 수정·**사소한 문서(오타·서식·노트)**·조사/탐색·스파이크(throwaway). 이때도 QA build/test는 돌린다. **load-bearing 문서(standing 규약·표준·바인딩 등 다른 작업이 의존)는 예외 아님 → `/review doc` 거쳐 커밋.**
- **조사·웹서칭·대량 읽기 = 서브에이전트 일임.** 범용 원칙 = global-rules 「위임 우선」(여기 안 베낌). engram 바인딩: OSS·설계 조사는 `/research`. 자율 모드("진행 쭉해")에서도 생략 금지.
- 메인은 각 에이전트 결과를 취합해 보고하고, 커밋은 게이트 통과 후에만.
- **역할→모델·effort 배치 = 전역 사전이 정본**(경로는 global-rules 「전역 사전」이 정의 — 여기 안 베낌). 메인 세션만 예외 명시 = **xhigh**(무가드 통합 노드 — 검수보다 메인에 싣는다; 그 위 ultracode는 effort↑가 아니라 워크플로우 자동화·세션한정).

---

## ★ 설계 결정 기록 (ADR) — 강제 ★

> **비자명한 설계 결정은 `docs/decisions/`에 ADR로 박제한다.** 작업 전 관련 ADR을 읽고, 새 결정 = 새 ADR, 번복 = 기존 ADR에 '폐기(Superseded by ADR-NNNN)' 표시 + 새 번호로 누적(덮어쓰지 않는다). 인덱스·규칙·템플릿: `docs/decisions/README.md`.
>
> ADR의 핵심 = **거부한 대안 + 그 이유** — 없으면 다음 세션이 같은 대안을 다시 꺼낸다. 아래 "핵심 불변식"·"세션 복원"은 요약일 뿐, 근거·대안은 해당 ADR에 있다.

**기계적 작업은 `/adr` 스킬(`scripts/adr.mjs`)이 자동화** — 채번·스캐폴드·인덱스 재생성·supersede 양방향(전체/부분)·drift lint. 폐기 판단과 본문 prose는 호출자(메인/사용자). 결정 날조 금지(거부한 대안·근거는 사용자 제공·fact-check).

### rot 방지 — 손으로 베끼는 리스트를 만들지 않는다
하드코딩 리스트("이 ADR 읽어라" 식)는 한쪽만 갱신돼 rot한다. 그래서:
- **상태는 ADR 본문 헤더에만 둔다.** 폐기는 *폐기당한 ADR*의 `상태:` 줄에 박는다 — 새 ADR에만 적는 단방향이면 옛 ADR만 읽는 다음 세션이 폐기된 결정을 따라간다.
- **다음 세션은 두 곳을 본다:** ① 만지는 영역의 인덱스(`docs/decisions/README.md`) ② 만지는 코드의 `// ADR-NNNN` 앵커(`rg "ADR-"`) — 앵커는 코드와 한 몸이라 rot하지 않는다. load-bearing 코드엔 앵커 한 줄(신규·수정분부터 점진).

### 핸드오프 종료 체크리스트 (세션 끝낼 때)
1. 새 설계 결정 → 새 ADR 썼나
2. 번복한 결정 → *폐기당한* ADR에 `폐기 (Superseded by ...)` 박았나
3. `docs/decisions/README.md` 인덱스 갱신했나(번호·제목·상태)
4. `docs/process/step-log.md`에 *언제/무엇* 추가했나

---

## ★ 아키텍처 원칙 (불변 — 아키텍트 구상 시 반드시 고려) ★

> **모든 기능은 추상 인터페이스 위에 구현하고, 내부 구현체는 교체(swappable)되는 형태로 짠다.**
> 특정 모델·전송 방식에 코드를 묶지 않는다. 이게 이 프로젝트를 10년 끌고 가는 법칙이다.
>
> **모든 시스템·메뉴는 LLM이 제어 가능해야 한다. LLM이 메인 조작 주체, 사용자의 직접 UI 조작은 서브다.** (§5)

### 0. 판단 기준 — 위험도 낮으면 over-engineering 쪽으로
**장기(10년) 유지보수**가 전제라 추상화 결정은 단순 YAGNI가 아니라 **위험도 × 기간**으로 판단한다:
- **저위험 + 장기**(인터페이스 경계·seam·타입 enum — 나중에 바꾸면 비싼 것) → **지금 충분히 깐다(over-engineering 허용).**
- **고비용·불확실**(실측 안 된 백엔드 내부·검증 안 된 가정) → **껍데기/정의만 두고 실측 때 채운다.**
- 예: seam·capability 구조·콘솔 백엔드는 지금, API transport 내부는 껍데기만(API 모델 등장 때). 상세: `docs/process/S10-backend-abstraction/`.

### 1. 출력/상태 계약 — `OutputSink` / `StatusSink`
출력·상태는 이 trait으로만 흐른다. 코어는 Tauri·전송 방식을 모른다 → headless 테스트 가능, 새 전송 경로는 sink 구현만 추가하면 흡수. (ADR-0003)

### 2. 세션 런타임 — 단일 인터페이스 + capability 매트릭스
모든 백엔드가 같은 인터페이스(start/write_input/resize/kill/output)를 구현한다. 차이는 구조가 아니라 **capability 유무**. 출력은 종류를 가정하지 않는다(터미널 강제 금지) — `OutputEvent`+`capabilities.output`로 구분하고 슬롯이 렌더러를 고른다(터미널=xterm / API=구조화·마크다운). capability 매트릭스와 산출 규칙(= transport(물리) ⊕ backend(프로그램) 합성)은 ADR-0002 / ADR-0030.

### 3. 백엔드별 지식 격리 — `backend/`
claude 전용 인자(`--session-id`/`--resume`)는 `backend/claude.rs` 한 곳에만. manager는 dispatch(`needs_session`/`build_command_spec`/`backend_for`)만 부르고 `CommandSpec`만 transport에 주입(transport는 백엔드 모름). codex/gemini는 CLI spike 후 variant 추가(현재 stub·미연결). (ADR-0004)

### 4. 코어 격리 규칙
- 코어 crate(`crates/engram-dashboard-core/src/`) 하위 **tauri import 0** (`rg "use tauri"` → 0줄 유지).
- **`AgentManager` 소유 = 데몬**(`Arc<AgentManager>`, 외부 Mutex 없음). src-tauri는 in-proc 호스팅 X·AppState 제거(ADR-0029).

### 5. LLM-우선 제어 — 모든 메뉴가 프로그래밍 가능해야 한다 (불변)
**모든 기능(백엔드 + UI/레이아웃 전부 — 화면 분할·슬롯 배치·레이아웃 저장/복원·트리 이동·diff accept/revert·테마 등)은 LLM이 제어 가능**해야 한다. LLM이 메인 조작 주체, 사람의 UI 클릭은 보조다.
- **손발/두뇌 분리(핵심 멘탈모델):** 프론트 = **순수 I/O**(출력 표시 + 입력 캡처), **렌더링만 소유·제어 소유 X**. 모든 기능은 **백엔드측 LLM(두뇌)**이 쥐는 "핸들"로 노출되고, 사람 클릭은 같은 핸들을 흔드는 보조 입력이다 — 프론트 액션 핸들도 백엔드측 LLM이 닿아야 한다. 죽음 감지·재시작 감독도 백엔드측 판단.
- **함의(현 갭):** 백엔드 동작은 `invoke`로 이미 LLM 제어 가능. **UI/레이아웃은 현재 프론트(Zustand) 전용 = LLM 제어 표면 없음.** 새 UI 기능엔 LLM 호출 경로(command/이벤트버스/문서화된 JS API)를 **함께** 만든다("UI 먼저, 제어는 나중" = 위반).
- **설계 지향:** UI 컴포넌트는 store 액션을 호출만 하고, 그 액션들을 LLM도 동일하게 부르는 단일 control surface(의도 단위 command 버스)로 모은다.
- **임시 경로:** 정식 제어 표면 전까지 `scripts/cdp.mjs eval`(WebView에서 임의 JS·invoke 실행) = LLM 제어/검증 임시 수단.

---

## 참조 구현 (기능 구현 시 비교 참고)

새 기능(데몬화·원격·스트리밍·재연결·멀티플랫폼·영속·오케스트레이션·프론트 상태 영속 등)은 바닥부터 짜지 말고 **성숙 OSS가 같은 문제를 어떻게 풀었나 먼저 조사** → engram 제약(Rust·교체성·LLM 제어)으로 트레이드오프 비교 → **선택지를 사용자에게 제시(임의 채택 금지)** → 굵은 결정이면 ADR. (TRD 단계, `/research`로 폭넓게.)

- **결함 수정에도 같은 원칙 (ADR-0038):** 비자명 기술결함을 솔로 추측·매직넘버로 맞추지 말고 OSS 사례 먼저. 트리거·제외·발화는 `docs/reference/debugging-conventions.md`.
- **참조 = 패턴 차용이지 코드 복붙 아님** — 그대로 옮길 때만 라이선스 법무 확인. 클론 소스: `I:\Engram_Workspace\references\`(git 추적 밖).
- **선정된 앵커 = ADR이 정본:** 데몬/전송 = ADR-0013 · 오케스트레이션 = ADR-0014(제안). 그 외 영역은 앵커 미선정 — 그때 위 절차로 정한다.

## 백엔드 모듈 맵 (Cargo workspace — 5 멤버: protocol · core · discovery · daemon · src-tauri)
> **개요만.** 파일별 책임은 코드/grep(`// ADR-` 앵커 포함)이 단일 출처 — 여기 베끼지 않는다(rot 방지). 불변식은 ↓ 핵심 불변식.

**데이터 흐름(S10 추상화):** `AgentManager → AgentSession(= OutputCore + dyn AgentTransport)`. 출력·상태는 `OutputSink`/`StatusSink` trait으로만 흐른다(코어는 Tauri·전송 방식 모름). 종료 분류는 reaper 단일 소비자(ADR-0019).

**crate 경계** (ADR-0029: 에이전트 호스트 = 데몬 프로세스):
- **core** — 에이전트 코어(agent·persistence·logging), **tauri import 0**. seam: `transport`(`AgentTransport` — pty 실물/api 껍데기) · `backend`(CommandSpec·claude 인자 격리, ADR-0004).
- **daemon** — `AgentManager` 소유, WS 서버, 단일 인스턴스 guard, portfile(`daemon.json`). 이벤트버스 single-push(ADR-0028).
- **discovery** — 데몬 발견 순수 로직(no WMI/no sleep, seam) + `ensure_daemon` + `default_data_dir`(ADR-0024).
- **protocol** — wire 계약(AgentCommand/Event/OutputChunk/DaemonInfo + codec + ts-rs).
- **src-tauri** — 데몬 클라이언트 셸(창·트레이·discovery·로컬 command). 에이전트 in-proc 호스팅 X. tray = §5 LLM 제어 핸들.

### 핵심 불변식 (변경 금지 — 근거·거부 대안은 `docs/decisions/`)
- **kill 인과(2동사):** `transport.shutdown()`(child.kill+wait → TerminateJobObject → master drop) → `core.join_pump(5s)`. master drop → reader EOF → pump break → `core.finish` → done_tx. (ADR-0001)
- **finalize 1회:** `OutputCore.finalized.swap(AcqRel)` — terminal 전이/알림 정확히 1회(pump 단독). (ADR-0005)
- **락 순서:** sessions RwLock은 Arc clone 후 즉시 해제 → 그 뒤 내부 접근. status lock 보유 중 외부 호출 금지. emit은 subscribers clone 후 lock 미보유 send. (ADR-0006)
- **상태 알림 분담:** 과도기 `Exiting`=manager(`enter_exiting`), terminal(`Killed`/`Exited`/`Failed`)=pump 단독. 프론트는 status_changed로 terminal 판정 금지 → `agent-list-updated`(목록)로 판정. (ADR-0005)
- **replay→live:** subscribers lock 보유 중 replay 전송(C4, 순서 역전 방지) + 프론트 seq dedup.
- **epoch:** 같은 AgentId 맵 교체(restart/fresh fallback)마다 +1 → 프론트 `[agentId, epoch]` 재구독. (ADR-0007)
- **소유권 분할:** transport=master/writer/child/shutdown/job · core=subscribers/replay/seq/status/finalized/drain_handle · session=id/cwd/epoch/cols/rows.

### S9 세션 복원 (요지 — 정본 ADR-0008)
spawn 시 `--session-id`로 **sid를 우리가 통제** → `--resume` 무손실 복원. 복원 정확성은 이 sid에만 의존(추적 파일은 best-effort — 이걸로 기능 *확장* 금지). resume 조기 종료 → fresh fallback(종점 Failed).

---

## 의존성 (확정 — 변경 시 보고)
- `tauri = "2"` (최신 2.x — Channel 무손실 Windows 실측 확인, spike)
- `portable-pty = "0.8.1"` · `uuid` · `thiserror` · `base64` · `regex`(로그 마스킹) · `tracing` · `dunce`(cwd canonicalize UNC 회피)
- `windows` (Job Object) — `#[cfg(windows)]`

## 빌드·검증 명령 (Cargo workspace — 루트에서 실행)
**Cargo workspace**: 멤버 구성은 위 모듈맵 참조. 코어(agent/persistence/logging)·tests/는 `crates/engram-dashboard-core`, `target/`는 워크스페이스 루트.
- `cargo test -p engram-dashboard-core` — 코어 unit + 통합 테스트(실 PTY로 단언)
- `cargo test -p engram-dashboard-protocol` — protocol codec golden + ts-rs 바인딩
- `cargo build` (루트) — 전체 workspace 빌드
- `cargo fmt --check` / `rg "use tauri" crates/engram-dashboard-core/src/` (→ 0줄) — 포맷·격리 게이트(검사형 `--check`)
- 프론트 게이트: `npm test`(vitest run) + `npx tsc --noEmit`(타입체크 — 별도 typecheck 스크립트 없음)
- 프로젝트 루트: `npm run tauri dev` — 전체 E2E
- 로그 ON: `RUST_LOG=debug` (기본 OFF=warn)

### GUI 시각/동작 검증 (`scripts/cdp.mjs`) — 실제 앱을 코드로 확인
실제 Tauri 창(WebView2)에 **CDP로 직접 붙어** 스크린샷·DOM 조회·실제 `invoke` 호출까지 한다(node 내장 WebSocket만, **Windows 전용**). 절차:
```bash
# 1) 디버그 포트 열고 앱 실행 (bash: env var 붙여 백그라운드)
WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223" npm run tauri dev
# 2) 포트 뜰 때까지 대기: curl http://127.0.0.1:9223/json/version
# 3) 검증
node scripts/cdp.mjs info                 # 페이지 목록
node scripts/cdp.mjs shot out.png          # 스크린샷 → Read로 확인
node scripts/cdp.mjs eval "<js>"           # 앱 안에서 JS 실행(결과 JSON 출력)
```
`eval`로 DOM 텍스트·백엔드 직접 호출(`window.__TAURI__.core.invoke(...)`) → spawn/write/interrupt/kill을 실제 IPC로 검증. **검증엔 스샷보다 `eval` 텍스트가 토큰·정확도 유리**(픽셀 해석 회피). 포트 9223 고정(9222=Gemini Chrome 충돌 회피, `CDP_PORT`로 변경).

## 컨벤션
- 중요 로직(동시성·kill·unsafe·비자명한 결정)에 **왜** 그런지 한국어 주석. 자명한 코드엔 주석 금지.
- **숨은 의도·불변식은 그 코드에 박는다** — 시그니처·타입만 봐선 안 보이는 load-bearing 의미("이 분기가 어떤 race를 막나" 등). 빠뜨리면 다음 세션이 모르고 지우거나 잘못 바꾼다. 상세 규약·사례 = `docs/reference/commenting-conventions.md`(ADR-0032).
- **load-bearing 파일은 `//!` overview 헤더**(역할·불변식·진입점 요약 — 만지는 파일부터 boy-scout). 상세는 위 reference.
- 자격증명을 `profile.env`에 넣지 말 것(agents.json 평문 저장 — persistence가 경고).
- 모듈마다 build/test/커밋. 커밋 메시지 끝에 Co-Authored-By 트레일러.

---

## 기술 스택 (프론트)

React 19 + TS + Vite · Zustand · @xterm/xterm(+fit) · allotment · react-arborist · @monaco-editor/react · react-router(hash) · CSS 변수(Tailwind X) · Tauri v2 셸. 의존성 상세는 package.json.

## 프론트 구조·제어 표면 (`src/`)

- **제어 표면(★불변):** 컴포넌트·스토어는 `agentClient` 인터페이스(단일 `ProtocolClient`)에만 의존(`ptyApi` 직접 호출 X — ADR-0011). carrier = transport seam — 운영은 `WsTransport` 고정(데몬 attach, ADR-0029 daemon-only). 교체점은 transport(InProc은 테스트 mock·ADR-0020 흔적).
- `eventBus`가 Tauri 이벤트 1회 등록(agent-list-updated / status-changed / restore-result). 폴더: api · store · components(layout/agent/slot/diff) · pages. 파일별은 코드.
- **통합 micro-rules(코드와 함께):** 구독 effect deps `[agentId, epoch]`(재spawn 시 reset→재구독→replay) · `terminal.reset()` 구독 전 · seq dedup · `delete channel.onmessage`(null 아님, #13133) · 입력 가드 · resize debounce 50ms.

## 창 구성 (src-tauri/tauri.conf.json)
창 3개: main(대시보드, visible) · slot-popup(`/popup?slotId=N`, hidden) · agent-tree(`/tree`, hidden).

## 테마 CSS 변수 (`data-theme` on `:root`: dark/light/e-ink)
CSS 변수·폰트 정의는 `src/styles/theme.css`·`font.css` 참조.
