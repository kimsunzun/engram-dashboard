# ADR-0053: 채팅 슬롯 오버레이 스크롤바 = Radix ScrollArea 채택 (네이티브 CSS·전용 라이브러리·자작 거부)

- 상태: 확정 (2026-07-07, 근거: /research medium + Codex cross-family 적대 리뷰 + package.json 실측)
- 관련: CLAUDE.md §기술스택(Tailwind+shadcn)·§5 LLM 제어 · package.json:19 · src/components/ui/scroll-area.tsx (앱 전역 승격 2026-07-12 — 구 slot/chat/ChatScrollArea.tsx) · RichSlot.tsx · ADR-0047·0051 · step-log S15

## 맥락
채팅 슬롯(구조화/JSON 출력) 스크롤 영역의 스크롤바 요구 5:
(1) 진짜 overlay — 네이티브 스크롤바 숨기고 콘텐츠 위에 떠 공간 차지 0 · (2) 평소 숨김 · (3) 스크롤할 때만 등장 · (4) 스크롤 멈춘 뒤 ~0.5s 후 숨김 · (5) 얇은 다크 thumb, CSS 변수 커스터마이즈. (초기 요구는 "hover 후 0.5s 뒤 표시"였으나, GUI 실측서 채팅이 창을 꽉 채워 "영역 hover=상시 뜸"이 확인돼 scroll-트리거로 확정.)
현재 = `overflow-y-auto` 한 줄(RichSlot.tsx:181) → WebView2 네이티브 회색 스크롤바(공간 점유·상시 표시)로 요구 전부 미충족.

## 결정
스크롤 컨테이너를 **Radix ScrollArea(`@radix-ui/react-scroll-area`, shadcn `scroll-area` 래퍼)** 로 교체한다.
- 요구 (1)(2)(5) = Radix 기본(overlay · 평소 숨김 · Tailwind/CSS-var 테마)으로 충족.
- 요구 (3)(4) = **`type="scroll"` + `scrollHideDelay=500`** — 실제 스크롤(휠/드래그)할 때만 표시, 멈추면 ~0.5s 뒤 숨김. Radix 기본 스코프라 CSS 커스텀 불필요. (트레이드오프: scroll-트리거는 thumb를 잡아 드래그하기 어렵다 — 사용자 수용.)
- 얇은 `ChatScrollArea` seam으로 감싸 교체 가능성 유지(컴포넌트는 seam에만 의존, 직접 Radix Root 노출 금지). auto-scroll(하단 고정)은 seam이 forwardRef로 Radix **Viewport**(실제 스크롤 노드)를 노출해 보존.
- thumb 색은 테마별(`:root[data-theme=...]`)로 둔다 — dark=밝은 반투명, light/e-ink=어두운 반투명(테마-무관 `:root`에 흰색 하나로 두면 light/e-ink서 안 보임, 실측·리뷰 적출).

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
- **트리거 진화(GUI 실측)** — 초안(hover + CSS 0.5s 표시-지연)은 `scrollHideDelay`가 "숨김 지연"이라 표시-지연을 CSS로 얹어야 했고, 실측서 영역 hover가 상시 뜸으로 드러나 **`type="scroll"`(스크롤 시에만 표시 + 0.5s 숨김-지연)** 로 전환 — Radix 기본 스코프라 CSS 커스텀·reduced-motion 분기 모두 불필요해짐.

## 영향 / 불변식
- 스크롤 컨테이너는 `ChatScrollArea` seam 경유 — 직접 Radix Root를 컴포넌트에 노출하지 않는다(교체점 보존, §5 손발/두뇌 분리).
- 헤더 "JSON ● idle" 제거는 이 결정과 **별건**(단순 UI 변경, ADR 무관 — 흐름은 step-log에만).
- §5: 스크롤바 스타일이 LLM 제어 대상이 되면 chatStyle control surface(ADR-0051)에 얹는다(현재는 CSS 변수 고정, 미노출).
- **스코프 확장 (2026-07-12):** 이 seam은 더 이상 chat 전용이 아니라 **앱 전역 스크롤 primitive** `src/components/ui/scroll-area.tsx`(`ScrollArea`)다 — 구 `slot/chat/ChatScrollArea.tsx`를 중립 이름으로 승격. DOM 스크롤 표면(에이전트 트리·프리셋·모니터링 픽커·DomSlot·ThoughtRow·RichSlot)이 전부 이 하나를 경유 → 스크롤 동작·토큰 한 곳 변경이 전역 전파. 위 불변식(overlay·`type=scroll`·500ms·ref→Viewport·Root 비노출) 전부 그대로 적용. 새 결정이 아니라 기존 seam의 적용 범위 확장이라 별도 ADR 없이 여기 기록.
  - **예외 (seam 밖 — 의도적):** xterm 터미널은 자체 스크롤바를 소유해 컴포넌트로 못 감싼다 → `.xterm-viewport`를 동일 theme 토큰(`--scrollbar-*`)으로 전역 CSS(`src/index.css`) 통일(컴포넌트 아니어도 토큰 공유). `StructuredTextView` 코드블록 가로 스크롤·`TabBar` 가로 스크롤은 세로 오버레이 패턴 대상이 아니라 raw overflow 유지.
