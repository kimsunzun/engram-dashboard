# 프론트 트리 · 드래그 · 가상화 · 표 · 마키 — 라이브러리 조사 (클라이언트 단계 자료)

- **상태:** medium · 2026-10-31 · 수집자 1 + 메인 grounding + codex 적대 리뷰(effort high) — 판정 FIX. TRD `docs/process/S25-preset-library/trd.md` §5-1 step 0a(구현 부분별 외부 라이브러리 조사).
- **★클라이언트 단계 자료 — 서버 우선 규칙(G20)★:** 아래 선택은 서버 단계가 끝난 뒤 클라이언트 단계에서 **다시 확인한다**. 지금 값은 잠정이다. 확인 때 돌릴 스파이크 4건은 「공백 · 실측 필요」.
- **계기:** TRD 가 드래그 시스템(B6)에 `@dnd-kit/core`(메모 113), 마키에 `react-selecto 1.26.3`, 가상화에 `@tanstack/react-virtual 3.14.19`, 표에 `@tanstack/react-table 9.3.1` 을 골라 둠. 그 선택이 현재 판 · 이미 잠근 `react-arborist 3.9.0` 과 맞는지 확인.
- **확신도 범례:** 확실 = 1차 출처 + 독립 교차확증(메인 grounding 또는 codex 재확인이 일치) · 가능성 높음 = 단일 1차 출처, 반박 없음 · 불확실 = 추정/미검 · contested = 리뷰가 반증과 함께 반박.

## 결론

메인 결정(MAIN DECISION) 줄은 입력에 없다. 아래는 조사 결론이며 확정 선택이 아니다.

| 필요 | 잠정 선택 | 이유 한 줄 | 차선 |
|---|---|---|---|
| B6 드래그 한 시스템 | **react-dnd + HTML5Backend 유지**(arborist 내장) — 단 ★**아래 보정 적용**★ | arborist 는 react-dnd 만 받는다(확실). dnd-kit 으로는 구동 불가 | `@atlaskit/pragmatic-drag-and-drop 4.0.0` · `@dnd-kit/react`(별도 평가) |
| 가상화(목록) | `@tanstack/react-virtual 3.14.24` | React 19 · MIT · 2026-10-29. TRD 의 3.14.19 와 minor 차이 | `react-window 2.5.2` |
| 표 | `@tanstack/react-table 9.3.1` | TRD 와 일치. v9 API 가 계획과 맞는지는 미검 | — |
| 마키 | **미정 — Selecto 교체 주장은 철회** | 가상화와의 결합을 비교하기 전엔 교체 근거 없음 | `react-selecto 1.26.3`(유지 후보) · `@viselect/vanilla 3.10.1` · `@air/react-drag-to-select 5.0.11` |

### 리뷰가 강제한 보정 (적용됨)

- **「공유 react-dnd manager 하나로 트리 · 목록 · 폴더 행이 다 섬」 → 철회·정정.** arborist 는 drop 타입 `"NODE"` 는 받지만 옮겨질 ID 를 **자기 트리 상태에서** 가져오고, 드래그 원본 hook 은 **출발한 트리만** 초기화한다. manager 를 공유해도 목록→트리 · 트리→트리 이동은 저절로 안 된다. **커스텀 drop 대상/어댑터가 필요하고 양방향을 검증해야 한다.** (리뷰가 코드로 반박 → 스파이크 2)
- **「react-dnd 16 추천」 → 호환 조합을 먼저 못 박아야 한다.** 저장소는 arborist 3.9.0 잠금 → react-dnd 14(`package-lock.json:7196`). arborist 4.0.1 은 react-dnd/backend 16 이 필요하다. 14 와 16 을 섞으면 안 된다. 「3.x 에 머문다」(TRD §1-12) 가 아직 미해결이므로 **기존 AgentList 를 포함한 구체 조합을 정해야 한다.** (스파이크 3)
- **「AgentList 가 WebView2 에서 HTML5 드래그 작동을 입증」 → 과장, 미검증으로 표시.** `AgentListReparent.test.tsx:49` 가 arborist 를 mock 한다. 배선만 증명할 뿐 WebView2 네이티브 드래그는 아니다. 런타임 관측이 따로 없으면 미검.
- **「dnd-kit 부적합」 → 일반화 과다, 범위 축소.** 거부 근거는 legacy `@dnd-kit/core`(포인터 기반 · 단일 문서)뿐이다. 새 `@dnd-kit/react` 는 네이티브 HTML DragSensor 를 갖는다(`packages/dom/src/core/sensors/drag/DragSensor.ts` · dndkit.com 이전 가이드). 따로 평가해야 하며 창 간 드래그는 여전히 미증명.
- **「viselect 가 더 낫다」 → 근거 없음, 철회.** viselect 는 **마운트된 DOM 의 rect** 와 교차를 본다. 가상화로 언마운트된 행은 추가 로직(안정 ID 선택 · 재마운트 · 자동 스크롤)이 필요하다. Selecto 를 갈아끼우기 전에 이 점을 비교해야 한다. (스파이크 4)
- ★**TRD 메모 113(`@dnd-kit/core`) 과 충돌한다**★ — 수집 결과는 dnd-kit 이 arborist 를 구동할 수 없다고 본다(확실, `.d.ts`). 위 보정으로 dnd-kit 전체가 아니라 legacy core 만 탈락. TRD 메모 113 은 클라이언트 단계에서 다시 정해야 한다.

## B6 — 드래그 한 시스템

- **arborist 의 결합(확실, 4.0.1 `.d.ts` 확인):** `getDndProviderProps` 는 react-dnd manager(`dndManager`) 또는 backend(`dndBackend`)만 받는다. 내부 `useDrag`/`useDrop` 이 react-dnd 다. dnd-kit 을 쓰려면 `disableDrag` + `disableDrop` 후 트리 drop 대상을 직접 다시 만들어야 한다.
- **Tauri 맞음(확실 — 코드 + 리뷰 재확인):** `src-tauri/tauri.conf.json:21` `dragDropEnabled false`, `webview_env.rs:196` `disable_drag_drop_handler`. 이게 꺼져 있어야 Windows 에서 HTML5 DnD 가 발화한다(Tauri 설정 레퍼런스로 codex 재확인).
- **react-dnd 상태:** 16.0.1 · 2022-06-25 · peer react >=16.14 · React 19 명시 없음(불확실). 저장소는 이미 React 19 위에서 react-dnd 14 를 arborist 3.9.0 아래 돌리고 있다. 휴면 상태. 정렬(sortable) 도우미 없음.
- **arborist 4.x:** `dndManager` · `dndRootElement` 노출(재확인). 전체 상호운용은 증명 안 됨.
- **창 간 팝아웃 드래그:** dnd-kit(legacy)은 포인터 · 단일 문서라 불가(가능성 높음). HTML5 DnD 는 OS 드래그 세션이라 될 수도 있으나 **불확실 · 미시험.** WebView2 창 사이 커스텀 MIME `dataTransfer` 동작 미상. 위험: `dragDropEnabled false` 일 때 원본이 드래그 끝까지 다시 그려지지 않는다는 버그 보고(Tauri 이슈 #9445 · #13761 스니펫 — 불확실).

| 이름 | 판 | 날짜 | 라이선스 | 이미 잠금? | 맞음 | 확신도 | 출처 |
|---|---|---|---|---|---|---|---|
| react-dnd (+ HTML5Backend) | 16.0.1 (잠금은 14) | 2022-06-25 | — (입력에 없음) | 14 잠금 (`package-lock.json:7196`) | arborist 내장. 공유 manager 만으론 목록↔트리 이동 불가 | 판 · 휴면 = 확실(npm + 메인) / React 19 = 불확실 | npm registry |
| @atlaskit/pragmatic-drag-and-drop | 4.0.0 | 2026-09-24 | Apache-2.0 | 아니오 | 네이티브 HTML5 · hitbox 패키지. arborist 에 못 붙임. react-dnd 와 공존 미상 | 가능성 높음 | npm registry |
| @dnd-kit/core (legacy) | 6.3.1 | 2024-12 | — | 아니오 | 포인터 기반 · 단일 문서 · arborist 구동 불가 | 확실(구동 불가) | npm registry · `.d.ts` |
| @dnd-kit/react (신) | 0.5.x | — | — | 아니오 | 네이티브 HTML DragSensor 있음. 평가 안 됨 | 불확실(미평가) | dndkit.com 이전 가이드 · `DragSensor.ts` (리뷰 인용) |

## 가상화

- 트리는 arborist 가 가상화한다(react-window 1.x). 목록은 별도.
- `@tanstack/react-virtual` 3.14.24 · 2026-10-29 · MIT · React 19 → TRD 의 3.14.19 와 minor 차이(가능성 높음 — npm 단일 출처, 반박 없음). 잠금 여부는 입력에 없음.
- 차선 `react-window 2.5.2`.

## 표

- `@tanstack/react-table` 9.3.1 · 2026-10-12 · MIT · react >=18 → TRD 와 일치. **v9 API 가 계획과 맞는지는 미검**(불확실).

## 마키 (영역 드래그 선택)

| 이름 | 판 | 날짜 | 라이선스 | 이미 잠금? | 맞음 | 확신도 | 출처 |
|---|---|---|---|---|---|---|---|
| react-selecto (TRD 선택) | 1.26.3 | 2023-12-03 | — | 입력에 없음 | 마지막 갱신 2년 전. React 19 명시 없음 | 판 · 날짜 = 확실(메인 확인) / React 19 = 불확실 | npm registry |
| @viselect/vanilla | 3.10.1 | 2026-09-04 | MIT | 아니오 | 프레임워크 무관. 마운트된 DOM rect 만 교차 → 가상 행 처리 별도 필요 | 가상화 한계 = 확실(리뷰가 소스 인용) / 「더 낫다」 = contested | npm · `packages/vanilla/src/index.ts` |
| @air/react-drag-to-select | 5.0.11 | 2026-10-09 | — | 아니오 | React 16–19 | 가능성 높음 | npm registry |

- 중재 필요: 행 위에서 누르면 드래그, 빈 곳에서 누르면 마키.

## react-arborist

- 4.0.1 · 2026-10-11 · peer react-dnd ^16 · react >=18 · Node >=22.12 · **ESM only** — TRD §1-12 「3.x 유지」의 사실 근거는 확인됨(codex 재확인 = 4 매니페스트 일치). 4 로 올릴 이득은 미상(changelog 미확인). 현재 `package.json` 은 `^3.9.0`(메인 확인).

## 적대 리뷰 결과 (codex · effort high · FIX)

| # | 수준 | 지적 | 처리 |
|---|---|---|---|
| 1 | HIGH | 공유 manager 로 트리 · 목록 · 폴더 행이 다 선다는 주장 — arborist 는 이동 ID 를 자기 트리 상태에서 가져오고 원본 hook 은 출발 트리만 초기화(`src/dnd/drop-hook.ts` · `interfaces/tree-api.ts`) | 반영 — 주장 철회, 커스텀 어댑터 + 양방향 검증으로 정정 · 스파이크 2 |
| 2 | MED | react-dnd 16 추천인데 「3.x 유지」 미해결 — 잠금은 14 · arborist 4 는 16 | 반영 — 호환 조합 확정을 선행 과제로 · 스파이크 3 |
| 3 | MED | viselect 「더 낫다」 근거 없음 · 가상화 누락 | 반영 — 교체 주장 철회, 가상화 결합 비교로 · 스파이크 4 |
| 4 | MED | dnd-kit 거부가 legacy core 에만 해당 — 새 `@dnd-kit/react` 는 네이티브 DragSensor | 반영 — 범위 축소 · 별도 평가 필요(남김) · 창 간은 여전히 미증명 |
| 5 | MED | AgentList 가 WebView2 네이티브 드래그를 입증한다는 과장 — 테스트가 arborist 를 mock(`AgentListReparent.test.tsx:49`) | 반영 — 미검증으로 표시 |
| — | — | 재확인: Tauri `dragDropEnabled` 요건 확인 · arborist 4 매니페스트 확인 · `dndManager` 존재, 전체 상호운용 미증명 | 반영 — 위 본문 |

## 공백 · 실측 필요 (클라이언트 단계 스파이크)

1. **창 간 드래그:** Tauri 창 둘(`dragDropEnabled false`)에서 HTML5 DnD 로 팝아웃 창 사이 드래그가 되는가 · 커스텀 MIME `dataTransfer` 가 넘어가는가 · 원본이 드래그 끝까지 다시 그려지지 않는 버그가 재현되는가.
2. **공유 react-dnd manager + 커스텀 어댑터:** 트리 · 목록 · 폴더 행이 한 manager 를 공유할 때 목록→트리 · 트리→트리 이동이 양방향으로 서는가(arborist 의 ID 출처 문제를 어댑터로 우회).
3. **호환 조합:** react-dnd 14 vs 16 / arborist 3 vs 4 중 하나를 기존 AgentList 포함해 고르고 React 19 에서 실제로 도는지 확인(arborist 4 의 이득은 changelog 확인 포함).
4. **마키 vs 가상화:** Selecto · viselect · `@air/react-drag-to-select` 를 가상 목록에서 비교(언마운트 행 선택 · 안정 ID · 자동 스크롤 · 행 드래그와 중재).
- 그 밖: `@dnd-kit/react` 0.5.x 미평가 · react-dnd + pragmatic 공존 미상 · react-table v9 API 대 계획 미검 · 패키지 크기는 unpacked 기준 · AgentList 의 WebView2 실측 부재.

## 출처

- npm registry(`npm view` · `npm pack react-arborist@4.0.1` 의 `.d.ts`): react-arborist · react-dnd · @dnd-kit/core · @atlaskit/pragmatic-drag-and-drop · @tanstack/react-virtual · @tanstack/react-table · react-selecto · @viselect/vanilla · @air/react-drag-to-select · react-window
- 코드: `src-tauri/tauri.conf.json:21` · `src-tauri/src/webview_env.rs:196` · `package.json`(react-arborist ^3.9.0) · `package-lock.json:7196` · `AgentListReparent.test.tsx:49`
- 리뷰 인용 소스: react-arborist `src/dnd/drop-hook.ts` · `interfaces/tree-api.ts` · viselect `packages/vanilla/src/index.ts` · dnd-kit `packages/dom/src/core/sensors/drag/DragSensor.ts` · dndkit.com 이전 가이드 · Tauri 설정 레퍼런스(tauri.app)
- 스니펫(미정독): Tauri 이슈 #9445 · #13761 · dev.to puckeditor top-5 dnd · pkgpulse 2026 가이드
