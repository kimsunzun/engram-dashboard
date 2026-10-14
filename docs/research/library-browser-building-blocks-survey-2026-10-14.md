# 라이브러리 슬롯(폴더 브라우저)을 무엇 위에 얹나 — 부품 서베이

- **상태:** 적대 리뷰(BLOCK) 반영본 · 결정 대기(프리셋 PRD 이전 구상 — 구현 갈림길은 TRD 에서 확정)
- **날짜:** 2026-10-14
- **방법:** research medium · 설계-결정 모드. 갈래 3(① 이 저장소에 이미 있는 것 ② React 트리 · 목록 부품 ③ Rust 폴더 훑기 · 변경 감지 crate) · 메인 grounding(npm 레지스트리 실측 · 코드 확인) · cross-family 적대 리뷰 1회(아래).
- **확신도 범례:** 확실 = 원문/실측 + 교차확증 · 가능성 높음 = 단일 원문/실측 · 불확실 = 기억.
- **제약(사용자 결정 2026-10-14):** 데이터(라이브러리 훑기 · 프리셋 내용 · 쓰는 곳 · 다시 잇기)는 데몬이 갖고, 클라는 받아 그리기만 한다(보기 상태 = 폴더 · 선택 · 정렬 · 뒤로/앞으로 · 개인 묶음은 클라). 클라는 디스크를 직접 읽지 않는다. 스택 = React 19 · Tailwind v4 + shadcn/ui · Zustand · Tauri v2 · Windows 우선.

## 결론 (위층)

1. **이 저장소에 이미 있는 것(코드 확인).**
   - 트리: `react-arborist ^3.9.0` 이 의존성에 있고 `src/components/agent/AgentList.tsx:588-599` 에서 통제형 데이터 · 사용자 행 그리기 · 끌어 옮기기 · 우클릭으로 쓴다(다중 선택은 꺼 둠). 감싼 재사용 부품은 없다.
   - 목록 통째 보내기 길: `PresetListUpdated`(데몬 `crates/engram-dashboard-daemon/src/connection_core.rs` `broadcast_preset_list` → 셸 `src-tauri/src/daemon_client/events.rs` → 프론트 스토어). 걷어 낼 옛 「프리셋(작업 폴더 북마크)」의 길이지만 모양을 그대로 따라가면 된다.
   - 폴더 고르기 창: `@tauri-apps/plugin-dialog` 연결됨. 파일 열기 · 탐색기 열기: `@tauri-apps/plugin-opener` 는 있으나 권한이 URL 두 개로 묶여 있어 권한 설정을 바꿔야 한다.
   - **없음:** 칸 정렬 · 다중 선택 목록 · 뒤로/앞으로 · 폴더 훑기/변경 감지 crate(`walkdir` · `notify` 0건).
2. **데몬(Rust): `walkdir` + `notify` 8.x — 이벤트는 「바뀌었다」 신호로만 쓰고, 이벤트 없이도 도는 다시 훑기를 안전망으로 둔다.** 이벤트를 0.2~0.5초 모았다가 라이브러리를 다시 훑고 이전 목록과 비교한다(수천 개면 다시 훑기는 싸다). 윈도우 `ReadDirectoryChangesW` 버퍼는 고정 16 KiB 다. ★**`notify` 8.2.0 은 버퍼가 넘쳐도 「다시 훑어라」(`Rescan`) 신호를 내지 않는다**★ — 그 처리(`ERROR_NOTIFY_ENUM_DIR` · 0바이트 완료 → `Rescan`)는 `main`(9.0 RC 계열)에만 있다(메인이 두 태그 소스를 직접 대조 — 초안의 오류 · 리뷰 반증). 그래서 8.2 를 쓰면 넘친 변경은 **조용히 사라진다** → 슬롯을 열 때 · 포커스를 얻을 때 다시 훑기 + 주기적 다시 훑기를 안전망으로 둔다(9.0 정식이 나오면 `Rescan` 처리로 바꾼다). 이름 바꾸기 · 옮기기 짝 맞추기는 이벤트에 기대지 말고 우리 비교(파일 신원 · 이름)로 한다. 그 밖의 함정(리뷰 지적 · 메인 미확인): 폴더째 이름을 바꾸면 경로가 불완전하게 올 수 있음 · 감시 중인 루트 폴더 자체를 지우면 부모를 감시해야 알 수 있음 · OneDrive 「온라인 전용」 파일 · 윈도우 긴 경로. **가능성 높음.**
3. **화면(React) — 두 갈래.** 트리는 이미 쓰는 react-arborist(3.9 유지 — 4.x 는 `react-dnd` ^16 peer 에 더해 Node ≥22.12 · ESM 전용이라는 리뷰 지적 · 메인 미확인).
   - **(가) React Aria Components(Adobe · Apache-2.0 · 1.22.1 · React 19 peer — 실측):** `Table` 이 머리 클릭 정렬(`allowsSorting` · `onSortChange`) · 다중 선택(`selectionMode`) · 키보드 이동 · 행 동작(`onRowAction` — 더블클릭 · Enter 자리)을 준다(원문). 스타일은 없고 Tailwind 예제가 있다. `GridList` · `Tree` 도 같은 계열. **초안이 「탐색기식 다중 선택 · 키보드는 어느 라이브러리도 안 준다」고 한 것은 틀렸다(리뷰 반증)** — Shift · Ctrl 범위 선택이 문서에 명시돼 있진 않았다(그 페이지 요약 기준 — 미확인).
   - **(나) TanStack Table v9 + Virtual(MIT · headless):** 칸 · 정렬 · 선택 상태 · 통제형 데이터. 키보드 · 범위 선택은 직접. 리뷰에 따르면 shadcn data-table 안내는 이미 v9 기준이고 v9 는 `useTable` · 기능 등록 방식 등 v8 과 API 가 다르다(메인 미확인).
   - 어느 쪽이든 **뒤로/앞으로 · ★ · 쓰는 곳 · 찾을 수 없음 줄은 직접 짠다.** **가능성 높음.**
4. **통째 위젯(SVAR React File Manager 등)은 비추천.** 트리 + 목록 + 상세 + 키보드를 다 주지만 자체 CSS · 이벤트 연동 설계라, 데몬 스냅숏 · 사용자 행 · 뒤로/앞으로를 얹으려면 내부를 뚫어야 한다(문서 정독 안 함 — 불확실). Chonky 는 2022-04 이후 사실상 방치. 상용 위젯(Syncfusion · DevExtreme · Kendo)은 라이선스 판단이 법무 몫이라 제외.

## 후보 표 (npm 레지스트리 실측 2026-10-14)

| 후보 | 맡는 부분 | 버전 · 갱신 | 라이선스 | React 19 | 비고 |
|---|---|---|---|---|---|
| react-arborist | 트리(가상화 · 사용자 행 · 끌기) | 4.0.1 · 2026-10-11 (저장소는 ^3.9.0) | MIT | peer `>= 18` | 4.x 는 `react-dnd` · `react-dnd-html5-backend` ^16 을 peer 로 요구 — 3.x 유지냐 올리냐는 TRD 감 |
| @headless-tree/react | 트리(headless · 다중 선택 · 키보드 · 끌기) | 1.7.0 · 2026-05-17 | MIT | peer `*` | arborist 가 모자랄 때의 대안 |
| react-complex-tree | 트리(다중 선택 · 키보드 · 끌기) | 2.6.4 · 2026-08-24 | MIT | peer `>=16.0.0` | 가상화 없음(기억) |
| @tanstack/react-table | 목록 칸 · 정렬 · 선택 상태 | 9.3.1 · 2026-10-12 | MIT | peer `>=18` | v9 — shadcn data-table 선례는 v8 기준일 수 있어 API 차이 확인 필요(미검증) |
| @tanstack/react-virtual | 긴 목록 가상화 | 3.14.19 · 2026-10-13 | MIT | peer ~19 | 수백 개면 없어도 될 수 있음 |
| react-aria-components | 목록(Table · GridList) · 트리(Tree) — 정렬 · 다중 선택 · 키보드 · 행 동작 | 1.22.1 · 2026-10-09 | Apache-2.0 | peer ~19 | 리뷰가 찾음 · 메인 실측 · 원문 일부 확인 |
| SVAR React File Manager | 통째 위젯 | 2.6.0 · 2026-06(수집자) | MIT | 18 · 19(README) | 비추천(결론 4) |

| crate | 맡는 부분 | 버전 | 라이선스 | 비고 |
|---|---|---|---|---|
| walkdir | 폴더 훑기 | 2.5.0 | Unlicense/MIT | `spawn_blocking` 안에서 |
| notify | 변경 감지 | 8.2.0 안정 · 9.0.0-rc | CC0-1.0 | rust-analyzer · Zed · Deno · watchexec 가 쓴다(README) |
| notify-debouncer-full | 이름 바꾸기 이어 붙이기 | 0.8.0-rc.2 | MIT/Apache-2.0 | 선택 · RC 라 고정 필요 |
| ignore | .gitignore 식 제외 훑기 | 0.4.33 | Unlicense/MIT | 제외 규칙이 필요할 때만 |

## grounding

| 클레임 | 근거 | 판정 |
|---|---|---|
| react-arborist ^3.9.0 의존 · AgentList 에서 `disableMultiSelection` | `package.json:31` · `src/components/agent/AgentList.tsx:588-599` | 지지(메인 확인) |
| `broadcast_preset_list` 가 생성 · 삭제마다 불림 | `crates/engram-dashboard-daemon/src/connection_core.rs:1799,1808,1818` | 지지(메인 확인) |
| 저장소 Cargo.toml 에 walkdir · notify 없음 | `rg` 0건 | 지지(메인 확인) |
| npm 메타(버전 · 라이선스 · peer) | `npm view` 실측 | 지지(메인 확인) |
| notify Windows 16 KiB 버퍼 | `notify-8.2.0` · `main` 의 `notify/src/windows.rs` | 지지(메인 확인) |
| overflow 시 `Rescan` | `main` 에만 `completion_rescan_event`(ERROR_NOTIFY_ENUM_DIR) — `notify-8.2.0` 에는 없음 | **8.2 에 대해 미지지** → 결론 2 수정 |
| React Aria Table = 정렬 · 다중 선택 · 키보드 · `onRowAction` | https://react-aria.adobe.com/Table | 지지(Shift/Ctrl 범위는 요약에 없음) |
| TanStack Table = headless 칸 · 정렬 · 행 선택 | 수집자 | 메인 미재확인 |

## 적대 리뷰

codex(cross-family · effort high · 레벨 2 · 웹 검색) 1회. 판정 **BLOCK**. 반영:

| 지적 | 심각도 | 반영 |
|---|---|---|
| `notify` 8.2 는 overflow 에 `Rescan` 을 안 낸다 — 초안은 `main` 소스를 8.2 권장 근거로 씀 | 치명 | 메인이 두 태그 소스 대조로 확인 · 결론 2 에 안전망 다시 훑기 추가 |
| React Aria Table · GridList · Tree 누락 — 정렬 · 다중 선택 · 키보드 · 행 동작을 준다 | 높음(누락) | 결론 3 (가) · 표 · npm 실측 · 원문 일부 확인 |
| shadcn data-table 은 이미 TanStack v9 기준 · v9 API 변경 | 높음 | 결론 3 (나) 에 반영 · 메인 미확인 |
| react-arborist 4 = Node ≥22.12 · ESM 전용 · 깊은 import 제거 | 중간 | 결론 3 에 반영(3.9 유지) · 메인 미확인 |
| 감시 함정: 폴더 이름 바꾸기 경로 불완전 · 루트 삭제 · OneDrive 온라인 전용 · 긴 경로 | 높음 | 결론 2 에 목록 · 메인 미확인 |
| 누락: Ark UI Tree View · Tauri v2 fs watch · watchexec | — | 기록만 — 데몬 경계(감시는 데몬이 한다)상 Tauri fs 플러그인은 셸 쪽이라 맞지 않음(해석) |

- 반영 후 재리뷰는 돌리지 않았다(medium 단일 패스).

## 한계

- SVAR 의 통제형 동작 · headless-tree 기능 목록 · TanStack Table v9 변경점은 문서 정독 안 함.
- 수집자 제안: 트리 + 목록 + 다중 선택 최소 시제품을 파일럿 1건으로 만들어 구현 비용을 재 보는 것.

## 추가 (2026-10-14) — 성숙한 앱은 실제로 어떻게 조합했나

사용자 요청 「다른 곳의 라이브러리 조합을 보고 따라가자」로 수집 2갈래를 더 돌렸다(소스 · 매니페스트 직접 읽기 · light — 적대 리뷰 없음). 메인이 핵심을 원문으로 다시 확인했다.

**가장 가까운 피어 = Spacedrive**(39.2k★ · 2026-10-13 커밋) — 따로 된 코어 데몬 + Tauri 셸은 중계만(`invoke("daemon_request")` · `listen("core-event")`)이라 Engram 과 모양이 같다.

| 자리 | Spacedrive 가 쓰는 것 | 확인 |
|---|---|---|
| 감시 | `notify = "8.0.0"`(`crates/fs-watcher/Cargo.toml`) · debouncer crate 없음 | 메인 확인 |
| 훑기 | 직접 짠 병렬 `read_dir` 일꾼(`core/src/ops/indexing/phases/discovery.rs`) | 수집자 |
| 목록 | `@tanstack/react-table ^8.21.3` + `@tanstack/react-virtual ^3.13.12` | 메인 확인(`packages/interface/package.json`) |
| 다중 선택 | `react-selecto ^1.26.3`(상자 · 여러 개 고르기) | 메인 확인 |
| 끌어 놓기 | `@dnd-kit/core` · `sortable` | 메인 확인 |
| 상태 | `zustand ^5` · `@tanstack/react-query ^5` | 메인 확인 |
| 트리 | 전용 라이브러리 없음 — 직접 짠 것으로 보임 | 불확실(소스 미열람) |

그 밖: Yazi(42.8k★) · broot(13.1k★) · Zed 도 감시는 `notify`(8.x · Zed 는 포크). 훑기는 `ignore`(Zed) · 직접(Spacedrive · broot · Yazi) · `walkdir`(작은 앱 · rust-analyzer). react-arborist · React Aria 를 파일 브라우저에 쓴 성숙한 앱은 이번 표본에서 못 찾았다.

**바뀜 신호를 받은 뒤의 동작(소스 확인).**
- 작은 폴더: **모았다가 통째로 다시 만들기** — mdBook `notify_debouncer_mini::new_debouncer(Duration::from_secs(1))`(`src/cmd/watch/native.rs:26`) → 책 전체 다시 빌드 · Zola `notify_debouncer_full` 기본 1000 ms → 삭제 · 폴더 이름 바꾸기 등은 `recreate_site()`(`src/cmd/serve.rs`). 이름 바꾸기 · 넘침을 따로 처리하지 않아도 된다(무엇이든 통째로).
- 큰 트리: **바뀐 경로만 고치고 애매하면 그 아래만 다시 훑기** — Zed(100 ms) · watchexec(50 ms), 둘 다 `Rescan` 을 직접 처리. rust-analyzer 는 경로별 갱신 · 모으기 · 넘침 처리 없음.

→ Engram 규모(수백~수천)는 mdBook · Zola 쪽(모았다가 통째로 다시 훑기 + 비교)이 맞고, 화면 목록은 가장 가까운 피어인 Spacedrive 조합(TanStack Table + Virtual + 다중 선택 라이브러리)을 따라가는 것이 「성숙한 곳 따라하기」에 가장 가깝다(해석).
