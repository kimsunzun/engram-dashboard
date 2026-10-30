# 핸드오프: 스킬 프리셋 — 라이브러리 슬롯 · 담기 · 설정 에셋 · 부품 결정(PRD 이전) · 남은 질문 둘

## 한 줄 상태 · 다음 첫 액션

- **상태:** 프리셋 기능 PRD 이전 구상. 이번 세션에 라이브러리 슬롯 · 프리셋 편집 · 담기 · 설정 에셋 · 데이터 소유 · 서버 쪽 라이브러리를 거의 다 정했다. 브리핑은 「확정만 · 그림 위주」로 정리판(v25).
- **다음 첫 액션 — 사용자에게 하나씩 묻는다(라이브러리 결정 마무리):**
  1. **화면 부품 조합** — 트리 = react-arborist(이미 씀) · 목록 = TanStack Table + TanStack Virtual · 다중 선택 = react-selecto(가장 가까운 피어 Spacedrive 조합 · 매니페스트 확인)로 정할지. 대안 = 목록을 React Aria Table(정렬 · 다중 선택 · 키보드 내장 — 단 파일 브라우저에 쓴 성숙한 피어는 못 찾음). 사용자는 「성숙한 곳 따라하기」를 원한다.
  2. **「슬롯이 뜰 때 데이터 받아 오기」 공용화** — TanStack Query(Spacedrive 사용) 방향을 메모로만 둘지. 사용량 슬롯이 이미 같은 모양(`src/components/slot/UsageSlot.tsx` 마운트 `pull()` · `refresh()` · `onUsageLimitsUpdated`). 굵은 결정 → TRD `/research` + ADR.
- 그 뒤 = 브리핑 「열린 것」(섹션 10) 목록을 하나씩.
- **결정 정본 = `.claude/handoff/attachments/20261012-preset-decisions.md`(영어 · 맨 아래가 최신 — 2026-10-14 절들). 먼저 Read.**

## 이번 세션 결정 (요약 — 정본은 결정 메모)

- **라이브러리 = 폴더 하나**(여러 소스 등록 · PC 사이 맞추기 · 바깥 링크 폐기 · 다중 폴더는 나중). 남에게 줄 땐 폴더째(프리셋도 그 안 파일 = **에셋**). 받은 폴더는 하위 폴더로.
- **목록:** 탐색기식 — 고른 폴더 **바로 안의 것만** · 하위 폴더는 📁 줄 · 검색은 하위까지 · 머리 클릭 정렬(폴더 먼저 · ★ 칸 제외) · 칸 = 종류 · 이름 · 쓰는 곳. 상세 칸 「쓰는 곳」 이름 = 링크(그 프리셋으로 이동 · 선택).
- **즐겨찾기 묶음**(태그 대신) — 이 PC(클라)에만 · 한 단계 · ★ = **대상 묶음**에 넣고 빼기(대상 하나 · 묶음 더블클릭/우클릭 = 대상 · 목록 위 「★ = 대상」 칩) · 프리셋은 공유(데몬), 즐겨찾기는 개인(클라).
- **다시 잇기:** Engram 안 옮기기 · 이름 바꾸기 기능 **없음**. 밖에서 옮기면 이름으로 저절로 다시 잇고, 안 되면 「찾을 수 없음 · 다시 잇기」(가리키는 쪽 — 프리셋 줄 · 에이전트 칩 · 묶음에만 표시, 폴더 목록엔 없음). 리다이렉터 없음.
- **뒤로/앞으로:** 자리가 바뀔 때만 기록(폴더 · 링크 · 묶음 보기) · 같은 폴더 안 클릭은 마지막 선택만 갱신 · 편집은 기록 안 함 · 슬롯마다 따로. (사용자 동의 — 시안 반영은 아직)
- **프리셋 편집 = 라이브러리 슬롯 오른쪽 칸**(프리셋을 누르면 상세 칸이 편집 칸 · 잠금 없음). **담기:** 메인 = 「← 선택한 것 담기」(다른 라이브러리 슬롯 중 마지막으로 만진 것의 선택 · 고른 줄 뒤 / 없으면 맨 끝) · 보조 = 「+ 담기」 검색 팝오버(검색만 · 이미 담긴 것은 흐리게 「담김」 · 묶음 칩 없음). 라이브러리 우클릭 「편집 중인 프리셋에 담기」 = 거부.
- **설정 에셋 = YAML** — 정식 칸은 `backend` · `model` 뿐, 나머지는 백엔드별 원문(`claude.settings` = claude 설정 JSON 형식 → `--settings` · `codex.config` = `-c` 목록) · 담은 순서대로 이어 붙음 · Engram 은 원문 해석 안 함. 의도 → 벤더 설정 번역은 미래.
- **기본 프리셋**(내장 · 에이전트 만들 때 무조건 붙음 · 뗄 수 있음) = 프라이밍 + 기본 설정(claude `{"env":{"DISABLE_AUTOUPDATER":"1"}}` — 다른 세션 조사 `docs/research/child-process-shutdown-peers-2026-10-14.md` §6). 지금은 다른 세션이 `backend/claude` 에 박고, 프리셋이 오면 옮긴다. **settings `env` 로 먹히는지 미측정.**
- **데이터 소유:** 데이터(훑은 목록 · 프리셋 내용 · 쓰는 곳 · 다시 잇기)는 데몬 · 클라는 보기 상태(폴더 · 선택 · 정렬 · 뒤로/앞으로) + 개인 데이터만. 라이브러리 목록 = **필요할 때 받는 보고서(pull)** — 슬롯 열기 · 포커스 · 새로고침 · 폴더 이동 때 요청, 데몬이 훑어 답함 · 실행 직전 재확인. **실시간 감지(notify) 안 씀.**
- **같은 PC 제약**(클라 · 데몬) · 원격은 나중(화면은 기존 프리셋 붙이기만 · 편집은 LLM 명령) · 메인 · 서브 데몬 구상(나중). 화면은 OS 창을 직접 부르지 않고 「폴더 골라 줘 · 파일 열어 줘」 요청 하나를 거친다.
- **서버 쪽 라이브러리:** walkdir(폴더 훑기) + serde-saphyr(YAML — `serde_yaml` 은 `+deprecated`) · 비교 · 다시 잇기 = 직접. **감싸기 기준(사용자 동의):** 바깥과 닿나 · 바뀔 가능성 · 여러 곳에 퍼지나 중 하나라도 예 → **목적** 인터페이스 뒤(라이브러리 하나당 래퍼 X) · std · serde · tokio · React 는 안 감쌈. 적용 = 데몬 「라이브러리 저장소」(목록 · 읽기 · 쓰기 — walkdir · serde-saphyr · std::fs 를 이것 하나로) · 화면 부품 라이브러리는 따로 감싸지 않고 트리 · 목록 부품 안에서.
- **트래킹:** T-53(감싸기 기준 → CLAUDE.md 반영 · 기존 코드 점검) 추가 · tracking 머리에 「트리거 대기」 꺼내는 법. 사용자가 **「트래킹에서 트리거 대기 꺼내」**라고 하면 `rg -n "트리거 대기" docs/tracking.md`(지금 T-37 · T-53).

## repo 상태 (미커밋 — 사용자가 커밋을 지시하지 않았다)

- 브랜치 `v0.3.3/chore/misc`. 바뀐 것:
  - `docs/research/` 신규 9개(10-10 · 10-11 두 개 + 이번 세션 7개: `library-multi-root-display` · `favorite-star-vs-collections` · `settings-preset-file-format` · `library-to-bundle-add-patterns` · `browser-back-forward-history` · `library-browser-building-blocks` · `third-party-wrapping-criteria`).
  - `docs/process/step-log.md` — 조사 항목 아래 후속 서베이 링크 줄들(고아 방지).
  - `docs/tracking.md` — T-53 · 머리 한 줄.
  - **`CLAUDE.md` — 「참조 구현 › 받아 둔 클론」에 spacedrive 한 줄(load-bearing → 커밋 전 `/review doc` 필수).** 바깥에서 CLAUDE.md 를 개편할 예정이라 그 밖엔 손대지 않았다.
  - `.claude/handoff/attachments/` 결정 메모 · 브리핑 HTML · history 파일들.
- **repo 밖:** `I:\Engram_Workspace\opensource\spacedrive` 클론(`4d41a379` · blobless) + 그 폴더 `README.md` 「클론별 주의」 한 줄.
- 커밋 형식 `S21: docs(research): …` · 문서 경로만이면 CI 경로 필터 밖(머지 규약 = CLAUDE.md).

## 브리핑

- https://claude.ai/artifact/PWJR2yosYDPV2VJUN2SJZe — **v25** · 사본 `.claude/handoff/attachments/20261012-preset-ui-briefing.html`(정리판 — 섹션 1~11, 확정/미정 표시). v22 백업은 세션 스크래치(사라짐 — artifact 버전 기록이 대신).
- **시안에 아직 안 들어간 결정:** 뒤로/앞으로 버튼(라이브러리 슬롯 머리) · pull 방식(그림 변화 없음) · 감싸기 구조. 다음 갱신 때 함께.

## 검증 상태

- 조사 7건(medium 5 · light 2) — medium 은 전부 codex 적대 리뷰(판정 BLOCK 2 · FIX 3) 반영 · 반영 후 재리뷰 없음. 메인이 원문 · 소스 · npm/crates.io 실측으로 대조한 것은 각 보고서 grounding 표에 표시.
- **미측정 · 미확인:** DISABLE_AUTOUPDATER 가 settings `env` 로 먹히는지 · React Aria Table 의 Shift/Ctrl 범위 선택 · Spacedrive 트리 부품 · react-arborist 4.x 요구(Node ≥22.12 · ESM) · TanStack Table v9 변경점 · 라이브러리 다시 훑기 비용(수천 개 — 실측 없음). 브리핑 렌더는 사용자 스크린샷 한 번(v16)으로만 봤다.

## 실패한 접근 · 주의

- **사용자 확인 전에 앞서 나가지 말 것** — 이번 세션에 결정 안 된 것을 확정처럼 정리하거나 핸드오프로 먼저 넘어가 두 번 제지당했다(「자꾸 먼저 나가지 말고」 · 「아직 다 결정하고 핸드오프하라니깐」). 하나 묻고 답을 받고 다음.
- 사용자는 **글보다 그림**(브리핑은 그림 + 짧은 번호 설명), **「성숙한 곳 따라하기」**를 원한다. 내 기억 기반 서술은 반드시 확인 후.
- `notify` 8.2.0 은 Windows 버퍼 넘침 때 `Rescan` 을 안 낸다(그 처리는 `main` 에만 — 소스 대조). 실시간 감지를 되살리면 이 점부터.
- 셸에서 `python` 은 MS Store 스텁 — **`py`** 를 쓰고, 출력에 한글이 있으면 `PYTHONIOENCODING=utf-8`.
- WebFetch 요약기가 페이지에 없는 조직 문구(「우리 프로젝트(일곱 개의 대죄…)」)를 덧붙인 적이 있다 — 요약 꼬리는 원문이 아닐 수 있다. 조사 중 Zed 문서 요약에서 정체성 재정의 문구 의심 보고 1건(따르지 않음 · 사용자에게 보고함).

## 정지 조건

- PRD 확정 · TRD · 구현 진입 · 커밋은 사용자 지시 전 금지(CLAUDE.md 개발 스텝). CLAUDE.md 를 더 고치는 것은 바깥 개편과 겹치니 사용자에게 먼저.
