# Engram Dashboard

Tauri v2 + React 19 + Rust(portable-pty) 기반 **Claude 에이전트 관리 네이티브 대시보드**. 여러 claude(추후 codex·API) 에이전트를 PTY로 띄우고 터미널·트리·diff로 한 화면에서 관리한다.

이 파일은 **기조(불변 원칙)만** 담는다. 작업 전 「아키텍처 원칙」을 반드시 깐다.

## 상태는 여기가 아니라 docs에서 본다

- **상태·구조 허브:** `docs/README.md` — 새 문서의 종류·배치 규약도 여기(고아 문서 금지).
- **타임라인(언제·무엇):** `docs/process/step-log.md`
- **결정·거부한 대안(왜):** `docs/decisions/`
- **문서 시스템(플로우↔문서 매핑):** `docs/handbook/documentation-system.md`
- **세션 인계:** 정본 = handoff 스킬. 저장 위치·형식 전부 스킬이 정의하므로 경로를 여기 박지 않는다.

---

# 일하는 방식

## 개발 스텝

> 새 기능은 위에서 아래로 좁힌다. **굵은 설계는 메인이 임의 확정하지 않는다.**

1. **PRD — 무엇·왜.** `/research`로 옵션셋을, `/review prd`로 적대 검증을 거쳐 **트레이드오프와 놓친 대안**을 사용자에게 제시하고 사용자가 고른다.
2. **TRD — 어떻게.** 세부 구현·인터페이스 확정. 구현 갈림길(저장 위치·네이밍·기본값)도 사용자 선택. 굵은 결정은 **결정 즉시** ADR로 박는다.
3. **모듈 경계.** seam으로 분할해 각 모듈이 외부 의존(Tauri·네트워크·실제 프로세스)을 끊고 단독 검증 가능하게. (ADR-0012)
4. **구현 + TDD.** 사용자가 PRD·TRD 선택을 마친 뒤에만 코드 진입. 테스트는 *명세한 동작*의 회귀 안전망이지 완전성 보장이 아니다.

- **순서 불변:** 선택지 → **사용자 결정** → TRD → **사용자 결정** → 코더·리뷰어·QA. 사용자 선택 전 설계 확정·구현 진입 금지.
- **기록 분리:** ADR = *왜*(결정 + 거부한 대안) · step-log = *언제·무엇*.

## 구현 실행 규약

> **강제.** 메인 세션 = 오케스트레이터. 비자명한 코드 변경을 메인 스레드에서 직접 짜지 않는다. "진행 쭉해" 같은 자율 모드에서도 동일하다.

- **코더 = 『코더(복잡)』/『코더(단순)』 스폰.** 메인은 설계·지시·취합만 — 직접 구현 편집은 규약 위반.
- **리뷰어 = `/review` 스킬.** 단계 인자(prd/trd/code/doc)가 렌즈를 박는다(즉석 발명 금지). 스킵 금지. 판정 PASS/FIX/BLOCK, **불일치는 메인 임의 판정 금지 → 사용자에게.** 실행 정본 = review 스킬, 근거 = ADR-0031.
- **QA = `/qa` 스킬.** 테스트·타입체크 통과 ≠ 완료 — 실제 화면 동작 확인 전엔 미완.
- **TDD + 모듈 격리 — 강제.** 모든 모듈은 외부 의존을 seam으로 끊어 단독 실행 하네스를 갖춘다. 테스트는 누적해 워크스페이스 한 번에 회귀. (ADR-0012)
- **테스트 가능성 분리는 항상 검토하되 분리 여부는 사용자 결정.** (ADR-0012)
- **조사·웹서칭·대량 읽기 = 서브에이전트 일임.** OSS·설계 조사는 `/research`. 자율 모드에서도 생략 금지.
- **인라인 허용 예외:** 1~2줄 사소 수정 · 사소한 문서(오타·서식) · 조사 · 스파이크. **소스가 한 줄이라도 바뀌면 커밋 전 QA는 그대로 돌린다**(예외는 코더 스폰 생략이지 검증 생략이 아니다). **문서·설정만 바뀐 경우의 범위는 `/qa` 바인딩이 정한다.** **load-bearing 문서는 예외 아님 → `/review doc` 거쳐 커밋.**
- **역할→모델·effort 배치 = 전역 사전이 정본.** 메인 세션만 예외 명시 = xhigh.
- **커밋은 게이트 통과 후에만.**
### 브랜치·커밋

> 목적 하나 = **그래프에서 "이 커밋 뭉치가 무슨 작업이었나"를 나중에 식별.** 아래는 전부 그 한 줄에서 나온다. (사용자 결정 2026-08-16/17 — 옛 "브랜치 = 워크트리 이름" 규약 폐기)

- **브랜치 = 주제 하나.** master에서 판다. 그 주제의 모든 스텝·소주제를 여기 쌓는다 — 소주제마다 새로 파지 않는다.
- **자잘한 변경은 릴리스별 잡동사니 브랜치 하나(`v<다음 릴리스>/chore/misc`)에 모은다**(사용자 결정 2026-08-17). 오타·문서 한 줄·설정 한 줄은 *식별할 덩어리가 없어서* 자기 브랜치를 만들면 목록만 늘어난다 — 머지한 브랜치를 남기는 규약(아래)과 곱해지면 영구히 쌓인다. 그렇다고 master에 직접 커밋하지도 않는다: 통합 순서(push → CI 초록 → 머지)가 그 경로에서만 깨진다. **주제가 잡탕이라 이 브랜치의 머지 커밋 본문은 "무슨 작업이었나"를 못 남긴다 — 추적은 그 안 개별 커밋 제목이 진다.**
- **브랜치 이름 = `v<버전>/<타입>/<슬러그>`.** 버전은 **이 작업이 실려 나갈 다음 릴리스**다 — 매니페스트의 현재 버전이 아니다(그건 이미 태그가 붙어 나간 버전이라, 그 이름에 담으면 릴리스된 버전 폴더에 미출시 작업이 쌓인다 — 실발생 2026-08-17). 타입 어휘는 커밋과 같다(`feat`·`fix`·`refactor`·`docs`·`chore`·`ci`). 버전을 앞에 두는 이유 = git GUI가 `/`를 폴더로 접어 **릴리스별로 묶여 보인다**(사용자 결정 2026-08-17). **워크트리 이름(`wt1`·`wt2`) 금지** — 그래프에서 무슨 작업인지 안 보인다. ★브랜치 이름의 `v0.2.0/` 접두는 릴리스를 발행하지 않는다★ — 워크플로 트리거가 브랜치와 태그를 따로 받는다(실측 2026-08-17). 발행되는 건 **태그** `v*`뿐이다(아래).
- **머지한 브랜치를 삭제하지 않는다**(사용자 결정 2026-08-17). 삭제 조항은 이 규약에 없었는데 step-log의 옛 문장("통합 후 삭제")을 세션이 규약으로 읽고 지운 적이 있다(2026-08-17). 남긴다.
- **커밋 제목 = `S21: <타입>(<범위>): <요약>`.** 스텝 번호는 `docs/process/step-log.md`의 마지막 스텝을 잇는다 — **세션이 임의로 올리지 않는다**(새 스텝 시작 = 사용자 결정). 커밋 끊는 단위는 세션 판단. **머지 커밋엔 접두어를 붙이지 않는다.**
- **통합 = push → CI 초록 → `git merge --no-ff`.** ★fast-forward 금지★ — 머지 커밋이 없으면 덩어리 경계가 그래프에 안 남는다. **머지 커밋 본문에 무엇이 들어갔는지 항목으로 적는다** — 브랜치 이름은 자유 텍스트라 못 믿으므로, 이 본문이 "그 덩어리가 무슨 작업이었나"를 남기는 유일한 수단이다.
- **태그 = 버전 전용**(`v1.2.0`). 스텝·작업 태그 만들지 않는다. ★`v*` 태그를 push하면 릴리스가 발행된다★ — 되돌리기 어렵고 재시도도 안 된다(「검증」 절).

## 설계 결정 기록 (ADR)

> **강제.** 비자명한 설계 결정은 `docs/decisions/`에 박제한다. ADR의 핵심은 **거부한 대안과 그 이유** — 없으면 다음 세션이 같은 대안을 다시 꺼낸다.

- **새 결정 = 새 ADR. 번복 = 새 번호로 누적**(덮어쓰지 않는다). 인덱스·템플릿은 `docs/decisions/README.md`.
- **결정 날조 금지** — 거부한 대안·근거는 사용자가 준다. 채번·인덱스·폐기 도장 같은 기계 작업은 `/adr` 스킬.
- **상태는 ADR 본문 헤더에만 둔다.** 폐기 도장은 *폐기당한* ADR에 박는다 — 새 ADR에만 적는 단방향이면 옛 ADR만 읽는 세션이 죽은 결정을 따라간다.
- **손으로 베끼는 리스트를 만들지 않는다.** 다음 세션은 두 곳을 본다: 인덱스와 코드의 `// ADR-NNNN` 앵커(`rg "ADR-"`). 앵커는 코드와 한 몸이라 어긋나지 않는다.
- **load-bearing 코드엔 앵커 한 줄을 단다**(신규·수정분부터 점진). 위 두 발견 표면 중 하나를 *만드는* 쪽이라 빠뜨리면 다음 세션이 근거 없이 코드를 만난다.

---

# 설계 원칙

## 아키텍처 원칙 (불변)

> **모든 기능은 추상 인터페이스 위에 구현하고 내부 구현체는 교체 가능하게 짠다.** 특정 모델·전송 방식에 코드를 묶지 않는다. 이게 이 프로젝트를 10년 끌고 가는 법칙이다.
>
> **밖에서 이 절을 가리킬 땐 번호가 아니라 이름으로**(`CLAUDE.md 「LLM-우선 제어」`). 번호는 순서가 바뀌면 조용히 어긋난다 — 결정 기록 번호는 append-only라 안 밀리지만 이 번호는 밀린다. **번호가 남은 둘은 옛 참조가 많아 못 뗀 것이고, 생긴 순서지 중요도 순이 아니다.**

### 0. 판단 기준

추상화는 YAGNI가 아니라 **위험도 × 기간**으로 판단한다. **저위험 + 장기**(경계·seam·타입 — 나중에 바꾸면 비싼 것)는 지금 충분히 깔고, **고비용·불확실**(실측 안 된 내부)은 껍데기만 두고 실측 때 채운다.

### 코어 격리

**코어는 Tauri도 전송 방식도 모른다** — 출력·상태는 코어가 정의한 계약으로만 흐른다(그래서 앱 없이 테스트가 되고, 새 전송 경로는 계약 구현만 추가하면 흡수된다. 데몬 분리가 이것 덕이다). **벽은 스스로 서지 않으니 검증 게이트가 지킨다.** (ADR-0003)

### 백엔드 확장

**에이전트 백엔드 전용 코드는 없앨 수 없고 한 곳에 모을 수 있을 뿐이다 — 그 한 곳이 `backend`다.** claude의 `--session-id`/`--resume` 같은 지식이 거기서 새면 위반이고, manager는 dispatch만 부르고 transport는 백엔드를 모른다. 그래서 새 백엔드는 그 한 곳만 늘리면 흡수된다. (ADR-0004 · capability 산출 = ADR-0002/0030)

### 5. LLM-우선 제어

**모든 기능이 LLM으로 제어 가능해야 한다.** LLM이 메인 조작 주체, 사람 클릭은 보조 — 둘이 같은 핸들을 흔든다. 프론트는 렌더링만 소유하고 제어는 소유하지 않는다.

- **제어 표면은 이미 있다(실측). "없으니 만들어야 한다"고 읽고 두 번째 표면을 짓지 말 것.** 레이아웃·창·탭·슬롯은 백엔드 소유(ADR-0035/0057), 프론트는 `window.__engramCmd` 레지스트리(ADR-0055/0064).
- **남은 갭 3건:** 키바인딩 커스터마이징 미구현 · 버스 밖 병렬 핸들 공존(`__engramLayout`·`__engramChat` — "단일 표면"은 아직 지향) · 트리 선택 상태 미커버.
- **레이아웃은 디스크 영속이 없다** — 인메모리뿐이라 클라이언트 재시작 시 초기화된다.
- **새 UI 기능엔 LLM 호출 경로를 함께 만든다.** 기존 표면에 얹고 새 전역 핸들을 늘리지 않는다.

## 참조 구현

> 새 기능은 바닥부터 짜지 말고 **성숙 OSS가 같은 문제를 어떻게 풀었나 먼저 조사** → engram 제약으로 비교 → **선택지를 사용자에게 제시** → 굵은 결정이면 ADR. (TRD 단계, `/research`)

- **결함 수정도 같은 원칙.** 비자명 결함을 추측·매직넘버로 맞추지 않는다. 트리거·발화는 `docs/reference/debugging-conventions.md`. (ADR-0038)
- **참조는 패턴 차용이지 코드 복붙이 아니다.** 그대로 옮길 때만 라이선스 확인.
- **특정 도구를 조사 앵커로 선정하지 않는다.** 목록을 박으면 후속 조사가 거기 앵커링돼 직접 피어를 놓친다(실발생 2026-07-15).

---

# 코드 지도

## 백엔드 모듈 맵

> 개요만. 파일별 책임은 코드와 `// ADR-` 앵커가 단일 출처다. 멤버 목록 정본 = 루트 `Cargo.toml`.

**데이터 흐름:** `AgentManager → AgentSession(= OutputCore + dyn AgentTransport)`. 출력·상태는 `OutputSink`/`StatusSink` trait으로만 흐른다(「코어 격리」 계약의 실물). 종료 분류는 reaper 단일 소비자(ADR-0019).

- **core** — 에이전트 코어(agent·persistence·logging), tauri import 0. seam: `transport`·`backend`.
- **command** — 명령 버스 **도구**. **존재 이유 = 독립적으로 쓸 수 있고 순환을 막는다**(봉투·오류·선언 매크로·표·라우팅은 어느 소비자와도 무관하게 성립한다). ★**「의존 방향을 강제하려면 crate로 쪼개야 한다」를 이 crate의 근거로 쓰지 말 것 — 거짓이다**★(`pub(in path)`·모듈 규칙 테스트·이 저장소의 `rg`/`cargo tree` 게이트가 방향을 강제하는 실물 수단이다. ADR-0151 결정 4). **워크스페이스 crate 의존 0 · 명령 0개**는 그대로 사실이고 CI 의존 상한 게이트가 그것을 지킨다(「빌드·검증 명령」) — 바뀐 것은 그 사실을 존재 이유로 내세우던 *정당화*뿐이다. 어휘는 생산자 옆에서 선언한다. **불변식·격리 게이트·구성물 목록의 정본은 그 crate `src/lib.rs` 헤더.** 화살표는 **들어오는 쪽 한 방향뿐** — **`core`가 이 crate를 의존한다(코어의 첫 워크스페이스 의존 — 그 대가의 회계는 TRD S20 §5)** · `protocol`도 뒤따른다(Step 2). 그 반대는 없다. (ADR-0155 · 판정 기준 = ADR-0151)
- **messaging** — 메시징 커널. **워크스페이스 crate 무의존**(컴파일러 강제 벽). 접합은 lib이 소유한 포트 trait뿐이고 실물 어댑터는 데몬이 소유한다. (ADR-0110 — 턴 관측 명단·분류는 ADR-0127이 코어로 승격, TapHost 포트는 폐지)
- **net** — 데몬의 네트워크 행(WS·Origin·핸드셰이크·연결 수명·단일 writer·keepalive·팬아웃·프레임 포트·단일 인스턴스·portfile). **경계·격리 게이트·의존 상한의 정본은 그 crate `src/lib.rs` 헤더.** (ADR-0129)
- **daemon** — `AgentManager` 소유, 소켓 수락 루프와 네트워크 행 조립. 이벤트버스 single-push(ADR-0028). 메시징 호스트 조립실(ADR-0110).
- **discovery** — 데몬 발견·기동(`ensure_daemon`·WMI spawn·폴링) + `default_data_dir`. **판정 로직만 주입 seam 뒤로 분리**돼 WMI·sleep 없이 단독 테스트된다(crate 전체가 순수한 게 아니다). (ADR-0024)
- **protocol** — wire 계약 + codec + ts-rs 바인딩.
- **src-tauri** — 데몬 클라이언트 셸(창·트레이·discovery·로컬 command). **에이전트 in-proc 호스팅 X — `AgentManager` 소유는 데몬이다.** (ADR-0029)

### 핵심 불변식 (변경 금지)

- **kill 인과(2동사):** `transport.shutdown()`(child.kill+wait → TerminateJobObject → master drop) → `core.join_pump(5s)`. master drop → reader EOF → pump break → `core.finish` → done_tx. (ADR-0001)
- **finalize 1회:** `OutputCore.finalized.swap(AcqRel)` — terminal 전이·알림 정확히 1회(pump 단독). (ADR-0005)
- **락 순서:** sessions RwLock은 Arc clone 후 즉시 해제 → 그 뒤 내부 접근. status lock 보유 중 외부 호출 금지. emit은 subscribers clone 후 lock 미보유 send. (ADR-0006)
- **상태 알림 분담:** 과도기 `Exiting`=manager, terminal(`Killed`/`Exited`/`Failed`)=pump 단독. 프론트는 `status_changed`로 terminal 판정 금지 → `agent-list-updated`로 판정. (ADR-0005)
- **replay→live:** subscribers lock 보유 중 replay 전송(순서 역전 방지) + 프론트 seq dedup.
- **epoch:** 같은 AgentId 맵 교체마다 +1 → 프론트 재구독. 재구독 원칙은 `[agentId, epoch]`(ADR-0007)이고 실제 effect deps는 `viewId`가 앞에 붙는다(뷰 단위 구독 — ADR-0046 결정 3의 귀결).
- **소유권 분할:** transport=master/writer/child/shutdown/job · core=subscribers/replay/seq/status/finalized/drain_handle · session=id/cwd/epoch/cols/rows.
- **턴 관측 정리 = 두 지점뿐:** `finish` + `emit`의 finalize 재확인. **세 번째 호출자를 늘리면 인과가 갈라진다.** 빠지면 턴 도중 죽은 에이전트가 "진행 중"으로 남아 30분 상한(fail-open)이 풀 때까지 우편이 막힌다. (ADR-0127)
- **등록 순서:** sessions insert가 pump 시작보다 **먼저** — 뒤집히면 즉시 종료하는 세션이 명부에 오르기 전에 끝나 수거되지 않는다(런타임엔 무신호 — reaper 테스트가 회귀를 잡는다). (ADR-0019)

### 세션 복원

spawn 시 `--session-id`로 **sid를 우리가 통제** → `--resume` 무손실 복원. 복원 정확성은 이 sid에만 의존한다(추적 파일은 best-effort — 이걸로 기능 확장 금지). **resume 조기 종료는 fresh fallback 하지 않는다** — `Failed`로 직행하고 원인을 로그로 남긴다(자동 재spawn 없음. ADR-0082가 ADR-0008의 그 조항을 폐지). (정본 ADR-0008 + ADR-0082)

## 프론트 구조 (`src/`)

- **제어 표면(불변):** 컴포넌트·스토어는 `agentClient`(단일 `ProtocolClient`)에만 의존한다(개별 IPC 헬퍼 직접 호출 금지 — ADR-0011이 거부한 `ptyApi` 형태. 그런 모듈은 지금 없다). carrier = transport seam, 운영은 `TauriTransport` 고정(ADR-0036). 교체점은 transport이고, `WsTransport`는 테스트·직결 흔적이다(ADR-0020/0029).
- **폴더:** api · commands(제어 표면 — registry/dispatch/contributions) · components(layout/agent/slot/diff/ui) · i18n · lab · lib · pages · store · styles · theme · util.
- **구독(콜백) 수명은 `eventBus`가 한 곳에서 소유한다** — raw listener 수명은 각 등록 주체가 진다(`TauriTransport.close` · `viewStore` dispose). 등록 주체는 둘로 갈린다: **에이전트 이벤트 6종**(목록·상태·복원 결과·프로필·프리셋·연결 상태)은 `TauriTransport`가 `listen`을 걸고 `eventBus`는 `agentClient`의 추상 구독만 받는다 · **레이아웃·탭 이벤트**는 `viewStore`가 Tauri `listen`을 직접 건다(백엔드가 권위라 의도된 예외).
- **통합 micro-rules:** 구독 effect deps `[viewId, agentId, epoch]`(ADR-0046) · 구독 전 `terminal.reset()` · seq dedup · replay 경계 = gen 펜스 성공 마커 · `delete channel.onmessage`(null 아님) · 입력 가드 · resize debounce 50ms.

## 창 구성

**정적 창 2개**(`src-tauri/tauri.conf.json`): main(대시보드, visible) · agent-tree(hidden). 옛 정적 slot-popup 창은 제거됐다. **단 `/popup` 라우트는 살아 있다** — `PopoutPage`(탭을 소유하는 팝아웃 창, ADR-0057)가 쓰고, 창은 설정이 아니라 **런타임에 생성**된다.

## 기술 스택 (프론트)

React 19 + TS + Vite · Zustand · @xterm/xterm(+fit) · allotment · react-arborist · @monaco-editor/react · react-router(hash) · react-markdown + remark/rehype + katex(챗 마크다운·수식) · **Tailwind CSS v4 + shadcn/ui + lucide-react** · CSS 변수 테마 · Tauri v2 셸. 상세는 package.json.

- 스타일링은 Tailwind 채택(기존 "순수 CSS" 기조 전환 — ADR-0047). 테마는 CSS 변수를 유지하고 Tailwind 토큰이 `var()`를 참조한다.
- 테마 변수·폰트 정의는 `src/styles/theme.css`·`font.css`. `data-theme`은 `:root`에 dark/light/e-ink.

## 의존성 (변경 시 보고)

- `tauri = "2"` — Channel 무손실 Windows 실측 확인(spike).
- `portable-pty = "0.8.1"` · `uuid` · `thiserror` · `regex`(로그 마스킹) · `tracing` · `dunce`(cwd canonicalize UNC 회피).
- `windows`(Job Object) — `#[cfg(windows)]`.
- `inventory`(명령 선언 링커 수집) — S20 Step 1이 들인 **유일한 신규 서드파티 crate**다(`Cargo.lock` 신규 패키지 = 이것 + 워크스페이스 멤버 자신, 둘뿐 — 실측 2026-08-14). `core`를 타고 데몬·셸 릴리즈 바이너리에 함께 링크된다. (ADR-0155)
- `ts-rs = "10"` — 워크스페이스엔 이미 있었으나(`protocol`·`src-tauri`) **`core`의 production 의존으로 새로 들어왔다** — 선언 매크로가 `TS` derive를 달기 때문. 즉 코어가 TS 생성 도구를 운영 그래프에 안고 있다.

---

# 검증

## CI — push하면 자동으로 돈다 (`.github/workflows/ci.yml`)

**어느 브랜치든 push하면** 아래 「빌드·검증 명령」과 격리 게이트가 windows 러너에서 돈다. 로컬에서 같은 것을 다시 돌리지 않는다 — 범위 분담의 정본은 `/qa` 바인딩.

- ★**"같은 것"은 이제 바이트 단위로 같지 않다**★ — 워크스페이스 회귀에서 **CI는 `-- --test-threads=4`를 쓰지 않는다**(의도된 차이 — 사유는 아래 그 항목). 그러니 두 쪽 명령줄이 갈린 것을 드리프트로 보고 맞추지 말 것. 다른 게이트들은 그대로 같다.

- **CI가 못 하는 것 = GUI 실측**(창이 필요하다) **+ 실 claude 의존 테스트**(러너에 claude 없음 — 워크플로가 이름으로 제외하며 그 목록이 정본) **+ ADR-0130 재론 트리거**(게이트가 아니라 알림이라 CI에 못 얹는다). 셋 다 로컬 몫이다.
- **CI에만 있고 이 목록엔 없는 게이트 2건** — 생성물 sync(`protocol/bindings/` + `core/bindings/` — `git add -N -f` 뒤 `git diff --exit-code`. intent-to-add를 거치는 이유는 **`git diff`가 untracked 파일을 안 봐서** 처음 생성되는 `.ts`가 조용히 통과했기 때문이다)와 discovery async 반입. 로컬 fallback으로 돌 때 빠뜨리기 쉽다.
- **`v*` 태그를 push하면 릴리즈까지 간다** — 배포판 zip을 만들어 GitHub Release에 붙인다. 태그와 제품 버전이 다르면 빌드 전에 멈춘다(대조 대상 목록은 워크플로의 버전 게이트가 정본). 릴리즈 잡은 검증 3잡에 `needs`로 매달려 있어 **빨간 게이트에서는 배포가 시작되지 않는다.**
- 워크플로를 고치기 전에 **ADR-0131을 읽을 것** — 러너 단일화·검증 전용 범위·빌드/발행 잡 분리의 근거가 거기 있다.

## 빌드·검증 명령 (워크스페이스 루트에서 실행)

- `cargo test --workspace --exclude engram-dashboard -- --test-threads=4` — 전 workspace 회귀. src-tauri 패키지를 빼는 이유는 그 lib 테스트 타깃이 `0xc0000139 STATUS_ENTRYPOINT_NOT_FOUND`로 죽기 때문이다(실측 2026-08-05). **루트 bare `cargo test` 금지.** ★`-- --test-threads=4`도 빼지 말 것★ — 사내 보안 에이전트 DLL이 **프로세스 생성**을 후킹하는데, libtest 기본 병렬은 실 자식 프로세스를 띄우는 테스트를 한꺼번에 몰아 **터미널이 그 DLL 안에서 크래시하고 세션까지 함께 내려간다**(실 `engram.exe`를 띄우는 테스트 파일이 들어온 2026-08-11에 크래시가 0.6회/주 → 약 13회/주로 뛰었다). 4는 **실측 안전** — 43개 테스트 바이너리·1473 통과·0 실패, 약 2분, 터미널 생존(실측 2026-08-17 — 프로세스를 띄우는 스위트 전부 포함). **그래도 죽으면 2로 낮춘다.** 더 안전한 fallback = 테스트를 **터미널 프로세스 트리 밖에서 돌리고 출력은 파일로만** 받는 것(Windows 작업 스케줄러가 프로세스를 만든다 — `scripts/launch-detached.ps1`이 앱에 쓰는 그 기법이지만 인자를 못 받아 작은 래퍼 .bat이 필요하다).
  - ★**CI는 이 플래그를 쓰지 않으며 그것이 의도다**★ — 러너에는 크래시를 일으키는 그 사내 보안 에이전트가 없고, 병렬을 낮추면 **CI만 느려진다.** 그러니 `.github/workflows/ci.yml`을 이 줄에 맞춰 **"드리프트 수정" 하지 말 것.** 로컬만 4로 돈다. (이 예외의 정본이 여기다 — `docs/testing-strategy.md`·`/qa` 바인딩·`README.md`는 이 자리를 가리킨다.)
  - ★**병렬은 테스트 바이너리마다 걸린다**★ — 그래서 워크스페이스 명령에만 붙이면 불완전하다. **실 자식 프로세스를 띄우는 crate를 좁혀 돌릴 때도 같이 붙인다**(아래 core 줄이 그 예 · `-p engram-dashboard-daemon`도 해당 — 프로세스 레벨 CLI 스위트와 실 `.exe` spawn `#[ignore]` 분을 갖는다). **인메모리 단위 테스트뿐인 crate는 안 붙인다**(`command`·`protocol`·`messaging`·`net` — 층별 근거는 `docs/testing-strategy.md` §1).
- `cargo test -p engram-dashboard-core -- --test-threads=4` — 코어 unit + 통합(실 PTY로 단언). 플래그 근거 = 바로 위 항목(실 PTY = 실 자식 프로세스).
- `cargo test -p engram-dashboard-command` — 명령 버스 도구 단위(워크스페이스 의존 0 격리 하네스, ADR-0155)
- `cargo test -p engram-dashboard-protocol` — codec golden + ts-rs 바인딩
- `cargo test -p engram-dashboard-messaging` — 메시징 커널 단위(무의존 격리 하네스, ADR-0110)
- `cargo test -p engram-dashboard-net --all-features` — 네트워크 행 단위(ADR-0129). ★`--all-features`를 빼지 말 것★ — net의 기본 feature가 비어 있어 맨 명령은 `auth`만 컴파일해 6개만 돈다(켜면 42개, 실측 2026-08-14). **두 조합을 다 도는 것이 게이트 5.**
- `cargo test -p engram-dashboard --test layout_apply` · `cargo test -p engram-dashboard --test layout_commands` — 셸 패키지(`src-tauri`)의 **통합** 테스트 타깃 2종(IPC 핸들러 16종을 전송 중립 서비스 뒤로 옮긴 리팩터의 회귀망). `layout_apply`는 적용 서비스 자체(락이 어느 포트 안/밖에서 불리나)를 재고, `layout_commands`는 창·탭·슬롯 **명령 선언**(`layout::commands`)과 **인바운드 수신기**(데몬에서 온 명령을 적용 서비스로 라우팅하는 계층)를 잰다. ★**이 두 줄이 그 스위트들을 도는 유일한 경로다**★ — 맨 위 워크스페이스 회귀가 이 패키지를 통째로 빼기 때문. 빠진 근거인 `0xc0000139`로 죽는 것은 **lib 테스트 타깃뿐**이고, **통합 타깃은 둘 다 `engram_dashboard_lib`를 링크해 정상 기립한다**(실측 2026-08-17). 그래서 `--test`로 타깃을 각각 집는다 — `-p`만 쓰거나 `--tests`로 넓히면 죽는 lib 타깃을 도로 끌어온다. **`-- --test-threads=4`는 둘 다 붙이지 않는다** — 인메모리 fake 포트뿐이라 자식 프로세스를 하나도 안 띄운다(위 병렬 항목의 판정 규칙이 그대로 적용된 결과).
- `cargo build` — 전체 workspace 빌드. **★이걸로 지은 `engram-dashboard.exe`는 띄우지 않는다★** — `TAURI_CONFIG` 없이 도는 빌드는 debug 셸에 **release identifier**를 다시 찍고(`rerun-if-env-changed`라 변수를 빼는 것만으로 재빌드가 돈다 — 실측), 그 exe는 릴리즈 앱이 떠 있으면 창 없이 즉시 죽는다. 띄울 exe는 `node scripts/build-client-shell.mjs`로 짓는다(ADR-0137). ★**테스트 타깃은 컴파일하지 않는다**★ — 이 명령만으로는 위 셸 통합 스위트가 깨져도 안 보인다.
- `cargo fmt --check` — 포맷 게이트(검사형)
- `rg "^\s*use tauri" crates/engram-dashboard-core/src/` (→ 0줄) — 코어 격리 게이트. import 라인 앵커라 주석 자기인용이 오탐되지 않는다.
- `rg "engram_dashboard_(core|daemon|protocol|discovery|command)" crates/engram-dashboard-messaging/src/` (→ 0줄) — 메시징 커널 격리 게이트(ADR-0110)
- `cargo tree -p engram-dashboard-messaging --depth 1 --prefix none -e normal,dev,build --target all --all-features | rg "^engram-dashboard" | sort -u` (→ 정확히 1줄 = 자기 자신) — 메시징 커널 의존 상한. 바로 위 정규식이 **소스 텍스트**만 봐서 못 잡는 형태(따옴표·`[build-dependencies]`·rename)를 **해석된 의존 그래프**로 덮는다. **그중 가장 큰 구멍은 정규식이 crate 이름 알파벳을 손으로 박아 둔다는 것** — 새 crate는 누가 그 알파벳에 이름을 더할 때까지 **아예 안 보인다**(`command`를 더한 것이 이 게이트를 세운 계기다). net 상한 게이트와 같은 계기이고 플래그도 같은 이유로 줄이지 않는다.
- `cargo tree -p engram-dashboard-command --depth 1 --prefix none -e normal,dev,build --target all --all-features | rg "^engram-dashboard" | sort -u` (→ 정확히 1줄 = 자기 자신) — 도구 crate 의존 상한. **이 crate는 워크스페이스 의존 0을 유지하기로 돼 있고 이 줄이 그 벽이다** — 다만 그것이 crate로 존재하는 *이유*는 아니다(이유 = **독립적으로 쓸 수 있고 순환을 막는다** — 「백엔드 모듈 맵」 command 항목 · ADR-0151 결정 4). ★**정규식·상한 게이트에 공통으로 남는 구멍**★ — 둘 다 워크스페이스 멤버를 `engram-dashboard` **이름 접두**로 식별한다. 다른 이름을 단 멤버는 양쪽 다 그냥 통과한다. (ADR-0155)
- 프론트: `npm test`(vitest run) + `npx tsc --noEmit`(별도 typecheck 스크립트 없음)
- 전체 E2E: `scripts/`의 `run-*.bat` 런처로 띄운다(목록·용도 = README). **셸에서 직접 띄우지 않는다**(아래 「GUI 실측」). 로그 ON: `RUST_LOG=debug`(기본 warn — 분리 실행에선 스크립트 인자로 넘긴다)
- **CI 정본 = `.github/workflows/ci.yml`** — 로컬 게이트에 없는 검사가 더 있다(여기서 세지 않는다). 그중 wire 바인딩 동기 게이트는 실제로 깨진 적이 있으니, 생성물 drift를 남긴 채 밀지 말 것.

### 네트워크 행 격리 게이트

아래는 **발췌**다. 전체 목록·기대값·근거의 정본은 `crates/engram-dashboard-net/src/lib.rs` 헤더이고, 여기서 개수를 세지 않는다(세던 숫자가 게이트 추가 때 두 번 뒤처졌다).

- `rg "engram_dashboard_(daemon|messaging|discovery)" crates/engram-dashboard-net/src/` (→ 0줄) — 소스 참조
- `rg -o --no-filename "engram_dashboard_core::[A-Za-z0-9_:]+" crates/engram-dashboard-net/src/ | sort -u` (→ 정확히 2줄) — core 심볼 allowlist. 파일 단위가 아니라 **심볼 단위**다.
- `cargo tree -p engram-dashboard-net --depth 1 --prefix none -e normal,dev,build --target all --all-features | rg "^engram-dashboard" | sort -u` (→ 정확히 3줄) — 직접 워크스페이스 의존 상한. **해석된 의존 그래프**를 읽는다 — 매니페스트 텍스트 grep으로 바꾸지 말 것(rename·테이블 형·비활성 target·optional에 뚫린다). 플래그도 줄이지 않는다.

## GUI 실측 (`scripts/cdp.mjs`)

실제 Tauri 창(WebView2)에 CDP로 붙어 스크린샷·DOM 조회·실제 `invoke` 호출까지 한다(node 내장 WebSocket만, **Windows 전용**).

- **★앱을 셸에서 직접 띄우지 않는다★** — `scripts/`의 런처나 `scripts/launch-detached.ps1`로 띄운다. 셸에서 띄우면 앱이 터미널의 자손이 되고 **앱 출력이 그 사슬을 거슬러 올라간다** — 그 조합에서 터미널이 반복 크래시해 앱까지 함께 내려간다(실측 2026-08-16). **끊어야 할 조건이 둘이다 — 프로세스 트리 밖 + 출력은 파일로만.** `start`·백그라운드 잡·`nohup`은 둘 다 못 끊으므로 대체재가 아니다.
- 절차(기동 인자·환경변수·PID·teardown)는 `/qa` 바인딩 §full이 갖는다. 여기 되올리지 않는다.

## 컨벤션

> **주석 규약 정본 = `/code-conventions`의 주석 규약 파일.** 코더·리뷰어 지시서에 그 파일을 주입해 쓴다. **이 자리에 규칙을 베끼지 않는다** — 없는 정본을 가리키던 옛 문장이 세션마다 즉흥 규칙을 만들어냈고, 베껴 두면 같은 일이 두 출처로 반복된다.

- 자격증명을 프로필 env에 넣지 말 것(평문 저장 — persistence가 경고한다).
- 모듈마다 build/test/커밋. 커밋 메시지 끝에 Co-Authored-By 트레일러.
