# ADR-0053: 채팅 슬롯 오버레이 스크롤바 = Radix ScrollArea 채택 (네이티브 CSS·전용 라이브러리·자작 거부)

- 상태: 확정 (2026-07-07, 근거: /research medium + Codex cross-family 적대 리뷰 + package.json 실측)
- 관련: CLAUDE.md §기술스택(Tailwind+shadcn)·§5 LLM 제어 · package.json:19 · RichSlot.tsx:181 · ADR-0047·0051 · step-log S15

## 맥락
채팅 슬롯(구조화/JSON 출력) 스크롤 영역의 스크롤바 요구 5:
(1) 진짜 overlay — 네이티브 스크롤바 숨기고 콘텐츠 위에 떠 공간 차지 0 · (2) 평소 숨김 · (3) hover 시 등장 · (4) hover 후 ~0.5s delay 뒤 표시 · (5) 얇은 다크 thumb, CSS 변수 커스터마이즈.
현재 = `overflow-y-auto` 한 줄(RichSlot.tsx:181) → WebView2 네이티브 회색 스크롤바(공간 점유·상시 표시)로 요구 전부 미충족.

## 결정
스크롤 컨테이너를 **Radix ScrollArea(`@radix-ui/react-scroll-area`, shadcn `scroll-area` 래퍼)** 로 교체한다.
- 요구 (1)(2)(3)(5) = Radix 기본(overlay · `type="hover"` · Tailwind/CSS-var 테마)으로 충족.
- 요구 (4) 0.5s-delay-before-show = **CSS `transition-delay` 비대칭 패턴**(hover-on에 delay, hover-off 즉시)으로 얹는다.
- 얇은 `ChatScrollArea` seam으로 감싸 교체 가능성 유지(컴포넌트는 seam에만 의존, 직접 Radix Root 노출 금지).

## 거부한 대안
- **네이티브 `::-webkit-scrollbar` CSS만** — 진짜 overlay 원천 불가. Chromium은 `::-webkit-scrollbar`에 width 주는 순간 클래식(거터 점유)으로 전환하고, `overflow: overlay`는 Chrome 114(2023)에서 제거돼 `auto` alias가 됨 → 요구 (1) 불가. (grounded 확실: Chrome for Developers docs · chromestatus 5194091479957504 · blink-dev intent)
- **OverlayScrollbars** — overlay 전용 성숙 라이브러리(터치·키보드·접근성 두꺼움)지만 **신규 의존성 +15KB gz**. Radix가 이미 의존성이라 이점 상쇄, 채팅 스크롤엔 과함.
- **SimpleBar** — hover-표시가 내장 옵션 아님(open issue #650, CSS `.simplebar-mouse-entered` 수작업 필요), 테마가 selector 방식 → Radix/OSS보다 커스텀 부담↑.
- **DIY 자작** — 네이티브 숨김 + absolute thumb를 JS로 sync(scroll + ResizeObserver 2 + drag + track click, ~150-250줄 + 엣지케이스). 15KB MIT 유지 라이브러리가 존재하는데 재발명 → CLAUDE.md "재발명 금지"로 기각.
- **방치 라이브러리** — react-custom-scrollbars-2(2022, React18-only, non-overlay) · react-scrollbars-custom(2022, 유지보수 모집) · rc-scrollbars(React18-only) · react-perfect-scrollbar(원본 2021 정체). 전부 방치 or React19 미지원 or non-overlay.

## 근거
- **신규 의존성 0** — `@radix-ui/react-scroll-area ^1.2.11`이 이미 `package.json:19`에 존재(실측). 설치 없음.
- **React 19 명시 지원** — peerDeps `^19.0`(타 후보는 범위 추론 or React18-only).
- **스택 정합** — 프로젝트 Tailwind v4 + shadcn/ui 관례와 일치(ADR-0047). shadcn `scroll-area` = Radix 얇은 래퍼 → §5 교체 seam 자연 확보.
- **overlay 확증** — Radix 공식 문서 "sits on top of the scrollable content, taking up no space".
- 조사 = `/research` medium(조사 수집자 3 + Radix 백스톱 1) + Codex cross-family 적대 리뷰 — 누락 후보 Radix 적출로 결정 반전(초안 추천 OverlayScrollbars → Radix).
- **요구 (4) 공통 한계(차별점 아님)** — Radix `scrollHideDelay`·OverlayScrollbars `autoHideDelay` 모두 "숨김 지연"이지 "표시 지연"이 아님 → 어느 후보를 골라도 CSS `transition-delay`가 필요.

## 영향 / 불변식
- 스크롤 컨테이너는 `ChatScrollArea` seam 경유 — 직접 Radix Root를 컴포넌트에 노출하지 않는다(교체점 보존, §5 손발/두뇌 분리).
- 헤더 "JSON ● idle" 제거는 이 결정과 **별건**(단순 UI 변경, ADR 무관 — 흐름은 step-log에만).
- §5: 스크롤바 스타일이 LLM 제어 대상이 되면 chatStyle control surface(ADR-0051)에 얹는다(현재는 CSS 변수 고정, 미노출).
