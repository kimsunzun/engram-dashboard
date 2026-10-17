# TRD — 프리셋 + 라이브러리 (지시서 · 스킬 · 설정을 묶어 에이전트에 붙인다)

| 항목 | 값 |
|---|---|
| **범위 조정** | 2026-10-16 사용자 결정 — 작업 폴더(워크스페이스) 정리는 다음 구현으로 미룬다. 이번엔 지금 동작 그대로(만들 때 폴더를 고름 · 작업 폴더 필수). 미룬 것 = R34 · D3 · T4 · R31 · R48 · E15 의 워크스페이스 부분 · TRD 의 `workspace` 선택값 · `launched_workspace` · Unattended Engram 폴더 기본값 · `agent.setWorkspace` · 「만들 때 폴더를 묻지 않음」(TRD 해석이었고 사용자 결정 아님). 근거 = 결정 메모 2026-10-16 |
| **상태** | **초안(사용자 미확정)** — `/review trd` 3회차 반영본 · §2 의 결정을 사용자가 고르기 전 |
| **작성일** | 2026-10-15 |
| **리뷰 기록** | 2026-10-15 `/review trd` 1회차 — Claude doc-aware(Architect-breaker) **FIX** 12건 + codex blind(Designer) **FIX** 2건 · 상충 없음. 반영: 실행 전처리 순서 · 백엔드별 인자 · JSON 고르기 칸의 길 · T10 · 어휘 소유 · `merge_preserving_live` · `cwd` 키 유지 · `display_name` 고정 · 일지 판정 · `std::sync::RwLock` · `libraryView.*` 셸 명령 · 실행 재료 가드 · 배치 포트 · codex 생성물 · `AGENTS.override.md` 위험 · `trash` 거부 · `Library{n}` 유일성 · 셸 세대 · `DataLayout` 기본 경로 · snapshot off 범위 · **2회차(같은 날)** — 두 리뷰어 FIX(좁음) · 반영: 전처리를 예약 · 떠 있음 확인 바로 뒤로 + 명부 변경 mutate 한 번(비교-교체) + Engram 폴더 = 에이전트 id(멱등) + `SpawnByCwd` 를 `spawn_blocking` 으로 · JSON 고르기 칸의 재기동 대체안 삭제(모델 · 강도 둘 다 backend 요청 — 미지원 처리는 3회차가 멈춤 조건으로 바꿈) · 휴지통 = 전용 STA 스레드 · `trash` 거부 사유 정정 · codex 생성물 소유 표식(표식 + 내용 일치일 때만 걷기 · 떠 있는 동안 미룸) · §3 의 `allowedMcpServers` 문장을 T10 에 맞춤 · `DataLayout` 루트 출처 · `backend_args` 권위 · **3회차(같은 날)** — 두 리뷰어 FIX(좁음) · 반영: 명부에 없는 id(`SpawnByCwd` · 스모크 bin)를 전처리 앞에 넣기(`ensure_registered`) + 기본 프리셋을 다는 생성자 하나(`backend::builtin_backend_preset`) + 그 길에서도 Noop 회귀 · 점검은 명부 값을 읽고 mutate 는 그 뒤 안 바뀌었을 때만(한 번 다시 시도) · JSON 고르기 칸 확인을 14 단계 수용 관문 · 멈춤 조건으로 |
| **스텝** | S21 |
| **입력** | [PRD](prd.md)(R1–R42 · D1–D11 · E1–E6) + PRD 리뷰 델타 1차 10건(Δ1–Δ10) · 2차 9건(Δa–Δi) · 3차(「3차 델타」 — 내장 백엔드는 떼지 않고 쉰다 · 버스 실행 기본값 저장 · JSON 고르기 칸은 막지 않는다)(2026-10-15 · PRD 개정 진행 중) + 결정 메모 [`20261012-preset-decisions.md`](../../../.claude/handoff/attachments/20261012-preset-decisions.md) |
| **줄 포인터 기준** | HEAD `477d9b7a` + 미커밋 문서 |
| **표기** | (Dn) = 이 부분은 PRD 추천값 Dn 에 기댄다 · (Δn · Δx) = PRD 리뷰 델타를 반영 · 확신도 = 확실 / 가능성 높음 / 불확실 · 「T번호」 = §2-2 의 TRD 결정 |
| **화면 그림** | [briefing.html](briefing.html) — 여기선 그리지 않는다 |

## 0. 한 문단 요약 — 흐름

라이브러리 폴더는 **데몬 PC 의 것**이고 데몬의 새 모듈 `library` 만 읽고 쓴다. 화면은 디스크를 보지 않고 슬롯 열기 · 포커스 · 폴더 이동 때 **버스 명령**(`library.list` 등)을 보내 답을 그린다(요청형 — 감시 · 방송 없음). `engram` CLI 와 LLM 도 같은 명령을 부른다. 편집 · 파일 조작도 명령이고 데몬이 최신 파일에 적용한다. 실행 때 `AgentManager` 는 세션을 파생하기 **전에** 새 포트 `LaunchKitSource`(실물 = 데몬)로 점검 · 백엔드/워크스페이스 맞추기를 끝내고, 데몬이 펼친 프리셋을 휘발 폴더의 **백엔드 중립 실행 재료**로 받아 `backend` 만이 claude · codex 플래그로 옮긴다. 화면이 낸 실행은 막히면 묻고, 버스 · CLI 실행은 기본값(Engram 폴더 · codex 파일 안 만듦)을 저장하고 간다. 앱 안 옮기기 · 다시 잇기는 데몬 「옮김 기록」에 쌓여 각 클라이언트가 다음 요청 때 자기 묶음을 고친다. 번호 1~4 보기 상태와 묶음 · ★ 대상은 **셸**이 권위로 갖고(키에 데몬 신원), OS 동작은 `platform`(휴지통 · 정션)과 셸의 로컬 요청을 지난다.

## 1. 아키텍처

### 1-1. 소유 — 무엇이 어느 프로세스에 사는가 (추천)

| 데이터 | 주인 | 영속 자리 | 갈래 근거 |
|---|---|---|---|
| 라이브러리 루트 경로 · Engram 폴더(자동 워크스페이스) 위치 | 데몬 | `daemon\state\library.json` | 데몬 PC 에 묶인 값(메모 58) |
| 데몬 신원 `daemon_id` | 데몬 | `daemon\state\identity.json`(첫 부팅에 UUID) | 클라이언트 개인 데이터의 키(메모 71 · 72) |
| 라이브러리 훑기 결과 · 쓰는 곳 역색인 · 파싱 캐시 | 데몬 | 메모리(처음 · 루트 바꾸기 · refresh 때만 훑고 바뀐 파일만 다시 파싱 — 2026-10-16) | 클라이언트는 디스크를 안 읽는다(PRD 불변 제약) |
| 다시 잇기 색인(직전 훑기의 경로 → 종류 · 이름) · Engram 이 지운 경로 기록 | 데몬 | `daemon\state\library-index.json` | 「바깥에서 없어진 것」 판정의 재료(Δ2) |
| 진행 중 파일 조작 일지 | 데몬 | `daemon\state\library-ops\<op>.json`(끝나면 지움) | 여러 파일에 걸친 고침의 롤포워드(Δ3) |
| 옮김 기록(번호 · 옛 경로 → 새 경로 · 최근 N 개) | 데몬 | `daemon\state\library-moves.json` | 클라이언트 소유 묶음을 따라 고치게 하는 재료(Δc) |
| 다시 잇기 후보(확인 대기) | 데몬 | `library-index.json` 안 | 자동/확인 갈림(D10 · Δe) |
| 지시서 · 스킬 · 설정 · 프리셋 파일 | 데몬(파일은 라이브러리 폴더) | 사용자가 고른 폴더 | R7 |
| 내장 프리셋(기본 · claude 기본 · codex 기본) | 데몬 | 코드 + 배포판 `prompts\agent-priming.md` | Δ1 |
| 에이전트에 붙은 프리셋 · 기본 프리셋 여부 · 워크스페이스 · codex 파일 대답 · 백엔드별 패스스루 인자 · Engram 이 작업 폴더에 만든 파일 목록 | 데몬(agent crate 프로필) | `agents.json` | 에이전트와 같은 수명 |
| 실행 재료(합친 지시서 · 스킬 링크 · 합친 claude settings) | 데몬 | `daemon\run\launch\<id>-<epoch>\` — 회수 때 지우고 부팅 때 쓸어 낸다 | 기존 mcp-config 와 같은 수명 규칙 |
| 번호 1~4 의 보기 상태(폴더 · 뒤로/앞으로 · 칸 폭 · 정렬 · 필터 · 선택 · 마지막 만진 시각) | 셸 — **권위**(웹뷰는 그리기만) | `shell\state\state.json` 의 선택 칸 `library_views` | 보는 쪽 상태(메모 58) · 창이 여럿이어도 한 벌(⇐ 원천) · LLM 이 셸 명령으로 바로 닿는다 |
| 즐겨찾기 묶음 · ★ 대상 · 옮김 기록 커서 | 셸 | `shell\config\collections.json` — 데몬별(`daemon_id`) 묶음 + 그 데몬의 커서 | 개인 데이터 · 이 PC 만(R16 · 메모 71) |
| 「다시 묻지 않기」(기본 프리셋 떼기) | 셸 설정 | 기존 `settings.json` 키 하나 | 보는 사람의 확인 습관 |
| 펼침 · 상세 패널 · 팝오버 · 드래그 중 상태 · 칸 경계를 끄는 중의 폭 | 웹뷰 | 없음(끝난 값만 셸로) | 휘발 |

**거부한 대안**
- **셸이 라이브러리를 훑는다** — 클라이언트 디스크 읽기 금지(PRD 불변 제약)이고, 원격 데몬(나중)에서 성립하지 않는다.
- **번호별 보기 상태를 데몬에** — 보는 쪽 상태다. 클라이언트가 여럿이면 주인이 없다(메모 58 「레이아웃을 데몬에 두지 않는다」와 같은 이유).
- **보기 상태 · 묶음을 웹뷰 `localStorage` 에** — 창(웹뷰)마다 갈려 팝아웃 창의 라이브러리 슬롯과 메인 창 편집 칸이 ⇐ 원천을 나누지 못하고, LLM 이 셸 명령으로 닿지 못한다.
- **라이브러리를 별 crate 로** — 소비자가 하나다. 다른 crate 가 필요해지면 뺀다(메모 96).

### 1-2. 디스크 위의 모양 — 에셋 종류 · 파일 형식 · 참조

**종류 판정(데몬 훑기 한 곳)**

| 디스크 | 종류 | 목록 |
|---|---|---|
| `SKILL.md` 를 바로 안에 든 폴더 | 스킬 — **보통 폴더**(들어갈 수 있다 · 안도 훑는다 · 폴더와 함께 정렬 · 아이콘만 스킬 — R12 · 사용자 2026-10-17 — 옛 「항목이고 들어가지 않는다」 와 같은 날 「스킬 폴더 안은 훑지 않는다」 뒤집음) · 그 폴더 줄이나 그 `SKILL.md` 줄을 고르면 그 스킬 · 담으면 폴더 경로 · 스킬 안 스킬도 같은 규칙(바로 안에 `SKILL.md` 가 있으면 스킬) · 이름 = `SKILL.md` 앞머리 `name`, 없으면 폴더 이름 · `SKILL.md` 이름 찾기 = 그 OS 파일 시스템 규칙(Windows = `skill.md` 도 맞음 · 대소문자 가리는 OS = 정확히 `SKILL.md` — 강제로 무시하지 않는다, 사용자 2026-10-17) | 보임(스킬 아이콘) |
| 스킬 폴더 바로 안의 `SKILL.md` | 그 스킬을 대신한다(지시서 아님) · 더블클릭 = OS 기본 편집기(폴더 더블클릭 = 들어감) | 보임 |
| `.md` 파일 — 어디에 있든(스킬 폴더 안 포함 · `SKILL.md` 자신 제외) | 지시서 · 내용 자유(형식 없음) · 설명 = 앞머리 `description`(있으면) · `.txt` 는 지시서 아님(사용자 2026-10-17) | 보임 |
| `.yaml` · `.yml` 이고 맨 위 `kind: preset` | 프리셋 | 보임 |
| `.yaml` · `.yml` 이고 맨 위 `kind: settings` | 설정 | 보임 |
| `.yaml` · `.yml` 이고 `kind: preset` · `kind: settings` 인데 깨짐 · 모르는 `kind` 값(오타일 것) | 「오류 · <이유 · 줄>」 행 | 보임(R1) |
| `.yaml` · `.yml` 인데 `kind` 없음 · 아예 못 읽음 | 우리 것 아님 — `library.refresh` 가 다시 읽은 것이면 답에 「kind 없음」 으로 알린다(§1-7) | 안 보임(사용자 2026-10-17 — 옛 「오류」 행 바꿈) |
| 그 밖 파일(그림 · `.json` · `.txt` · 스크립트 등 — 스킬 폴더 안도) | 우리 것 아님 | 안 보임(사용자 2026-10-17) |
| `.` 으로 시작하는 폴더 · 파일 | 훑지 않음 | 안 보임 |

설정 · 프리셋을 **파일 안 표식**(`kind`)으로 가른다(D7). 확장자나 폴더 이름에 기대면 옮기기가 종류를 바꾼다.

**프리셋 파일**(serde-saphyr 가 읽고 쓰는 유일한 곳 = `library::format`)

```yaml
# Engram 이 고쳐 쓰는 파일입니다 — 손으로 단 주석은 Engram 이 고칠 때 사라집니다.
kind: preset
version: 1
description: PR 리뷰 전용
presets:
  - 공용/기본 규칙.yaml
instructions:
  - 지시서/리뷰 규칙.md
skills:
  - 스킬/리뷰/review
settings:
  - path: 설정/claude opus.yaml
    relinked_from: 옛 설정/claude opus.yaml   # 자동 다시 잇기 표시 — 확인하면 지운다(Δ2)
```

칸 = 종류별 목록 넷(R22). `instructions` · `settings` · `presets` 의 순서가 이어 붙는 순서다(D2). 항목은 경로 문자열이거나 `{path, relinked_from}` 이다.

**설정 파일**

```yaml
kind: settings
version: 1
description: claude · opus 고정
backend: claude          # 선택 — 있으면 「백엔드 정하는 설정」
model: claude-opus-5-5   # 선택 — 있으면 고정(R35)
claude:                  # claude 원문 — 실행 때 --settings 로 넘길 settings 객체(Engram 은 해석하지 않는다)
  env:
    SOME_VAR: "1"
codex:                   # codex 원문 — 실행 때 -c 로 하나씩 넘길 목록
  - model_reasoning_effort=high
```

정식 칸은 `backend` · `model` 뿐이고 강도(effort)는 원문 칸에 둔다(D6 · R5). 원문 칸 이름은 백엔드 낱말(`agent.new` 의 백엔드 어휘와 같은 철자 — 대소문자 무시)이다.

**참조 꼴 `LibPath`** — 라이브러리 맨 위부터의 `/` 경로 한 꼴(R3). 규칙: UTF-8 · 구분자는 `/` 만 · 앞뒤 `/` 없음 · 빈 칸 · `.` · `..` 없음 · `\` · `:` · 제어 문자 없음. 표현은 `relative-path` crate 의 `RelativePathBuf` 를 감싼 우리 타입(`library::path::LibPath`)이고, 검증 · 디스크 경로로 바꾸기(`to_path(root)`) · 옮기기 때 접두 바꾸기(`strip_prefix` + `join`)를 그 타입 하나가 맡는다. 같은 파일인지 비교는 OS 규칙(Windows = 대소문자 무시)을 `platform` 의 경로 비교 도우미로 한다 — 표시는 디스크의 철자 그대로.

**내장 네임스페이스 `engram:/`(Δ1)** — 라이브러리 밖의 고정 가짜 경로. `LibPath` 는 `:` 를 받지 않으므로 둘이 섞이지 않는다.

| 참조 | 표시 이름 | 내용 |
|---|---|---|
| `engram:/default` | 기본 | 지시서 `engram:/priming`(배포판 `prompts\agent-priming.md` — 지금 프라이밍 그대로) + 설정 `engram:/settings/no-autoupdate`(claude 원문 `{"env":{"DISABLE_AUTOUPDATER":"1"}}`) |
| `engram:/backend/claude` | claude 기본 | 설정 하나 — `backend: claude` 뿐 |
| `engram:/backend/codex` | codex 기본 | 설정 하나 — `backend: codex` 뿐 |

내장은 읽기 전용이고 라이브러리 프리셋에 담을 수 없다 — 라이브러리 프리셋 파일이 `engram:/…` 를 가리키면 그 줄이 「오류 · 내장 프리셋은 담을 수 없다」(Δ1). 에이전트에는 붙는다.

### 1-3. 해석 — 「실제로 받는 것」 (`library::resolve`, 순수 함수)

입력 = 에이전트(기본 여부 + 붙은 프리셋 목록) 또는 프리셋 하나 + 항목을 찾는 조회 함수(가짜 저장소로 시험). 출력 = `Resolution`.

1. **펼치기** — 에이전트면 [기본(붙어 있으면)] 다음 [붙은 프리셋 순서대로]. 프리셋 하나를 펼칠 때 = 그 안의 `presets` 를 순서대로 먼저 펼치고(D1 — 안에 든 프리셋의 지시서가 자기 지시서 **앞**), 다음 자기 `instructions` · `settings` · `skills`.
2. **중복** — 같은 프리셋이 두 길로 오면 처음 자리에서 한 번(R4). 지시서 · 설정 · 스킬도 같은 경로는 처음 자리 한 번(경로가 정체 — 내부 선택 §3).
3. **순환** — 펼치는 중인 스택에 다시 나오면 그 줄이 「순환」 오류.
4. **내장 백엔드의 물러남(Δ1 · 3차 델타)** — 사용자(라이브러리) 설정 중 `backend` 를 가진 것이 하나라도 있으면 붙어 있는 내장 `engram:/backend/*` 는 **쉰다**(해석에서 빠지고 화면에 흐린 「쉼」). 사용자 백엔드가 사라지면 다시 살아난다. 붙임 목록은 건드리지 않는다(붙일 때 자동으로 떼지 않는다 — 2차 델타 b 를 3차가 대체).
5. **충돌**(빨강 — 그대로는 실행 못 함) — 사용자 백엔드 정하는 설정 둘 이상(R4 · 내장은 세지 않는다) · 같은 이름 스킬 둘 이상(다른 경로) · `model` 칸 둘 이상(T3).
6. **경고**(노랑 — 알리기만) — 다른 백엔드 원문 칸이 있으나 안 쓰임 · codex 크기 한도를 올렸음(R38) · 문맥 차지 비율 · 자동으로 다시 이어짐(확인 전 — Δd).
7. **상태** — 찾을 수 없음(파일 없음) · 오류(못 읽음) · 순환 · 다시 이어짐(자동 적용 · 확인 전) · 다시 잇기 확인 대기(후보만 있음) — 가리키는 줄에만 붙는다(R6).
8. **막힘 사유(실행 직전)** — 찾을 수 없음 · 오류 · 충돌이 남음(E4) · 다시 잇기 확인 대기(확인 방식일 때 — Δe) · 백엔드 정하는 설정 없음(R33) · 라이브러리 미설정/없음인데 라이브러리 프리셋이 붙음 · 워크스페이스 없음(R34) · codex 터미널 + 사용자 폴더인데 파일 만들기 대답이 없음(R37) · codex 파일 사전 검사 실패. 앞 다섯은 누가 실행하든 막힌다. 워크스페이스 · codex 파일 대답 둘은 **화면이 낸 실행에서만** 막힘이고, 버스 · CLI 가 낸 실행은 기본값으로 채워 에이전트에 저장한다(Δg · §1-5).

같은 함수가 편집 칸의 충돌 · 경고 줄, 「에이전트 N개가 씀」, 실행 재료 셋 모두를 만든다 — 화면과 실행이 다른 규칙으로 갈리지 않게.

### 1-4. daemon — `library` 모듈 (신설 `crates/engram-dashboard-daemon/src/library/`)

| 파일 | 맡는 것 | 핵심 타입 · 함수 |
|---|---|---|
| `mod.rs` | 외관 · 직렬화(읽기는 함께, 쓰기는 하나씩 — **`std::sync::RwLock` 하나** · tokio 락 아님: 두 포트가 동기라 `blocking_handler` 스레드와 manager 의 동기 spawn 경로가 잡는다. ★그 spawn 경로를 tokio 워커가 부르는 곳이 하나 있다 — wire `SpawnByCwd`(`connection_core.rs:1543`)★ → 이 작업에서 `spawn_blocking` 으로 옮기고 기존 구조 시험(`activate_profile` 을 `spawn_blocking` 창 안에서만 부르는지 보는 그 시험)에 `spawn_agent` 를 더해 못 박는다) · 두 포트 구현 | `LibraryService` — `impl LibraryCommandHost`(버스 본문) · `impl LaunchKitSource`(실행 재료) |
| `path.rs` | 참조 꼴 | `LibPath` · `AssetRef { Library(LibPath), BuiltIn(BuiltinId) }` · `AssetRef::parse(&str)` |
| `store.rs` | **목적 포트 하나**(「라이브러리 저장소」 — 감싸기 조사 결론) | `trait LibraryStore { scan(root) · read(path) · write_atomic(path, bytes) · create_dir · rename · trash · exists }` · 실물 `FsStore`(walkdir + std::fs + `platform::fs::move_to_trash`) · 시험 `MemStore` |
| `format.rs` | YAML · 앞머리를 아는 유일한 곳 | `PresetDoc` · `SettingsDoc` · `Frontmatter` · `parse_*`(오류에 줄 번호) · `write_preset` |
| `catalog.rs` | 훑기 결과 → 항목 표 + 쓰는 곳 역색인 | `Catalog { items, used_by }` · `Catalog::build(scan, cache, profiles)` — 파싱 캐시 키 = (경로, 수정 시각, 크기) |
| `index.rs` | 다시 잇기 — **후보 찾기와 적용을 가른다**(Δe) | `LibraryIndex { root, entries, tombstones, pending }` · `find_relink_candidates(prev, now) -> Vec<RelinkCandidate>`(순수) · `apply_relink(candidate)` · 스위치 `RelinkMode { Auto, Confirm }` |
| `moves.rs` | 옮김 기록(Δc) | `MoveLog { head_seq, floor_seq, pairs }` · `append(pairs)` · `since(cursor) -> (pairs, head, truncated)` |
| `ops.rs` | 파일 조작 다섯 + 참조 고침 + 일지 | `move_items` · `rename_item` · `delete_items` · `new_folder` · `new_preset` · `OpJournal::{begin, finish, roll_forward_at_boot}` |
| `edit.rs` | 프리셋 편집 연산 | `add` · `remove` · `reorder`(판 확인) · `relink` · `ack_relink` — 각각 「최신 파일 읽기 → 연산 → 원자적 쓰기」 |
| `resolve.rs` | §1-3 | `resolve_agent` · `resolve_preset` → `Resolution` |
| `builtin.rs` | 내장 셋 | `BuiltinId` · 내용 표 |
| `launch.rs` | 실행 재료 | `DaemonLaunchKits::{check, prepare, release}` · 부팅 쓸기 |
| `config.rs` | 루트 · Engram 폴더 위치 · 신원 | `LibraryConfig` · `DaemonIdentity` |

**갱신 규칙(사용자 결정 2026-10-16 — 아래 흐름 ①의 「요청마다 훑기」를 대체)** — 전체 훑기는 데몬이 처음 라이브러리를 열 때 · 라이브러리 폴더를 바꿀 때만. 그 밖의 목록 요청(슬롯 열기 · 포커스 · 폴더 이동)은 데몬이 들고 있는 메모리 목록으로 답하고, Engram 안의 변경(옮기기 · 이름 · 지우기 · 새로 만들기 · 편집)은 그 목록에 바로 반영한다. 바깥 변경은 트리 우클릭 「새로 고침」(그 폴더 — 하위 포함 · 루트 머리 줄 = 전체) · F5 = 포커스한 칸 기준 — 트리 = 선택한 폴더(없으면 전체) · 목록 = 지금 보는 폴더, 둘 다 하위 포함(우리는 폴더에 들어갈 때 다시 읽지 않으므로 — 사용자 2026-10-17) 또는 `library.refresh {folder}`(생략 = 전체)로 들인다 — 에이전트에겐 Engram 밖에서 파일을 바꾼 뒤 이 명령을 부르라고 권한다(프라이밍 · 강제 아님 — PRD R41 · 사용자 2026-10-17) · 답에 다시 읽은 파일의 문제가 실린다(§1-7). 안전장치 = 명령이 목록에 없는 경로를 가리키면 그 경로 하나만 확인해 넣는다 · 실행 직전 붙은 파일을 다시 확인한다. 감시 · 구독 · 방송 · 주기 요청 없음. 구현: `library.list` 는 `Catalog` 을 읽기만 하고, 훑기(`FsStore::scan` + 다시 잇기 후보)는 `library.refresh` · 부팅 · `setRoot` 에서만 돈다.

**흐름 ① 목록 요청(R19 · 요청형 — 위 갱신 규칙이 우선)** — 버스 `library.list {folder}`(동기 포트 · `blocking_handler` 스레드) → 읽기 잠금 → `FsStore::scan`(walkdir · 루트 전체) → 바뀐 파일만 다시 파싱 → `Catalog` 재구성(쓰는 곳 = 프리셋 + 에이전트, 직접 + 건너서 — Δ3) → 다시 잇기 후보(②) → 그 폴더의 바로 안 항목 + 하위 폴더 줄(스킬 폴더 포함 · 스킬 표시 · 우리 종류 아닌 파일은 빠짐 — §1-2). 동시 요청은 훑기 세대 번호로 합친다(뒤에 온 요청이 진행 중 훑기의 결과를 기다려 쓴다 — `Mutex` + `Condvar`). 방송 · 구독 · 감시 없음.

**흐름 ② 다시 잇기(Δ2 · Δe · D10)** — 두 단계로 가른다.
- **후보 찾기(순수 · 늘 돈다)** — 훑기마다 직전 색인과 비교한다. 「바깥에서 없어진 것」 = 직전 색인에 있었고 · Engram 이 지운 기록에 없고 · 지금 없는 경로. 후보 = 이번에 새로 나타난 항목 중 **같은 종류 + 같은 이름**. 후보가 **정확히 하나**일 때만 `RelinkCandidate {old, new, holders}` 가 생긴다. 둘 이상이거나 없으면 후보 없음 → 「찾을 수 없음 · 다시 잇기」(R19 — 손으로 고른다).
- **적용(스위치 하나 — `RelinkMode`)** — `Auto` 면 쓰기 잠금으로 넘어가 그 경로를 가리키는 프리셋 파일 줄과 에이전트 붙임을 새 경로로 고치고 `relinked_from` 표시를 단다(파일에 쓴다 · 실행은 막지 않는다 · 사용자가 확인하면 표시를 지운다). `Confirm` 이면 참조는 **옛 경로 그대로** 두고 후보를 색인의 `pending` 에 적는다 — 그 참조는 실행할 수 없고(막힘 사유 「다시 잇기 확인 대기」) 화면은 줄에 「다시 이어짐? · [확인]」 한 번 누름을 보인다 → `library.relink` 가 같은 적용 함수를 부른다. 스위치 기본값은 PRD D10 의 결정을 따르는 상수 하나다(설정으로 드러낼지는 그 결정 몫).
- 적용된 쌍(자동이든 확인이든)은 옮김 기록에 쌓인다(흐름 ⑥).
- 표시(「다시 이어짐 · 원래 <경로>」)는 그 줄 · 목록 행 상태 · 상세의 쓰는 곳 · 에이전트 슬롯 머리에 함께 보인다(Δd).

**흐름 ③ 편집(Δ8)** — `preset.add/remove/reorder` 는 파일의 최신 내용에 적용된다. 읽기 답마다 `rev`(파일 바이트 해시)가 실리고, `add` · `remove` 는 낡은 판이어도 멱등 적용, `reorder` 는 `base_rev` 가 다르면 `CONFLICT` → 화면이 다시 받고 「다른 곳에서 바뀌어 새로 불러왔습니다」 한 줄. 바깥 편집기의 수정도 같은 길로 반영된다. 적용 뒤 이 프리셋을 (건너서) 쓰는 에이전트를 다시 해석해 **백엔드가 바뀌는 에이전트**를 답에 싣는다(Δ4). 여러 개 담기는 부분 적용 `{added, skipped[{path, reason}]}`(Δ7).

**흐름 ④ 파일 조작(R17 · R18 · Δ3)** — 쓰기 잠금 아래: (a) 계획 — 옛 → 새 대응표 · 고칠 프리셋 파일 · 고칠 에이전트 · **지움 기록에 더할 경로** · **옮김 기록에 쌓을 쌍과 그 번호(계획 때 예약)** (b) 일지 쓰기(위 계획 전부) (c) 디스크 조작(`rename` · 휴지통) (d) 프리셋 파일마다 줄 고쳐 원자적 쓰기 (e) 에이전트 붙임 고치기(`ProfileRegistry` 한 번 저장) (f) 지움 기록 · 색인 · 옮김 기록 갱신 (g) 일지 지움. 부팅 때 남은 일지는 (c)~(f) 를 **단계별 판정으로 다시 돈다**: (c) 옛 경로가 없고 새 경로가 있으면 끝난 것으로 건너뜀(둘 다 없으면 일지를 「실패」로 남기고 알림 · 옛 경로만 있으면 다시 시도) · (d)(e) 「옛 경로면 새 경로로」 — 이미 고쳐졌으면 그대로 · (f) 예약한 번호의 쌍이 이미 기록에 있으면 건너뜀 · 지움 기록은 집합이라 두 번 넣어도 같다. 진짜 원자성이 아니라 **멱등 롤포워드**다. 답 = 대응표 + 고친 수. 지우기는 쓰는 곳(프리셋 + 에이전트)이 있으면 `yes` 없이 `CONFLICT`(Δ9), 지운 경로는 「Engram 이 지움」 기록에 넣어 자동 다시 잇기를 막는다(Δ2).

**흐름 ⑤ 실행 재료** — §1-5.

**흐름 ⑥ 옮김 기록(Δc)** — 앱 안 옮기기 · 이름 바꾸기 · 적용된 다시 잇기가 쌍(옛 → 새)을 번호 붙여 쌓는다(최근 1,000 쌍 — 넘치면 오래된 것부터 버리고 바닥 번호를 올린다). 클라이언트는 라이브러리 요청을 낼 때마다(슬롯 열기 · 포커스 · 새로고침 · 폴더 이동) `library.moves {since: 커서}` 를 함께 불러 커서 뒤의 쌍을 받고, 자기 묶음을 고친 뒤 커서를 옮긴다. 커서가 바닥보다 오래됐으면 답에 `truncated` 가 서고, 그 사이 옮겨진 묶음 항목은 「찾을 수 없음」으로 남는다(받아들인 손실 — §6). 요청한 클라이언트만이 아니라 **모든** 클라이언트가 자기 다음 요청에 따라잡는다 — 방송 없이.

**루트 · 기본 경로** — `library.status` 가 `{daemon_id, root, state: unset|ok|missing, suggested_new}` 를 준다. `suggested_new` 와 Engram 폴더 기본 위치(T4)는 **데이터 폴더 배치(`DataLayout`)의 메서드**로 낸다. `DataLayout` 은 루트가 **어디서 왔나**(`ENGRAM_DATA_DIR` 지정 · 디버그 기본 · 릴리즈 기본)를 함께 든다 — 릴리즈 기본(루트 = `<배포판>\data`) = 그 부모의 `library` · `workspaces`, 그 밖 = 루트 **안**의 `library` · `workspaces`. 디렉터리 이름으로 짐작하지 않는다. 그래서 `ENGRAM_DATA_DIR` 로 격리한 시험은 실제 폴더를 건드리지 않는다. [새로 만들기]는 이 경로를 미리 채울 뿐이고 내용은 넣지 않는다(Δ1). 루트를 바꾸면 에이전트 붙임 · 묶음은 그대로 두고 **같은 경로를 새 루트에서** 찾는다 — 없으면 「찾을 수 없음」. 색인과 옮김 기록은 새로 시작하므로 루트를 넘는 자동 다시 잇기는 없다(Δf). 옮김 기록에는 루트마다 새로 뽑는 `log_id` 가 붙고, 클라이언트 커서의 `log_id` 가 다르면 쌍을 적용하지 않고 커서만 새 머리로 옮긴다.

### 1-5. agent — 프로필 · 실행 재료 포트 · manager · backend 번역

**프로필(`profile.rs`) 바뀜**

| 칸 | 뜻 |
|---|---|
| `presets: Vec<AttachedPreset { path, relinked_from }>` | 붙은 프리셋(순서 있음) — 내장 `engram:/backend/*` 도 여기 |
| `default_preset: bool` | 「기본」 칸 — 만들 때 `true`(R32) |
| `workspace: Option<PathBuf>` | 작업 폴더. `None` = 실행 때 정한다(R34 · D3). 디스크에는 **옛 키 `cwd` 를 그대로 쓴다**(아래 「호환」) |
| `backend_args: BTreeMap<백엔드 낱말, Vec<String>>` | 패스스루 인자를 **백엔드별로** — 백엔드가 바뀌어도 claude 인자가 codex 로 넘어가지 않는다 |
| `codex_files: Option<bool>` | codex 터미널 + 사용자 폴더의 파일 만들기 대답(R37). LLM 이 만든 codex 터미널 = `Some(false)`(Δ9) |
| `generated_paths: Vec<PathBuf>` | Engram 이 작업 폴더에 만든 파일 · 정션(codex 터미널). 걷는 조건은 아래 「생성물 소유」 |
| `launched_workspace: Option<PathBuf>` | 지금 저장된 세션을 만든 실행의 워크스페이스 — 다르면 새 대화(아래 전처리) |
| `preset_schema: u32` | 이주 표식 — 0 이면 읽을 때 이주한다 |

- **`command`(Claude/Codex/Shell + `extra_args` + `output_format`)** 는 디스크 · wire 모양을 그대로 두고 **해석 결과에서 파생하는 값**이 된다 — 변형 = 해석된 백엔드, 모드(JSON/터미널) = 에이전트 값(D5). 낱말 → 변형 대응은 `backend::command_for(word, output_format, args)` dispatch 한 곳이다(ADR-0004 — manager 는 변형을 모른다). **인자의 권위는 `backend_args`** 다: `command.extra_args` 는 내림용으로 지금 백엔드의 인자를 **비춰 쓸 뿐**이고, 읽을 때 `backend_args` 가 있으면 무시한다. `CreateProfile`(wire) · `agent.new` 로 들어온 `extra_args` 는 그 백엔드의 `backend_args` 칸으로 옮겨 넣는다.
- **`display_name` 고정** — 만들 때와 이주 때 `None` 이면 그때의 표시 이름을 박는다. 이름이 `cwd` basename 에서 파생되던 길이 워크스페이스(비어 있거나 나중에 Engram 폴더로 채워짐)를 따라 바뀌면 우편 주소 · 트리 이름이 말없이 바뀐다.
- **새 프로필 생성자 하나** — `AgentProfile::new`(지금 `SpawnByCwd` · 스모크 bin · `CreateProfile` 이 이미 지나는 그 생성자)가 기본값을 단다: `default_preset = true` · `presets = [backend::builtin_backend_preset(&command)]`(backend dispatch — claude/codex 는 그 내장, Shell 은 없음) · `backend_args = {그 백엔드: 받은 extra_args}` · `preset_schema = 1`. `agent.new` · wire `CreateProfile` · `SpawnByCwd` · 스모크 bin · 이주가 모두 이 한 길로 같은 기본값을 얻는다(이주는 같은 dispatch 를 부른다).
- **`merge_preserving_live` 보존 칸에 더한다**(「런타임이 저자인 칸」) — `presets` · `default_preset` · `workspace` · `codex_files` · `backend_args` · `generated_paths` · `launched_workspace` · 맞춰진 `command` 변형. 이 칸들의 저자는 프리셋 명령 · 실행 전처리이지 spawn 이 뜬 옛 사본이 아니다.

**이주(Δ1 · Δh)** — `preset_schema == 0` 인 프로필을 읽을 때: `default_preset = true` · `presets = [engram:/backend/<지금 백엔드>]` · `workspace = Some(옛 cwd)` · `launched_workspace = Some(옛 cwd)`(지금 세션을 이어받을 수 있게) · `backend_args = {지금 백엔드: 옛 extra_args}` · `display_name` 고정 · codex 터미널이면 `codex_files = Some(false)`(지금 codex 터미널 에이전트는 전부 사용자 폴더다 — 파일 안 만듦 · 프라이밍만, 지금 동작과 같다). Shell 에이전트는 프리셋을 받지 않는다(해석 대상 아님). 다음 저장에 표식이 1 로 앉는다.

**호환 · 내림(정직하게)** — 새 빌드는 `cwd` 키를 계속 쓴다(`workspace == None` 이면 빈 문자열). 옛 빌드는 모르는 칸을 무시하므로(`deny_unknown_fields` 없음 — 확인함) 새 `agents.json` 을 **읽는다**. 그러나 ① 옛 빌드가 다시 저장하면 새 칸이 전부 사라지고, 다시 새 빌드로 올리면 표식이 없어 이주가 다시 돌아 **붙인 프리셋 · 다른 백엔드의 인자를 잃는다**(기본 + 지금 백엔드 내장으로 돌아간다) ② 빈 `cwd` 인 에이전트를 옛 빌드가 띄우면 어떻게 되는지는 재지 않았다(아마 실패). 내림을 지원하지 않는다는 것은 이 손실을 받아들인다는 뜻이다.

**포트(`launch.rs` 신설)** — 데몬이 꽂는다. 기본 구현 `NoopLaunchKits` 는 「지금과 같은 결과」(백엔드 = 명령 변형, 지시서 = 프라이밍 파일 그대로)를 내고 **argv 가 바이트 단위로 같다**(`--system-prompt-snapshot off` 도 안 붙는다 — 아래 번역 표).

- `trait LaunchKitSource { fn check(&self, profile, origin: LaunchOrigin) -> Result<LaunchPlan, LaunchBlocked>; fn prepare(&self, plan, id, epoch, priming: Option<&Path>) -> Result<LaunchKit, String>; fn release(&self, id, epoch); }` — `check` 는 프로필을 고치지 않고, 무인 실행의 기본값 · 맞춰야 할 칸을 `LaunchPlan.profile_changes` 로 돌려준다(포트가 프로필 저장소를 직접 만지지 않는다).
- `LaunchPlan` = 해석된 백엔드 낱말 · 모델 · 지시서 원천 목록 · 스킬(이름 · 절대 폴더) · 그 백엔드 원문 목록 · 프라이밍 포함 여부 · 경고 · 프로필 변경 · `real: bool`(Noop = false).
- `LaunchKit` = `LaunchPlan` + 휘발 폴더 · 합친 지시서 파일 경로 · 스킬 목록(절대 경로).
- `LaunchBlocked { reasons: Vec<BlockReason> }` — 종류가 있는 사유(`NeedsWorkspace` · `NeedsCodexFilesAnswer` · `NoBackend` · `Conflict` · `Missing` · `Broken` · `RelinkUnconfirmed` · `LibraryUnavailable` · `CodexFilesBlocked`).

**manager 실행 흐름** — 두 덩이로 가른다.

*(가) 실행 전처리(`prelaunch`) — `SpawnReservation` 예약과 「이미 떠 있음」 확인 **바로 뒤**.* 둘은 `profile.id` 만 읽으므로 그 뒤에 두면 전처리가 **에이전트마다 하나씩 줄 선다**(같은 에이전트의 두 실행이 함께 돌지 않는다). `epoch_for_spawn` · 작업 폴더 canonicalize · 세션 id 파생보다는 **앞**이다. `activate_profile` 의 「활성화 진행 중」 조기 반환도 그대로 앞에 남는다.

0. **명부에 없는 id 를 먼저 넣는다** — wire `SpawnByCwd`(`connection_core.rs:1530-1543`) · 스모크 bin(예 `bin/priming_smoke.rs:168-179`)은 명부에 없는 프로필로 `spawn_agent` 를 부른다. 그래서 `register_for_spawn` 의 **「없을 때 넣는」 부분**(용량 검사 · 이름 배정(`display_name` 고정) · upsert)을 `ensure_registered` 로 떼어 전처리 **앞**으로 옮긴다. 「있을 때 갱신하는」 부분과 나머지 일은 지금 자리에 남는다. 그래서 전처리는 늘 **명부에 있는 항목**을 고친다.

1. `kits.check(명부에서 읽은 프로필, origin)` — 호출자가 넘긴 사본이 아니라 **예약 뒤 명부에서 다시 읽은 값**으로 점검한다. `origin` = **누가 실행을 냈나**(Δg): `Interactive`(화면 — wire `Spawn` · 활성화) 또는 `Unattended`(버스 `agent.spawn` · 제어 CLI · 부팅 자동 복원). 막히면 `PtyError::SpawnFailed(사유 문장)`로 끊고 `failed_reason` 에 남긴다(지금 실패 표시 길). 화면은 실행 전에 `agent.launchCheck` 로 같은 사유를 구조로 받아 대화상자를 띄운다(§1-7). `Unattended` 에서 워크스페이스가 없으면 Engram 폴더를, codex 터미널 + 사용자 폴더의 파일 대답이 없으면 「안 만듦 · 프라이밍만」을 채운다(3차 델타 — 저장되고 에이전트 슬롯 머리에 보인다).
2. **맞추기 — 명부 변경 한 번(비교-교체)** — 1 에서 읽은 뒤 `presets` · `default_preset` · `workspace` 가 **바뀌지 않았을 때만** 적용한다. 바뀌었으면 다시 읽어 1 부터 **한 번** 더 하고, 또 바뀌었으면 「실행 준비 중 프리셋 · 워크스페이스가 계속 바뀌었다 — 다시 시도하라」로 끊는다. `backend::command_for` 로 해석된 백엔드의 변형 · 그 백엔드 인자를 고른다. 바뀔 칸 전부(무인 기본값 · 맞춰진 변형 · `launched_workspace` · 세션 놓기)를 **`ProfileRegistry` 의 mutate 한 번**으로 **지금 명부 값에 대해** 적용한다 — 예: `workspace` 는 아직 `None` 일 때만 채운다(그 사이 사람이 정했으면 그 값을 쓴다). **백엔드가 저장된 변형과 다르거나 워크스페이스가 `launched_workspace` 와 다르면** 같은 mutate 안에서 `release_session_id` 와 같은 일(저장된 세션을 옛 목록으로)을 하고 `launched_workspace` 를 새 값으로 적는다(R33 · claude 세션은 작업 폴더에 묶인다). Engram 폴더는 **에이전트 id 로만** 정하고(`<workspaces>\<agent-id>` — 바뀌는 이름을 쓰지 않는다) 있으면 그대로 쓴다(멱등 생성).
3. **명부에서 다시 읽기** — 그 mutate 가 돌려준 프로필(= 명부의 지금 값)에서 이후 모든 단계(등록 · 화신 표식 · canonicalize · 세션 id · 모드)를 파생한다. 세션을 놓았으면 호출자가 `Resume` 을 청했어도 `Fresh` 로 간다(결말은 `Started` — 지금 `resume_no_fallback` 의 같은 결말 길). 그래서 첫 제출 래치(ADR-0226)는 맞춰진 백엔드 · 워크스페이스와 어긋나지 않는 id 만 본다.

*(나) 기동 — 지금 순서 그대로에 셋을 끼운다.*

4. 제어 채널 `provision`(지금 그대로 — 프라이밍 경로가 여기서 나온다).
5. `kits.prepare(plan, id, epoch, endpoint.priming_file)` — 데몬이 휘발 폴더에 합친 지시서(기본이 붙었으면 프라이밍 + 프리셋 지시서 순서대로)를 쓰고 스킬 목록을 낸다. **실행 재료 전용 RAII 가드(`LaunchKitGuard`)** 가 바로 무장된다 — `ProvisionGuard` 와 독립이고, 스폰이 끝나기 전 어디서 실패하든 `kits.release(id, epoch)` 를 부른다.
6. `backend::materialize(command, kit, workspace, fs)`(새 dispatch) — 백엔드만 아는 디스크 배치(claude 플러그인 폴더 정션 · codex 작업 폴더 파일). 쓰기는 **배치 포트 `LaunchFs`**(`write_file` · `link_dir` · `unlink_dir` · `remove` · `exists`)로만 한다 — 운영 = std + `platform::fs`, 시험 = 메모리 가짜(디스크 없이 backend 배치를 잰다). 작업 폴더에 만든 것은 `generated_paths` 로 돌려줘 프로필에 남고, 스폰이 실패하면 가드가 이번에 만든 것을 걷는다. 결과 = `LaunchArtifacts`(그 백엔드가 쓸 경로 · 인자 조각).
7. `build_command_spec(…, control, &artifacts)` · `open_spawn(…, &artifacts)` — 두 함수가 실행 재료 descriptor 를 하나 더 받는다(`ControlEndpoint` 와 같은 방식).
8. 회수(reaper · kill)에서 `kits.release(id, epoch)` — 휘발 폴더를 지운다. 데몬 부팅 때 남은 `launch\` 를 쓸어 낸다.

**생성물 소유(codex 터미널)** — 만드는 것마다 **확인 가능한 소유 표식**을 단다: `AGENTS.override.md` 첫 줄 `<!-- engram:generated agent=<agent-id> sha=<내용 해시> -->` · 정션 이름 `engram-<agent-id 앞 8자>-<스킬>`. 걷기는 표식이 있고 **내용이 Engram 이 만든 그대로일 때만**(파일 = 해시 일치 · 정션 = 대상이 그 스킬 폴더) 하고, 아니면 남기고 경고한다(사용자가 손댄 것을 지우지 않는다). 전처리는 **기존 파일 검사(R37 막힘)보다 먼저** 지난 실행의 남은 생성물을 이 규칙으로 정리한다(스폰 도중 죽은 경우 포함). 에이전트가 떠 있는 동안의 떼기 · 백엔드 바꾸기는 걷기를 **멈출 때 · 다음 실행 전처리로 미룬다**(돌고 있는 codex 가 읽는 파일을 빼지 않는다).

**워크스페이스 바꾸기** — `agent.setWorkspace` 는 **떠 있는 에이전트에는 `CONFLICT`** 로 거절한다(멈춘 뒤에만 바뀐다 — 돌고 있는 세션의 작업 폴더를 말없이 옮기지 않는다). 바뀐 값은 다음 실행 전처리 2 가 새 대화로 맞춘다.

**backend 번역** (`backend/claude/launch.rs` · `backend/codex/launch.rs` 신설 — 플래그 지식은 여기 둘뿐)

| 재료 | claude(두 모드) | codex JSON(app-server) | codex 터미널 |
|---|---|---|---|
| 지시서 | `--append-system-prompt-file <합친 파일>`(지금 프라이밍 자리를 대신) + **실 재료(`real`)일 때만** `--system-prompt-snapshot off`(이어받기에도 새 글 — 실측 확실 · 같은 글이면 캐시 손해 없음 실측) | 새 대화 = `thread/start.developerInstructions`(실측 확실). 이어받기 = 지시서 해시가 지난 적용값과 다를 때만 첫 턴 `additionalContext`(실험 API — T6) | Engram 폴더 또는 「만든다」 대답 = 작업 폴더 `AGENTS.override.md`(머리에 생성 표식 줄 · 실행마다 결정적으로 다시 씀 · 그 폴더에 사용자 `AGENTS.md` 가 있으면 그 내용을 먼저 옮겨 싣는다 — 아래 위험). 「안 만든다」 = 프라이밍만 `-c developer_instructions=`(지금 길 · 명령줄 8191자 한도라 프리셋 지시서는 싣지 않고 경고) |
| 스킬 | 휘발 폴더 `engram\skills\<이름>` = 라이브러리 스킬 폴더로 가는 디렉터리 정션 → `--plugin-dir <휘발>\engram` → 이름 `engram:<스킬>`(T7 · 플러그인 폴더 적재는 실측 확실, 정션 따라가기는 미측) | 같은 정션 폴더를 `skills/extraRoots/set`(프로세스마다 · 실측 가능성 높음) | 작업 폴더 `.agents\skills\engram-<id8>-<이름>` 정션(codex 가 정션을 따라감 — 실측 확실). 지난 것은 「생성물 소유」 규칙으로 먼저 걷는다 |
| 원문 설정 | 데몬의 제어 settings 조각(`allowedMcpServers`) + 프리셋 claude 원문들을 담은 순서로 합쳐 휘발 폴더 `settings.json` 하나 → `--settings` 하나. `allowedMcpServers` 는 **Engram 값이 권위** — 프리셋이 그 키를 쓰면 버리고 경고(T10 · ADR-0109 결정 9 대체) | 프리셋 codex 원문 줄을 `-c` 로 하나씩 · 패스스루 뒤 · 우리 덮어쓰기 앞 | 같음 — 명령줄 가드에 걸리는 줄(따옴표 등)은 빼고 경고 |
| 모델 | `--model <m>`(패스스루 뒤 · 변이 그룹 앞 · 덮으면 경고) · 없으면 안 싣는다(R35) | `-c model=<m>`(TOML 로 안 읽히면 문자 그대로 — `--help` 원문) | 같음 |
| 크기 한도 | — | — | 합친 지시서 > 32 KiB 면 `-c project_doc_max_bytes=<n>` · 스킬 문맥 몫이 크면 `-c skills.max_context_tokens=<n>` — 실행마다 계산(저장 안 함) · 바뀌었을 때만 경고 줄(R38) |

**JSON 모드 모델 · 강도 고르는 칸(T8 — 마지막 단계)** — 막지 않는다(3차 델타): 설정 모델(있으면 잠김) 또는 CLI 기본값이 미리 골라져 있고, 첫 메시지가 어디서 오든(화면 · 우편 · 버스) 그때의 고른 값으로 나간다. 고른 값은 그 세션 것이고 저장하지 않는다(R35). **모델과 강도 둘 다 첫 사용자 메시지 전에 backend 요청으로 세션에 닿는다** — 길은 backend 가 정한다.
- claude: stream-json 의 control request 로 모델 · 강도를 바꾼다(SDK 의 바꾸기와 같은 요청 꼴). 목록은 `initialize` 응답(메모 27 실측).
- codex: 첫 `turn/start` 의 모델 · 강도 덮어쓰기 칸. 목록은 `model/list`.
- **수용 관문(14 단계)**: JSON 백엔드 **각각**(claude · codex)에서 모델 · 강도 둘 다 첫 메시지 전에 바뀌는지 실 CLI 로 확인한다(다음 턴 표기).
- **멈춤 조건**: 어느 백엔드에서든 그 요청이 안 되면 그 부분의 구현을 **멈추고 사용자에게 돌아간다**(R35 를 고치거나 지원되는 다른 길을 찾는다). 칸을 조용히 끄는 것은 허용되는 대체가 아니다. 대기 입력 명부 · 첫 제출 래치의 소유는 건드리지 않는다.

`DISABLE_AUTOUPDATER` 는 내장 기본 프리셋의 claude 원문(settings `env`)으로 실린다 — 이 브랜치 · HEAD 에는 claude 백엔드에 박힌 값이 없다(grep 0건). 다른 세션이 박았다면 그 머지 때 이 길로 옮긴다(R32).

**명령 어휘의 자리와 소유(ADR-0155 결정 1 · ADR-0251 선례)** — `library.*` · `preset.*` · 에이전트 프리셋 명령은 **agent crate 의 `declare_commands!` 블록에 선언한다**(catalog 6 → 7). 어휘(이름 · 인자 · 답 행 타입)의 주인은 agent crate 이고, 데몬은 그 어휘를 **구현**만 한다 — 포트 `LibraryCommandHost`(실물 = 데몬 `LibraryService`) + `blocking_handler`. `agent.*` 프리셋 명령도 같은 포트로 간다(붙임 검증에 라이브러리가 필요하다). 거부 = 데몬 자기 블록 + 표 합성(ADR-0251 이 `usage.*` 에서 같은 이유로 거부) · 라이브러리 전용 crate(소비자 하나).

### 1-6. platform — OS 층 동작 셋

| 함수(자리표시) | Windows | 그 밖 | 바탕 |
|---|---|---|---|
| `fs::move_to_trash(paths)` | 휴지통 — platform 안의 **전용 STA 작업 스레드 하나**가 `IFileOperation`(`FOFX_RECYCLEONDELETE` · `FOF_ALLOWUNDO` · `FOF_NO_UI`)을 **이미 쓰는 `windows` 0.58** 로 부른다. 부르는 쪽 스레드의 COM 아파트와 무관하다(요청은 채널로 넘기고 결과를 기다린다) | 지금은 `Unsupported`(macOS 이식 때 Finder 휴지통) | `windows` 0.58 에 `Win32_UI_Shell` 등 feature 를 더한다(합집합 한 의존 — ADR-0275 결정 13) |
| `fs::link_dir(target, link)` · `fs::unlink_dir(link)` | 디렉터리 정션(관리자 권한 불요) · `unlink_dir` 는 정션이 아니면 거절 | 심볼릭 링크(std) | `junction` 2.1.0 — `windows-sys` `>=0.59, <=0.61` 을 쓰는데 그 판들(0.59.0 · 0.60.2 · 0.61.2)이 이미 `Cargo.lock` 에 있어 새 사본이 늘지 않는다(가능성 높음 — 해석기가 고르는 판은 2 단계에서 `cargo tree` 로 확인) |
| `fs::same_path(a, b)` | 대소문자 무시 비교 | 그대로 비교 | std |

- **게이트** — OS crate 의존 게이트(`/qa` 4g · CI `platform gate 5`)의 손 목록에 `junction` 을 더한다. 새 OS 별 코드 · 시험은 platform 안에 두며, platform **밖**의 시험 파일이 OS `cfg` 를 써야 하면 OS cfg 파일 명단 게이트(`/qa` 4f · CI `platform gate 4`)의 명단도 같은 커밋에서 고친다(되도록 platform 도우미를 불러 `cfg` 없이 쓴다).
- **`trash` crate 를 쓰지 않는 이유** — 셋째 `windows` 사본이 들어오고(0.58 · 0.61.3 에 더해), 호출 스레드에서 `CoInitialize` 가 실패하면 패닉한다는 리뷰 지적(재지 않았다). 전용 STA 스레드가 그 두 문제를 함께 피한다.

### 1-7. 명령 표면 — 화면 단추와 LLM 이 같은 것을 부른다

**웹뷰 → 데몬 왕복(선행 작업)** — 웹뷰는 이미 `forward_daemon_command` 로 wire 명령을 보내고 답을 request_id 로 맞춘다. 버스 봉투(`Command{envelope}`)의 답 `CommandReply` 는 상관 키가 봉투 안이라 웹뷰 `handleEvent` 가 아직 못 맞춘다(셸 `daemon_client/inbound.rs` 헤더의 그 틈). 그 갈래를 더하고 `agentClient.callCommand(name, args)` 를 연다. 데몬은 자기 표의 이름을 그 자리에서 답한다(`command_delivery::deliver` 의 `locals.claim`).

**데몬 명령(agent 블록 선언 · 본문 = `LibraryService`)**

| 이름 | 하는 일 | 답 / 비고 |
|---|---|---|
| `library.status` | 루트 · 상태 · 데몬 신원 · 새로 만들기 제안 경로 · Engram 폴더 위치 | R8 · R9 |
| `library.setRoot {path, create?}` | 루트 바꾸기(`create` = 없으면 만든다) | R9 — 화면 머리 메뉴 · 설정 · LLM 같은 명령 |
| `library.folders` | 폴더 트리 전체 — 폴더 + 스킬 폴더(스킬 표시 — 트리에 스킬 아이콘 · 사용자 2026-10-17) | 트리 칸 |
| `library.refresh {folder?}` | 그 폴더(하위 포함) 다시 훑기 · 생략 = 전체 — 트리 우클릭 「새로 고침」 · 에이전트에게 부르라고 권한다(2026-10-16 · 권고 — 2026-10-17) | 바뀐 항목 수 + 이번에 다시 읽은 파일의 문제 — 「오류 — <경로> · <줄> · <이유>」(깨진 종류 파일 · 모르는 `kind`) · 「kind 없음 — 목록에 안 보임 — <경로>」(다시 읽은 `.yaml` 에 `kind` 없음 — 평소 목록엔 말없이 숨지만 부른 쪽엔 알린다) · 「찾을 수 없음 — <프리셋 경로> → <없는 경로>」 — 코드 + 인자로(R49 · 사용자 2026-10-17) |
| `library.list {folder?, query?, kinds?, builtins?}` | 폴더 바로 안 항목 + 하위 폴더 줄(스킬 폴더 포함 — 종류 = 스킬) · `query` 면 하위까지 검색 · `builtins` 면 고정 「내장」 묶음(기본 · claude 기본 · codex 기본)을 따로 싣는다 | 행 = 경로 · 종류 · 이름 · 상태(다시 이어짐 포함 — Δd) · 쓰는 곳 수 · 설명. 에이전트 슬롯의 `+` 검색이 `builtins` 를 켠다(Δa) |
| `library.moves {since}` | 옮김 기록에서 커서 뒤의 쌍 | `{log_id, pairs[{seq, old, new}], head, truncated}`(Δc) |
| `library.read {path}` | 항목 상세 — 설명 · 위치(라이브러리 경로 + 절대 경로) · 쓰는 곳(프리셋 · 에이전트, 직접/건너서) · 상태 · 지시서 = 본문 앞부분(미리보기용 · 길이 상한) · 설정 = 백엔드 · 모델 · 원문 칸 이름 · 오류 = 이유 · 줄 (R15 · 2026-10-17) | R15 · R41(절대 경로를 돌려준다) |
| `library.move {paths, toFolder}` · `library.rename {path, newName}` | 옮기기 · 이름 바꾸기 + 참조 고침 | 대응표(옛 → 새) · 고친 수(Δ3) |
| `library.delete {paths, yes?}` | 휴지통으로 | 쓰는 곳 있으면 `yes` 없이 `CONFLICT`(Δ9) |
| `library.newFolder {parent, name}` · `library.newFile {folder, name, kind: markdown|settings|preset}` | 만들기 — 마크다운 = 빈 `.md` · 설정 · 프리셋 = `kind` 틀(2026-10-16 · 옛 `library.newPreset` 대신) | 새 경로 |
| `library.relink {holder, from, to}` · `library.ackRelink {holder, path}` | 손으로 다시 잇기 · 확인 대기 후보 확인(같은 적용 함수) · 자동 표시 지우기 | `holder` = 프리셋 경로 또는 에이전트 |
| `preset.get {path}` | 종류별 칸 · 각 줄 상태 · 충돌 · 경고 · `rev` · 「에이전트 N개가 씀」(Δ4) | 안에 든 프리셋 펼침은 화면이 그 경로로 `preset.get` 을 또 부른다 |
| `preset.add {path, items, after?}` · `preset.remove {path, items}` · `preset.reorder {path, section, order, baseRev}` | 담기 · 빼기 · 순서 — 최신 파일에 적용(흐름 ③) | `add` · `remove` 는 낡은 판이어도 멱등 · `reorder` 는 `baseRev` 가 다르면 `CONFLICT` · 여러 개 담기는 `{added, skipped}`(Δ7) · 백엔드가 바뀌는 에이전트를 싣는다(Δ4) |
| `agent.presets {agent}` | 에이전트의 「기본」 · 프리셋 칸 · 해석된 백엔드 · 모드 · 충돌 · 경고 | R31 |
| `agent.attachPreset {agent, presets, after?}` · `agent.detachPreset {agent, presets, yes?}` · `agent.reorderPresets {agent, order, baseRev}` | 붙이기 · 떼기 · 순서 | 프리셋만 받는다(Δ7) · 부분 적용 한 줄 결과 · 해석된 백엔드가 바뀌면 「다음 실행은 새 대화」를 싣는다 · 내장 백엔드는 떼지 않고 쉬게 둔다(3차 델타). 기본 떼기는 `yes` 없이 `CONFLICT`(Δ9) |
| `agent.setWorkspace {agent, path?, engramFolder?}` | 워크스페이스 정하기 / Engram 폴더 만들기 | R34 · 떠 있으면 `CONFLICT`(멈춘 뒤에만) |
| `agent.setCodexFiles {agent, allow}` | codex 터미널 파일 대답 | R37 |
| `agent.launchCheck {agent}` | 막힘 사유(구조) · 경고 | 화면 대화상자의 재료 |
| `agent.new` (바뀜) | `backend` + 모드 + `cwd?`(없으면 Engram 폴더 — R34) + `presets?` + `files?`(codex 터미널 · 기본 = 안 만듦 — Δ9) | 옛 `preset`(작업 폴더 북마크 id) 칸은 걷는다 |
| `agent.spawn` (뜻 그대로) | 버스 · CLI 실행 = `Unattended` | 대화상자 없음 · 기본값을 채워 저장(Δg · 3차 델타) |
| `workspace.getRoot` · `workspace.setRoot {path}` | Engram 폴더 위치 | 새 에이전트에만(R34) |

**셸 명령(셸 블록 선언 · catalog 15 → 16)**

| 이름 | 하는 일 |
|---|---|
| `collection.list` · `collection.create {name}` · `collection.rename` · `collection.delete` | 묶음 관리(지금 붙은 데몬의 묶음) |
| `collection.add {collection, paths}` · `collection.remove` · `collection.setTarget {collection}` | 항목 넣고 빼기 · ★ 대상 |
| `collection.applyMoves {daemonId, logId, pairs, head}` | 옮김 기록 쌍을 묶음에 반영하고 커서를 옮긴다(Δc) |
| `slot.openUnique {content, target?}` | 같은 내용의 슬롯이 어느 창에든 있으면 그 슬롯에 포커스(창 앞으로) · 없으면 대상(생략 = 기존 열 자리 고르기 규칙)에 채움. 유일성 자체는 레이아웃 관리자가 지킨다(§1-9) |
| `libraryView.open {number}` · `libraryView.navigate {number, folder}` · `libraryView.back` · `libraryView.forward` | 번호 슬롯 열기 · 폴더 이동 · 기록(R14 — 기록 규칙은 셸의 순수 함수) |
| `libraryView.reveal {number?, path}` | 그 항목 자리로 — 트리 펼치고 고름(R15 · R28) · 번호 생략 = Δ6 규칙 |
| `libraryView.select {number, paths}` · `libraryView.setLayout {number, widths, sort, filter}` · `libraryView.get {number}` | 선택 · 칸 폭 · 정렬 · 필터 · 지금 상태 |
| `agentPresets.open {agentId}` | 에이전트 프리셋 슬롯을 하나 더 연다(R30) |

`libraryView.*` 는 **셸이 가진 번호별 보기 상태 위의 셸 명령**이다(R40 의 「레이아웃 명령」 쪽). 셸이 상태를 바꾸고 그 번호의 슬롯을 가진 창에만 `library-view:updated {number, state}` 를 쏜다. 화면은 사람 몸짓도 같은 명령으로 보내고(invoke) 받은 상태를 그리기만 한다 — 새 전역 핸들 · 웹뷰 레지스트리 명령 없음.

**CLI** — 일반형 `engram <이름> --인자 값` 이 데몬의 런타임 표를 읽으므로 CLI 코드는 안 바뀐다. 불리언은 값 없이 선다(`--yes`). 화면 단추도 같은 명령이다 — 확인 창을 거친 뒤 `yes=true` 로 부른다.

### 1-8. wire · 세대

- **옛 프리셋 wire 를 걷는다** — `ListPresets` · `CreatePreset` · `DeletePreset` · `RenamePreset` · `PresetListUpdated` · `PresetList` · 도메인 `Preset` · `PresetId`. `PROTOCOL_VERSION` 7 → 8.
- **`cwd` 의 wire · 화면 모양** — `CreateProfile.cwd` 는 선택이 된다(만들 때 폴더를 안 묻는다). wire `AgentProfile.cwd` · `AgentInfo.cwd` · 버스 `AgentRow.cwd` 는 문자열 그대로 두고 **빈 문자열 = 아직 작업 폴더 없음**으로 정한다(칸 모양을 안 바꿔 생성물 · 옛 소비자가 그대로 읽는다). 화면은 빈 값을 「작업 폴더 없음 · 실행 때 정함」으로 그리고, 이름은 고정된 `display_name` 을 쓴다(빈 basename 으로 파생하지 않는다). 이 변경은 3 단계(§5)에 함께 든다.
- 라이브러리용 새 wire 메시지는 없다 — 전부 버스 봉투(§1-7).
- agent catalog 6 → 7 · shell catalog 15 → 16 — **선언을 처음 바꾸는 단계에서 올린다**(1 단계: 옛 `agent.new` 칸 · `SlotContentKind` 의 `preset_palette` 를 걷으므로 둘 다). 같은 브랜치의 뒤 단계는 릴리스 전이라 같은 번호를 쓰되 생성물(`commands.schema.json` · 바인딩)은 단계마다 다시 굽는다. 병행 브랜치가 먼저 올렸으면 머지 때 그 +1.

### 1-9. shell (`src-tauri`)

- **슬롯 종류(`layout/types.rs`)** — `SlotContent::Library { number: u8 }`(1~4 밖 값 = 빈 슬롯) · `SlotContent::AgentPresets { agent_id: String }`. `PresetPalette` 는 걷고 옛 `{"type":"preset_palette"}` 는 읽을 때 빈 슬롯으로 접는다. 버스 `SlotContentKind` 거울 · 영속 모양 · 소진 검사(`state/schema.rs` · `convert.rs`)를 함께 고친다. 평범한 슬롯이라 팝아웃 · 도킹(wt2)이 그대로 붙는다(R42).
- **`Library{n}` 유일성은 레이아웃 관리자가 지킨다** — 슬롯 내용을 쓰는 단일 길(`write_slot_content` — `set_slot_content` · 버스 `layout.setSlotContent` 가 지난다)이 같은 번호가 이미 어느 창에든 있으면 `CONFLICT` 로 거절하고, 복원(`from_persisted`)은 같은 번호의 둘째부터 빈 슬롯 + 복원 경고로 접는다. 슬롯 옮기기(`move_slot_to_new_view`)는 복제가 아니라 통과. `slot.openUnique` 는 그 위의 편의(있으면 포커스)일 뿐이다.
- **번호별 보기 상태(신설 `library_view/` — 권위)** — 메모리 `[LibraryViewState; 4]` + `state.json` 선택 칸 `library_views`(빠져도 읽히는 칸 — `version` 안 올림 · 저장 스레드 스냅숏에 함께). 바꾸는 길은 `libraryView.*` 셸 명령 하나(사람 몸짓 · LLM 공용 — §1-7)이고 기록 규칙(R14)은 여기의 순수 함수다. 바뀌면 그 번호의 슬롯을 가진 창에 `library-view:updated` 를, 선택이 바뀌면 모든 창에 요약 `library-views:selection {number, kinds, touched_at}` 을 쏜다 — 다른 창 편집 칸이 ⇐ 활성을 정하고(R24), ⇐ 를 누르면 그 번호의 선택 경로를 셸에서 받는다.
- **묶음(신설 `collections/`)** — `shell\config\collections.json` = `{version, daemons: {<daemon_id>: {collections:[{id, name, paths}], target, moves_cursor: {log_id, seq}}}}` · 원자적 쓰기 · 바뀌면 모든 창에 `collections-updated`. `collection.applyMoves` 는 멱등이다(커서가 이미 지난 쌍은 버린다 — 창 둘이 동시에 따라잡아도 안전).
- **로컬 OS 요청(신설 `commands/local_os.rs`)** — `os_pick_folder` · `os_open_path(abs)` · `os_reveal(abs)` — 셸 Rust 가 dialog · opener 플러그인 Rust API 를 부른다(웹뷰 opener 권한을 넓히지 않는다). 지금은 늘 켜짐(같은 PC — 메모 63) · 원격이면 끈다. 웹뷰의 직접 `open({directory:true})` 셋(`agentCommands.ts:22` · `presetCommands.ts:76` · `slotContentCommands.ts:73`)도 이 길로 옮긴다.
- **설정 키** — `confirm.detachDefaultPreset`(「다시 묻지 않기」 · 기본 켬). 설정 창이 없으므로 되돌리기는 `settings.set` 명령으로만(T5 · E3).

### 1-10. frontend (`src/`)

**부품(감싸기 = 서드파티는 이 부품 안에서만 import)**

| 부품 | 맡는 것 | 안에 감싼 서드파티 |
|---|---|---|
| `components/library/LibrarySlot.tsx` | 번호 슬롯 · 에이전트 슬롯 공용 껍데기 · `mode = 'full' \| 'editor-only'` — 에이전트 슬롯은 트리 · 목록을 **마운트하지 않는다**(훑기 없음 — R30 의 성능 걱정을 마운트 단위로 푼다) | — |
| `LibraryTopBar` | ← → · 누르는 경로(R11) | — |
| `LibraryTree` | 폴더 트리(스킬 폴더 포함 — 폴더 = 폴더 아이콘 · 스킬 = 스킬 아이콘 `FolderCog` 후보 · §6-2) · 루트 머리(이름 + 경로 · 「라이브러리 폴더 바꾸기…」) · 빈/없음 안내(R8) · 놓기 대상 · 우클릭 「새로 고침」(폴더 = 그 폴더 · 루트 머리 줄 = 전체) | react-arborist(통제형 데이터) |
| `CollectionsPane` | 「즐겨찾기」 머리 · 첫 줄 「기본」(지울 수 없음) · 「+ 새 묶음」(R16) | — |
| `LibraryList` | 탐색기식 목록 · 열 ★ · 종류 · 이름 · 쓰는 곳 · 이름 앞 종류 아이콘(폴더 · 스킬 `FolderCog` · 지시서 `FileText` · 설정 `SlidersHorizontal` · 프리셋 `Layers` — 이름은 후보 · §6-2 · 사용자 2026-10-17) · 정렬(R12) · 📁 줄(스킬 폴더 포함 — 더블클릭 = 들어감 · 그 `SKILL.md` 더블클릭 = OS 기본 편집기) · 다중 선택 · 끌기 시작 · 필터 ▾ · 검색 · ★ 대상 칩 | @tanstack/react-table · @tanstack/react-virtual · react-selecto |
| `LibraryDetail` | 부품 상세(설명 · 위치 · 쓰는 곳 링크 — 단추 없음 · R15) · 상태별 = 안 고름 안내 · 폴더 · 지시서 미리보기(react-markdown — 챗과 같은 렌더러) · 설정 값 요약 · 여러 개 개수 · 오류(여러 개 개수는 클라이언트가 선택에서 센다) | — |
| `components/preset/PresetEditor.tsx` | **공용 편집 부품 하나**(R21) — 대상 = 프리셋 또는 에이전트 · 칸 = (에이전트면 「기본」 +) 프리셋 · 지시서 · 스킬 · 설정 · `readOnly` · `depth` 를 받아 안에 든 프리셋을 **같은 부품으로 재귀** 렌더(R27) · 칸 아래 충돌/경고 줄 · 「에이전트 N개가 씀」 · 백엔드 바뀜 경고(Δ4) | — |
| `EditorSection` | 칸 제목 `+` · `⇐` · 폴더 돋보기(`FolderSearch` — 아이콘 이름 확인함) · ⋮⋮ 순서(지시서 · 설정 · 프리셋 — D2) · × · 줄 호버 돋보기 · 지시서 줄 👁 = `LibraryDetail` 카드를 왼쪽 팝오버로(닫기 = × · 바깥 누름 · Esc · 다른 줄 👁 = 바꿈 · R27) | @dnd-kit/sortable |
| `AddItemsPopover` | 「+ 담기」 · 칸 `+` 검색 팝오버(R25) — 열면 검색칸 커서 · 체크 = 즉시 담기 · 담김/순환/충돌 비활성 + 이유 · 에이전트 대상이면 프리셋만 + 맨 위 고정 「내장」 묶음(Δa) | @radix-ui/react-popover · @tanstack/react-virtual |
| `DetailFlyout` | 편집 칸 줄 상세 — 창의 레이아웃 뿌리에 포털로 그리고 슬롯 오른쪽 가장자리에 붙여 옆 슬롯 위에 겹침 · 창 끝이면 일부 잘림 · **전부 창 밖이면 슬롯 안 오른쪽 가장자리에서 안쪽으로 겹침**(D11 · Δi) · 같은 줄 = 닫힘 · Esc(R29 · E1) | — |
| `LibraryDnd` | 목록 → 트리 끌어 놓기(옮기기) 문맥 | @dnd-kit/core |
| `DeleteConfirm` · `RelinkPicker` · `WorkspaceAsk` · `CodexFilesAsk` · `DetachDefaultConfirm` | 확인 창들 | 기존 대화상자 꼴 |

**단축키(R50)** — 라이브러리 슬롯 키는 기존 키 표(`src/commands/keybindings.ts` — 조합 → 명령, ADR-0055)에 넣고 **항목마다 Windows 키 · Mac 키를 따로 적는다**(VS Code 의 `mac:` · Spacedrive 의 `{macos, default}` 와 같은 꼴). OS 판정은 그 표 한 곳에서만 한다. 새 단축키 라이브러리는 들이지 않는다(Ctrl↔Cmd 바꾸기는 한 줄). 슬롯 포커스 안에서만 듣고 입력 칸이면 넘긴다(기존 포커스 가드 `isEditableTarget`). F5 · Ctrl+F · Alt+← → 는 웹뷰 기본 동작(다시 읽기 · 찾기 · 기록 이동)을 `preventDefault` 로 막고 우리 동작에 붙인다. react-arborist 가 내장한 Enter = 이름 바꾸기 · Backspace = 지우기는 탐색기 키와 부딪히므로 끄거나 덮는다.

**데이터(요청형)** — `api/libraryClient.ts` 가 `agentClient.callCommand` 위의 타입 있는 얇은 층(인자 타입 = agent crate 생성 바인딩). `api/libraryQueries.ts` 가 TanStack Query 를 import 하는 **유일한 파일**이다(T1) — 키 = `[daemonId, 종류, 인자]`. 슬롯 열기 · 포커스(슬롯 포커스 이벤트) · 새로고침 · 폴더 이동 때 그 슬롯의 쿼리를 무효화해 다시 받고, 같은 계기에 `library.moves {since}` → 새 쌍이 있으면 `collection.applyMoves` 를 부른다(Δc). 창 포커스 자동 재요청 · 주기 재요청은 끈다(R19). 편집 답은 그 자리의 쿼리 데이터를 바로 바꾼다. 다른 슬롯은 자기 포커스 때 맞춰진다.

**표시** — 「다시 이어짐 · 원래 <경로>」 표시는 편집 칸 줄 · 목록 행 상태 · 상세의 쓰는 곳 · 에이전트 슬롯 머리에 같은 부품으로 보인다(Δd). 확인 방식(D10)이면 같은 자리에 「다시 이어짐? · [확인]」. 내장 백엔드가 쉬면 그 줄은 흐린 「쉼」(3차 델타). 에이전트 슬롯 머리 = 이름 + 해석된 백엔드 · 모드 + 워크스페이스(버스 · CLI 실행이 Engram 폴더를 채웠으면 그 사실) + codex 파일 대답(R31 · 3차 델타).

**보기 상태** — 권위는 셸(§1-9). `store/libraryViewStore.ts`(zustand)는 셸이 쏜 `library-view:updated` 를 받아 둔 **그리기용 사본**이고, 몸짓은 `libraryView.*` 를 invoke 한다. 끄는 중의 칸 폭 같은 순간 값만 웹뷰에 머물고 끝난 값을 `libraryView.setLayout` 으로 보낸다.

**메뉴 · 진입**
- 빈 슬롯 「새 콘텐츠 ▸ 라이브러리 ▸ 1~4」 — 서브메뉴 한 단 더(ADR-0065 덧붙임 · §3) · 「프리셋 팔레트」 항목은 걷는다.
- 에이전트 트리 줄 메뉴 「프리셋…」 → `agentPresets.open`(인라인 메뉴지만 명령 id 로 부른다).
- 에이전트 트리 판 메뉴 「에이전트 생성 ▸ claude/codex × JSON/터미널」은 그대로 두고, 만들 때 폴더를 묻지 않는다 — 데몬이 기본 + 맞는 내장 백엔드 프리셋을 붙인다(Δ1). 활성화 전 `agent.launchCheck` → `NeedsWorkspace` 면 「어디서 일할까요?」 · `NeedsCodexFilesAnswer` 면 파일 질문.
- 폴더 돋보기(Δ6) — 에이전트 슬롯에선 마지막 만진 라이브러리 슬롯, 없으면 닫힌 번호 중 가장 작은 것 · 라이브러리 슬롯 칸 제목에선 **다른** 라이브러리 슬롯(같은 규칙, 자기 제외) · 그 종류로 `필터 ▾` 를 맞춘다(D7).

### 1-11. 시나리오

| 상황 | 흐름 |
|---|---|
| 첫 실행 | 「라이브러리 ▸ 1」 → `library.status` = unset → 빈 트리 안 안내 + [폴더 열기](셸 `os_pick_folder` → `library.setRoot`) · [새로 만들기](제안 경로를 채운 고르기 → `library.setRoot {create:true}`) |
| 폴더가 사라짐 | `library.status` = missing → 같은 자리 「<경로>를 찾을 수 없습니다」 + 같은 단추 · 값은 그대로(R8) |
| 팝아웃 | 슬롯 팝아웃 그대로 · 번호 상태 · 선택이 셸에 있어 창이 달라도 ⇐ 가 선다 |
| 재연결 · 데몬 재시작 | 쿼리 실패 → 연결 회복 뒤 포커스 · 새로고침에서 다시 받기. `rev` 는 데몬 프로세스 안에서만 뜻이 있어 재시작 뒤 첫 순서 바꾸기는 `CONFLICT` → 다시 받기(무해) |
| 옛 데이터 | 프로필 이주(§1-5) · `presets.json` 은 읽지도 지우지도 않고 남긴다(ADR-0264 「옛 데이터는 버린다」) · 옛 프리셋 팔레트 슬롯 = 빈 슬롯 |

### 1-12. 서드파티 라이브러리

현재 유무는 `package.json` · `Cargo.toml` · `Cargo.lock` 기준(2026-10-15). 버전 = `npm view` · `cargo search` 실측.

| 라이브러리 | 신규/기존 | 버전 | 쓰는 일 | 자리(감싼 부품) | 고른 이유(대안 대비 한 줄) |
|---|---|---|---|---|---|
| walkdir | 신규 직접(지금 간접) | 2.5.0 | 라이브러리 훑기 | daemon `library::store::FsStore` | 링크 순환 · 깊이 처리 내장 · `ignore` 는 과함 |
| serde-saphyr | 신규 | 2.0.0 | 프리셋 · 설정 YAML 과 `.md` 앞머리 읽고 쓰기 | daemon `library::format` | `serde_yaml` 은 0.9.34 + deprecated · `serde_yml` 0.0.x · `serde_norway` 2024-12 정체(메모 91) |
| relative-path | 신규 | 2.0.2 | `LibPath` 표현 · 루트 결합 · 옮기기 접두 바꾸기 | daemon `library::path` | `/` 고정 · `strip_prefix`/`join` 이 참조 고침을 덮는다(손 구현의 `\` · 접두 경계 버그 회피). `..` · 드라이브 거절은 우리 검증 |
| trash | **거부**(리뷰) | 5.2.9 | — | — | 셋째 `windows` 사본 + 호출 스레드 `CoInitialize` 실패 시 패닉(지적 · 미측) → 전용 STA 스레드의 `IFileOperation`(§1-6) |
| windows(feature 추가) | 기존 | 0.58 | 휴지통 — 전용 STA 스레드의 `IFileOperation` | platform `fs::move_to_trash` | 새 의존 0 · 부르는 쪽 COM 아파트와 무관 |
| junction | 신규 | 2.1.0 | 스킬 폴더 정션 | platform `fs::link_dir` | 관리자 권한 불요 · `windows-sys` 0.59–0.61 은 이미 lock 에 있다 · `mklink /J` 자식 프로세스는 링크마다 프로세스 · 인용 문제 |
| notify | **거부** | — | — | — | 요청형 확정(메모 99 · R19) — 나중 감시는 덧붙일 수 있다 |
| serde_json · uuid · tokio · tracing · thiserror · tempfile(dev) | 기존 | — | settings 합침 · 신원 · 기반 | 여러 곳 | 기반 — 감싸지 않음 |
| react-arborist | 기존 | ^3.9.0(최신 4.0.1) | 폴더 트리 | `LibraryTree` | 이미 씀 · 4.x 는 Node ≥22.12 · ESM · `react-dnd` peer 요구라 3.x 유지 |
| @tanstack/react-table | 신규 | 9.3.1 | 목록 열 · 정렬 · 선택 상태 | `LibraryList` | 사용자 결정(메모 113 · Spacedrive 조합) — React Aria 대안 거부 |
| @tanstack/react-virtual | 신규 | 3.14.19 | 긴 목록 가상화 | `LibraryList` · `AddItemsPopover` | 같은 조합 |
| react-selecto | 신규 | 1.26.3 | 목록 상자 끌어 다중 선택 | `LibraryList` | 사용자 결정(메모 113) |
| @dnd-kit/core · @dnd-kit/sortable | 신규 | 6.3.1 · 10.0.0 | 목록 → 트리 옮기기 · ⋮⋮ 순서 | `LibraryDnd` · `EditorSection` | 사용자 결정(메모 113) · 한 라이브러리로 두 몸짓 |
| @tanstack/react-query | 신규(T1) | 5.104.3 | 슬롯 데이터 받기 · 캐시 · 무효화 | `api/libraryQueries.ts` 한 파일 | 요청형 · 포커스 무효화 · 중복 요청 합치기가 그대로 맞음 — 손 zustand 캐시 거부 |
| @radix-ui/react-popover | 신규 | 1.3.1 | 「+ 담기」 검색 팝오버 | `AddItemsPopover` | 쓰던 radix 계열 · dropdown-menu 는 입력칸을 품기 어렵다 |
| 기존 그대로: Radix context-menu · dropdown-menu · tooltip · scroll-area · lucide-react(`FolderSearch` 설치본 확인) · zustand · plugin-dialog · plugin-opener | 기존 | — | 우클릭 · `필터 ▾` · 이유 툴팁 · 아이콘 · 그리기용 사본 · 폴더 고르기 · 열기/보이기 | 라이브러리 부품 · 셸 `local_os`(플러그인은 셸 Rust API 로만) | 이미 씀 |
| 위치 잡기 라이브러리(Floating UI 등) | **안 씀** | — | — | `DetailFlyout` | 뒤집기 없는 「가장자리에 붙고 잘림」 — 슬롯 사각형으로 충분 |

새 의존이 들어오면 T-62(라이선스 고지 파일)의 재생성 대상이다.

## 2. 사용자 결정

### 2-1. 이 설계가 기대는 PRD 추천값 (바뀌면 고칠 자리)

| PRD | 이 문서가 가정한 값 | 바뀌면 |
|---|---|---|
| D1 | 안에 든 프리셋의 지시서가 **앞** | `resolve.rs` 펼침 순서 한 곳 |
| D2 | 프리셋 칸도 끌어서 순서 | `EditorSection` 의 끌기 칸 목록 · `preset.reorder` 의 `section` 값 |
| D3 | 워크스페이스 = 에이전트 값 | 프로필 `workspace` · `agent.setWorkspace` · 슬롯 머리 |
| D4 | Δ1 로 대체 — 만들 때 기본 + 내장 백엔드 프리셋 | 프로필 생성자 · 이주 |
| D5 | 모드 = 에이전트 값 | `AgentCommand.output_format` 그대로 |
| D6 | 강도 = 백엔드 원문 칸 | 설정 파일 꼴 |
| D7 | 파일 안 `kind` 표식 · 폴더 돋보기 = 필터 | 종류 판정 · 돋보기 동작 |
| D8 | 「기본」 칸이 빈 채 남고 거기서 다시 붙임 | `PresetEditor` 기본 칸 |
| D9 | 세션 해석 넷 그대로 | `LibrarySlot` `editor-only` · 생성자 · JSON 고르기 칸 수명 |
| D10 | 다시 잇기 = 자동 적용 또는 한 번 확인 — **열린 결정** | 상수 `RelinkMode` 하나(§1-4 흐름 ②) — 두 길 다 설계돼 있다. 확인 방식이면 확인 전 참조는 실행 불가 |
| D11 | 상세 패널이 전부 창 밖이면 슬롯 안 오른쪽 가장자리에서 겹침(추천값) | `DetailFlyout` 위치 계산 한 곳 |

### 2-2. TRD 단계 결정 (각각 추천 + 이유 한 줄)

- **T1 슬롯 데이터 받기 = TanStack Query** — 추천: 채택 · `libraryQueries.ts` 한 파일에 가둔다. 이유: 요청형 · 포커스 무효화 · 편집 답 반영 · 중복 요청 합치기를 그대로 준다(메모가 예정한 `/research` 는 **아직 안 돌렸다** — 원하면 이 결정 전에).
- **T2 프리셋 파일의 손 주석** — 추천: Engram 이 고칠 때 사라짐을 받아들이고 그 사실을 적은 머리 주석을 Engram 이 늘 써 넣는다 · 메모는 `description` 칸. 이유: serde 왕복은 주석을 못 지키고, 주석 보존 crate 는 검증 안 됐다.
- **T3 `model` 칸 둘 이상** — 추천: 충돌. 이유: 백엔드 둘 = 충돌과 같은 규칙 · 나중 것이 이기면 어느 모델인지 화면에 안 보인다.
- **T4 Engram 폴더 기본 위치** — 추천: 릴리즈 `<배포판>\workspaces\<agent-id>` · 디버그 · `ENGRAM_DATA_DIR` = 데이터 루트 안 `workspaces\<agent-id>`(`DataLayout` 이 낸다) · 화면에는 고정된 이름을 보인다. 이유: 이름은 바뀌므로 id 로만 정해야 멱등 생성(있으면 재사용)이 서고, 프로그램 데이터와 작업물을 가르며 시험 격리가 폴더 하나로 된다.
- **T5 설정 창이 코드에 없다**(R9 · R34 · E3) — 추천: 첫 판은 트리 머리 메뉴 · 명령(`library.setRoot` · `workspace.setRoot` · `settings.set`)만 · 설정 창은 별 작업. 이유: 설정 창은 이 기능보다 넓은 화면 설계다.
- **T6 codex JSON 이어받기에서 바뀐 지시서** — 추천: 지시서 해시가 바뀐 때만 첫 턴 실험 API `additionalContext` 로 「바뀌었다」 + 새 글(backend/codex 안). 이유: R36 을 codex 에서 지키는 유일한 실측 길(공식 `developerInstructions` 는 이어받기에서 말없이 버려짐). 대안 = 새 대화까지 미반영 + 경고.
- **T7 claude 스킬 싣는 길** — 추천: `--plugin-dir`(이름 `engram:review`). 대안 = `--add-dir`(접두 없음 · 네이티브 스킬과 이름이 겹치면 승자를 모른다). 이유: 사용자 `~/.claude/skills` 와 안 섞이고 적재가 실측됐다.
- **T8 JSON 모드 모델 · 강도 고르는 칸** — 코드에 없다(R35 는 있는 듯 적혀 있다). 추천: 이 기능의 **마지막 단계**(§5 14). 막지 않고(미리 골라짐 · 어느 출처 첫 메시지든 그 값), 모델 · 강도 둘 다 첫 메시지 전에 backend 요청으로 세션에 닿게 한다. **claude · codex 각각의 실측 확인이 그 단계의 수용 관문**이고, 안 되면 구현을 멈추고 사용자에게 돌아간다(R35 개정 또는 다른 길) — 칸을 조용히 끄거나 재기동하지 않는다. 이유: 프리셋 없이도 서는 독립 부분이다.
- **T9 codex 압축 뒤 지시 재주입** — 추천: 첫 판 제외(지금 프라이밍도 같은 틈 — 압축이 developer 메시지를 버린다 · 실측). 이유: 압축 감지 + `inject_items`(실험 API)라는 별 경로가 필요하다.
- **T10 프리셋 claude 원문을 `--settings` 파일 하나에 합치기 = ADR-0109 결정 9(「조각은 `allowedMcpServers` 단일 키」) 대체** — 추천: 합치되 `allowedMcpServers` 는 Engram 값이 권위(프리셋은 넓히지도 덮지도 못하고 그 키는 버리고 경고). 이유: 두 번째 `--settings` 의 동작이 문서에 없어 한 파일이 유일하게 확실하고, 결정 9 가 지키려던 우편 서버 허용은 그 키의 권위로 지킨다. 대안 = 두 번째 `--settings`(미측 — 거부).

## 3. 내가 정한 내부 선택 (보고)

- **로컬라이제이션(PRD R49 · 2026-10-16)** — 화면 문자열은 `src/i18n` 키(`t()` · `ko.ts`)로만. 데몬 → 화면 사유 · 상태는 `{code, args}` 꼴(예 `{code:"conflict.backend_twice", args:{a, b}}`)로 답하고 화면이 `t(code, args)` 로 그린다 — `Resolution` · `LaunchBlocked` · `CONFLICT` 답 · 다시 잇기 표시가 해당. CLI 사람용 출력은 지금처럼 한국어(T-64 에서 정책 결정). 내장 프리셋 표시 이름 = 화면 키, 저장 값 = `engram:/…`.

본문에 근거가 있는 것은 자리만 가리킨다.

- **명령 선언 = agent crate 블록 + 포트**(§1-5) — `usage.*` 선례. 데몬 자기 블록 + 표 합성은 거부.
- **웹뷰 → 데몬 버스 왕복을 완성해 쓴다**(§1-7) — 라이브러리용 wire 메시지를 따로 만들지 않는다.
- **실행 전처리 = 예약 뒤 · 세션 파생 앞**(§1-5 (가)) — 에이전트마다 줄 서고, 명부 변경은 mutate 한 번(비교-교체)이라 사이에 바뀐 값을 덮지 않으며, 래치(ADR-0226)와 화신 표식은 맞춰진 프로필만 본다. 막힘도 제어 채널 발급 **앞**에서 끊는다.
- **실행 재료 = `check` → `prepare` + backend `materialize`** · **전용 RAII 가드**(`ProvisionGuard` 와 독립) · **배치 포트 `LaunchFs`**(디스크 없이 backend 배치를 시험).
- **패스스루 인자 = 백엔드별 저장**(`backend_args`) · `command` 는 해석에서 파생(디스크 · wire 모양 유지) · 낱말 → 변형은 `backend::command_for` 한 곳.
- **codex 작업 폴더 생성물 = 에이전트별 목록 + 내용 속 소유 표식** — 표식과 내용이 그대로일 때만 걷고, 떠 있는 동안은 멈춘 뒤로 미룬다(사용자 파일 보호).
- **지시서는 파일 하나로 합친다** — `--append-system-prompt-file` 반복 가능 여부를 묻지 않아도 되게.
- **claude `--system-prompt-snapshot off` 는 실 재료에만** — 이어받기가 바뀐 지시서를 보게(같은 글이면 캐시 손해 없음 — 실측). `NoopLaunchKits` 의 argv 는 바이트 그대로(회귀 골든).
- **claude settings 합침** — 객체 재귀 · 배열 이어 붙이고 같은 값 하나로 · 값은 나중 것. `allowedMcpServers` 는 Engram 값이 권위다 — 프리셋의 그 키는 합치지 않고 버리며 경고한다(T10).
- **덮어쓰기 순서** — codex 는 ADR-0216 규율(패스스루 → 프리셋 원문 → 모델 → Engram, 덮으면 경고) · claude 는 ADR-0094(`--allowedTools` 맨 끝).
- **내장 백엔드의 물러남 = 해석 규칙**(저장 변경 아님) — 사용자 백엔드를 떼면 저절로 돌아온다.
- **실행 출처는 진입점이 정한다** — wire `Spawn` · 활성화 = `Interactive` · 버스 `agent.spawn` · 제어 CLI · 부팅 자동 복원 = `Unattended`(물을 사람이 없다).
- **다시 잇기 = 후보 찾기 + 적용 함수 + 스위치 상수** — D10 이 어느 쪽이든 바뀌는 것은 상수 하나와 화면 줄 하나.
- **옮김 기록 = 최근 1,000 쌍 · 루트마다 `log_id`** — 넘친 커서는 손실을 받아들인다.
- **`rev` = 파일 바이트 64비트 해시(std)** — 데몬 프로세스 안에서만 비교 · 해시 crate 를 늘리지 않는다.
- **잠금 = 라이브러리당 `std::sync::RwLock` 하나** — 포트가 동기라 tokio 락이 아니다. 쓰기 잠금 안에서 `ProfileRegistry` 를 부르는 한 방향뿐이라 다른 락과 순환 간선이 없다.
- **내장 참조 = ASCII 고정 id**(`engram:/default` 등) · 표시 이름만 한국어.
- **기본 프리셋 = `default_preset` 불리언** — 따로 보이는 칸이고 순서에 안 낀다. 명령은 `engram:/default` 경로도 받는다.
- **옛 `preset_palette` 슬롯 = 빈 슬롯으로 접기** — 모르는 내용 보존은 걷은 기능을 지키는 장치가 아니다.
- **슬롯 메뉴 서브메뉴 한 단 더(ADR-0065 덧붙임)** — R10 의 「라이브러리 ▸ 1~4」 모양. 평평한 「라이브러리 1~4」는 거부.
- **번호별 보기 상태 = 셸이 권위 · `libraryView.*` 셸 명령** — 그 번호의 슬롯을 가진 창에 쏘고, 선택 요약은 모든 창에(⇐ 원천 — 팝아웃 사이에서도 선다).
- **로컬 OS 동작 = 셸 Rust 가 플러그인 Rust API 로** — 웹뷰 opener 권한을 넓히지 않는다.
- **칸 폭 = 번호별 보기 상태 안 · 기존 `Splitter` 계열** — `react-resizable-panels` 는 들이지 않는다.

## 4. 테스트 계획 (seam · 하네스 단위)

| 층 | seam · 하네스 | 무엇을 잰다 |
|---|---|---|
| daemon `library` | `MemStore`(디스크 없음)로 `LibraryService` 를 세운다 | 펼침 순서(D1) · 중복 · 순환 · 충돌 셋 · 내장 백엔드 물러남과 되살아남 · 막힘 사유 · 쓰는 곳(직접/건너서 · 에이전트 포함) · 다시 잇기 후보(한 후보만 · 지운 것 · 루트 바꿈 뒤 없음) × 스위치 두 값(자동 = 표시 · 확인 = 실행 불가) · 옮김 기록(커서 · 넘침 · `log_id` 바뀜) · 편집 멱등 · `reorder` 판 거절 · 부분 담기 결과 · 백엔드 바뀜 보고 · 실행 출처별 기본값 채움 |
| daemon `library::ops` | `MemStore` 에 실패 지점 주입 | 일지 롤포워드 — (c)~(f) 각 단계 뒤에 죽인 뒤 부팅 재생이 같은 끝 상태(지움 기록 · 옮김 기록 쌍이 한 번만) · 지우기 `yes` 거절 |
| daemon `library::format` · `path` | 순수 · 골든 | YAML 왕복 골든 · 오류 줄 번호 · `LibPath` 거절 표(`..` · 앞 `/` · `\` · `:` · 빈 칸) · 접두 바꾸기 |
| daemon `FsStore` · 기본 경로 · 구조 시험 | `tempfile` 임시 폴더 · `ENGRAM_DATA_DIR` · `connection_core` 구조 시험 | 실 훑기 · 원자적 쓰기 · 휴지통 · 정션(실 OS — `-- --test-threads=4` 줄) · 기본 경로가 루트 출처별로 갈림(`ENGRAM_DATA_DIR` = 루트 안) · `SpawnByCwd` 의 `spawn_agent` 가 `spawn_blocking` 창 안에서만 불림 |
| platform `fs` | 실 OS | 휴지통으로 간 파일이 원래 자리에 없음(전용 STA 스레드 — 부르는 스레드가 MTA 여도) · 정션 만들기/걷기 · 진짜 폴더는 `unlink_dir` 가 거절 |
| agent manager | `NoopLaunchKits` · 가짜 `LaunchKitSource` | Noop = 오늘과 바이트 같은 argv(회귀) · 막힘 = 스폰 안 함 + 제어 채널 미발급 · 없는 id 가 전처리 앞에 명부에 들어감 · 전처리가 예약 · 떠 있음 확인 뒤, 화신 표식 · 세션 파생 앞(순서 단언 — 기존 `squashed_production_body` 꼴) · 점검이 명부 값을 읽음 · 명부 변경이 mutate 한 번 · 비교-교체(그 사이 정해진 `workspace` 를 덮지 않음 · 한 번 다시 시도 뒤 실패) · 네 생성 길의 기본값이 같음 · Engram 폴더 = id 로 멱등 · 백엔드 · 워크스페이스 바뀜 = `release_session_id` 후 Fresh · 실행 재료 가드가 어느 실패 지점에서도 `release` · 생성물 걷기 |
| agent backend | 지금 `build_spec` 시험 꼴(argv 골든) + 메모리 `LaunchFs` | claude · codex 두 모드 × 재료 표(§1-5) · 순서 불변(`--allowedTools` 맨 끝 · 덮어쓰기는 패스스루 뒤) · `allowedMcpServers` 권위 · 명령줄 가드에 걸린 원문 줄 = 빠짐 + 경고 · codex 터미널 배치 · 사전 검사 · `AGENTS.md` 옮겨 싣기 · 소유 표식(표식 + 내용 일치일 때만 걷기 · 남은 생성물 정리가 사전 검사보다 먼저 · 떠 있는 동안 미룸) · 인자 권위(`backend_args` 가 있으면 `command.extra_args` 무시) |
| agent 명령 | 기존 `command_declarations` · `command_alphabet` + 가짜 `LibraryCommandHost`(사용량 선례) | 선언 · 인자 검문 · `--yes` 불리언 · 포트로 넘김 |
| 프로필 이주 · 호환 | 옛 `agents.json` 골든 | 기본 + 내장 백엔드 붙음 · `cwd` → `workspace` · 인자 → 지금 백엔드 칸 · `display_name` 고정 · codex 터미널 = 파일 안 만듦 · 두 번 읽어도 같음 · 새 파일을 **옛 구조체**로 파싱해도 선다(`cwd` 키 유지) · `merge_preserving_live` 가 새 칸을 지킴 |
| shell | 기존 `lib_unit` · `layout_commands` 하네스 | 새 슬롯 종류 영속 왕복 · `preset_palette` 접기 · `Library{n}` 중복 = 쓰기 길 `CONFLICT` · 복원 접기 · `library_views` 왕복 · `libraryView.*` 기록 규칙(R14) · 그 창에만 이벤트 · 묶음 파일 왕복 · `collection.applyMoves` 멱등 · 커서 `log_id` 바뀜 |
| front | vitest + 가짜 `agentClient` | 정렬(R12) · ⇐ 활성(R24) · `PresetEditor` 재귀 읽기 전용(R27) · 칸 제목 단추 순서(R23) · 쿼리 무효화 계기 |
| 실 CLI(로컬 · `#[ignore]` 또는 수동 — qa 「실 CLI 레인」) | 실 claude · codex | `--plugin-dir` 가 정션을 따라가나 · `--system-prompt-snapshot off` 이어받기 · `DISABLE_AUTOUPDATER` settings `env` · codex `extraRoots` 실제 턴 · codex 터미널 `AGENTS.override.md` |
| GUI 실측 | `/qa full` · `scripts/cdp.mjs` | PRD §4 수용 기준 전부 |

회귀망은 Windows 에 묶지 않는다 — 정션 · 휴지통 시험은 OS 별 기대값을 두고 파일 통째 `#![cfg(windows)]` 를 쓰지 않는다.

## 5. 이행 순서 (커밋마다 빌드 · 시험 초록 — 어디서 멈춰도 선다)

각 단계는 「자료구조를 먼저 갈아엎고 호출부를 나중에」가 아니라 **새 것을 옆에 세우고 → 잇고 → 옛 것을 걷는** 순서다.

0. **ADR 박제**(사용자 결정 직후 · 코드 없음) — §7 · ADR-0061 · ADR-0070(프리셋 이름 부분)에 폐기 도장.
1. `S21: refactor: 옛 프리셋(작업 폴더 북마크) 걷기` — `PresetPalette` · `presetCommands.ts` · 배경 우클릭 프리셋 피커 · `agentStore` 프리셋 · 옛 wire 여섯 · `PresetRegistry` · `presets.json` 저장소 · `agent.new` 의 `preset` 칸 · `SlotContent::PresetPalette`(옛 값 → 빈 슬롯). `PROTOCOL_VERSION` 7 → 8 · agent catalog 6 → 7 · shell catalog 15 → 16(`SlotContentKind` 이 바뀐다 — 선언을 바꾸는 이 단계에서). **먼저 걷는 이유** = 이름 `preset` 의 두 뜻 공존을 없애고 `AgentManager` 생성자 자리를 새 포트에 비운다(걷어도 트리 판 메뉴로 만들기는 선다).
2. `S21: feat(platform): 휴지통 · 디렉터리 링크 · 경로 비교` — `IFileOperation`(기존 `windows` 0.58 feature 추가) · `junction`(해석된 `windows-sys` 판을 `cargo tree` 로 확인) · 게이트 4g 목록(필요하면 4f 명단).
3. `S21: refactor(agent): 프로필 workspace(Option) · 프리셋 칸 · 백엔드별 인자 · 이주` — `cwd` → `workspace`(디스크 키는 `cwd` 유지 · 빈 문자열 = 없음) · `backend_args` · `display_name` 고정 · `merge_preserving_live` 보존 칸 · wire `cwd` 빈 문자열 뜻 · 화면의 빈 작업 폴더 표시(§1-8). `None` 이면 지금처럼 스폰 실패(사유 문장). 기존 에이전트 동작 불변.
4. `S21: feat(agent): 실행 전처리 + 실행 재료 포트 + manager 배선(Noop = 오늘과 같은 argv)` — `ensure_registered`(없는 id 를 전처리 앞에 넣기) · `prelaunch`(예약 뒤 · 명부에서 읽기 · 세션 파생 앞 · mutate 한 번 비교-교체 · 다시 시도 한 번 · Engram 폴더 id 멱등) · 생성자 기본값(`backend::builtin_backend_preset`) · `SpawnByCwd` 를 `spawn_blocking` 으로 · `backend::command_for` · `LaunchKitSource` · `LaunchKitGuard` · `LaunchFs` · `materialize` dispatch · `build_spec`/`open_spawn` 인자 · 회귀 골든. ★Noop 회귀는 `SpawnByCwd` · 스모크 bin 길에도 선다★ — 생성자가 붙인 프리셋을 Noop 은 읽지 않고, `extra_args` 는 `backend_args` 로 옮겨도 같은 순서로 실리며, 이름 배정은 자리만 앞으로 옮겼을 뿐 같은 함수다(골든에 그 두 길을 넣는다).
5. `S21: feat(agent): claude 실행 재료 번역` — 합친 지시서 · snapshot off · plugin-dir · 합친 settings · 모델.
6. `S21: feat(agent): codex 실행 재료 번역(JSON · 터미널)` — developerInstructions · extraRoots · 원문 `-c` · 모델 · 작업 폴더 파일 · 크기 한도 · 이어받기 `additionalContext`(T6). 둘로 쪼개도 된다(JSON / 터미널).
7. `S21: feat(daemon): library 모듈 — 경로 · 형식 · 저장소 seam · 훑기 · 해석 · 내장` — 아직 아무도 안 부른다.
8. `S21: feat(daemon): 실행 재료 실물 꽂기` — `DaemonLaunchKits` → manager. ★여기서 기존 에이전트가 내장 기본 + 백엔드 프리셋으로 뜬다 — 차이는 snapshot off · `DISABLE_AUTOUPDATER` · 합친 지시서 파일뿐★. 실 CLI 레인으로 claude · codex 각 1회 기동 확인.
9. `S21: feat(daemon): 루트 · 신원 · 기본 경로(DataLayout) · 다시 잇기(후보 + 스위치) · 옮김 기록 · 파일 조작 · 편집 · 일지(단계별 판정)` — 실행 출처별 기본값 채움도 여기서 켠다(그 전까지 manager 는 늘 `Interactive` 처럼 막는다).
10. `S21: feat(agent,daemon): 라이브러리 · 프리셋 · 에이전트 프리셋 명령 선언 + 포트` — catalog 7 유지(1 단계에서 올림 · 같은 브랜치) · 바인딩 재생성 · CLI 로 전부 부를 수 있다.
11. `S21: feat(front): 웹뷰 → 데몬 버스 왕복 + libraryClient` — `CommandReply` 갈래 · `callCommand`.
12. `S21: feat(shell): 슬롯 종류 둘 · Library{n} 유일성(쓰기 길 · 복원) · slot.openUnique · libraryView.* · 번호별 보기 상태 · 묶음 · 로컬 OS 요청` — shell catalog 은 1 단계에서 올린 16 그대로(같은 브랜치) · 생성물 다시 굽기 · 웹뷰 직접 다이얼로그 셋 이관.
13. 화면 — 여러 커밋. ① `LibrarySlot`(읽기 · 트리 · 목록 · 상세 · 빈/없음 안내 · 기록 · 묶음) ② 파일 조작(우클릭 · 끌어 옮기기 · 지우기 확인 · 다시 잇기) ③ `PresetEditor` + 담기 세 길 + 상세 패널 ④ 에이전트 프리셋 슬롯 + 트리 「프리셋…」 + 기본 떼기 확인 + 실행 전 대화상자(워크스페이스 · codex 파일). 각 커밋 = 그 화면이 선 채 `/qa`.
14. `S21: feat: JSON 모드 첫 메시지 전 모델 · 강도 고르기`(T8) — 따로 떨어진 마지막 단계. ★첫 커밋 = 수용 관문★: claude · codex 각각 모델 · 강도 요청이 첫 메시지 전에 먹는지 실 CLI 확인. 하나라도 안 되면 **여기서 멈추고 사용자 결정**(R35 개정 또는 다른 길) — 칸 끄기 · 재기동으로 넘어가지 않는다.
15. `S21: docs: CLAUDE.md 모듈 맵 · step-log · tracking` — load-bearing 이라 `/review doc`.

## 6. 위험 · 미결 · 확인 항목

### 6-1. 이 문서를 쓰며 확인한 것

| 항목 | 결과 | 확신도 |
|---|---|---|
| claude 2.1.284 `--help` | `--plugin-dir`(반복 가능 · 폴더 안 플러그인 각각) · `--add-dir` · `--settings <file-or-json>`(한 번 꼴) · `--model` · `--effort` · `--system-prompt-snapshot <on\|off>` 있음 | 확실 |
| codex 0.156.1 `--help` | `-c key=value`(TOML 로 안 읽히면 문자 그대로) · `-m` | 확실 |
| `DISABLE_AUTOUPDATER` 를 settings `env` 로 | 공식 문서(settings · env-vars)엔 이름이 없다. 설치본 바이너리 문자열: 업데이터가 `process.env.DISABLE_AUTOUPDATER` 를 읽고, 그 이름이 다른 알려진 환경변수들과 함께 한 목록에 있다(그 목록의 뜻은 모른다). settings `env` 가 프로세스 환경에 앉는 시점이 업데이터보다 앞인지는 모른다 | **가능성 높음 — 실측 필요(8 단계)** |
| 웹뷰 → 데몬 버스 답 | 웹뷰 `handleEvent` 가 `CommandReply` 를 안 맞춘다(셸 주석) · 데몬은 자기 이름을 그 자리에서 답한다 | 가능성 높음(코드 읽기 · 미측) |
| 코드에 없는 것 | 설정 창(라우트는 `/` · `/popup` 뿐) · JSON 모델 고르기 칸 · 2단 슬롯 서브메뉴(`slotMenu.ts` 「중첩 금지」) · 선택적 작업 폴더(`AgentProfile.cwd` 는 필수) — §6-4 | 확실 |

### 6-2. 확인 안 한 것 (해당 단계에서 잰다)

- claude `--plugin-dir` 가 **정션**을 따라 스킬을 싣나(5 단계) — 안 되면 휘발 폴더에 스킬 폴더를 복사한다(실행마다 · 결정적).
- codex `thread/start` 에 `model` 칸이 있나 — 없으면 `-c model=` 로(이미 그쪽이 기본안).
- codex `skills/extraRoots/set` 실제 모델 턴 적재(조사 공백) · `additionalContext` 의 압축 뒤 생존.
- 전용 STA 스레드의 `IFileOperation` 휴지통 — 2 단계에서 실 OS 로 잰다(대화상자 없이 · 여러 파일 · 정션을 품은 폴더).
- `AGENTS.override.md` 가 같은 폴더의 사용자 `AGENTS.md` 를 가린다 — 조사 보고서는 소스 · fallback 실측으로 「가린다」고 적었으나 대화형 TUI 는 미측. 대응(그 내용을 생성 파일 앞에 옮겨 싣기)과 함께 6 단계에서 잰다.
- JSON 고르기 칸의 모델 · 강도 바꾸기 요청(claude control request · codex `turn/start` 덮어쓰기) — 14 단계 첫 커밋의 수용 관문. **멈춤 조건**: 어느 JSON 백엔드에서든 안 되면 그 구현을 멈추고 사용자에게 돌아간다.
- `@dnd-kit` 과 react-arborist 내부 react-dnd 의 공존(목록 → 트리 놓기) — 안 되면 트리 행 그리기에 놓기 대상을 직접 단다.
- TanStack Table v9 API(조사 미확인).
- 라이브러리 훑기 비용 — 7 단계에서 생성한 5천 개 트리로 잰다. 목표는 정하지 않았다(실측 뒤).
- `forward_daemon_command` 30초 마감 대비 큰 훑기 — 비용 실측과 함께.
- 웹뷰(WebView2)가 F5 · Ctrl+R 로 앱 화면을 다시 읽는지, `preventDefault` 로 막히는지(R50).
- react-arborist 의 내장 키(Enter = 이름 바꾸기 · Backspace = 지우기)를 끄거나 덮을 수 있는지(R50).
- 맥 Finder 기본 단축키(R50 의 macOS 줄) — 기억 기준, 맥 이식 때 원문 확인.
- 종류 아이콘의 lucide 이름(`FolderCog` · `FileText` · `SlidersHorizontal` · `Layers`)이 설치된 lucide-react 판에 있는지 — 안 봤다(후보 · 사용자 2026-10-17). 없으면 가까운 이름으로.

### 6-3. 위험

| 위험 | 근거 · 대응 |
|---|---|
| 기본 프리셋을 떼면 프라이밍과 자동 업데이트 끄기가 함께 빠진다 | R32 확인 창 · CLI `--yes`. 업데이트가 설치를 깬 사례는 다른 세션 조사(이 브랜치에 없음) |
| 내장 백엔드가 말없이 쉬어 사용자가 백엔드를 바꾼 줄 모른다 | 붙일 때 「다음 실행은 새 대화」 한 줄 · 편집 때 백엔드 바뀜 경고(Δ4) · 흐린 「쉼」 표시 |
| 옮김 기록 커서가 바닥보다 오래되면(1,000 쌍 넘게 못 따라잡음) 그 사이 옮겨진 묶음 항목을 잃는다 | 받아들인 손실(§3) — 「찾을 수 없음」으로 보이고 손으로 다시 넣는다 |
| 확인 방식(D10)이면 바깥 옮기기 뒤 확인 전까지 그 에이전트가 안 뜬다 | 의도(Δe) — 막힘 사유가 「다시 잇기 확인 대기」를 말한다. 버스 · CLI 실행도 막힌다(확인은 사람 몫) |
| 사용자 폴더의 `AGENTS.md` 가 Engram 생성 `AGENTS.override.md` 에 가려진다 | 그 내용을 생성 파일 앞에 옮겨 싣고 실행마다 다시 만든다(실행 사이 바뀐 것도 다음 실행에 따라감) · 실측 전까지 가능성 높음 |
| 내림(옛 빌드) 뒤 다시 올리면 붙인 프리셋 · 다른 백엔드 인자를 잃는다 | §1-5 「호환 · 내림」 — 받아들인 손실 |
| 같은 이름이 흔하면 자동 다시 잇기가 거의 안 일어난다 | 손 다시 잇기(R19) · PRD 위험 그대로 |
| codex 터미널 명령줄 8191자 — 프리셋 지시서를 `-c` 로 못 싣는다 | 파일 길(R37) · 「안 만든다」면 프라이밍만 + 경고 |
| 「편집 맡은 에이전트에만 쓰기 권한」(메모 38) | 지금 claude 는 `bypassPermissions` 라 강제 수단이 없다 — 이 TRD 범위 밖 |

### 6-4. PRD 와 코드가 어긋나는 곳 (PRD 개정 또는 사용자 확인 필요)

- **설정 창이 없다** — R9 「설정 창에 같은 칸」 · R34 「Engram 폴더 위치는 설정」 · E3 「다시 묻지 않기 되돌리기 = 설정 창」(T5).
- **JSON 모드 모델 고르기 칸이 없다** — R35 는 있는 것처럼 적혀 있다(T8).
- **R32 「다른 세션이 claude 백엔드에 박는 `DISABLE_AUTOUPDATER`」** — 이 브랜치 · HEAD 에 없다(grep 0). master 나 다른 워크트리는 보지 않았다.
- **R36 「이어받기 포함」** — claude 는 `--system-prompt-snapshot off` 가 있어야 성립하고, codex 는 실험 API 없이는 성립하지 않는다(T6).
- **R34 「워크스페이스가 없으면」** — 프로필이 지금은 작업 폴더를 필수로 갖는다 → 3 단계의 칸 변경이 전제다.
- **R30 「이미 열려 있어도 하나 더」 + R24 ⇐** — 편집 칸과 원천 라이브러리 슬롯이 다른 창(팝아웃)에 있을 수 있어 선택을 셸이 나눠 들어야 한다(§1-9) — PRD 엔 드러나 있지 않다.
- **R10 「라이브러리 ▸ 1~4」** — 빈 슬롯 메뉴가 이미 「새 콘텐츠 ▸」 아래라 두 단 서브메뉴가 되고, 지금 메뉴 스키마는 한 단만 허용한다(§3).

## 7. ADR 제안 (번호는 `/adr` 가 준다)

1. **프리셋 · 라이브러리 = 데몬 소유 에셋, 클라이언트 요청형** — 라이브러리 폴더 하나 · 프리셋도 그 안의 YAML 파일 · 데몬 `library` 모듈 단독 읽기/쓰기 · 감시 없음. 거부: 셸 훑기 · notify 감시 + 방송 · 프리셋 내부 목록. **ADR-0061 · ADR-0070(프리셋 이름 부분) 폐기.**
2. **참조 = 라이브러리 루트 `/` 경로 + 내장 `engram:/` 네임스페이스** — 거부: 프리셋 파일 기준 상대 · 이름 참조 · ID 파일 · URI 꼴.
3. **실행 재료 포트(`LaunchKitSource`) + backend `materialize`** — 해석은 데몬, 플래그 번역은 backend. 거부: manager 가 프리셋을 해석 · backend 가 라이브러리를 읽음 · `ControlEndpoint` 에 칸 덧붙이기.
4. **claude 실행 재료 = 합친 지시서 파일 + `--system-prompt-snapshot off` + `--plugin-dir` 정션 + 합친 `--settings` 하나** — 거부: `--add-dir` 스킬 · 여러 `--settings`.
5. **codex 실행 재료** — JSON = developerInstructions · extraRoots · 이어받기 `additionalContext`(실험) · 터미널 = 작업 폴더 `AGENTS.override.md` + `.agents\skills\engram-*` 정션 · 「안 만든다」면 프라이밍만. 거부: `CODEX_HOME` 바꾸기 · 로컬 플러그인 설치.
6. **편집 = 데몬이 최신 파일에 적용하는 연산 + 판 확인(순서만)** — 거부: 클라이언트 초안 + 저장 단추 · 전부 판 확인.
7. **파일 조작 = 참조 고침 + 일지 롤포워드 · 지우기 = 휴지통** — 거부: 바깥 정리 + 이름 다시 잇기만 · 영구 삭제.
8. **다시 잇기 = 후보 찾기(바깥에서 없어진 것 · 같은 종류 + 이름 · 후보 하나) + 적용 스위치(자동 표시 / 한 번 확인 — D10)** — 거부: 이름만 보고 늘 · 리다이렉터 파일 · 루트를 넘는 다시 잇기.
9. **옮김 기록(번호 붙은 쌍) + 클라이언트 커서 따라잡기** — 클라이언트 소유 묶음을 방송 없이 고친다. 거부: 요청한 클라이언트에게만 대응표(다른 클라이언트가 못 따라잡는다) · 데몬이 묶음을 소유.
10. **실행 출처(`Interactive` / `Unattended`)가 대화상자 여부를 정한다** — 버스 · CLI 실행은 기본값을 채워 저장. 거부: 늘 막기(LLM 이 못 띄운다) · 늘 기본값(사람이 고를 기회가 없다).
11. **내장 백엔드 프리셋은 사용자 백엔드가 있으면 쉰다(해석 규칙)** — 거부: 붙일 때 자동으로 떼기 · 충돌로 막기.
12. **보는 쪽 상태(번호별 보기 · 묶음 · ★ 대상)는 셸, 키에 데몬 신원** — 거부: 데몬 · 웹뷰 `localStorage`.
13. **웹뷰 → 데몬 버스 왕복 완성 — 화면과 LLM 이 같은 명령** — 거부: 기능마다 타입 있는 wire 메시지.
14. **서드파티 감싸기 = 목적 포트 · 부품 경계**(walkdir · serde-saphyr → 라이브러리 저장소 · 형식 / 트리 · 목록 · 끌기 · 쿼리 → 우리 부품 한 곳씩) — 거부: 라이브러리마다 1:1 래퍼.
15. **슬롯 메뉴 서브메뉴 두 단(ADR-0065 덧붙임 개정)** — 거부: 평평한 「라이브러리 1~4」.
16. **슬롯 데이터 받기 = TanStack Query 한 파일**(T1 확정 시) — 거부: 손으로 짠 zustand 캐시.
17. **실행 전처리(점검 · 백엔드 · 워크스페이스 맞추기 · 무인 기본값)는 예약 · 떠 있음 확인 뒤, 세션 파생 앞 · 명부 변경은 mutate 한 번(비교-교체) · Engram 폴더는 id 로 멱등** — 거부: 기동 중간에 맞추기(래치 · 화신 표식이 옛 사본을 본다) · 예약 앞(같은 에이전트 두 실행이 겹친다) · 이름으로 폴더 정하기.
18. **패스스루 인자 = 백엔드별 저장(`backend_args` 가 권위) · `command` 는 해석에서 파생 · `extra_args` 는 내림용 비춤** — 거부: 한 목록을 백엔드 사이에 들고 다니기.
19. **ADR-0109 결정 9 대체 — 프리셋 claude 원문을 `--settings` 파일 하나에 합치되 `allowedMcpServers` 는 Engram 이 권위**(T10) — 거부: 두 번째 `--settings` · 원문을 안 싣기.
20. **휴지통 = platform 의 전용 STA 작업 스레드가 부르는 `IFileOperation`(기존 `windows` 0.58)** — 거부: `trash` crate(셋째 `windows` 사본 · `CoInitialize` 실패 패닉).
21. **번호별 보기 상태 = 셸 권위 + `libraryView.*` 셸 명령 · `Library{n}` 유일성은 레이아웃 관리자** — 거부: 웹뷰 레지스트리 명령 · 열기 명령에서만 유일성.
