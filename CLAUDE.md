# Engram Dashboard

Tauri v2 + React 19 + Rust(portable-pty) 기반 **Claude 에이전트 관리 네이티브 대시보드**.
여러 claude(추후 codex·API) 에이전트를 PTY로 띄우고, xterm 터미널·트리·diff로 한 화면에서 관리한다.

이 파일은 대시보드 폴더에서 claude를 실행할 때의 프로젝트 컨텍스트다. 작업 전 아래 **아키텍처 원칙(불변)**을 반드시 깐다.

## 진행 상태는 이 파일이 아니라 docs에서 본다

이 파일은 **기조(불변 원칙)** 만 담는다. 현재 상태·타임라인·결정은 코드와 함께 갱신되는 docs에서 추적한다:
- **상태/구조 허브:** `docs/README.md`
- **문서 시스템(개발 플로우↔문서 매핑·자동화 맵):** `docs/handbook/documentation-system.md`
- **타임라인(언제/무엇):** `docs/process/step-log.md`
- **결정·거부한 대안(왜):** `docs/decisions/`
- **새 문서 = 발견 체인에 연결(고아 금지).** 종류·배치 규약은 `docs/README.md`.

검증 흐름(코딩 → 리뷰 → QA 게이트)의 강제 규약은 아래 **구현 실행 규약** 참조.

---

## ★ 개발 스텝 (매크로 흐름) ★

> 새 기능·단계는 위에서 아래로 좁힌다: **(컨설)→PRD → TRD → 모듈 경계(DDD) → 구현+TDD.** 굵은 설계는 메인이 임의로 확정하지 않는다 — 컨설로 선택지를 깔고 **결정권은 사용자**가 쥔다. ADR(결정)과 step-log(흐름)는 이 과정 내내 옆에서 계속 기록된다. (플로우↔문서 매핑 = `docs/handbook/documentation-system.md`.)

1. **PRD — 무엇/왜 (컨설로 선택지 → 사용자 결정).** 굵은 설계 결정은 먼저 **`/research`(OSS 서베이·옵션셋 → 선택지, 설계-결정 모드) + `/review prd`(opus + Codex 적대검증)** 로 **옵션셋 + 트레이드오프 + 놓친 대안**을 만들어 **사용자에게 제시하고, 사용자가 고른다**(임의 채택 금지). 고른 결과로 요구사항을 고정한다. (`docs/process/.../spec/`)
2. **TRD — 어떻게.** 세부 구현·인터페이스를 확정한다. 구현상 갈림길(저장 위치·네이밍·기본값 등)이 나오면 **다시 사용자에게 선택**을 받는다. 굵은 설계 결정은 이 단계부터 ADR로 박는다(끝나고가 아니라 결정 나는 즉시).
3. **모듈 경계 긋기 (DDD).** seam으로 영역을 분할한다 — 각 모듈이 외부 의존(Tauri/네트워크/실제 프로세스)을 끊고 **단독 검증 가능**하게. (ADR-0012)
4. **구현 + TDD.** PRD·TRD 선택을 사용자가 마친 뒤에만 코드에 들어간다. 기능 단위로 테스트를 먼저(또는 함께) 쓴다. 테스트는 *명세한 동작*을 지키는 회귀 안전망·환각 거름망이지 "완전성 보장"이 아니다(경계를 잘 그어야 강해진다). 실제 코드 변경은 아래 **구현 실행 규약**(코더→리뷰어→QA 서브에이전트)으로 수행.

**순서 불변(섞지 말 것):** 컨설/선택지 → **사용자 결정(PRD)** → TRD → (구현 갈림길도 **사용자 결정**) → 코더·리뷰어·QA. 메인이 설계를 임의 확정하거나, 사용자 선택 전에 구현(코더)에 들어가지 않는다.

**기록 분리(섞지 말 것):** ADR = *왜*(결정 + 거부한 대안, `docs/decisions/`) · step-log = *언제/무엇*(진행 흐름, `docs/process/step-log.md`).

---

## 사용자 협업·브리핑 방식

> 개발 스텝의 "결정권은 사용자" 기조를 *실천하는 법*. 기조 자체(순서 불변·굵은 설계 분리·ADR 강제)는 완화 아님 — 그대로.

- **결정은 "동작·정책" 언어로 번역해 제시.** 사용자가 체감하는 동작·정책·데이터 위치 = **사용자 결정**. 안 드러나는 순수 내부 구현(라이브러리·상수·코드 배치)은 메인이 정하되 **반드시 보고** — 결정 떠넘기기도, 무보고 독단도 아님.
- **2층 브리핑:** ① 개념 흐름(쉬운 말) → ② 시나리오/엣지 체크(BDD식) → ③ 용어 넣은 풀 브리핑 → ④ 수용 기준. 각 선택엔 *거부한 대안 한 줄* → ADR. (용어는 처음 나올 때 한 줄로 풀고 이후 그냥 씀)
- **PRD/TRD 묶기:** 작은 기능은 한 문서로 묶어도 됨(흐름이 곧 본질). 단 **굵은 설계는 분리·ADR 강제.**

---

## ★ 구현 실행 규약 (강제 — 비자명한 코드 변경마다) ★

> **메인 세션은 오케스트레이터다. 비자명한 코드 변경을 메인 스레드에서 직접 짜지 않는다.** 역할을 **서브에이전트로 분리 스폰**한다 — 코더·리뷰어·QA. 이건 권장이 아니라 강제다. "진행 쭉해" 같은 자율 모드에서도 동일하게 적용한다(처리량을 이유로 생략 금지).

- **코더 = opus(복잡)/sonnet(단순) 스폰.** 메인은 설계·지시·취합만. 메인이 직접 구현 편집하는 건 규약 위반.
- **리뷰어 = `/review` 스킬(opus + Codex 2인 적대, 다른 family).** 단계 인자(prd/trd/code/doc)가 특화 Advocate/Adversary 렌즈를 박는다. Codex=`mcp__codex__codex`. 리뷰 스킵 절대 금지. 정본·역할표 = `.claude/skills/review/references/flow.md §2`, 근거 = `docs/research/review-pipeline-design-draft.md`.
- **QA = `/qa` 스킬로 build/test + GUI 실측(`scripts/cdp.mjs`) 수행.** 코드(test/tsc)가 통과해도 실제 화면에서 동작 확인 전엔 미완으로 본다.
- **TDD + 모듈 격리 — 강제.** 기능 단위로 **테스트를 먼저(또는 함께) 작성**하고, 모든 모듈은 외부 의존(Tauri/네트워크/실제 프로세스)을 seam으로 끊어 **단독 실행 가능한 격리 하네스**를 갖춘다 — 코어=Noop/테스트 sink로 headless, transport/session=smoke bin, 데몬=integration harness bin. 테스트는 누적해 `cargo test`(workspace 루트) 한 번에 전 모듈 회귀. (ADR-0012)
- **예외(인라인 허용):** 1~2줄 사소 수정·문서·조사/탐색성 작업·스파이크(throwaway). 이때도 QA build/test는 돌린다.
- **조사·웹서칭·대량 읽기도 서브에이전트로 일임(컨텍스트 위생 — 강제 지향).** 퀄리티에 지장 없으면 메인 스레드에서 직접 WebSearch/WebFetch·광범위 파일 스윕·OSS 조사를 하지 말고 서브에이전트(Explore/general-purpose)·`/research`에 위임해 **결론만 회수**한다. 메인은 오케스트레이션·판단·사용자 보고에 집중. (핀포인트 1~2파일 조회나 즉답 가능한 단발 확인은 인라인 허용. 판단 기준: "결론만 있으면 되는 수집성 작업인가 → 서브에이전트".)
- 메인은 각 에이전트 결과를 취합해 사용자에게 보고하고, 커밋은 게이트 통과 후에만 한다.
- **effort 배치:** 메인 세션 = **xhigh**(영구 effort 천장 — 그 위 ultracode는 effort↑가 아니라 워크플로우 자동화·세션한정), 코더·리뷰어 = **high**(Codex는 medium 기본, 동시성·lifetime 치명 변경만 high). 무가드 통합 노드인 메인에 검수보다 effort를 싣는다.

### 리뷰어 역할 — 단계별 특화 (정본은 review 스킬)
구조 고정 = **Advocate(옹호·강화) vs Adversary(공격·대척)** 2인, 단계(prd/trd/code/doc)마다 전용 렌즈(즉석 발명 금지). 단계별 역할표·블라인드·모델매핑(맥락 필요=opus doc-aware / 신선=Codex blind)·공통 규약(판정 **PASS/FIX/BLOCK** 점수화 금지·취합 순서/라벨 무관·**불일치→사용자**)은 **`.claude/skills/review/references/flow.md §2`가 실행 정본**, 근거·체크리스트는 `docs/research/review-pipeline-design-draft.md §2`.

---

## ★ 설계 결정 기록 (ADR) — 강제 ★

> **비자명한 설계 결정은 `docs/decisions/`에 ADR로 박제한다.** 작업 전 관련 ADR을 읽고, 새 결정은 새 ADR로 추가하며, 번복은 기존 ADR을 '폐기(Superseded by ADR-NNNN)'로 표시하고 새 번호로 기록한다(덮어쓰지 않고 누적). 인덱스·규칙·템플릿: `docs/decisions/README.md`.
>
> ADR의 핵심은 **거부한 대안 + 그 이유**다 — 그게 없으면 다음 세션이 같은 대안을 다시 꺼낸다. 아래 "핵심 불변식"·"세션 복원"은 요약일 뿐, 근거·대안은 해당 ADR에 있다.

**기계적 작업은 `/adr` 스킬(`scripts/adr.mjs`)이 자동화한다** — 채번·템플릿 스캐폴드·인덱스 재생성·supersede 양방향(전체/부분)·drift lint. 전체/부분 폐기 판단과 본문 prose는 호출자(메인/사용자). 결정 날조 금지(거부한 대안·근거는 사용자 제공·fact-check).

### rot 방지 — 손으로 베끼는 리스트를 만들지 않는다
ADR 인덱스·"이 ADR 읽어라" 식 하드코딩 리스트는 한쪽만 갱신돼 rot한다. 그래서:
- **상태는 ADR 본문 헤더에만 둔다.** 폐기는 *폐기당한 ADR*의 `상태:` 줄에 `폐기 (Superseded by ADR-NNNN)`로 박는다(새 ADR에만 적고 끝내지 말 것 — 단방향이면 옛 ADR만 읽는 다음 세션이 폐기된 결정을 따라간다).
- **다음 세션은 열거 리스트가 아니라 두 곳을 본다:** ① 만지는 영역의 인덱스(`docs/decisions/README.md`) ② 만지는 코드의 `// ADR-NNNN` 앵커 주석(`rg "ADR-"`). 앵커는 코드와 한 몸이라 리스트처럼 rot하지 않는다 — load-bearing 코드엔 앵커 한 줄을 붙인다(신규·수정분부터 점진).

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
이 프로젝트는 **장기(10년) 유지보수**가 전제다. 그래서 추상화 결정은 단순 YAGNI가 아니라 **위험도 × 기간**으로 판단한다:
- **저위험 + 장기** (인터페이스 경계, seam, 타입 enum 등 나중에 바꾸면 비싼 것) → **지금 충분히 깐다(over-engineering 허용).**
- **고비용·불확실** (실제 동작을 모르는 백엔드 내부, 검증 안 된 가정) → **껍데기/정의만 두고 실측 때 채운다.**
- 예: seam·capability 구조·콘솔 백엔드는 지금, API transport 내부는 껍데기만(API 모델 등장 때). 상세: `docs/process/S10-backend-abstraction/`.

### 1. 출력/상태 계약 — `OutputSink` / `StatusSink`
출력·상태는 이 trait으로만 흐른다. 코어는 Tauri·전송 방식을 모른다 → headless 테스트 가능, 새 전송 경로는 sink 구현만 추가하면 흡수. (ADR-0003)

### 2. 세션 런타임 — 단일 인터페이스 + capability 매트릭스
모든 백엔드가 같은 인터페이스(start/write_input/resize/kill/output)를 구현한다. 차이는 구조가 아니라 **capability 유무**. 출력은 종류를 가정하지 않는다(터미널 강제 금지) — `OutputEvent`+`capabilities.output`로 종류를 구분하고 슬롯이 렌더러를 고른다(터미널=xterm / API=구조화·마크다운). capability 매트릭스(resume/resize/모델옵션 × claude_console/codex_console/codex_api)와 산출 규칙(= transport(물리) ⊕ backend(프로그램) 합성)은 ADR-0002 / ADR-0030.

### 3. 백엔드별 지식 격리 — `backend/`
claude 전용 인자(`--session-id`/`--resume`)는 `backend/claude.rs` 한 곳에만. manager는 dispatch(`needs_session`/`build_command_spec`/`backend_for`)만 부르고, `CommandSpec`만 transport에 주입(transport는 백엔드 모름). codex/gemini는 CLI spike 후 variant 추가(현재 stub·미연결). (ADR-0004)

### 4. 코어 격리 규칙
- 코어 crate(`crates/engram-dashboard-core/src/`) 하위 **tauri import 0** (`rg "use tauri"` → 0줄 유지).
- **`AgentManager` 소유 = 데몬**(`Arc<AgentManager>`, 외부 Mutex 없음). src-tauri는 in-proc 호스팅 X·AppState 제거(ADR-0029).

### 5. LLM-우선 제어 — 모든 메뉴가 프로그래밍 가능해야 한다 (불변)
**모든 기능(백엔드 + UI/레이아웃 전부 — 화면 분할·슬롯 배치·레이아웃 저장/복원·트리 이동·diff accept/revert·테마 등)은 LLM이 제어 가능**해야 한다. LLM이 메인 조작 주체, 사람의 UI 클릭은 보조다.
- **손발/두뇌 분리(핵심 멘탈모델):** 프론트는 **순수 I/O**(출력 표시 + 입력 캡처), **렌더링만 소유·제어 소유 X**. 모든 기능은 **백엔드측 LLM(두뇌)** 이 쥐는 "핸들"로 노출되고, 사람 클릭은 같은 핸들을 흔드는 보조 입력일 뿐이다. 그래서 프론트 액션 핸들도 백엔드측 LLM이 닿아야 한다. 죽음 감지·재시작 감독도 백엔드측에서 판단.
- **함의(현 갭):** 백엔드 동작은 `invoke`로 이미 LLM 제어 가능. 그러나 **UI/레이아웃은 현재 프론트(Zustand) 전용 — LLM 제어 표면이 없다.** 새 UI 기능엔 LLM 호출 경로(command/이벤트버스/문서화된 JS API)를 **함께** 만든다("UI 먼저, 제어는 나중"은 위반).
- **설계 지향:** UI 컴포넌트는 store 액션을 호출만 하고, 그 액션들은 LLM도 동일하게 부르는 단일 control surface(의도 단위 command 버스)로 모은다.
- **임시 경로:** 정식 제어 표면 전까지 `scripts/cdp.mjs eval`이 WebView에서 임의 JS·invoke 실행 = LLM 제어/검증 임시 수단.

---

## 참조 구현 (기능 구현 시 비교 참고)

새 기능(데몬화·원격·스트리밍·재연결·멀티플랫폼·영속·오케스트레이션·프론트 상태 영속 등)은 바닥부터 짜지 말고 **성숙 OSS가 같은 문제를 어떻게 풀었나 먼저 조사** → engram 제약(Rust·교체성·LLM 제어)으로 트레이드오프 비교 → **선택지를 사용자에게 제시(임의 채택 금지)** → 굵은 결정이면 ADR. (개발 스텝 TRD 단계, 서브에이전트/`/research`로 폭넓게.)

- **참조 = 패턴 차용이지 코드 복붙 아님** — 그대로 옮길 때만 라이선스 법무 확인. 클론 소스: `I:\Engram_Workspace\references\`(git 추적 밖).
- **데몬/전송 앵커:** tmux · Zellij(Rust) · Mosh · ttyd/gotty · Hermes Agent — 상세 ADR-0013.
- **오케스트레이션 후보:** Erlang OTP/Ractor · Temporal/Restate · LangGraph/MS Agent Framework · A2A — 상세 ADR-0014(제안).
- 그 외(프론트 상태 영속·메시지 시스템 등) 앵커 미선정 — 그때 위 절차로 정한다.

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

### S9 세션 복원 메커니즘
spawn 시 `--session-id <uuid>`로 **우리가 sid를 통제** → 재시작 `--resume`로 무손실 복원. `/clear` drift는 `~/.claude/sessions/<pid>.json` 폴링(best-effort). **복원 정확성은 우리 통제 sid에만 의존** — 추적 파일은 못 읽어도 무손상 강등, 이 파일로 기능 *확장* 금지. resume 조기 종료(3s 윈도) → 새 sid fresh fallback(종점 Failed). (ADR-0008)

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
- 프론트 게이트: `npm test`(vitest run) + `npx tsc --noEmit`(타입체크 — package.json에 별도 typecheck 스크립트 없음)
- 프로젝트 루트: `npm run tauri dev` — 전체 E2E
- 로그 ON: `RUST_LOG=debug` (기본 OFF=warn)

### GUI 시각/동작 검증 (`scripts/cdp.mjs`) — 실제 앱을 코드로 확인
실제 Tauri 창(WebView2)에 **CDP로 직접 붙어** 스크린샷·DOM 조회·실제 `invoke` 호출까지 한다. MCP·새 세션·재시작 불필요(node 내장 WebSocket만). **Windows 전용**(WebView2). 절차:
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
- **숨은 의도·불변식은 그 코드에 박는다** — 시그니처·타입만 봐선 안 보이는 load-bearing 의미("이 분기가 어떤 race를 막나", "이건 detached여야 한다" 등). 빠뜨리면 다음 세션이 모르고 "불필요"로 지우거나 잘못 바꾼다. *의도가 섞인 지점*만 깊게. 상세 규약·사례 = `docs/reference/commenting-conventions.md`(ADR-0032).
- **load-bearing 파일은 `//!` overview 헤더**로 역할·불변식·진입점 요약(점진 권고 — 만지는 파일부터 boy-scout). 상세는 위 reference.
- 자격증명을 `profile.env`에 넣지 말 것(agents.json 평문 저장 — persistence가 경고).
- 모듈마다 build/test/커밋. 커밋 메시지 끝에 Co-Authored-By 트레일러.

---

## 기술 스택 (프론트)

React 19 + TS + Vite · Zustand · @xterm/xterm(+fit) · allotment · react-arborist · @monaco-editor/react · react-router(hash) · CSS 변수(Tailwind X) · Tauri v2 셸. 의존성 상세는 package.json.

## 프론트 구조·제어 표면 (`src/`)

- **제어 표면(★불변):** 컴포넌트·스토어는 `agentClient` 인터페이스(단일 `ProtocolClient`)에만 의존(`ptyApi` 직접 호출 X — ADR-0011). carrier = transport seam — 운영은 `WsTransport` 고정(데몬 attach, ADR-0029 daemon-only). 교체점은 transport(InProc은 테스트 mock·ADR-0020 흔적).
- `eventBus`가 Tauri 이벤트 1회 등록(agent-list-updated / status-changed / restore-result). 폴더: api · store · components(layout/agent/slot/diff) · pages. 파일별은 코드.
- **통합 micro-rules(코드와 함께):** 구독 effect deps `[agentId, epoch]`(재spawn 시 reset→재구독→replay) · `terminal.reset()` 구독 전 · seq dedup · `delete channel.onmessage`(null 아님, #13133) · 입력 가드 · resize debounce 50ms.

## 창 구성 (tauri.conf.json)
창 3개: main(대시보드, visible) · slot-popup(`/popup?slotId=N`, hidden) · agent-tree(`/tree`, hidden).

## 테마 CSS 변수 (`data-theme` on `:root`: dark/light/e-ink)
CSS 변수·폰트 정의는 `src/styles/theme.css`·`font.css` 참조.
