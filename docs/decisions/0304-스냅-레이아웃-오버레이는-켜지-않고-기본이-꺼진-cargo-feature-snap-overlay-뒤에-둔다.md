# ADR-0304: 스냅 레이아웃 오버레이는 켜지 않고 기본이 꺼진 cargo feature snap-overlay 뒤에 둔다

- 상태: 확정 (2026-10-26, 근거: 사용자 결정 2026-10-23(Q1 — 짧은 스파이크 · 싸게 안 되면 포기 · 사용자는 그 기능을 쓰지 않는다) · 2026-10-24(Q1 — 켜지 않는다 · 코드는 기본이 꺼진 cargo feature 뒤에 남긴다) + 메인 결정 M19(주석이 아니라 feature · CI 가 켠 빌드를 컴파일) + 스파이크 사용자 실측 2026-10-24(S6 — 메뉴가 떴다) + 구현 U9 `8836c537` · `b6b3c780` · 리뷰 수정 `c7295bdb`)
- 관련: Amends ADR-0275 (결정 13 의 「platform 의 cargo feature 는 test-support 하나다」) · PRD `docs/process/S21-window-chrome/prd.md`(0.5판 — R15 · §2 · W8 · AC19 없음) · TRD `docs/process/S26-window-chrome/trd.md`(0.3판 — §8-3 · §8-4 · §10-1 · §10-4 · §14) · 조사 `docs/research/custom-titlebar-tauri-2026-10-18.md` §3 · §4 · ADR-0266(OS 코드 = platform) · ADR-0278(OS 경계 게이트) · ADR-0303(OS 틀을 걷으면 스냅 메뉴를 잃는다) · 참고 저장소 `Hyph-M/tauri-plugin-snap-layout`(MIT) · `crates/engram-dashboard-platform/src/window/snap.rs` · `src-tauri/src/commands/snap_overlay.rs` · `src/components/layout/titleRow/snapOverlay.ts` · `.github/workflows/ci.yml`(`snap-overlay gate 1` · `2`) · `/qa` 바인딩 standard 「스냅 오버레이 기능 게이트」 · step-log S26

## 맥락

OS 틀을 걷으면(ADR-0303) Windows 11 스냅 레이아웃 메뉴(최대화 단추에 올리면 뜨는 배치 고르기)를 잃는다 — 끌기 영역 기법 두 갈래 모두 잃는다(PRD §5-2 표). 되찾는 길은 최대화 단추 위에 Win32 자식 창(오버레이)을 덮어 OS 가 메뉴를 띄우게 하는 것이다(조사 제목 §3 · §4 ② — `platform` 에 직접 짓기 · 참고 플러그인의 MIT 귀속 유지). 스파이크(`t66_snap_spike.rs`)에서 메뉴가 떴다(S6 — 사용자 관측 2026-10-24).

ADR-0275 결정 13 은 platform 의 `windows` 바인딩을 기능별 cargo feature 로 나누지 않으며 「platform 의 cargo feature 는 `test-support` 하나다」라고 적었다.

## 결정

1. **스냅 레이아웃 메뉴를 켜지 않는다 — 스냅 메뉴 없음을 회귀로 받아들인다**(사용자 결정 2026-10-24 — Q1). 화면 끝으로 끌기 스냅 · Win+화살표는 남는다(OS 이동 루프). AC19 는 없다.
2. **코드는 남긴다 — 주석이 아니라 기본이 꺼진 cargo feature `snap-overlay` 뒤로**(사용자 결정 2026-10-24 · 메인 결정 M19):
   - platform `[features] snap-overlay = []` · 모듈 `window::snap`(`place_overlay(parent, rect, on_hover)` · `remove_overlay(parent)` — 스파이크를 옮겨 다시 지었다 · 참고 저장소 귀속을 모듈 머리에 남긴다) 전체가 그 feature 뒤.
   - 셸 `src-tauri/Cargo.toml` `[features] snap-overlay = ["engram-dashboard-platform/snap-overlay"]`(셸의 첫 `[features]` 표 · 기본 꺼짐). invoke `report_max_button_rect` · 맴돎 사건 `window:snap-hover` · 그 등록이 같은 feature 뒤.
   - 웹 `titleRow/snapOverlay.ts` 는 늘 컴파일 · 시험되고, 창 사실 `caption.snap_overlay`(= 셸의 `cfg!(feature = "snap-overlay")` — 기본 빌드는 늘 거짓)가 참일 때만 최대화 단추 사각형을 보고한다. 셸 쪽 타입이 feature 뒤라 ts-rs 바인딩은 없다 — 웹이 같은 모양을 손으로 적는다.
3. **ADR-0275 결정 13 을 고친다 — platform 의 cargo feature 는 `test-support` · `snap-overlay` 둘이다.** 결정 13 의 나머지(`windows` 바인딩을 기능별 feature 로 나누지 않는다 · 의존 하나에 합집합 feature)는 그대로다 — 오버레이가 쓰는 바인딩(`Win32_UI_WindowsAndMessaging` · `Win32_UI_Input_KeyboardAndMouse`)은 이미 켜져 있어 더하지 않았다(TRD §8-3 — 4g 블록 PASS).
4. **켠 빌드가 썩지 않게 CI 가 잰다**(U9) — `snap-overlay gate 1`(백엔드 잡) = 셸 `cargo check --locked -p engram-dashboard --features snap-overlay` + platform `cargo test --locked -p engram-dashboard-platform --features snap-overlay -- window::snap`(오버레이 실 창 시험 일곱 — 컴파일 포함 · 이름 거르기가 0 건이어도 종료코드 0 이라 `test window::snap::… ok` 를 1 줄 이상 센다) · `snap-overlay gate 2`(게이트 잡) = 셸 운영 그래프에 그 feature 0 줄 + 짝(켜면 `engram-dashboard-platform feature "snap-overlay"` 1 줄 이상). `/qa` 바인딩 standard 에 같은 넷을 실었다.
5. **U9 리뷰 수정**(2026-10-26 · 메인) — 나감(`WM_NCMOUSELEAVE`)은 우리 몫(맴돎 칸 · 콜백)을 한 뒤 앞 창 프로시저로 넘긴다(스파이크와 같은 꼴) · `place_overlay` · `remove_overlay` 는 부모 창을 만든 스레드가 아니면 아무것도 하지 않고 `Err` 다.

## 거부한 대안

- **켜기**(PRD Q1 ② — 오버레이로 메뉴를 되찾음 · 스파이크에서 떴다) — 사용자가 켜지 않기로 정했다(2026-10-24). 사용자 결정 2026-10-23 이 든 사정 = 사용자는 그 기능을 쓰지 않는다. 켜는 대가 = Win32 자식 창 · 단추 자리를 웹이 재서 넘기는 길 · 올림/누름 중계 · DPI · 팝아웃마다 하나(PRD Q1 ② · W8). 기각 = 사용자 결정.
- **주석으로 막아 두기** — 주석으로 막은 코드는 컴파일되지 않아 썩고 켜는 날 고칠 것이 쌓인다(메인 결정 M19 · TRD §8-4). 기각 강도 = 서술.
- **코드를 지우기** — 사용자가 「코드는 남긴다」로 정했다(2026-10-24). 지우는 갈래를 버린 사유는 기록되지 않았다. 기각 = 사용자 결정(사유 기록 없음).
- **플러그인 의존**(조사 제목 §3 · §4 의 ③ 길 — PRD R15) — 스파이크와 U9 는 ② 직접 짓기로 갔다. ③ 을 고르지 않은 사유는 기록되지 않았다(플러그인 셋은 빌드 · 실행해 보지 않았다 — PRD W8). 기각 사유 기록 없음.
- **CI 의 platform 몫을 `cargo check` 로만** — 오버레이의 실 창 시험이 feature 뒤라 다른 어디서도 돌지 않는다(TRD §8-4). 기각 강도 = 코드.
- **`WM_NCMOUSELEAVE` 를 삼키기**(U9 첫 구현) — 메뉴가 뜬 것을 본 것은 그 메시지를 넘기는 스파이크다(S6). 스파이크와 같은 꼴로 돌렸다(U9 리뷰 · TRD §8-4). 기각 강도 = 실측(스파이크 관측과 다른 꼴).

## 근거

- 사용자 결정 PRD §5-1(Q1 — 2026-10-23 · 2026-10-24).
- 스파이크 S6(사용자 관측 2026-10-24 — 메뉴가 떴다).
- TRD §8-3(실측 2026-10-26): 4f 의 OS `cfg` matcher 는 `#[cfg(feature = "snap-overlay")]` · `cfg!(feature = "snap-overlay")` 를 잡지 않는다(그것을 담은 셸 파일 넷과 표본은 안 걸리고 같이 넣은 `#[cfg(windows)]` 표본만 걸림 · 4f 블록 PASS) · 4g 블록 PASS.
- 오버레이 실 창 시험 일곱(U9 여섯 + 리뷰 수정 하나 — TRD §10-1 platform `window::snap` 행).

## 영향 / 불변식

- 기본 빌드(운영 · 릴리즈)에는 오버레이 코드가 없다 — `snap-overlay gate 2` 가 셸 운영 그래프에 그 feature 가 켜지면 빨갛다. 짝(1 줄 이상)을 지우면 feature 이름이 바뀌었을 때 0 기대가 눈먼 채 통과한다.
- OS 경계 게이트(4f · 4g · 4h)는 바뀌지 않는다 — `feature` 술어는 OS 술어가 아니다.
- 켜는 법 = `--features snap-overlay`(셸) — CLAUDE.md 「빌드·검증 명령」에 한 줄을 둔다(U10).
- ★**켤 때 다시 볼 것(재지 않음 — TRD §8-4)**★: 배율만 바뀌고 CSS 크기가 그대로인 때 다시 재지 않는다 · 오버레이 위 탭 놓기(W14) · `STATIC` 오버레이가 배경을 불투명하게 칠한다(U9 리뷰 codex 탐침 — 시험 픽셀 `ff00ff` → `f0f0f0` · 스파이크에서는 달리 보였다) · 크기 조절 띠와의 z 순서 경주 · 맴돎 emit 의 `RefCell` 재진입 가능성 · 순번 없는 사각형 보고의 뒤바뀜 · 귀속 꼴(모듈 머리에 참고 저장소 이름과 라이선스만 적었다) — 그것으로 라이선스 요구를 채우는지는 켜기 전에 법무 · 라이선스 담당에게 확인한다.
- 앵커 — platform `window/snap.rs` 머리 · platform · 셸 `Cargo.toml` 의 `[features]` · `commands/snap_overlay.rs` · `snapOverlay.ts` 에 `// ADR-0304`(TOML 은 `# ADR-0304`)를 단다.
