# ADR-0051: 채팅 렌더 스타일(간격·폰트)을 LLM 제어 프론트 control surface로 노출 — Zustand+CSS변수+localStorage 영속

- 상태: 확정 (2026-07-06, 근거: 사용자 결정 + Explore 조사 2건)
- 관련: CLAUDE.md §5(LLM-우선 제어) · ADR-0050(채팅 렌더 자체 구현) · `src/components/slot/StructuredTextView.tsx` · `src/components/slot/chat/chat.css` · `src/store/themeStore.ts`(계승 패턴) · `src/api/eventBus.ts`(window 핸들 노출) · step-log 2026-07-06

## 맥락
ADR-0050에서 채팅 렌더를 자체 구현하고 Claude Code VSCode 확장 룩에 맞춰 시각 refine 중이다. 사용자가 우리 렌더와 확장을 나란히 비교해 "줄 간격이 좁고 전체적으로 산만하다"고 지적했다 — 핵심 원인은 ① 행 간 수직 간격 부족 ② 유저 메시지가 대화를 덩어리로 끊어주지 못함 ③ dot-rail 연결선 clean-ends 미처리다.

조사 결과, 채팅 렌더의 **색상**은 이미 CSS 변수 토큰(`--text`/`--border`/`--surface-elevated` 등, theme.css + index.css `@theme`)이지만, **간격·폰트·라인하이트는 전부 하드코딩된 Tailwind 클래스**(`StructuredTextView.tsx`의 `pt-2.5`/`pt-3`/`top-[-12px]`/`top-[9px]`/`text-[13px]`/`leading-[1.45]` + `chat.css`의 margin·padding)로 흩어져 있다. CLAUDE.md §5는 "모든 UI 기능은 LLM 제어 가능, UI 먼저·제어 나중 = 위반"을 못 박으므로, 이 값들을 그냥 간격만 튜닝해 하드코딩으로 두면 규약 위반이다.

사용자 지시: 간격·폰트 설정값을 에이전트(LLM)가 명령 가능하게 빼면서 작업하되 **저장(영속)이 되어야 한다**. 제어 표면 패턴은 이미 확립돼 있다 — 레이아웃은 `window.__engramLayout`(invoke 루프, src-tauri 권위), 테마·렌더모드·폰트프리셋은 프론트 전용 Zustand + CSS 변수 핸들.

## 결정
채팅 렌더의 **간격+폰트 세트**(row-gap, 유저버블 여백, dot-rail 연결선/점 오프셋, base font-size, line-height)를 하드코딩 Tailwind 클래스에서 **CSS 변수로 추출**하고, **프론트 전용 control surface**로 LLM에 노출한다.

- **권위 = 프론트 전용:** 새 Zustand chat-style slice가 값을 소유하고 CSS 변수(`:root`)를 갱신한다. 기존 테마·렌더모드·폰트프리셋의 "Zustand slice + CSS 변수 갱신 + `window.__engramXxx` 핸들" 패턴을 계승한다.
- **LLM 핸들:** `window.__engramChat`(getter + setter)로 노출 — 사람 UI 조작과 LLM 조작이 **같은 store 액션**을 부른다(단일 control surface, §5 손발/두뇌 분리 유지: 프론트=순수 I/O).
- **영속 = localStorage:** 값 변경 시 localStorage에 저장하고 부팅 시 로드→CSS 변수 적용. 새로고침·클라이언트(src-tauri 셸) 재시작해도 유지한다.
- **dot-rail 연결선 clean-ends:** run-position(연속 assistant-run에서 top/mid/bottom/single 위치) 계산을 추가해 첫 dot 위로 선이 튀어나오는 문제를 해소한다. 연결선 오프셋은 위 CSS 변수와 커플링을 명시화한다.

## 거부한 대안
- **백엔드 영속(Tauri command + settings.json + emit 루프, 레이아웃 패턴)** — §5 "두뇌=백엔드" 완전 분리이나, 순수 렌더 프리퍼런스에 데몬·파일·emit 배선 비용이 이득을 넘는다. 테마·렌더모드도 프론트 전용 전례가 있어 프론트 권위로 충분. (사용자 선택: 프론트)
- **CSS 하드코딩 유지(간격만 튜닝)** — LLM 제어 표면이 없어 §5 "UI 먼저·제어 나중 = 위반"에 정면 위배.
- **영속 없음(메모리 store만)** — 사용자가 "저장은 되야함" 명시. 현재 테마 slice가 영속이 없어 새로고침 시 dark로 초기화되는 함정을 그대로 반복하게 됨.
- **chat.css 전체 토큰화(heading/code/table margin·padding·radius까지)** — 지금 당장 안 건드릴 값까지 CSS 변수화 = YAGNI. 저위험이나 현 라운드에 불필요해 간격+폰트 세트로 범위 한정(추후 필요 시 같은 패턴으로 확장).

## 근거
- CLAUDE.md §5: 모든 UI 기능은 LLM 제어 가능 + 단일 control surface(사람 클릭 = LLM 호출 = 동일 진입점).
- CLAUDE.md 아키텍처 원칙 §0(위험도×기간): CSS 변수 토큰 레이어 = 저위험+장기 → seam을 지금 제대로 깐다. 나중에 범위 넓히기 쉬움.
- 기존 패턴 계승(테마·렌더모드·폰트프리셋)이라 새 추상화·배선 최소.
- Explore 조사 2건으로 grounding: 스타일 값 위치 맵 + 현 제어 표면 현황(`window.__engramLayout`/`__ENGRAM_AGENT__` 등).

## 영향 / 불변식
- 새 Zustand chat-style slice + CSS 변수(theme.css/index.css `@theme`) + `window.__engramChat` 핸들 추가. `src/api/eventBus.ts` 부팅 시 핸들 노출.
- `StructuredTextView.tsx`/`chat.css`의 간격·폰트 하드코딩 값이 CSS 변수 참조로 교체됨.
- **rail 연결선 오프셋 ↔ outer padding 커플링을 CSS 변수로 명시화** — 둘이 한 변수 그룹으로 묶여 함께 조정(기존 `top-[-12px]`↔`pt-3` 암묵 커플링 제거).
- localStorage 키 추가(부팅 로드→CSS 변수 적용). 값 부재/파싱 실패 시 기본값 fallback.
- §5 준수 유지: 프론트는 순수 I/O(store 액션 호출만), LLM은 `window.__engramChat`로 동일 조작. `StructuredTextView`는 순수 렌더 유지(state/effect 추가 금지 — ADR-0050 불변식).
- 코드 앵커: 새/수정 load-bearing 지점에 `// ADR-0051` 한 줄.
