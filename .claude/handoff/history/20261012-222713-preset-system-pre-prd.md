# 핸드오프: 스킬 프리셋 시스템 — PRD 이전 구상 · UI 러프 브리핑(v12) 진행 중

## 한 줄 상태 · 다음 첫 액션

- **상태:** 「지시서 · 스킬 · 설정을 조합한 프리셋을 claude · codex 에이전트에 똑같이 붙이는」 기능의 PRD 이전 구상 단계. 주입 가능성은 실측으로 확정했고, 사용자 조작 화면을 러프 HTML 브리핑으로 다듬는 중.
- **다음 첫 액션:** 사용자가 브리핑 v12(https://claude.ai/artifact/PWJR2yosYDPV2VJUN2SJZe · 사본 `.claude/handoff/attachments/20261012-preset-ui-briefing.html`)의 **화면 2-1 · 2-1b(담는 세 길 · 두 방향 팝오버)**를 확인하려던 참이다 — 그 의견부터 받는다. 그 뒤 남은 선택지(아래 「미결」)를 하나씩.
- 결정 누적 메모(영어 · 정본) = `.claude/handoff/attachments/20261012-preset-decisions.md` — **먼저 Read**. 아래는 요약.

## repo 상태 (미커밋 — 사용자가 커밋을 지시하지 않았다)

- 브랜치 `v0.3.3/chore/misc`(clean 에서 시작). 바뀐 것:
  - `docs/research/external-skill-dir-loading-claude-codex-2026-10-10.md`(신규) — 주입 · 이어받기 · 압축 · 캐시 실측 전부.
  - `docs/research/agent-preset-components-survey-2026-10-11.md`(신규) — 부품 서베이 + UI 패턴 · 소스 매칭 · 모델 목록 · 담기 · 태그 · 스냅샷 동기화 덧붙임.
  - `docs/process/step-log.md` — 맨 끝 조사 항목 한 줄 + 하위 줄(고아 방지 링크).
  - `.claude/handoff/attachments/20261012-*` 두 파일(결정 메모 · 브리핑 HTML).
- 문서만이라 CI 경로 필터 밖. 커밋할 때 `S21: docs(research): …` 형식 · 머지 규약은 CLAUDE.md.

## 확정된 것 (실측 근거 = 위 research 문서)

- **claude(터미널 · JSON 둘 다):** `--append-system-prompt-file` + **`--system-prompt-snapshot off`** 면 이어받을 때도 새 md 반영(기본 on 은 첫 요청 사본을 계속 씀 — 실측). 캐시 손해 없음(md 그대로면 전부 적중 · 바꾼 그 한 번만 md 이후 재기록 — 실측). CLAUDE.md · 스킬 목록 등 「첨부」는 이어받기에 다시 읽힘(실측). 네이티브 스킬 = `--add-dir`(`<dir>/.claude/skills`) · `--plugin-dir`. 바깥 파일 읽기는 기본 거부 → `--settings` 의 `permissions.additionalDirectories`.
- **codex JSON(app-server 0.156.1):** 새 대화 `developerInstructions` 됨 · **이어받기의 `developerInstructions` · `config.developer_instructions` 는 말없이 버림**(실측 · 상류 버그 #19045 open) → 대신 `turn/start.additionalContext{kind:"application"}` 또는 `thread/inject_items`(실험 API · 덧붙이기만). 스킬 = `skills/extraRoots/set`(프로세스마다 다시). **압축 때 우리가 넣은 developer 메시지가 전부 빠진다**(실측) → 압축 뒤 재주입(같은 값은 중복 제거되니 rev 표시 또는 inject_items). ★곁가지: **지금 우리 codex JSON 프라이밍(ADR-0215)도 압축 뒤 빠진다** — 기존 결함(이어받은 프로세스에서 쟀고 같은 프로세스 압축은 미측).
- **codex 터미널:** 이어받기 `-c developer_instructions` 무시 · 스킬 폴더를 더하는 인자 · 설정 · 환경변수 없음 · AGENTS.md(작업 폴더)는 이어받기에 다시 읽힘(실측) · `AGENTS.override.md` 가 AGENTS.md 를 가림 · 폴백 이름은 AGENTS.md 없을 때만.
- **모델 목록:** claude = JSON 통로에 첫 메시지 전 `{"type":"control_request","request_id":"…","request":{"subtype":"initialize"}}` → models + 모델별 강도(실측 · 응답에 계정 정보 있음 — 로그 금지). codex = `model/list`.
- **피어:** orca · herdr 는 시스템 프롬프트 주입을 안 한다(orca = 작업 지시를 사용자 차례로 친다) — 선례 없음.

## 사용자 결정 (요약 — 정본은 결정 메모)

- 부품 = 지시서 · 스킬 · 설정 · 워크스페이스. **모든 부품은 한 칸짜리 프리셋**, 프리셋은 이름 있고 중첩 가능, 에이전트 = 깡통 + 프리셋 여러 개. **흐름 한 방향**(부품은 프리셋에서 담고 에이전트는 프리셋만 고름).
- 설정 프리셋이 백엔드 + 모델을 고른다(둘이면 오류). 설정 = 공통 + 백엔드별(+원문 칸 — 사용자의 「각자 스크립트」 해석은 미확정).
- 충돌 = 같은 자리 두 부품 → 오류. 순환 오류 · 같은 프리셋 두 경로 = 하나로. 네이티브 발견(~/.claude 등)은 그대로 두고 중복 검사 · 문서 안내 없음.
- 동적 주입(a): 시작 · 이어받기 · 압축 뒤 · 프리셋 변경. 부품 변경은 다음 실행 때 반영. 프라이밍 = 기본 켜진 부품(끌 수 있음 + 경고).
- 워크스페이스 없으면 실행 때 폴더 묻기(「Engram 폴더 자동 생성」 버튼) · CLI 경로 없으면 Engram 폴더 · 에이전트 폴더 위치는 설정(바꾸면 새 에이전트만).
- codex 터미널: Engram 폴더 = 지원(파일 깔기) · 사용자 폴더 = 실행 때 「파일 만들까요?」 묻기(git 숨김은 사용자 몫) · 프라이밍만은 지금처럼 `-c`.
- 크기 한도(codex 32KiB · 스킬 목록): 매 실행 계산해 `-c` 로 올림 · 크기 바뀔 때만 알림.
- 실측 시험 레인 = 경로 또는 CLI 버전이 바뀔 때만 — **qa 스킬 `feedback.md` 에 한 행 올림**(2026-10-12 · 미반영).
- 라이브러리 = 사용자가 등록한 소스 폴더 여러 개(복사 없이 훑기) · 내부 고유 id · 프리셋도 소스 폴더에 파일(git 공유) · import/export 는 나중.
- UI: 라이브러리 슬롯 = 왼쪽 소스 폴더(체크박스 켜고 끄기) · 가운데 목록(검색 + 칩 즐겨찾기/쓰는 중/최근 + 종류) · 오른쪽 상세(버튼 없음). 더블클릭 = 기본 연결 프로그램 · 우클릭 = 메뉴. 고치기는 에이전트 몫(CLI 로 경로). 라이브러리와 프리셋 편집 = **따로 된 슬롯**(여러 개 가능 · 데이터는 데몬 하나 · 저장 버튼 없이 즉시 반영). 전역 동작 = 탭 바 오른쪽 버튼(세부 나중). 연결 그래프(언리얼 레퍼런스 뷰어식)는 나중.
- **기존 「프리셋」(작업 폴더 북마크 · ADR-0061 · `presets.json`)은 걷어낸다 — 옮기지 않는다.** TRD 때 ADR-0061/0070 폐기 ADR 필요.

## 미결 (사용자에게 물을 것 — 하나씩)

1. 브리핑 2-1 · 2-1b 의견(담기 ① 팝오버 기본 · ②③ 지름길은 나중 · 즉시 반영 + 되돌리기).
2. 브리핑 선택지 2-A(최종 구성 옆 패널) · 2-B(빈 것 + 복제) · 2-C(설정 백엔드별 = 탭 + 원문 칸) · 3-A(붙이는 곳) · 4-A(모델 선택 결과 저장 위치) · T-1(태그 = 중첩 `a/b`) · T-2(태그 저장 = 파일 안).
3. 소스 매칭: 폴더 안 id 표식(B 추천) vs PC 설정 이름(A) — 조사가 기억 기반이라 결정 전에 문서 확인 필요.
4. 「빠른 생성」(폴더 + 백엔드만)과 인라인 부품 — 사용자가 「나중에 추가로 얘기」.
5. 프리셋 목록 push 에 오래된 갱신 거르는 번호 붙이기 · 파일 감지(지금 코드에 없음) — TRD 감.

## 검증 상태

- 실측(스크래치 · 원본은 세션 스크래치라 사라짐 — 결과는 research 문서에 기록): claude 이어받기 사본 · CLAUDE.md 재읽기 · 캐시 · 플러그인/추가 폴더 스킬 · 바깥 읽기 거부 · codex 지시문/스킬 새 대화 · 이어받기 · 압축 · additionalContext · inject_items · AGENTS.md · 폴백 이름 우선순위 · claude initialize 모델 목록.
- **미측:** codex 대화형 TUI(전부 `codex exec` 로 대신) · codex Engram 폴더 지시서 파일 방식(이어받기 · 읽는 시점 · 터미널 압축) · claude 대화형 `/resume` · claude 압축 뒤 시스템 프롬프트 · 같은 프로세스 codex 압축 뒤 developerInstructions · 「바뀌었다」 문구 없는 지시 충돌(n=1).
- **약한 근거(미검증 표시함):** UI 서베이 화면 캡처 없음 · 소스 매칭 관행 · 태그 · 담기 팝오버 세부는 조사자 기억.

## 실패한 접근 · 주의

- `model_instructions_file` 은 codex 기본 지시문을 **대체**한다 — 프리셋 통로로 쓰지 말 것. `baseInstructions` 도 같음.
- codex 터미널 명령줄 `-c` 값에 따옴표 · `%` · `\` 불가(cmd 8191) — 경로 · 배열 값 넘기기 어렵다.
- 브리핑 HTML 은 미리보기 점검 도구가 없어 렌더를 직접 못 봤다.
- 셸에서 `cat >> 파일` 을 heredoc 없이 쓰면 stdin 대기로 멈춘다(이번에 한 번 걸림).

## 정지 조건

- PRD 확정 · TRD · 구현 진입은 사용자 결정 전 금지(CLAUDE.md 개발 스텝). 커밋도 사용자 지시가 있을 때만.
