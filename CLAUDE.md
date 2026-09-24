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
- **통합 = push → CI 초록 → 머지**(실행 절차는 아래 master 항목 — ★`git merge --no-ff`를 돌릴 워크트리가 없다★). ★fast-forward 금지★ — 머지 커밋이 없으면 덩어리 경계가 그래프에 안 남는다. **머지 커밋 본문에 무엇이 들어갔는지 항목으로 적는다** — 브랜치 이름은 자유 텍스트라 못 믿으므로, 이 본문이 "그 덩어리가 무슨 작업이었나"를 남기는 유일한 수단이다.
- ★**master를 어느 워크트리도 상시 체크아웃하지 않는다**★ — 붙들고 있으면 나머지 워크트리가 통합을 못 한다. 쥐고 있으면 `git checkout --detach`로 놓는다.
  - **통합은 체크아웃 없이 한다.** 옛 master SHA를 먼저 잡아 둔다(③이 쓴다).
    1. `git merge-tree --write-tree master <브랜치>` → 첫 줄이 트리 해시. ★**종료코드로 가른다 — 0 만 깨끗이고 그 외는 충돌**★: 충돌해도 **트리 해시는 그대로 찍히고 그 트리엔 `<<<<<<<` 마커가 들어 있다**(실측). 첫 줄만 잡으면 마커째 master에 실리고 **아래 확인 넷이 하나도 못 잡는다.** 충돌이면 멈추고, 해소는 임시 워크트리(`git worktree add`)에서 평범한 `git merge`로 한다.
    2. `git commit-tree <트리> -p master -p <브랜치> -F <본문파일>` — ★부모 순서 = master 먼저★(뒤집으면 `--first-parent`가 master 줄기 대신 토픽을 탄다). 본문이 여러 항목이라 `-m` 대신 `-F`나 stdin을 쓴다.
    3. `git update-ref -m "<사유>" refs/heads/master <새 커밋> <옛 master SHA>` — ★**넷째 인자(옛 SHA)를 빼지 말 것**★: 그 사이 다른 워크트리가 master를 옮겼어도 **거부되지 않고 덮어써 남의 머지가 사라진다**(실측). `-m`이 없으면 reflog 사유가 빈다.
  - ★**체크아웃된 채로 `update-ref` 금지**★ — 그 워크트리가 어긋난다.
  - **밀기 전 확인:** 양쪽 부모가 조상인가(`git merge-base --is-ancestor`) · **첫 부모가 옛 master인가**(`git rev-list --parents -n1` → 3토큰, 둘째 토큰이 옛 master) · 트리가 1의 해시와 같은가(`git rev-parse master^{tree}`) · 실물이 실렸나(`git ls-tree`).
  - ★**이 경로는 커밋 훅을 건너뛴다**★(`pre-commit`·`commit-msg`·`pre-merge-commit` 등). `reference-transaction`과 push의 `pre-push`는 그대로 탄다.
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
- **남은 갭:** 키바인딩 커스터마이징 미구현 · 트리 선택 상태 미커버 · **챗 스타일은 릴리스 빌드에 제어 경로가 없다**(ADR-0169) · **버스 밖 전역 핸들 공존** — "단일 표면"은 지향이지 현황이 아니다.
  - ★**그 핸들 명단을 여기 적지 말 것 — 명단은 낡고, 낡은 명단은 없는 갭을 지키게 만든다**★. **찾는 법 = `rg "\)\.__ENGRAM_|\)\.__engram" src/ -g '!*.test.*'`** — `).__NAME` 대입 형태만 잡아 **살아 있는 전역 대입만** 돌려준다(이름 문자열을 그냥 grep하면 타입 주석·테스트·주석까지 딸려 와 열 배로 부푼다). ★**그 결과에서 `__engramCmd` 한 줄은 곁문이 아니라 정식 표면이다**★ — 위 항이 가리키는 그것이고, 나머지가 갭이다.
  - ★**"왜 아직 있나"를 각 핸들 주석에서 찾을 수 있다고 기대하지 말 것**★ — 스스로 임시라 자인하는 것도 있고, **정식 §5 표면인 척 적혀 있는 것도 있다.** 자인하던 핸들들은 이미 걷혔고, 남은 쪽이 더 조용하다. 판정은 이 절이 하고 주석은 근거가 아니다.
- **레이아웃은 디스크 영속이 없다** — 인메모리뿐이라 클라이언트 재시작 시 초기화된다.
- **새 UI 기능엔 LLM 호출 경로를 함께 만든다.** 기존 표면에 얹고 새 전역 핸들을 늘리지 않는다.

## 참조 구현

> 새 기능은 바닥부터 짜지 말고 **성숙 OSS가 같은 문제를 어떻게 풀었나 먼저 조사** → engram 제약으로 비교 → **선택지를 사용자에게 제시** → 굵은 결정이면 ADR. (TRD 단계, `/research`)

- **결함 수정도 같은 원칙.** 비자명 결함을 추측·매직넘버로 맞추지 않는다. 트리거·발화는 `docs/reference/debugging-conventions.md`. (ADR-0038)
- **참조는 패턴 차용이지 코드 복붙이 아니다.** 그대로 옮길 때만 라이선스 확인.
- **특정 도구를 조사 앵커로 선정하지 않는다.** 목록을 박으면 후속 조사가 거기 앵커링돼 직접 피어를 놓친다(실발생 2026-07-15).
- ★**아래 목록은 표본이지 전수가 아니다 — 여기 있는 것만 보고 조사를 마쳤다고 하지 않는다.**★ 위 앵커 금지 조항이 그대로 유효하다. ★**이 분야는 크고 어리다**★ — 한 큐레이션 목록만 223개를 세고 상위 편중이 극심하다(실측 2026-09-19). 그러니 목록 갱신 자체를 조사 대신으로 쓰지 않는다.
  - **클론 위치 = `I:\Engram_Workspace\opensource\`**(이 repo 기준 `../../../Engram_Workspace/opensource`). **repo 밖이라 추적되지 않는다** — 다른 PC엔 없을 수 있고, 없으면 그때 clone한다. ★**이 PC 의 클론 상태(체크아웃 위치·깊이·빌드 여부) = `I:\Engram_Workspace\opensource\README.md` 「클론별 주의」**★ — 여기 베끼지 않는다(같은 사실을 두 곳에 적었다가 서로 다르게 틀린 적이 있다, 2026-08-21). 아래 항목에는 PC 와 무관한 사실만 둔다. **옛 `I:\Engram_Workspace\references\` 는 2026-09-23 에 여기로 합쳤다** — 옛 기록이 그 경로를 가리키면 이 폴더로 읽는다. **찾는 법 = `git grep -n "Engram_Workspace[\\/]references"`**(bash·PowerShell 양쪽에서 같게 돈다 — 이 줄과 그날 step-log 항목 자신도 걸린다). ★**단 ttyd·zellij 는 그렇게 읽으면 틀린다**★ — 두 옛 사본은 지웠고 여기 있는 것은 **다른, 더 새 스냅숏**이다(`docs/process/step-log.md:122` 의 갭 분석이 읽은 Zellij 가 그 옛 사본이다). 옛 사본이 어느 커밋이었는지는 기록이 없다. Engram 과 무관한 학습용 클론은 `I:\Engram_Workspace\study\repos\` 에 따로 둔다. ★**「아직 안 받은 것」 절은 이름만 있고 실물이 없다**★ — 그 폴더에 있다고 가정하지 말 것.
  - ★**아래 ★·활동성 수치는 2026-09-19 실측이고 낡는다**★ — 「죽었나」를 판정할 땐 베끼지 말고 그 자리에서 `gh api repos/<경로>`로 다시 본다.
  - ★**이 축의 합의 둘 — 개별 피어보다 이게 먼저다**★ (서베이 2026-09-19)
    - ★**붙는 클라이언트에게 스크롤백을 흘려보내는 곳이 하나도 없다.**★ 성숙한 구현은 예외 없이 **현재 화면만 다시 그리고**(서버가 VT를 돌려 화면 상태를 주거나, 경계 있는 바이트를 되감거나, 번호로 빠진 것만 따라잡거나), 스크롤백은 **따로 명시적으로 당기는 별도 경로**다. ★**이력과 화면을 한 줄기 스트림으로 되감으려던 곳이 있었고, 화면이 깨져서 되돌렸다**★ — `coder/boo` #22: 되감기 대상이 **Claude Code TUI** 였고, 포맷터가 끝의 빈 줄을 잘라내는 바람에 커서 복원이 **정확히 3줄 어긋났다**. 우리 replay 버퍼를 늘릴 생각이 들 때 먼저 읽을 사례다.
    - ★**Windows 에서는 데몬이 PTY를 *만든* 프로세스여야 한다.**★ `HPCON` 은 그것을 만든 프로세스가 쥐고 있는 동안 세션을 살려 두고, **다른 프로세스에 넘기는 API가 문서에 없다** — POSIX 의 `reptyr`(남의 프로세스를 내 터미널로 끌어오는 것) 같은 수단이 ConPTY 에는 성립할 수 없다. 그래서 **임의 프로세스를 나중에 입양하는 길은 Windows 에 없고**, 실제로 그걸 하려는 wrapper 넷이 전부 미성숙(archived·별 1개·「세션 지원 없음」 자인)이다. ★**확신도 = 가능성 높음**★ — MS 문서 두 곳의 필연적 독해이고 「못 넘긴다」를 문장으로 적어 둔 곳은 없다.
  - **받아 둔 클론**
    - **herdr**(`herdrdev/herdr` · 39.5k★ · Apache-2.0 · 활발) — Rust 단일 바이너리 **백그라운드 서버가 터미널을 소유**하고, 에이전트는 계속 돌며 **어디서든(ssh 포함) 재부착**한다. 에이전트용 socket API도 있다. ★우리 데몬 모델의 가장 가까운 피어★이고 **다른 프로젝트가 이것을 터미널 백엔드로 골라 쓴다**(awslabs CAO). ★**codex 세션 id 회수의 실물 선례가 여기 있다**★ — `SessionStart` **훅 하나만** 설치하고 stdin 페이로드에서 `session_id` 를 받아 되돌려 보낸다(POSIX = 소켓 · **Windows = 자기 바이너리 CLI 재진입**). **신뢰(trust)는 자동화하지 않고** 심사 화면을 「막힘」으로 띄워 **사람이 한 번 승인하게** 넘긴다. ★**「신뢰를 안 다루니 id 를 못 받는다」로 적혀 있던 옛 서술은 틀렸다**★(클론 정독 2026-09-19 — 그 추론이 CLAUDE.md·조사 보고서 양쪽에 실려 있었다). ★**2026-09-23 전 문서의 herdr `파일:줄` 포인터는 옛 master `624dfd4`(2026-08-20) 기준이다 → `git show 624dfd4:<경로>` 로 연다**★(master 는 그 뒤로 움직였다 — 다른 커밋에선 줄이 어긋난다). 그 커밋은 지금도 master 의 조상이다(`gh api repos/herdrdev/herdr/compare/624dfd4796559042ec13ccf4d4b54374902ab81d...master` → `behind_by=0`, 2026-09-23). ★**새로 받은 shallow 클론에는 그 커밋이 없어 이 명령이 실패한다**★ — 전체 또는 `--filter=blob:none` 클론이면 들어 있다(clone 자체는 돌려 보지 않았다). `v0.9.1` 의 소스 빌드엔 Zig 0.16.0 이 필요하다 — README 개발 절에는 없고 `AGENTS.md` 에만 있다(상세 = `docs/research/oss-peer-hands-on-2026-09-23.md`).
    - **orca**(`stablyai/orca` · 72k★ — 이 분야 최대 · MIT · 활발) — 여러 에이전트 백엔드를 **워크트리별로 나란히** 돌리는 오케스트레이터 + GUI. 「백엔드 확장」과 겹친다. ★**디스크 replay 선례가 여기 있다**★ — 데몬이 ANSI 버퍼를 주기적으로 디스크에 체크포인트하고 cold-restore 시 되감는다. **훅 신뢰를 app-server RPC로 부여하고**(우리와 같은 경로) 출처 검증 실패 시 **resume 인자를 떼고 깨끗한 새 세션으로 띄운 뒤 고지**한다(ADR-0208이 참조). 단 백엔드는 **디렉터리 하나당 하나**로 갈라 놔 플러그형이 아니다. ★**그 「Orchestration」은 앱이 알아서 모는 기능이 아니다**★ — 실행 중인 앱 런타임이 Run·Task·Dispatch 상태와 라우팅을 쥐지만 워커를 스케줄·배치하지 않고, **코디네이터 에이전트가 CLI + 스킬로 그 런타임을 불러 몬다**(그 문서가 읽은 소스는 2026-08-20 스냅숏 `df7460af` 다 — Windows 쪽 이슈 현황까지 상세 = `docs/research/oss-peer-hands-on-2026-09-23.md`).
    - **zellij**(`zellij-org/zellij` · 35.5k★ · MIT · 활발) — 멀티플렉서. 세션 생존 + 클라이언트 재부착 + 페인/탭 구조를 서버-클라이언트로 나른다.
    - **ttyd**(`tsl0922/ttyd` · 12.4k★ · MIT · 활발) — PTY를 웹소켓으로 나르고 **크기를 협상하는** 최소 구현(리사이즈 메시지 → ConPTY·POSIX 양쪽).
    - **paseo**(`getpaseo/paseo` · 17.7k★ · 활발) — **데몬이 에이전트를 소유하고 desktop·web·mobile·CLI 가 붙는다**. 다중 클라이언트 재부착이 우리와 겹친다. **에이전트 경로엔 PTY가 없다**(백엔드마다 SDK·JSON-RPC이고 codex 는 `codex app-server` + 줄단위 JSON-RPC). ★**단 「그래서 전송 계층은 참조가 안 된다」는 과했다**★ — 자기 셸 탭용으로 node-pty 를 싣고 **ConPTY 우회를 주석으로 적어 둔다**(실측 2026-09-19). 에이전트 전송은 참조가 안 되고 **터미널 탭 전송은 된다.** 재개는 백엔드에 위임한다 — 핸들 하나(`nativeHandle`)에 codex thread id·claude resume 토큰을 담고, 재접속 시 provider 이력을 다시 흘려보낸다. ★**라이선스는 AGPL-3.0이 아니라 Apache-2.0이다**★ — 포함된 제3자 구성요소는 각자의 라이선스를 유지한다는 통상 고지가 붙어 있고, 그 때문에 GitHub 자동 식별이 `NOASSERTION`으로 실패한다. ★그 검출 실패를 「라이선스가 어긋났다」로 읽지 말 것★ — LICENSE 본문이 Apache-2.0 전문을 싣고 있다(적대 리뷰가 잡은 오독, 2026-09-19).
    - **t3code**(`pingdotgg/t3code` · 23k★ · MIT · 활발) — 서버가 프로바이더·PTY·git·파일을 소유하고 데스크톱·웹·모바일이 **인증된 RPC(HTTP+WS)로** 붙는다. 백엔드는 어댑터 seam(`ProviderAdapter`)이고 ACP 클라이언트와 codex app-server 클라이언트를 따로 갖는다. ★**단 그 조합은 고유 축이 아니다 — paseo 가 같은 모양이다**★(어댑터 분리 + codex 직접 구현 + 범용 ACP 어댑터 + provider 플러그인. 적대 리뷰가 잡았다, 2026-09-19). **그래서 이건 「새 축」이 아니라 같은 모양의 더 활발한 둘째 표본이고**, 그 값어치는 ① 릴리즈가 살아 있고 Windows 설치본이 있다 ② PTY와 구조화 경로를 **한 서버가 함께** 소유한다는 데 있다. 축이 겹치므로 **paseo 를 이미 읽었다면 여기서 새로 배울 것은 적다.**
    - **vibe-kanban**(`BloopAI/vibe-kanban` · Apache-2.0) — 워크트리 하나에 에이전트 하나를 CLI 하위 프로세스로 띄우는 매니저(Rust + TS/React). 이 repo 의 인용은 예컨대 이런 자리다(전수 아님 — **찾는 법 = `git grep -n -i "vibe.kanban"`**) — ① 서드파티 매니저 비교의 한 표본(`docs/research/multi-agent-hosting-orchestration-research-2026-06-22.md` §B) ② 에이전트 간 메시징의 **도구 설명문 크기** 비교(97 바이트 단독 — ADR-0211) ③ codex app-server 프로토콜 이주 선례(`newConversation` → `thread_start`/`thread_fork` — `docs/process/S21-codex-backend/trd-phase2a.md` D5 · 공식 `codex_app_server_protocol` crate 를 쓴다는 원자료 = `.claude/handoff/attachments/codex-app-server-survey.md:61`) ④ ★**설계 결정 하나를 받쳤다**★ — codex 프로세스 모델을 「대화마다 하나」로 확정할 때 paseo 와 함께 든 피어 선례다(`.claude/handoff/history/20260908-013000-codex-phase1-착지-리뷰-FIX-대기.md:148`). ★**README 가 서비스 종료(sunsetting)를 알린다**★(`README.md:19`) — 설계만 본다.
    - **tmux**(`tmux/tmux` · ISC) · **mosh**(`mobile-shell/mosh` · GPL-3.0 + OpenSSL 링크 예외) · **gotty**(`sorenisanerd/gotty` · MIT) · **cline**(`cline/cline` · Apache-2.0) — 초기 설계의 참조다(옛 `references\` 에서 옮겨 왔다 · 라이선스는 각 클론의 `LICENSE`·`COPYING` 본문으로 확인, 2026-09-23). **tmux·mosh** = 데몬 참조 3대장 중 둘(ADR-0013 — tmux = 데몬 모델·attach/detach·control mode · mosh = 원격·재연결 견고성)이고 데몬 수명·콘솔 뷰어 분리의 prior-art(ADR-0015)와 갭 분석(`docs/process/step-log.md:122` — WS keepalive·resize 협상)을 받쳤다. **gotty** = PTY→WebSocket 브리지, 전송 참고용 보조(ADR-0013). ★**단 「보조」라서 지워도 된다고 읽지 말 것 — 살아 있는 코드가 그 패턴에 기댄다**★: 구독 직후 초기 크기를 한 번 보내는 resize 가 gotty 패턴이라고 적혀 있다(`src/components/slot/TerminalSlot.tsx:300` · `src/lab/terminal/TerminalView.tsx:10,44`). **cline** = 채팅 렌더 참조 — 잎 컴포넌트 포트와 그 Apache-2.0 귀속(ADR-0048)을 ADR-0050 이 함께 걷어 코드베이스엔 남지 않았다(ADR-0047 의 OSS 후보 조사에도 나온다).
  - **아직 안 받은 것** (필요할 때 clone — 위 받아 둔 클론이 안 덮는 축을 메운다. 한때 여기 있던 t3code 는 받아 두었으므로 위로 옮겼다, 2026-09-23)
    - **superset**(`superset-sh/superset` · 14.4k★ · Elastic-2.0 · 활발) — ★**디스크 매니페스트로 재입양한다**★: 데몬이 PTY를 쥐고, 디스크의 pid·port·secret 매니페스트로 cold restore 후 다시 붙는다. **우리 discovery·portfile 축의 유일한 피어다.** ★이름 충돌 주의★ — `apache/superset`(74.8k★)은 BI 도구로 무관하다.
    - **Coder v2**(`coder/coder` · 15.3k★ · AGPL-3.0 · 활발) — ★**「replay 를 밀면서 live 로 넘어가는 그 틈」에 대한 외부 선례가 여기 하나뿐이다**★. PR #9094 가 그 결함을 이름 붙여 고쳤고 사유가 커밋에 그대로 적혀 있다 — 조건변수 대기가 락을 놓기 때문에 **버퍼를 쓴 뒤 명부에 올리기 전 사이의 출력이 사라졌다**. 우리 「subscribers lock 보유 중 replay 전송」 불변식(「핵심 불변식」)이 막는 바로 그것이다. 덧붙여 **replay 구현 둘을 나란히 싣고 자기 소스가 그중 하나를 「(buggy) buffer replay fallback」이라 부른다**(64 KiB 링버퍼 · 리눅스에서만 GNU screen 위임).
    - **sshx**(`ekzhang/sshx` · 7.7k★ · MIT · **2025-06 이후 정체**) — ★**번호로 이어받는다**★: 클라이언트가 `Subscribe(shell, chunk번호)`로 **스트림의 어디서 재개할지 자기가 지목**하고, 서버는 그 지점부터 청크를 돌려준다. 셸당 2 MiB 상한이고 넘치면 오래된 것을 버리며 **인덱스는 단조 유지**돼 잘린 지점을 물으면 현재 바닥부터 준다. 우리 seq dedup 축의 최근접 선례. 정체 중이라 **설계만 본다**.
    - **qm**(`yc-software/qm` · 15.2k★ · MIT · 활발) — ★**재부착도 CLI resume도 아닌 제3의 재개 모델**★: PTY를 아예 안 쓰고, append-only 기록이 **모델 컨텍스트의 유일한 공급원**이며 멱등 키로 같은 자식을 회수한다. 백엔드는 harness 라우터 + 백엔드당 파일 하나. **우리가 아는 재개 모양이 둘뿐이라고 가정하지 않게 하는 반례.**
    - **ACP**(`agentclientprotocol/agent-client-protocol` · 4.3k★ · Apache-2.0 · 활발) — 도구가 아니라 **프로토콜**이다(JSON-RPC 2.0 over stdio). 클라이언트가 에이전트를 자식으로 띄우고 **에이전트가 `sessionId`를 발급**한다. **재개가 두 갈래다** — `session/load`(이력을 되감는다) · `session/resume`(되감지 않는다). ★**「신뢰 게이트가 없다」고 적지 말 것 — 선택적 에이전트 인증이 있다**★(`authenticate`·`authMethods`·터미널 인증·`logout`). 우리에게 없는 것은 **훅 출처와 resume 토큰의 provenance 결속**이고 그건 이 규격이 표준화하지 않는다(적대 리뷰가 잡은 과장, 2026-09-19). Zed·JetBrains·Kiro 채택. 「백엔드 확장」을 코드가 아니라 프로토콜로 푸는 갈래.
  - **뺀 것과 사유** (같은 것을 다시 올리지 않게 — 2026-09-19)
    - **wezterm** — 같은 축을 zellij가 덮고(Rust 멀티플렉서 · 세션 생존 · 재부착 · 페인/탭 구조), 「데몬이 터미널을 소유」 쪽은 herdr이 더 가깝다. 유일하게 고유했던 **버전 박힌 mux wire 프로토콜**은 우리 자체 조사(`docs/research/daemon-version-mismatch-2026-08-14.md`)가 이미 덮는다. ★클론은 2026-09-23 에 지웠다 — 지우기 전 체크아웃은 `770d8e1a7519a9a698090cd7d717d0c64aa0a755`(2026-08-20)였고, 그 전 문서의 wezterm `파일:줄`(예: `docs/research/connection-dispatch-concurrency-survey-2026-09-17.md` 의 「로컬 클론 실측」)은 이 커밋을 읽은 것이다★(그 문서의 `mux/src/lib.rs:103` 이 이 커밋과 줄까지 맞는다). **되찾는 법** = `git clone --filter=blob:none <원격>` 뒤 `git checkout <SHA>` — 그 커밋은 지금도 기본 브랜치의 조상이다(`gh api repos/wezterm/wezterm/compare/<SHA>...main` → `behind_by=0`, 2026-09-23). clone 자체는 돌려 보지 않았다.
    - **octoally** — 100★·2개월 정체·비OSI 라이선스(Apache-2.0 + Commons Clause)이고, 같은 tmux 위임 니치를 훨씬 활발한 피어들이 점유한다. ★**그 항목이 달고 있던 「SQLite 되감기는 순수 셸 탭 전용, 에이전트 탭은 `capture-pane`」 서술은 양쪽 다 틀렸다**★(로컬 클론 실측 2026-09-19): 삽입은 워커의 출력 핸들러에서 **세션 모드 분기 없이** 큐에 들어가고(`agent` 모드도 같은 tmux·`pipe-pane` 경로를 탄다), `capture-pane` 은 **주경로가 아니라 폴백**이다(인메모리 버퍼가 비었거나 리사이즈가 있었을 때만, 2초 TTL 캐시). 주경로는 약 200 KB 인메모리 버퍼다. ★**그러니 이 항목은 「낡아서」가 아니라 「틀린 서술을 실어 나르고 있었기 때문에」도 뺀다 — 되살리려면 그 서술부터 다시 쓸 것.**★ 디스크 replay 선례 자리는 위 orca 가 대신 채운다. 위 실측을 한 클론(`ai-genius-automations/octoally`)은 2026-09-23 에 지웠다 — 지우기 전 체크아웃은 `47ca9cb03a159a2d5722fa038ce9fcee1195be43`(2026-07-11)이고, **그 커밋은 2026-09-23 현재도 기본 브랜치 끝 그대로다**(`gh api …/compare/<SHA>...main` → `identical`). 그러니 2026-07-11 이후 문서의 octoally 인용(ADR-0208·ADR-0216·`docs/research/codex-session-id-recovery-survey-2026-09-19.md` 등)이 읽은 것도 이 커밋이다. 서술을 다시 쓰려면 wezterm 항목과 같은 법으로 되찾는다.

---

# 코드 지도

## 백엔드 모듈 맵

> 개요만. 파일별 책임은 코드와 `// ADR-` 앵커가 단일 출처다. 멤버 목록 정본 = 루트 `Cargo.toml`.

**데이터 흐름:** `AgentManager → AgentSession(= OutputCore + dyn AgentTransport)`. 출력·상태는 `OutputSink`/`StatusSink` trait으로만 흐른다(「코어 격리」 계약의 실물). 종료 분류는 reaper 단일 소비자(ADR-0019).

- **base** — 바닥 인프라 잎 crate. 지금 입주자는 둘 = `logging`(tracing 전역 초기화 + 파일 로그) · `platform`(PID liveness·프로세스 시작시각·자식 PID 열거). **워크스페이스 crate 의존 0 · 도메인 지식 0**이고 그 벽은 CI 의존 상한 게이트다(「빌드·검증 명령」). ★**게이트는 그것 하나가 아니다**★ — 상한 게이트는 워크스페이스 멤버만 세므로 서드파티 `tauri`도, 입주자끼리 엮이는 것도 못 본다. 나머지 둘(`use tauri` 0줄 · 입주자 상호 무참조)이 그 두 축을 각각 맡는다. **셋의 정본은 그 crate `src/lib.rs` 헤더.** **입주 조건 셋(소비자 둘 이상 · 도메인 지식 0 · 입주자끼리 무참조)의 정본은 그 crate `src/lib.rs` 헤더**이고, ★**셋째 입주자를 받기 전에 ADR-0175 「거부한 대안」 첫 항목(로깅·플랫폼을 각각 독립 crate로)을 다시 연다**★ — 그 지점이 `bevy_core` 경로다. `bindings/`를 만들지 않는다(serde·ts-rs·`declare_commands!` 전부 없음). (ADR-0175 결정 1)
- **agent** — 에이전트 런타임(수명·펌프·`transport`·`backend`·도메인 모델·`commands`·`types`·`persistence`·`platform`의 Job Object 래퍼), tauri import 0. seam: `transport`·`backend`. ★**`logging`과 PID 판정 헬퍼는 여기 없다 — 2026-08-25에 `base`로 이사했다**★(ADR-0175 결정 1). 남은 `platform`은 Job Object 래퍼 하나뿐이다. **2026-08-25에 `core` → `agent`로 개명하며 안쪽 `agent/` 모듈을 crate 루트로 접었다** — 옛 `engram_dashboard_core::agent::types::X`가 지금은 `engram_dashboard_agent::types::X`다(ADR-0175). 옛 기록에서 **crate 이름꼴**(`engram-dashboard-core`·`engram_dashboard_core::`)을 만나면 이 crate를 가리킨다 — ★**맨 `core`는 아니다**★: 이 저장소에서 그 낱말은 원칙 「코어 격리」이기도 하고 `OutputCore`·`connection_core` 같은 식별자의 조각이기도 하다.
- **command** — 명령 버스 **도구**. **존재 이유 = 독립적으로 쓸 수 있고 순환을 막는다**(봉투·오류·선언 매크로·표·라우팅은 어느 소비자와도 무관하게 성립한다). ★**「의존 방향을 강제하려면 crate로 쪼개야 한다」를 이 crate의 근거로 쓰지 말 것 — 거짓이다**★(`pub(in path)`·모듈 규칙 테스트·이 저장소의 `rg`/`cargo tree` 게이트가 방향을 강제하는 실물 수단이다. ADR-0151 결정 4). **워크스페이스 crate 의존 0 · 명령 0개**는 그대로 사실이고 CI 의존 상한 게이트가 그것을 지킨다(「빌드·검증 명령」) — 바뀐 것은 그 사실을 존재 이유로 내세우던 *정당화*뿐이다. 어휘는 생산자 옆에서 선언한다. **불변식·격리 게이트·구성물 목록의 정본은 그 crate `src/lib.rs` 헤더.** 화살표는 **들어오는 쪽 한 방향뿐** — **`agent`가 이 crate를 의존한다(코어의 첫 워크스페이스 의존 — 그 대가의 회계는 TRD S20 §5)** · `protocol`도 뒤따른다(Step 2). 그 반대는 없다. (ADR-0155 · 판정 기준 = ADR-0151)
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
- **화신 표식(필드명은 아직 `epoch`):** 화신마다 새로 뽑는 32비트 난수 — **비교는 일치/불일치만**(대소로 "더 새 것"을 유도 금지). 읽기는 건너뛰고 쓰기는 `0` 자리채움 — ★이 비대칭은 의도★(ADR-0163). ★**그 비대칭의 범위는 `agents.json` 디스크 serde 뿐이다 — wire 로 넓히지 말 것**★: 소켓으로 흐르는 표식에서 `0` 은 2^-32 확률로 실제 뽑히는 정당한 값이라, 거기서 `0` 을 특별취급하면 그것이 회귀다. 재부착 계기는 소켓이 아니라 **권위 명부 관측 단독**이고, 구독 effect deps는 `[viewId, agentId]` — ★표식을 넣지 않는다★(넣으면 replay 도착 전에 화면이 지워지고 표식까지 잃어 회전 판정이 못 선다. ADR-0164).
- **소유권 분할:** transport=master/writer/child/shutdown/job · core=subscribers/replay/seq/status/finalized/drain_handle · session=id/cwd/epoch/continues_conversation/cols/rows + 첫 제출 래치(제출 쪽 손잡이 — 수령 쪽은 backend·통로가 sink 로 쥔다 · ADR-0226).
- **턴 관측 정리 = 두 지점뿐:** `finish` + `emit`의 finalize 재확인. **세 번째 호출자를 늘리면 인과가 갈라진다.** 빠지면 턴 도중 죽은 에이전트가 "진행 중"으로 남아 30분 상한(fail-open)이 풀 때까지 우편이 막힌다. (ADR-0127)
- **등록 순서:** sessions insert가 pump 시작보다 **먼저** — 뒤집히면 즉시 종료하는 세션이 명부에 오르기 전에 끝나 수거되지 않는다(런타임엔 무신호 — reaper 테스트가 회귀를 잡는다). (ADR-0019)
- **세션 id 영속 = 첫 제출 래치 한 지점, 비교-교체로:** backend 세션 id 는 화신의 첫 제출 래치가 commit 할 때만 프로필에 영속된다(포트는 화신당 많아야 한 번 불린다). 래치는 첫 사용자 턴이 나가기 직전에 commit 하므로 id 가 첫 턴 전에 영속되고, 덮어쓰기(`observe_session_id`)가 아니라 비교-교체인 것은 claude 추적기가 첫 제출 전에 적은 값을 스폰 때 값으로 되감지 않기 위해서다. 스폰 때 발급해 영속하는 경로 같은 다른 기록 경로를 되살리면 0턴 id 가 영속되고, 다음 활성화가 없는 대화를 이어받으려다 실패한다. 유일한 의도된 예외 = claude `/clear` 추적(ADR-0008 best-effort · ADR-0226 결정 5) · ★「첫 턴 전에 영속」은 codex 터미널에서 성립하지 않는다(결정 11)★. (ADR-0226)

## 프론트 구조 (`src/`)

- **제어 표면(불변):** 컴포넌트·스토어는 `agentClient`(단일 `ProtocolClient`)에만 의존한다(개별 IPC 헬퍼 직접 호출 금지 — ADR-0011이 거부한 `ptyApi` 형태. 그런 모듈은 지금 없다). carrier = transport seam, 운영은 `TauriTransport` 고정(ADR-0036). 교체점은 transport이고, `WsTransport`는 테스트·직결 흔적이다(ADR-0020/0029).
- **폴더:** api · commands(제어 표면 — registry/dispatch/contributions + 버스 다리) · components(layout/agent/slot/diff/ui) · i18n · lab · lib · pages · store · styles · theme · util.
- **구독(콜백) 수명은 `eventBus`가 한 곳에서 소유한다** — 단 **raw `listen` 수명은 각 등록 주체가 따로 진다**. ★**등록 주체를 세지 말 것 — 늘어난다**★(옛 문장이 "둘로 갈린다"였는데 실제로는 넷이 됐고, 그 어긋남을 두 번 연속 리뷰가 잡았다). 대신 **가름 규칙**을 쓴다: `eventBus`는 `agentClient`의 **추상 구독**만 받고, **백엔드가 권위인 표면**은 Tauri `listen`을 직접 걸되 **거는 쪽이 자기 disposer를 소유한다**. 오늘 그 예외에 드는 것 = 에이전트 이벤트(전송 계층) · 레이아웃·탭 · 창 레이아웃 · UI 설정 · 버스 다리. **찾는 법 = `rg "from '@tauri-apps/api/event'" src/`** — 손으로 적은 명단은 또 낡는다.
- **통합 micro-rules:** 구독 effect deps `[viewId, agentId]`(화신 표식 제외 — ADR-0046 구독 키 + ADR-0164) · 구독 전 `terminal.reset()` · seq dedup · replay 경계 = gen 펜스 성공 마커 · `delete channel.onmessage`(null 아님) · 입력 가드 · resize debounce 50ms.

## 창 구성

**정적 창 2개**(`src-tauri/tauri.conf.json`): main(대시보드, visible) · agent-tree(hidden). 옛 정적 slot-popup 창은 제거됐다. **단 `/popup` 라우트는 살아 있다** — `PopoutPage`(탭을 소유하는 팝아웃 창, ADR-0057)가 쓰고, 창은 설정이 아니라 **런타임에 생성**된다.

## 기술 스택 (프론트)

React 19 + TS + Vite · Zustand · @xterm/xterm(+fit) · allotment · react-arborist · @monaco-editor/react · react-router(hash) · react-markdown + remark/rehype + katex(챗 마크다운·수식) · **Tailwind CSS v4 + shadcn/ui + lucide-react** · CSS 변수 테마 · Tauri v2 셸. 상세는 package.json.

- 스타일링은 Tailwind 채택(기존 "순수 CSS" 기조 전환 — ADR-0047). 테마는 CSS 변수를 유지하고 Tailwind 토큰이 `var()`를 참조한다.
- 테마 변수·폰트 정의는 `src/styles/theme.css`·`font.css`. `data-theme`은 `:root`에 dark/light/e-ink.

## 의존성 (변경 시 보고)

- `tauri = "2"` — Channel 무손실 Windows 실측 확인(spike).
- `portable-pty = "0.8.1"` · `uuid` · `thiserror` · `tracing` · `dunce`(cwd canonicalize UNC 회피).
- `regex`(로그 마스킹) — **`base`에만 있다**(ADR-0175 결정 1로 `agent`에서 이사했고 그쪽에선 지웠다. 워크스페이스 전체에서 이 crate를 선언하는 매니페스트가 그것 하나뿐 — 실측 2026-08-26).
- `tracing-subscriber`(env-filter) — **로깅 초기화를 소유하는 것은 `base`**이고 `agent`에선 지웠다(같은 이사). ★**단 "`base`에만 있다"는 거짓이다**★ — `daemon`도 `[dependencies]`에 선언해 둔다(`crates/engram-dashboard-daemon/Cargo.toml:124`). **그런데 그 crate의 사용은 0이다**(`rg tracing_subscriber crates/engram-dashboard-daemon/src crates/engram-dashboard-daemon/tests` → 0줄, 실측 2026-08-26). 이사 전부터 그랬고 이번 변경이 건드리지 않았다 — 걷어낼지는 별건이라 손대지 않았다.
- `windows` — `#[cfg(windows)]`. **feature가 crate별로 갈린다**: `agent`는 Job Object 쪽(`Win32_System_JobObjects`·`Win32_Security`·`Win32_Foundation`·`Win32_System_Threading`) **+ Restart Manager 쪽**(`Win32_System_RestartManager` — 「이 파일을 누가 열고 있나」를 묻는 넷이 그 뒤에 있고, codex 세션 id 회수가 그 답 위에 선다 — ADR-0218), `base`는 PID 판정 쪽(`Win32_Foundation`·`Win32_System_Threading`·`Win32_System_Diagnostics_ToolHelp`). ★`agent`에서 `Win32_Security`를 빼지 말 것★ — `CreateJobObjectW`가 그 feature 뒤에 있고 `Win32_System_JobObjects`가 그것을 함의하지 않는다(근거 정본 = 그 crate `Cargo.toml` 주석).
- `inventory`(명령 선언 링커 수집) — S20 Step 1이 들인 **유일한 신규 서드파티 crate**다(`Cargo.lock` 신규 패키지 = 이것 + 워크스페이스 멤버 자신, 둘뿐 — 실측 2026-08-14). `agent`를 타고 데몬·셸 릴리즈 바이너리에 함께 링크된다. (ADR-0155)
- `ts-rs = "10"` — 워크스페이스엔 이미 있었으나(`protocol`·`src-tauri`) **`agent`의 production 의존으로 새로 들어왔다** — 선언 매크로가 `TS` derive를 달기 때문. 즉 코어가 TS 생성 도구를 운영 그래프에 안고 있다.

---

# 검증

## CI — push하면 자동으로 돈다 (`.github/workflows/ci.yml`)

**어느 브랜치든 push하면** 아래 「빌드·검증 명령」과 격리 게이트가 windows 러너에서 돈다. 로컬에서 같은 것을 다시 돌리지 않는다 — 범위 분담의 정본은 `/qa` 바인딩.

- ★**"같은 것"은 이제 바이트 단위로 같지 않다**★ — 워크스페이스 회귀에서 **CI는 `-- --test-threads=4`를 쓰지 않는다**(의도된 차이 — 사유는 아래 그 항목). 그러니 두 쪽 명령줄이 갈린 것을 드리프트로 보고 맞추지 말 것. 다른 게이트들은 그대로 같다.

- **CI가 못 하는 것 = GUI 실측**(창이 필요하다) **+ 실 claude 의존 테스트**(러너에 claude 없음 — 워크플로가 이름으로 제외하며 그 목록이 정본) **+ ADR-0130 재론 트리거**(게이트가 아니라 알림이라 CI에 못 얹는다). 셋 다 로컬 몫이다.
- **CI에만 있고 이 목록엔 없는 게이트** — 생성물 sync(`git add -N -f` 뒤 `git diff --exit-code`. intent-to-add를 거치는 이유는 **`git diff`가 untracked 파일을 안 봐서** 처음 생성되는 `.ts`가 조용히 통과했기 때문이다)와 discovery async 반입. 로컬 fallback으로 돌 때 빠뜨리기 쉽다. ★**개수를 세지 않는다**★ — 세던 숫자는 게이트가 늘 때마다 뒤처진다(실제로 뒤처졌다).
  - ★**그 sync 게이트는 커밋되는 생성물 디렉터리 셋을 전부 본다**★ — `crates/engram-dashboard-protocol/bindings/` · `crates/engram-dashboard-agent/bindings/` · **`src-tauri/bindings/`**. 셋째가 들어올 수 있었던 것은 CI가 그 8개를 굽는 내보내기 테스트만 이름으로 골라 도는 스텝을 따로 세웠기 때문이다(아래 「빌드·검증 명령」의 `lib_unit` 줄). ★**「그 디렉터리는 게이트 밖이다」·「게이트 확장은 실패 수정이 먼저다」로 적힌 자리를 만나면 낡은 것이다**★.
- **`v*` 태그를 push하면 릴리즈까지 간다** — 배포판 zip을 만들어 GitHub Release에 붙인다. 태그와 제품 버전이 다르면 빌드 전에 멈춘다(대조 대상 목록은 워크플로의 버전 게이트가 정본). 릴리즈 잡은 검증 3잡에 `needs`로 매달려 있어 **빨간 게이트에서는 배포가 시작되지 않는다.**
- 워크플로를 고치기 전에 **ADR-0131을 읽을 것** — 러너 단일화·검증 전용 범위·빌드/발행 잡 분리의 근거가 거기 있다.

## 빌드·검증 명령 (워크스페이스 루트에서 실행)

- `cargo test --workspace -- --test-threads=4` — 전 workspace 회귀. ★**더는 어느 멤버도 빼지 않는다(사용자 결정 2026-08-25)**★ — `--exclude engram-dashboard`를 걷었다. 제외를 떠받치던 사유 둘이 차례로 죽었기 때문이다: `0xc0000139 STATUS_ENTRYPOINT_NOT_FOUND` 즉사(해법 = ADR-0174)와 그 뒤 남았던 단위 스위트의 알려진 실패(지금 0 red — 수치 정본은 아래 `lib_unit` 줄). ★**「54개」는 바이너리 수가 아니라 `test result:` 줄 수다 — 세는 법을 헷갈리지 말 것**★(실측 2026-09-19 = `Running` 46 + `Doc-tests` 8 = 결과 줄 54). **`Running` 줄만 세면 46이 나오고, 그것을 회귀로 읽으면 오진이다.** 아래 이력의 수치들도 같은 세는 법이다. **실측 2026-09-24 = 결과 줄 58개·2781 통과·0 실패·20 무시**(ADR-0226 Phase A 착지 뒤 — 줄 수는 그대로다). 그 직전 값 = **실측 2026-09-23 = 결과 줄 58개·2724 통과·0 실패·20 무시**(★이 라운드가 `tests/` 루트에 더한 파일은 0 이다 — 53 → 58 은 그 전에 master 로 들어온 라운드들 몫이고, 이 목록이 `transport` crate 를 비롯해 뒤처져 있었다★). 그 직전 값 = **실측 2026-09-21 = 결과 줄 53개·2538 통과·0 실패·20 무시**(★줄 수는 그대로 53 이다 — ADR-0218 회수 경로는 `tests/` 루트에 파일을 더하지 않고 기존 바이너리 안에서만 늘었다★. 같은 날 직전 값 = 그 경로가 들어오기 **전**의 53개·2504 통과 — 순증 34 건이고, 그 안에서 상태줄 스크래핑 스위트 삭제분과 락 회수·프로세스 나무 신설분이 함께 움직였으므로 **총 추가분은 22 보다 크다**. 그 직전 값 = 같은 날 훅 삭제 **전**의 54개·2542 통과·0 실패·20 무시 — 그 라운드의 기록 = step-log 의 ADR-0216 항목(ADR 본문 「근거」 절에는 테스트 수치가 없다 — codex-cli 측정만 싣는다), 그 직전 = 2026-09-19 의 54개·2465 통과·0 실패(그 기록이 무시 수를 안 남겼다), 그 직전 = 2026-09-15 의 52개·2365 통과·20 무시, 그 직전 = 2026-09-14 의 44개·2324 통과. S21 codex 라운드들이 회귀망을 더하며 올랐다). ★**54 → 53 은 ADR-0216 결정 9 의 훅 기계장치 삭제다 — 이 목록에서 줄 수가 **줄어든** 첫 사례이고 회귀가 아니다**★: `crates/engram-dashboard-daemon/tests/control_hook.rs` 가 통째로 사라져 `tests/` 루트의 테스트 바이너리가 하나 줄었다. ★**나머지 삭제분(`control/hook.rs` 11 · `bin/engram.rs` 의 훅 구획 10 · codex 백엔드의 훅 빌더 시험 7 · `manager.rs` 2 = 30)은 이 수를 안 내린다**★ — 전부 **기존** 테스트 바이너리 안이라 줄 수가 아니라 통과 수만 내린다(2542 → 2504 의 38 건은 **삭제된 테스트 전체**이고, 그중 8 건은 위 사라진 바이너리(`tests/control_hook.rs`) 몫이다 — 30 + 8 = 38. ★이 38 을 「나머지」로 읽어 8 을 두 번 세지 말 것★). 바이너리 수는 42 → 43 이 ADR-0175 결정 1의 `base` crate 신설이고, ★43 → 44 를 만든 것은 규명하지 않았다★ (`[[bin]]` 테스트 하네스도 정상 기립 — bin 의 테스트 하네스는 kind 가 여전히 Bin 이라 `-bins` 링크 인자를 그대로 받는다). **52 → 54 는 `tests/` 루트에 새로 든 파일 둘이다** — `crates/engram-dashboard-agent/tests/submit_delivery_boundary.rs`(`73ecdb8` · ADR-0206)와 `crates/engram-dashboard-daemon/tests/control_hook.rs`(`75bdfda` · ADR-0208 — ★그 파일은 ADR-0216 결정 9 로 지워졌고, 그것이 위 54 → 53 이다★). ★**같은 커밋이 들인 `control/hook.rs` 의 단위 스위트는 이 수를 올리지 않았다**★ — 그 `#[cfg(test)]` 는 데몬 lib 의 **기존** 테스트 바이너리 안으로 들어간다(바이너리를 새로 세우는 것은 `tests/` 루트의 새 파일뿐이다). ★**그래도 아래 셸 타깃 5줄을 지우지 말 것**★ — 이 줄은 cargo 의 *기본 선택*을 쓰므로 `[lib] test = false`가 지워지면 죽은 내장 타깃이 되살아나 여기서 요란하게 터지지만, **`[[test]] lib_unit` 선언이 사라지는 것은 못 본다**(타깃 소실은 실패가 아니라 침묵이라 스위트가 통째로 증발한 채 초록이 된다). 그쪽은 이름을 집는 `--test lib_unit` 줄만이 잡는다 — 두 층이 ADR-0174 의 서로 다른 다리를 지킨다. **루트 bare `cargo test` 금지.** ★`-- --test-threads=4`도 빼지 말 것★ — 사내 보안 에이전트 DLL이 **프로세스 생성**을 후킹하는데, libtest 기본 병렬은 실 자식 프로세스를 띄우는 테스트를 한꺼번에 몰아 **터미널이 그 DLL 안에서 크래시하고 세션까지 함께 내려간다**(실 `engram.exe`를 띄우는 테스트 파일이 들어온 2026-08-11에 크래시가 0.6회/주 → 약 13회/주로 뛰었다). 4는 **실측 안전** — 43개 테스트 바이너리·1473 통과·0 실패, 약 2분, 터미널 생존(실측 2026-08-17 — 프로세스를 띄우는 스위트 전부 포함). **그래도 죽으면 2로 낮춘다.** 더 안전한 fallback = 테스트를 **터미널 프로세스 트리 밖에서 돌리고 출력은 파일로만** 받는 것(Windows 작업 스케줄러가 프로세스를 만든다 — `scripts/launch-detached.ps1`이 앱에 쓰는 그 기법이지만 인자를 못 받아 작은 래퍼 .bat이 필요하다).
  - ★**CI는 이 플래그를 쓰지 않으며 그것이 의도다**★ — 러너에는 크래시를 일으키는 그 사내 보안 에이전트가 없고, 병렬을 낮추면 **CI만 느려진다.** 그러니 `.github/workflows/ci.yml`을 이 줄에 맞춰 **"드리프트 수정" 하지 말 것.** 로컬만 4로 돈다. (이 예외의 정본이 여기다 — `docs/testing-strategy.md`·`/qa` 바인딩·`README.md`는 이 자리를 가리킨다.)
  - ★**병렬은 테스트 바이너리마다 걸린다**★ — 그래서 워크스페이스 명령에만 붙이면 불완전하다. **실 자식 프로세스를 띄우는 crate를 좁혀 돌릴 때도 같이 붙인다**(아래 `-p engram-dashboard-base`·`-p engram-dashboard-agent` 줄이 그 예 · `-p engram-dashboard-daemon`도 해당 — 프로세스 레벨 CLI 스위트와 실 `.exe` spawn `#[ignore]` 분을 갖는다). ★**아래 줄들을 「바로 아래」처럼 자리로 가리키지 말 것**★ — 이 목록은 crate가 늘 때마다 사이에 줄이 끼어들어 자리가 밀린다(실발생 2026-08-25 — `base` 신설이 그렇게 밀었다). 가리킬 땐 `-p` 이름으로 부른다. **인메모리 단위 테스트뿐인 crate는 안 붙인다**(`command`·`protocol`·`messaging`·`net` — 층별 근거는 `docs/testing-strategy.md` §1). ★**셸 패키지(`src-tauri`)의 테스트 타깃 다섯은 하나도 안 붙는다**★ — 그 패키지에는 프로세스를 만드는 줄 자체가 없다(`rg "Command::new|std::process|\.spawn\(\)" src-tauri/src/` → 0줄, 실측 2026-08-24). 소켓뿐이라 이 플래그가 겨냥하는 조건에 애초에 안 든다.
- `cargo test -p engram-dashboard-base -- --test-threads=4` — 바닥 crate 단위 + 로깅 통합 2종(워크스페이스 의존 0 격리 하네스, ADR-0175 결정 1). 플래그 근거 = 위 「병렬은 테스트 바이너리마다 걸린다」 항목 — `platform`의 `child_pids_finds_spawned_child`가 실 `cmd.exe`를 띄운다(로깅 쪽은 아무것도 안 띄우지만 플래그는 테스트 바이너리마다 걸리므로 crate 단위로 붙는다).
- `cargo test -p engram-dashboard-agent -- --test-threads=4` — 에이전트 런타임 unit + 통합(실 PTY로 단언). 플래그 근거 = 위 「병렬은 테스트 바이너리마다 걸린다」 항목(실 PTY = 실 자식 프로세스).
- `cargo test -p engram-dashboard-command` — 명령 버스 도구 단위(워크스페이스 의존 0 격리 하네스, ADR-0155)
- `cargo test -p engram-dashboard-protocol` — codec golden + ts-rs 바인딩
- `cargo test -p engram-dashboard-messaging` — 메시징 커널 단위(무의존 격리 하네스, ADR-0110)
- `cargo test -p engram-dashboard-net --all-features` — 네트워크 행 단위(ADR-0129). ★`--all-features`를 빼지 말 것★ — net의 기본 feature가 비어 있어 맨 명령은 `auth`만 컴파일해 6개만 돈다(켜면 42개, 실측 2026-08-14). **두 조합을 다 도는 것이 게이트 5.**
- `cargo test -p engram-dashboard --test layout_apply` · `cargo test -p engram-dashboard --test layout_commands` · `cargo test -p engram-dashboard --test daemon_client_pending` · `cargo test -p engram-dashboard --test daemon_client_replay` — 셸 패키지(`src-tauri`)의 **통합** 테스트 타깃 4종(앞 둘 = IPC 핸들러 16종을 전송 중립 서비스 뒤로 옮긴 리팩터의 회귀망). `layout_apply`는 적용 서비스 자체(락이 어느 포트 안/밖에서 불리나)를 재고, `layout_commands`는 창·탭·슬롯 **명령 선언**(`layout::commands`)과 **인바운드 수신기**(데몬에서 온 명령을 적용 서비스로 라우팅하는 계층)를 잰다. `daemon_client_pending`은 겹친 `request_id`가 연결 태스크를 패닉시키지 않고 옛 대기자가 영구 hang 대신 **오류로 깨어나며** 새 요청이 그 번호를 승계함을 잰다(겹친 번호가 옛 `debug_assert`를 때려 그 뒤 모든 명령이 끊기고 재연결도 없던 GUI 실측 2026-08-18). `daemon_client_replay`는 거절당한 구독이 슬롯을 풀고 병합된 **다음 세대 `Subscribe`가 실제로 만들어져 보낼 명령으로 돌아오며** 이미 acked된 구독은 거절로 풀리지 않음을 잰다(2026-08-19 출력 두절 결함의 회귀망). ★**소켓으로 나가는 것은 이 타깃이 재는 범위가 아니다**★ — 돌려받은 명령을 실 소켓에 미는 줄은 무검증 잔여로 남는다(그 테스트 파일 헤더 「무엇이 안 덮이나」가 정본). ★**이 네 줄은 각 통합 스위트를 따로 떼어 도는 경로다**★ — 맨 위 워크스페이스 회귀도 이제 이 패키지를 포함하지만(`--exclude` 는 2026-08-25 에 걷혔다) 판정이 한 출력에 섞인다. ★**단 이 패키지의 테스트 타깃 명단은 이제 넷이 아니라 다섯이다**★ — 바로 아래 `lib_unit` 줄이 다섯째다. 통합 타깃 넷은 `engram_dashboard_lib`를 링크해 정상 기립한다(실측 2026-08-17 · `daemon_client_replay`에서 2026-08-20 재확인). 그래서 `--test`로 타깃을 각각 집는다 — `-p`만 쓰거나 `--tests`로 넓히면 **`lib_unit`까지 함께 끌려와 통합 스위트의 판정이 그 안에 묻힌다**(그 타깃이 빨개서 이 줄을 물들이던 시절은 지났다 — 지금은 0 red, 아래 줄. 남은 사유는 판정을 갈라 읽는 것 하나다. 옛 즉사 `0xc0000139`도 2026-08-24에 해소됐다). **`-- --test-threads=4`는 넷 다 붙이지 않는다** — 인메모리 하네스뿐이라 자식 프로세스도 소켓도 하나 안 띄운다(위 병렬 항목의 판정 규칙이 그대로 적용된 결과).
- `cargo test -p engram-dashboard --test lib_unit` — 같은 셸 패키지의 **단위** 스위트(`src/**`의 `#[cfg(test)]` 전부 — 수치는 아래 「현재 수치」). ★**실행 명령·현재 수치·CI 등재 상태의 정본이 이 줄이다**★ — 다른 문서·주석은 여기를 가리키기만 하고 수치를 베끼지 않는다(베낀 수치는 실제로 하루 만에 낡았다). ★**`-- --test-threads=4`를 붙이지 않는다**★ — 이 스위트는 자식 프로세스를 하나도 안 띄운다(소켓뿐 — 위 병렬 항목의 판정 규칙 그대로).
  - **현재 수치(실측 2026-09-24) = 260건 전부 통과 · 0 실패 · 0 무시.** 254(2026-09-23 기록) → 260 은 ADR-0226 A3 가 마커 표식 회귀망을 더하면서다 · 그 전 248 → 251 은 ADR-0195 가 회귀망 셋을 더하면서다. 약 1초에 스스로 끝난다. 20건이 는 것은 ADR-0175 결정 3이 `replay_flight`를 이 패키지 안으로 옮기면서다 — 그 20건이 `agent` 쪽에서 빠져 이쪽으로 왔을 뿐이라 **워크스페이스 회귀 총계는 그대로다**(그 총계의 정본은 위 `--workspace` 줄이고 여기 베끼지 않는다 — 이 줄의 첫 문장이 금하는 바로 그것이다). ★한때 `#[ignore]`로 묶여 있던 hang 두 건은 유계 대기(`tokio::time::timeout` 5초)를 얻어 매달리는 대신 깨끗이 실패한다★ — 그래서 `-- --ignored`도 즉시 돌아온다.
  - **한때 빨갛던 32건은 없어졌다.** 원인은 하나였다 — `src-tauri/src/daemon_client/mod.rs`의 `start_connection`이 `app: None`이면 연결 태스크를 띄우지 않은 채 `Ok(())`로 단락했고, 테스트 생성자가 전부 그 값이었다. 그 단락을 지우고 emit을 포트로 끊어(`src-tauri/src/daemon_client/events.rs`) 해소했다. **설명·되살리지 말 것의 정본은 그 파일 주석**이고 여기 되풀어 적지 않는다.
  - **CI 등재는 전량 등재다.** CI는 이 스위트를 **통째로** 돈다(`cargo test --locked -p engram-dashboard --test lib_unit` — 이름 필터 없음. 판정 = 실패 0). ★한때 여기 적혀 있던 「부분 등재 — `-- export_bindings_` 이름 필터」는 낡은 서술이다★ — 그 필터를 두던 사유(나머지 32건이 빨갛다)가 죽으면서 워크플로에서 걷혔는데 이 줄만 뒤처져 있었다. 그 한 스텝이 둘을 함께 지킨다: ① `src-tauri/bindings/*.ts`가 CI에서 실제로 다시 구워져 **위 「CI」 절의 sync 게이트가 그 디렉터리까지 덮는다** ② 이 타깃을 세운 세 다리(`[lib] test = false` · `[[test]] lib_unit` · `build.rs`의 `cargo:rustc-link-arg-tests=`)가 살아 있다는 것 — `lib_unit` 테스트를 하나라도 돌리면 셋이 함께 증명된다(`[[test]]`가 빠지면 cargo가 "no test target named `lib_unit`"으로 죽고, 링크 인자가 빠지면 exe가 로드 단계에서 죽는다).
  - **그래서 `src-tauri/src/**`의 `#[cfg(test)]`에 새로 쓰는 단언은 CI 신호를 받는다.** ★한때 여기 있던 「전체 등재는 결정 대기 — CI 신호 0」은 위 항목과 같은 낡은 서술이다★. 등재를 막던 사유(32건이 빨갛다)와 `--workspace` 제외를 떠받치던 사유는 하나였고, 둘 다 그 사유가 죽으면서 함께 걷혔다.
  - ★**`--lib`·`--all-targets`는 여전히 함정이다**★ — 명시로 부르면 manifest 없는 내장 타깃이 그대로 골라져 도로 `0xc0000139`로 즉사한다(실측 2026-08-24). 부르는 법은 `--test lib_unit` 하나뿐이고, **그 함정의 설명 정본은 `src-tauri/Cargo.toml`의 주석**이다.
  - **왜 이 모양으로 세웠나·무엇을 거부했나 = ADR-0174.** 링크 인자 네 형태의 실측, 중복 리소스 충돌, rustflags 주입 기각이 전부 거기 있다 — 여기 되풀어 적지 않는다.
- `cargo build` — 전체 workspace 빌드. **★이걸로 지은 `engram-dashboard.exe`는 띄우지 않는다★** — `TAURI_CONFIG` 없이 도는 빌드는 debug 셸에 **release identifier**를 다시 찍고(`rerun-if-env-changed`라 변수를 빼는 것만으로 재빌드가 돈다 — 실측), 그 exe는 릴리즈 앱이 떠 있으면 창 없이 즉시 죽는다. 띄울 exe는 `node scripts/build-client-shell.mjs`로 짓는다(ADR-0137). ★**테스트 타깃은 컴파일하지 않는다**★ — 이 명령만으로는 위 셸 통합 스위트가 깨져도 안 보인다.
- `cargo fmt --check` — 포맷 게이트(검사형)
- `rg "^\s*use tauri" crates/engram-dashboard-agent/src/` (→ 0줄) — 코어 격리 게이트(ADR-0003). import 라인 앵커라 주석 자기인용이 오탐되지 않는다. ★**경로가 사라져도 매치는 0이라 통과로 읽힌다**★ — 개명·이사 뒤엔 경로 존재를 먼저 확인한다(`/qa` 바인딩의 4-pre).
- `rg "^\s*use tauri" crates/engram-dashboard-base/src/` (→ 0줄) — 같은 ADR-0003 불변식의 바닥 crate 쪽 짝. ★**`agent` 쪽 `use tauri` 줄과 둘이 한 벽이다 — 하나만 돌리면 절반만 지킨다**★ — 불변식이 걸리는 축은 crate 이름이 아니라 **어느 바이너리에 링크되나**이고, `base`는 headless 데몬·`net`·`discovery`에 링크된다. ADR-0175 결정 1이 `logging`·`platform`을 여기로 옮기면서 그 두 모듈이 위 줄의 스캔 범위 밖으로 나갔다 — 이 줄이 없으면 어느 게이트도 안 본다. ★**"잎 crate라 안전하다"로 지우지 말 것**★: 잎 성질은 워크스페이스 crate 축의 말이고, 아래 의존 상한 게이트는 워크스페이스 멤버만 세므로 서드파티 `tauri`를 그냥 통과시킨다. 경로 존재 확인도 같다(`/qa` 바인딩의 4b-pre).
- `rg "(crate|super)::(logging|platform)" crates/engram-dashboard-base/src/` (→ 0줄) — `base` 입주 조건 ③(입주자끼리 서로 참조하지 않을 것) 게이트. 입주자가 엮이면 그건 한 덩어리이고, 한 덩어리를 바닥에 두면 그 전체가 모든 소비자에게 딸려 간다 — 그 crate 헤더가 이름 붙인 `bevy_core` 실패의 시작점이다. ★**`crate::` 단독으로 넓히지 말 것**★ — 그건 평범한 Rust라 게이트가 아니라 잡음이 된다. 대가로 모듈 이름을 손으로 박으므로 **입주자가 늘면 여기 이름을 더한다**. (ADR-0175 결정 1)
- `rg "^(?:[^/]|/[^/])*?\b(tauri|tokio|engram_dashboard_[a-z_]+|std::net)::" src-tauri/src/daemon_client/replay_flight.rs` (→ 0줄. 앞에 `test -f`로 **파일** 존재를 먼저 본다 — `-d`가 아니다) — `replay_flight` 순수성 게이트(ADR-0175 결정 3). ★**이 파일엔 컴파일러 backstop이 없다**★ — `agent`·`base`는 `tauri`를 아예 의존하지 않아 위반이 컴파일 에러로 먼저 죽고 위 두 줄은 값싼 조기 신호일 뿐이지만, 셸 패키지는 `tauri`·`tokio`·`protocol`을 전부 의존해 위반이 그냥 컴파일된다. 그래서 이 정규식이 **유일한 벽**이고, 위 줄들처럼 `use` 라인만 앵커하면 인라인 완전경로(`tokio::spawn(…)`) 한 줄에 뚫린다. 접두 `^(?:[^/]|/[^/])*?`가 「아직 `//`를 안 만났다」를 뜻해 **주석 밖**만 잡는다(헤더의 자기인용 오탐 없음). `\b`+`::`가 이 모듈이 정당하게 쓰는 `std::time::Instant`와 `tokio::time::Instant`를 가른다. 한계·근거의 정본은 그 파일 헤더와 `ci.yml`의 같은 스텝 주석.
- `rg "engram_dashboard_(agent|base|daemon|protocol|discovery|command)" crates/engram-dashboard-messaging/src/` (→ 0줄) — 메시징 커널 격리 게이트(ADR-0110)
- `cargo tree -p engram-dashboard-messaging --depth 1 --prefix none -e normal,dev,build --target all --all-features | rg "^engram-dashboard" | sort -u` (→ 정확히 1줄 = 자기 자신) — 메시징 커널 의존 상한. 바로 위 정규식이 **소스 텍스트**만 봐서 못 잡는 형태(따옴표·`[build-dependencies]`·rename)를 **해석된 의존 그래프**로 덮는다. **그중 가장 큰 구멍은 정규식이 crate 이름 알파벳을 손으로 박아 둔다는 것** — 새 crate는 누가 그 알파벳에 이름을 더할 때까지 **아예 안 보인다**(`command`를 더한 것이 이 게이트를 세운 계기다). net 상한 게이트와 같은 계기이고 플래그도 같은 이유로 줄이지 않는다.
- `cargo tree -p engram-dashboard-command --depth 1 --prefix none -e normal,dev,build --target all --all-features | rg "^engram-dashboard" | sort -u` (→ 정확히 1줄 = 자기 자신) — 도구 crate 의존 상한. **이 crate는 워크스페이스 의존 0을 유지하기로 돼 있고 이 줄이 그 벽이다** — 다만 그것이 crate로 존재하는 *이유*는 아니다(이유 = **독립적으로 쓸 수 있고 순환을 막는다** — 「백엔드 모듈 맵」 command 항목 · ADR-0151 결정 4). ★**정규식·상한 게이트에 공통으로 남는 구멍**★ — 둘 다 워크스페이스 멤버를 `engram-dashboard` **이름 접두**로 식별한다. 다른 이름을 단 멤버는 양쪽 다 그냥 통과한다. (ADR-0155)
- `cargo tree -p engram-dashboard-base --depth 1 --prefix none -e normal,dev,build --target all --all-features | rg "^engram-dashboard" | sort -u` (→ 정확히 1줄 = 자기 자신) — 바닥 crate 의존 상한. **잎 성질(입주 조건 ②)을 컴파일러가 강제하지 않으므로 이 줄이 그 조건의 유일한 벽이다.** ★**단 이 줄이 `base`의 유일한 게이트는 아니다**★ — 위 `use tauri` 줄(서드파티 축)과 입주 조건 ③ 줄이 각각 다른 축을 맡는다. 메시징과 달리 **워크스페이스 crate 이름을 훑는** 소스 정규식 짝은 두지 않는다 — `base`는 *남을 안 부르는 것*이 불변식이라 부르는 이름의 알파벳을 손으로 관리할 대상이 없다. (ADR-0175 결정 1)
- 프론트: `npm test`(vitest run) + `npx tsc --noEmit`(별도 typecheck 스크립트 없음)
- 전체 E2E: `scripts/`의 `run-*.bat` 런처로 띄운다(목록·용도 = README). **셸에서 직접 띄우지 않는다**(아래 「GUI 실측」). 로그 ON: `RUST_LOG=debug`(기본 warn — 분리 실행에선 스크립트 인자로 넘긴다)
- **CI 정본 = `.github/workflows/ci.yml`** — 로컬 게이트에 없는 검사가 더 있다(여기서 세지 않는다). 그중 wire 바인딩 동기 게이트는 실제로 깨진 적이 있으니, 생성물 drift를 남긴 채 밀지 말 것.

### 네트워크 행 격리 게이트

아래는 **발췌**다. 전체 목록·기대값·근거의 정본은 `crates/engram-dashboard-net/src/lib.rs` 헤더이고, 여기서 개수를 세지 않는다(세던 숫자가 게이트 추가 때 두 번 뒤처졌다).

- `rg "engram_dashboard_(daemon|messaging|discovery)" crates/engram-dashboard-net/src/` (→ 0줄) — 소스 참조
- `rg -o --no-filename "engram_dashboard_base::[A-Za-z0-9_:]+" crates/engram-dashboard-net/src/ | sort -u` (→ 정확히 2줄) — base 심볼 allowlist. 파일 단위가 아니라 **심볼 단위**다.
- `rg "engram_dashboard_(a)gent" crates/engram-dashboard-net/src/` (→ 0줄) — 에이전트 런타임 재유입 금지. ★**base 심볼 allowlist 줄과 짝이다 — 둘 중 하나만 남기지 말 것**★: PID 헬퍼가 `agent`에서 `base`로 이사하며(ADR-0175 결정 1) 기대값 2가 crate 이름을 갈아탔고, 이 0줄은 그 자리가 도로 채워지는 것을 잰다. 0 기대 홀로는 안 깨지는 벽이지만 2 기대와 함께 서면 "심볼이 사라진 게 아니라 옮겨 갔다"를 단언한다. ★**패턴이 맨 crate 이름이고 괄호가 붙은 것은 의도다**★ — 0 기대 게이트는 심볼 모양을 요구할 이유가 없고, 심볼 꼬리를 붙였던 옛 형태는 중괄호 전개·별칭 import를 못 잡았다(실측). 괄호는 net 헤더가 자기 자신에 걸리는 것을 막는 방어이고 근거의 정본은 그 헤더다. ★**이 게이트의 값어치를 부풀리지 말 것**★ — 살아 있는 벽은 net의 직접 워크스페이스 의존 상한 게이트다(의존 선언 없이는 심볼을 부를 수조차 없다).
- `cargo tree -p engram-dashboard-net --depth 1 --prefix none -e normal,dev,build --target all --all-features | rg "^engram-dashboard" | sort -u` (→ 정확히 3줄 — 둘째 자리가 `agent`에서 `base`로 갈렸을 뿐 개수는 그대로다) — 직접 워크스페이스 의존 상한. **해석된 의존 그래프**를 읽는다 — 매니페스트 텍스트 grep으로 바꾸지 말 것(rename·테이블 형·비활성 target·optional에 뚫린다). 플래그도 줄이지 않는다.

## GUI 실측 (`scripts/cdp.mjs`)

실제 Tauri 창(WebView2)에 CDP로 붙어 스크린샷·DOM 조회·실제 `invoke` 호출까지 한다(node 내장 WebSocket만, **Windows 전용**).

- **★앱을 셸에서 직접 띄우지 않는다★** — `scripts/`의 런처나 `scripts/launch-detached.ps1`로 띄운다. 셸에서 띄우면 앱이 터미널의 자손이 되고 **앱 출력이 그 사슬을 거슬러 올라간다** — 그 조합에서 터미널이 반복 크래시해 앱까지 함께 내려간다(실측 2026-08-16). **끊어야 할 조건이 둘이다 — 프로세스 트리 밖 + 출력은 파일로만.** `start`·백그라운드 잡·`nohup`은 둘 다 못 끊으므로 대체재가 아니다.
- 절차(기동 인자·환경변수·PID·teardown)는 `/qa` 바인딩 §full이 갖는다. 여기 되올리지 않는다.

## 컨벤션

> **주석 규약 정본 = `/code-conventions`의 주석 규약 파일.** 코더·리뷰어 지시서에 그 파일을 주입해 쓴다. **이 자리에 규칙을 베끼지 않는다** — 없는 정본을 가리키던 옛 문장이 세션마다 즉흥 규칙을 만들어냈고, 베껴 두면 같은 일이 두 출처로 반복된다.

- 자격증명을 프로필 env에 넣지 말 것(평문 저장 — persistence가 경고한다).
- 모듈마다 build/test/커밋. 커밋 메시지 끝에 Co-Authored-By 트레일러.
