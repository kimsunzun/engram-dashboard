# 핸드오프: 프리셋 PRD 이전 — 데몬 쪽 결정 마무리 · 참조 경로 · 파일 조작 · 화면 부품 · 라이선스 고지(T-54 다른 세션 진행 중)

## 한 줄 상태 · 다음 첫 액션

- **상태:** 프리셋 기능 PRD 이전 구상. 이번 세션에 **데몬 쪽 결정을 다 끝냈다**(목록 전달 · 감지 · 저장소 자리 · 참조 경로 · 기본 라이브러리 폴더 · 첫 판 파일 조작 · 휴지통) + **화면 부품 조합**까지 정했다. 라이선스 고지는 조사 · 결정까지 하고 **실제 생성은 사용자가 다른 세션 에이전트에게 맡겼다(진행 중)**.
- **다음 첫 액션(순서대로 · 하나씩 묻는다):**
  1. **T 번호 충돌부터 정리** — 아래 「⚠ T 번호 충돌」. 다른 세션 보고: wt1 미커밋 `docs/tracking.md` 의 T-53 · T-54 가 master 의 T-53 · T-54(전혀 다른 항목 — master T-54 = `taskkill /T` 관련)와 번호가 겹친다. 사용자에게 어떻게 할지 묻는다(라이선스 일은 다른 세션이 하고 있으니 그쪽 결과와 맞춘다).
  2. **브리핑 갱신**(사용자 「이따가 브리핑 문서에 어떻게 활용할지 써놔」) — 아래 「브리핑」.
  3. 브리핑 「열린 것」(섹션 10) 목록을 하나씩.
- **결정 정본 = `.claude/handoff/attachments/20261012-preset-decisions.md`(영어 · 맨 아래가 최신 — 2026-10-14 · 2026-10-15 절). 먼저 Read.** 이번 세션 결정은 전부 **이유 · 버린 안 · 사용자 원문**과 함께 적어 두었다.

## ⚠ 사용자 응대 — 이번 세션에 크게 실패했다(반복 금지)

- **사용자가 화를 냈다(욕설 포함).** 원인: ① 같은 내용을 여러 번 되묻게 만드는 설명(용어 · 표기를 앞세움 — `./` 표기를 계속 강조해 「상대 vs 절대」라는 단순한 선택을 복잡하게 보이게 함, 「라이선스」를 전문 · 허용 목록 두 뜻으로 섞어 씀, 「라이브러리 맨 위부터의 경로」처럼 돌려 말함) ② **핸드오프를 반복 제안**(사용자는 「50% 까지는 가야지 알아서 내가 얘기할게」라고 했다 — 세션 끊기는 사용자가 정한다. 다시 꺼내지 말 것) ③ 사용자가 물은 것과 다른 축으로 답함(엔진 · 플러그인을 비교 대상으로 가져옴 — 사용자는 「우리 같은 가벼운 브라우저형」을 원했다).
- **할 것:** 결론 한 줄 먼저 · 쉬운 말 · 사용자가 쓴 말(절대경로 · 상대경로)을 그대로 쓴다 · 과정은 **순서대로(무엇이 생기고 → 언제 → 어디로)** · 한 번에 하나만 묻는다 · 「제 기억」은 확인하거나 아예 빼고 말한다.
- 사용자는 **글보다 그림**, **「성숙한 곳 따라하기」** — 단 **비교 대상은 우리와 같은 부류(가볍고 · 폴더 브라우저형 · 에이전트 도구)**. 엔진(Unity · Unreal)은 「다 처리해야 해서 무거운」 예외로 친다.
- 결정 안 된 것을 결정처럼 적지 말 것 — 이번에도 한 번 「USER DECISION」으로 미리 적었다가 사용자가 끊어 「LEANING, NOT CONFIRMED」로 고친 뒤 확정 때 바꿨다.

## 이번 세션 결정 (요약 — 정본은 결정 메모, 이유 · 버린 안도 거기)

1. **라이브러리 저장소 자리 = 데몬 crate 안의 따로 된 모듈 + 처음부터 trait**(목록 · 읽기 · 쓰기 — walkdir · serde-saphyr 를 그 뒤에). base 아님(도메인 지식 · 소비자 하나). 다른 crate 가 필요해지거나 로직이 커지면 그때 crate 로 뺀다. 사용자: 「애매하긴 하지만 그렇게 해」 · trait 쓸모 = 시험 때 메모리 가짜로 갈아 끼우기 · 나중에 구현 교체.
2. **목록 = 클라가 요청(pull) 하나뿐.** 슬롯 열기 · 포커스 · 새로고침 · 폴더 이동 때 클라가 요청 → 데몬이 훑어 그 클라에게만 답 · 실행 직전 재확인. **notify(실시간 감지) · 등록/방송 · 보이는 동안 주기 요청 전부 없음.** 사용자 이유: 「화면 보고 있는 동안 바로 보여 줄 필요 없음」 · 원래 이유(직전 핸드오프가 잃었던 것): **사용량 슬롯(토큰 감시기)과 통일** — 필요할 때만 서버에 요청. 원격 · 모바일도 계기가 전부 클라 동작이라 성립. 버린 안: 앱 활성화만 계기(원격에서 성립 안 함) · 보이는 동안 주기 요청(JupyterLab 식 — 필요 없음) · 등록한 동안 감시 + 등록자에게 신호(LSP · Syncthing 식 — 세션이 추천했으나 거부). 조사 = `docs/research/library-folder-change-sync-survey-2026-10-14.md`(codex FIX → 반영).
   - 코드 사실: 사용량 슬롯은 순수 요청/응답이 아니다 — `UsageSubscribe`(연결 구독 집합 통째 교체) + 구독 연결에만 `UsageLimitsUpdated` · `RefreshUsageLimits` 는 Ack 만(`crates/engram-dashboard-protocol/src/messages.rs` 약 360~375행). 라이브러리는 그 구독 모양을 **따르지 않기로** 했다(감지가 없으니 등록이 필요 없다 — 사용자 지적).
3. **프리셋 안의 참조 = 절대경로 — 사용자가 라이브러리로 지정한 폴더 기준 · 범위 제한 없음**(스킬 · 지침 · 설정 · 다른 프리셋). 이유(사용자): 스킬은 여러 프리셋이 나눠 쓴다(여러 대 여러) — 한 프리셋 옆에 붙이면 다른 그룹 프리셋은 둘 데가 없다 · 프리셋 안에 프리셋 · Engram 은 폴더 브라우저라 항목 정체 = 자리. 버린 안: 상대경로(프리셋 파일 기준 — 묶음 안 파일만 가리키는 플러그인 · Compose · Cargo 용) · 이름 참조(Claude Code 플러그인 의존 · Continue `owner/name` — 범위가 명확한 곳에서만 성립, 브라우저에선 같은 이름이 흔하다) · ID 파일(Unity · Godot — 무거운 엔진). 가벼운 브라우저형 선례 = Obsidian(볼트 맨 위 기준 경로 · 앱 안 이름 바꾸기가 링크 고침 — 도움말 원문 확인) · Lightroom · REAPER · Ableton(수집자만). 조사 = `docs/research/preset-asset-reference-base-survey-2026-10-14.md`(추가 · 추가 2 절 포함).
4. **기본 라이브러리 폴더 = 배포판 폴더 아래 `library\`**(`data\` 옆 · 데이터 폴더 안 아님 · 포터블 ADR-0134 식). zip 을 덮어 풀면 남는다(같은 이름만 덮인다 — 사용자 지적으로 세션의 과장 정정). 나중에 배포판에 기본 내용을 `library\` 로 실으면 사용자가 고친 같은 이름 파일이 덮일 수 있음 — 그때 정한다. 개발 빌드의 기본 자리는 논의 안 함(TRD).
5. **첫 판 파일 조작 = 옮기기 · 이름 바꾸기 · 지우기 · 새 폴더 · 새 프리셋.** (2026-10-14 「Engram 안 옮기기 없음 — 다시 잇기 불편하면 나중」을 대체 — 사용자: 원래 「나중」이었지 「없음」 아님.) 이유: 절대경로라 옮기는 순간 프리셋 줄을 고치고, 지우기 전에 「쓰는 프리셋 N개」 경고할 수 있는 건 Engram 뿐. 이름 바꾸기는 모든 항목에(바깥에서 이름을 바꾸면 이름으로 다시 잇기도 못 찾는다 · 구현은 옮기기와 같다). **뺀 것: 새 마크다운 · 새 스킬**(내용 쓰기는 편집기 · 에이전트 일 · 나중에 더해도 문제없음 — 사용자). **에이전트:** 내용 편집은 파일 직접 · 옮기기 · 이름 바꾸기 · 지우기는 `engram` 명령으로(프라이밍에 넣는다). 바깥 옮김은 이름으로 다시 잇기가 받는다. 모양: 트리 부품 제스처(react-arborist 3.9.0 에 onMove · onRename · onDelete · onCreate 있음 — node_modules 타입 확인) → 데몬 명령 → 데몬이 파일 조작 + 절대경로 고침 → 답 → 클라 재요청. 「새로 만들기」 = 폴더 우클릭 「새 폴더 · 새 프리셋」.
6. **지우기 = 항상 OS 휴지통 자동**(모든 항목 · 에이전트 명령도) · 쓰는 프리셋 있으면 먼저 경고 · 데몬 PC 의 휴지통 · 구현(Rust 휴지통 crate 등) = TRD, 미확인.
7. **화면 부품 = Spacedrive 조합, 우리가 조립**: 트리 react-arborist(기존) · 목록 @tanstack/react-table + @tanstack/react-virtual · 박스 다중 선택 react-selecto · 목록→트리 끌어 놓기 @dnd-kit · 슬롯 데이터 받아 오기 TanStack Query(방향 수락 · TRD `/research` + ADR 로 확정). 우리 트리 · 목록 부품 안에 감싼다. 근거: npm 주간(2026-10-15 조회) react-query 약 6,500만 · react-virtual 약 2,800만 · react-table 약 2,400만(비교 react-aria-components 약 490만 · react-arborist 약 65만) · shadcn/ui Data Table 이 "built using TanStack Table"(v9 — 원문 확인). 사용자: 「뭔 라이브러리가 이렇게 많아」 하고 수락. 완제품 파일 관리자 부품은 평가 안 함(사용자가 확인 거절).
8. **(별건) 오픈소스 라이선스 고지 = T-54**(wt1 미커밋 `docs/tracking.md`): 지금은 손으로 — cargo-about(Rust) + generate-license-file(npm)로 `THIRD_PARTY_LICENSES.txt` 한 번 뽑아 커밋 · `scripts/build-release.ps1` manifest · 복사 · 검증에 추가(정확 일치 tripwire — ADR-0100) · 갱신도 손으로 「얘기 나올 때마다」 · 자동화는 나중. **범위 = 의존성 전체 목록**(Windows Terminal · PowerToys `NOTICE.md` · Zed · VS Code 쪽 — 「외부에서 쓰는 것 표시도 해 놓을 겸」). 버린 안 = 에이전트 도구 쪽 최소 표시(Codex CLI `NOTICE` = 들여온 Ratatui 코드만 · herdr = ConPTY 바이너리만 · t3code = libghostty 만 · Goose = `deny.toml` 검사만 · 나머지 없음 — 직접 확인). 개인 프로젝트라 법무 경유 아님(사용자). 조사 = `docs/research/third-party-license-notice-survey-2026-10-15.md`.
   - **이 세션이 띄운 코더는 착수 직전 멈췄다 — 아무 파일도 안 바뀜 · cargo-about 미설치 · 남은 cargo 프로세스 없음(확인).** 사용자가 **다른 세션 에이전트에게 맡겼다(상급 워커가 생성 · 스크립트 반영 중 — 결과 미확인)**. 이 세션은 그 일을 하지 말 것.
   - 사용자에게 설명한 두 설정 파일 내용(`about.toml` = 대상 고르기 ②③④ + 허용 목록 검사 ① · `about.hbs` = 모양만)의 칸 이름 중 `targets` · `private` · 템플릿 칸(`licenses` · `used_by` · `text`)은 **미확인**(수집자 · 기억). `accepted` · `ignore-dev-dependencies` · `ignore-build-dependencies` 는 cargo-about 설정 문서 원문 확인.

## ⚠ T 번호 충돌 (다른 세션 보고 — 사용자 붙여넣기)

- 「라이선스 T-54는 wt1의 미커밋 docs/tracking.md에만 있고, master의 T-54는 전혀 다른 항목(taskkill /T)입니다. wt1의 T-53도 master의 T-53과 번호가 겹칩니다.」
- wt1 의 T-53(서드파티 감싸기 기준 — 직전 세션) · T-54(라이선스 고지 — 이번 세션)는 **아직 커밋 안 됨**. 커밋 · 머지 전에 master 의 마지막 번호를 보고 **다시 매겨야** 한다(`git show master:docs/tracking.md | rg "^### T-"`). 결정 메모 · 보고서 · step-log 가 그 번호를 가리키는지 `rg "T-53|T-54"` 로 같이 고친다. 라이선스 쪽은 다른 세션이 쓴 번호와 맞춘다 — 사용자에게 먼저 묻는다.

## repo 상태 (전부 미커밋 — 사용자가 커밋을 지시하지 않았다)

- 브랜치 `v0.3.3/chore/misc`(HEAD `1cb724d8`). `git status --short` 20줄:
  - 수정: `.claude/handoff/latest.md` · `CLAUDE.md`(직전 세션 — 「참조 구현」 spacedrive 한 줄 · load-bearing → 커밋 전 `/review doc`) · `docs/process/step-log.md`(조사 링크 줄들 — 이번 세션 둘 추가: 폴더 변경 반영 · 참조 경로 기준) · `docs/tracking.md`(T-53 · **T-54 이번 세션** — 번호 충돌 위 참조).
  - 새 파일: `.claude/handoff/attachments/20261012-preset-decisions.md`(결정 정본 · 이번 세션 절 추가) · `20261012-preset-ui-briefing.html`(브리핑 사본 — 이번 세션 안 고침) · history 2개 + 이번 것 · `docs/research/` 12개(이번 세션 신규 3: `library-folder-change-sync-survey-2026-10-14.md` · `preset-asset-reference-base-survey-2026-10-14.md` · `third-party-license-notice-survey-2026-10-15.md`).
- 커밋 형식 `S21: docs(research): …` 등 · 문서만이면 CI 경로 필터 밖(CLAUDE.md 머지 규약).

## 브리핑

- https://claude.ai/artifact/PWJR2yosYDPV2VJUN2SJZe — v25 · 사본 `.claude/handoff/attachments/20261012-preset-ui-briefing.html`.
- **반영 안 된 결정(사용자 「이따가 브리핑 문서에 어떻게 활용할지 써놔」):** ① **화면 부품 쓰임 그림**(어느 부품이 슬롯의 어디를 맡나 — 트리 · 목록 · 박스 선택 · 끌어 놓기 · 데이터 받아 오기, 우리 부품이 감싼다) ② 뒤로/앞으로 버튼 ③ 목록 = 클라 요청(pull) · 감지 없음 ④ 감싸기 구조(데몬 라이브러리 저장소 모듈 + trait) ⑤ 절대경로(지정 폴더 기준) ⑥ 기본 라이브러리 폴더 `library\` ⑦ 첫 판 파일 조작 + 휴지통 + 에이전트는 `engram` 명령. 「확정만 · 그림 위주」 정리판 형식 유지.

## 검증 상태

- 조사 3건: 폴더 변경 반영(medium · 수집 3 · codex 적대 리뷰 FIX → 7건 반영 · 재리뷰 없음 — Unity 「감시 없음」 · JupyterLab 「설정할 때만 폴링」 두 건은 초안 오류였다) · 참조 경로 기준(light + 추가 둘 — 적대 리뷰 없음) · 라이선스 고지(light · 수집 2 — 적대 리뷰 없음). 메인 원문 확인분은 각 보고서 grounding 에 표시.
- **미측정 · 미확인:** 라이브러리 훑기 비용(수천 개) · `DISABLE_AUTOUPDATER` 가 settings `env` 로 먹히는지 · cargo-about `targets` · `private` 칸 · Rust 휴지통 crate · Obsidian 의 이름 겹침 처리 · 이 저장소에 남의 코드를 직접 복사해 들여온 것이 있는지(있으면 저장소에도 표시 필요).

## 실패한 접근 · 주의

- 위 「사용자 응대」 전부.
- codex 리뷰를 bash 에서 `cmd //c "codex exec … \"$(cat prompt)\" …"` 로 부르면 따옴표가 깨져 즉사 — **지시를 파일로 쓰고 `codex exec … - < prompt.txt`**(research `feedback.md` 2회 관측으로 적립).
- WebFetch 요약기는 원문이 아닐 수 있다 — 따옴표 인용은 원문 확인분만.
- 셸 `python` = MS Store 스텁 → `py`.

## 정지 조건

- PRD 확정 · TRD · 구현 진입 · 커밋은 사용자 지시 전 금지. CLAUDE.md 추가 수정은 바깥 개편과 겹치니 먼저 묻는다.
- 라이선스 고지(T-54) 작업은 다른 세션 몫 — 이 세션이 손대지 않는다. T 번호 정리는 사용자에게 먼저 묻는다.
- **핸드오프 · 세션 끊기를 먼저 제안하지 않는다** — 사용자가 정한다.
