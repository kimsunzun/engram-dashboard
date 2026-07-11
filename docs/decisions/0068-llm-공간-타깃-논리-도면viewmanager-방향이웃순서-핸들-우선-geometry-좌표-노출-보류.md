# ADR-0068: LLM 공간 타깃 = 논리 도면(ViewManager) 방향·이웃·순서 핸들 우선 (geometry 좌표 노출 보류)

- 상태: 확정 (2026-07-11, 근거: `/research medium`(설계-결정 모드) OSS 서베이(tmux·zellij·kitty·i3/sway·wezterm·VS Code·JetBrains) + Codex 적대 리뷰 FIX 보정 + 사용자 결정("도면만으로 충분"))
- 관련: Amends ADR-0066 (결정 3: LLM 공간 타깃 = geometry {x,y,w,h} 좌표 노출 우선 → 논리 도면 기반 방향/이웃/순서 핸들 우선으로 개정 (좌표계·실측 픽셀 보류)) · CLAUDE.md §5(LLM-우선 제어) · ADR-0035(레이아웃 권위 = 클라이언트 Rust ViewManager) · ADR-0022/0055(command registry) · ADR-0011(agentClient assign) · `src-tauri/src/layout/manager.rs`(ViewManager) · step-log "LLM 공간 타깃 = 논리 도면 방향·이웃·순서 핸들"

## 맥락

ADR-0066 결정 3은 "LLM 공간 타깃 = 슬롯 geometry `{id,x,y,w,h}`를 control surface로 노출 → LLM 수치추론('우하단' = x+w·y+h 최대)으로 slot id 도출"로 잡았고, 방향/이웃 sugar 커맨드는 **후순위**로 밀어뒀다. 구현 착수 전 재검에서 두 문제가 드러났다:

1. **용어 혼동.** ADR-0035/0066이 쓰는 "백엔드 ViewManager"는 **에이전트 호스팅 데몬(서버)이 아니라 클라이언트(src-tauri 셸)의 Rust 측 레이아웃 소유자**다. 레이아웃은 창마다 다른 UI 상태라 데몬과 무관한 **클라이언트 관심사**다. 이 결정의 "누가 좌표를 아나"는 데몬-클라이언트 축이 아니라 **한 클라이언트 프로세스 안의 Rust(ViewManager) ↔ JS(프론트 React) 경계** 문제다.

2. **좌표 노출 = stated need 대비 과설계.** "우하단에 놓아줘 → slot id"가 실수요인데, 이는 좌표 산술이 아니라 **상대 위치·순서 판정**이다. 좌표계를 신설하지 않아도 ViewManager가 이미 소유한 논리 트리(split 구조·비율)에서 풀린다. 실제 렌더 픽셀은 프론트만 알지만("건물"), 논리 구조는 ViewManager가 결정한 주인이라("도면") 프론트 왕복 없이 안다.

## 결정

LLM 공간 타깃의 1차 제어 표면 = **ViewManager(클라이언트 Rust)가 자기 논리 레이아웃 트리에서 산출하는 방향·이웃·순서 핸들**로 한다. geometry 좌표계·실측 픽셀 노출은 **보류**한다.

1. **1차 표면 = 논리 도면 파생 핸들** — 각 슬롯의 이웃(left/right/up/down 이웃 slot id) · 순서(ordinal) · 방향 토큰(tmux `{bottom-right}`/`{left-of}` 결). "우하단"·"이 슬롯 오른쪽"·"제일 왼쪽"을 논리 관계로 slot id에 매핑. 좌표 산술 불필요.
   - **ordinal 정의(코드리뷰 FIX-2로 확정):** 각 leaf 중심 `(center_y, center_x)`의 **전역 사전순** index(위→아래, 동률이면 왼쪽→오른쪽). 트리 pre-order가 아니라 center 전역 정렬이다 — leaf rect가 서로 disjoint라 중심쌍이 유일해 결정적. 비대칭 분할에서 전체높이 열이 좌열 슬롯 사이 ordinal에 끼어들 수 있음(열/행 응집 비보장)을 감수한 총순서 정의. bottom-right/corner 토큰 해소는 이와 독립(코너 거리 최소).
2. **산출 주체 = ViewManager** — 이 핸들은 ViewManager가 이미 소유한 트리에서 파생한다(프론트 왕복 0·픽셀/DPI 무관). 사람 클릭·LLM이 같은 command 표면(§5), 권위는 백엔드(ADR-0035).
3. **실측 픽셀·좌표계 = 보류** — `getBoundingClientRect` 실측 rect나 정규화 좌표계 신설은 **진짜 픽셀공간 use case**(스크린샷 좌표 매핑·픽셀 교차판단·외부 AX/CDP 좌표 대조)가 등장할 때 별도 capability로 얹는다. 그때 모델 = 프론트가 실측 rect를 **versioned 관측값**으로 ViewManager에 보고(권위는 여전히 ViewManager, 프론트는 관측만 — §5). 정규화 `[0,1]` 논리 rect가 필요하면 같은 트리에서 파생 가능하나, 1차는 심볼릭 핸들이다.

## 거부한 대안

- **프론트 `getBoundingClientRect`를 1차 좌표 출처로:** ① 권위 방향 역전 — 프론트가 레이아웃 진실을 쥐면 §5(프론트=순수 I/O)·ADR-0035(백엔드 권위) 위반. ② 창크기·애니메이션 중 staleness, CSS-px vs device-px(×devicePixelRatio) 모호(WebView2 DPI). ③ stated need("우하단")엔 불필요한 왕복. → 픽셀이 진짜 필요한 별도 use case로 미루고, 그때도 "관측값 보고"(권위 아님)로만.
- **백엔드 투영 물리 px (`inner_size`+`scale_factor`로 ratio→px):** 정확하려면 ViewManager가 gutter·sash·min-size·collapse까지 모델링해야 함 → 렌더 엔진 로직 중복·drift 위험. coarse 방향 타깃엔 과함. 보류.
- **geometry 좌표 노출을 1차로 (ADR-0066 결정 3 원안):** OSS의 "레이아웃 권위가 논리좌표 소유" 선례는 **LLM 근거가 아니라 각 엔진 내부 사정**(터미널=셀 격자, 컴포지터=픽셀)이라 우리 표현 선택의 근거로 약하다. 실제 자동화 클라이언트의 pane 타깃은 raw 좌표 산술이 아니라 **심볼릭 방향/이웃/순서 토큰**을 쓴다(tmux `{bottom-right}`·kitty `neighbor:right`·i3 `focus left`). 좌표계 신설은 실수요 대비 과설계 → 방향·이웃·순서 핸들 우선으로 뒤집음.
- **"정규화 논리좌표면 모든 공간지시 충족":** 과장(Codex 적대 리뷰 High). min-size 제약·접힌 슬롯·sash/border/스크롤바·터미널 chrome로 실제 렌더 크기가 비율과 어긋난다 — "제일 넓은 거"·픽셀공간 교차는 실측 rect가 독립적으로 필요. i3가 `percent`(비율)와 `rect`(픽셀)를 **둘 다** 노출하는 게 방증. → 정규 논리는 coarse 순서 표면, 픽셀은 별도 표면(보류)으로 분리.

## 근거

- **`/research medium`(설계-결정 모드) OSS 서베이:** tmux(`#{pane_left}` 셀·서버 소유 + `{bottom-right}`/`{left-of}` 토큰)·zellij(`PaneInfo` 셀·엔진 소유 + `move-focus`)·kitty(`neighbor` 방향그래프 + `num` ordinal을 **1차 제어 표면**으로)·i3/sway(`rect` 픽셀 + `percent` 비율 **둘 다** + `focus` 방향)·wezterm(셀+px 혼합, Windows px 신뢰 경고)·VS Code(`setEditorLayout` 비율 `size` 0~1, editor-group 픽셀 rect API 없음 — #208658)·JetBrains(`Splitter.myProportion` 비율). → 권위는 논리 소유, 타깃은 심볼릭 방향/이웃/순서가 지배.
- **Codex 적대 리뷰 FIX 보정:** VS Code #94817은 closed(픽셀-rect 부재 근거는 #208658) · allotment `onChange`=px는 문서 미명시(하향) · "정규 논리 완전충족"은 과장(위 거부 대안 4).
- **§0 판단기준(저위험·장기):** 방향·이웃·순서 핸들 = 저위험·수요 명확 → 지금 깐다. 좌표계·실측 픽셀 = 불확실·미검증 use case → 껍데기도 안 만들고 보류(실 use case 등장 시 실측 채움).
- **사실/해석 구분:** "i3/sway가 픽셀인 건 컴포지터라 픽셀이 네이티브 단위이기 때문"은 서베이 해석(직접 근거 아님, Codex 지적) — 결정을 지지하되 확정 사실로 인용하지 않는다.

## 영향 / 불변식

- **공간 타깃 핸들은 ViewManager(클라이언트 Rust)가 소유·산출** — 사람 클릭·LLM 동일 command 표면(§5). 프론트는 렌더만, 레이아웃 진실을 안 쥔다(ADR-0035). 프론트가 별도 "공간 상태"를 들면 §5 위반(리뷰 reject).
- **이웃/순서/방향 핸들 = 논리 트리 파생** — 픽셀·DPI·창크기 무관, 프론트 왕복 0.
- **실측 픽셀 추가 시(미래)** — 프론트 실측 rect는 **versioned 관측값**(레이아웃 진실 아님): 창크기/render epoch 바뀌면 stale, ViewManager 권위 불변(dual-authority 금지 — ADR-0035).
- **미구현(이 ADR가 여는 것):** neighbor/ordinal/방향 토큰 command의 실제 구현, 실측 픽셀 capability는 후속 슬라이스. 이 ADR는 방향·우선순위만 확정한다.
- **★리사이즈 command 도입 시 제약(cross-family 리뷰 ①②)★:** 공간 계산은 `edge_eq`/overlap 판정에 절대 epsilon(`EPS=1e-4`)을 쓴다. 어떤 leaf가 폭/높이 `< EPS`로 눌리면 두 결함이 발현한다 — (①) 그 leaf가 다시 분할되면 하위 두 slot의 직교축 겹침이 정확히 `EPS`라 `overlap > EPS`(strict)에 탈락 → 상하/좌우 이웃을 못 찾음. (②) sub-EPS 폭 slot은 그 너머 slot이 `edge_eq`로 "인접"으로 오판돼 건너뛰어짐. **분할별 ratio clamp(`[EPS,1-EPS]`)로는 방어 불가** — leaf 절대 크기 = 경로상 ratio의 곱이라 중첩되면 sub-EPS로 내려간다. **정본 해법 = 리사이즈 command가 UX 최소 칸 크기를 강제**(그 이하 드래그 = 제거/스냅, tmux·i3 선례 — 값은 OSS 조사 후 ADR). 이때 최소 크기·아래로 내릴 때 동작(제거 vs 스냅)은 사용자 결정(§5 체감 동작).
- **대표 이웃 비대칭 = by-design(cross-family 리뷰 ③):** 한 slot이 방향 하나에서 두 slot을 마주하면(L-shape의 전체높이 좌열이 우측 상·하 2칸을 마주함) `neighbor_in_dir`는 **대표 이웃 하나**만 반환한다 — tie는 순회 첫 후보 유지. 그래서 `rb.left==left`인데 `left.right==rt(≠rb)`인 비대칭이 생긴다. 단일 대표 이웃 모델의 필연적 성질이라 결함이 아니며, `spatial.rs` neighbor_in_dir doc·테스트 `l_shape_bottom_right_neighbors`가 명시·수용한다(왕복 상호성 미보장).
- load-bearing 경로에 `// ADR-0068` 앵커(구현 시).
