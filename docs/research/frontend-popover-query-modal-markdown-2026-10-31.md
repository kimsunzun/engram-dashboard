# 프론트 외부 라이브러리 — 상세 팝오버 · 쿼리 캐시 · 모달 · 마크다운 (조사 · medium)

- **상태:** medium · 2026-10-31 · 수집자 1 + 메인 grounding + codex 적대 리뷰(effort high) — 판정 FIX. TRD `docs/process/S25-preset-library/trd.md` §5-1 step 0a(구현 부분별 외부 라이브러리 조사)의 **클라이언트 단계 자료**다 — 서버 먼저 규칙(G20)에 따라 서버 쪽이 확정된 뒤에 쓰는 재료이고 여기서 확정하지 않는다.
- **계기:** TRD §1-10 의 DetailFlyout(창 레이아웃 루트에 띄우는 상세 카드 · 뒤집기 D11 뒤 자르기) · 막힌 실행 창 · 라이브러리 데이터 캐시(T1) · 미리보기 마크다운에 얹을 외부 라이브러리를 고른다.
- **확신도 범례:** 확실 = 1차 출처 + 독립 교차확증(메인 grounding 또는 codex 재확인이 일치) · 가능성 높음 = 단일 1차 출처, 반박 없음 · 불확실 = 추정·미검 · contested = 리뷰가 반증과 함께 반박.
- **저장소 실측(수집자):** Radix 직접 의존 = react-context-menu · react-dropdown-menu · react-scroll-area · react-tooltip(1.2.9). react-popover · hover-card · dialog 없음. `@floating-ui/{core,dom,react-dom 2.1.8,utils}` 는 Radix 를 타고 들어와 있음. `@tanstack/react-query` 없음. 마크다운 = react-markdown ^9.1.0 + remark-gfm · remark-math · rehype-katex · rehype-highlight · katex. `UsageSlot.tsx`(약 991)에 손으로 쓴 `role="dialog"` 가 있다.

## 결론

| 필요 | 고른 것 | 이유 한 줄 | 차선 | 리뷰가 강제한 정정 |
|---|---|---|---|---|
| 상세 카드(DetailFlyout) | **`@floating-ui/react-dom` 직접**(이미 잠금 파일에 2.1.8, 최신 2.1.9) | TRD §1-10 의 뒤집기→밀기→자르기 사다리(오른쪽 → 왼쪽 → 앵커 위에 겹침)는 미들웨어 순서를 직접 정해야 한다 | `@radix-ui/react-popover` 1.3.1 — 사다리가 증명되면 | ★정정★ 수집자의 첫 추천은 Radix Popover 였으나 **리뷰가 반증했다**(설치된 popper 가 밀기를 뒤집기보다 먼저 건다). 사다리를 먼저 증명하지 못하면 Radix 를 쓰지 않는다 |
| 막힌 실행 창 | **TRD 대로 별도 Tauri 창 유지** | 페이지 안 모달은 작은 main 창을 벗어나지 못한다 | — | 소유 창(`parent` 옵션)은 리뷰가 재확인 |
| 데이터 캐시(T1) | **`@tanstack/react-query` 5.105.0** | TRD 가 이미 확정한 T1 결정(L563 가 손으로 쓴 Zustand 캐시를 거부) | Zustand + eventBus 손수 구현 — TRD 가 거부함 | ★정정★ 「필요하면」이 아니라 **확정**이고 `libraryQueries.ts` 안에 가둔다. 로컬 버스 질의는 **`networkMode: 'always'`** 필수 |
| 모달 | **`@radix-ui/react-dialog` 1.3.1** | 포커스 가둠 · aria · Esc · portal 컨테이너 | 네이티브 `<dialog>` | ★정정★ 네이티브 `<dialog>` 에도 `cancel`/`close` 사건과 CSS 들어옴·나감 애니메이션이 있다(「훅이 없다」는 틀렸다). Radix 겹친 층 Esc 회귀 확인 필요 |
| 마크다운 미리보기 | **react-markdown ^9.1.0 재사용, rehype-raw 없이** | 원시 HTML 은 기본으로 이스케이프된다 | 10.1.0 업그레이드(선택) | ★정정★ 안전 판단은 원시 HTML 만이 아니라 **설정된 플러그인 · components · URL 변환**까지 믿음 경계로 본다 |

## 상세 카드 (뒤집기→밀기→자르기)

| 이름 | 판 | 날짜 | 라이선스 | 이미 잠금 파일에? | 맞음 | 확신도 | 출처 |
|---|---|---|---|---|---|---|---|
| `@floating-ui/react-dom` | 2.1.9(잠금 2.1.8) | 미기재 | 미기재 | 예(Radix 경유) | 미들웨어 순서를 직접 정함 → 사다리에 맞음 | 가능성 높음(수집자 + 메인 grounding 의 의존 사슬 확인) | npm registry |
| `@radix-ui/react-popover` | 1.3.1 | 2026-10-14 | MIT | 아니오(직접 의존 없음) | **사다리에 안 맞음(contested)** | contested — codex 가 설치된 `@radix-ui/react-popper` `dist/index.mjs:139` 에서 `shift()` 가 `flip()` **앞**에 걸리고, 교차축 밀기를 끄고, 가용 크기 변수를 내보내되 제한은 적용하지 않음을 지목 | npm registry · codex 리뷰 |
| `@floating-ui/react` | 0.27.20 | — | — | 아니오 | Radix 와 겹침 | 가능성 높음 | 수집자 |
| `radix-ui` 우산 패키지 · hover-card | — | — | — | — | 저장소는 패키지별 의존 · 클릭 토글이라 hover-card 아님 | 가능성 높음 | 수집자 |

- **메인 grounding:** `@radix-ui/react-popover` 1.3.1 → `@radix-ui/react-popper` 1.4.0 → `@floating-ui/react-dom` ^2.0.0 사슬 확인(지지) — 같은 패밀리라 새 floating-ui 사본이 안 늘어난다는 수집자의 이점은 맞다. 그러나 이점이 사다리 불일치를 이기지 못한다.
- Radix 는 props 만 노출(side · align · avoidCollisions · collisionPadding · collisionBoundary · sticky · hideWhenDetached · onPlaced)하고 사용자 미들웨어를 못 넣는다. 수집자가 적은 「정확한 미들웨어 순서」는 소스를 안 읽은 추정이었고 **리뷰가 소스로 반증했다**.
- portal 컨테이너 = 창 레이아웃 루트(Radix `container` prop 이든 직접 렌더든 같다).

## 막힌 실행 창

- TRD 대로 **별도 Tauri 창**(main 이 소유 · `WebviewEnv::finish` 경유) 유지. 페이지 안 모달로는 작은 main 창을 벗어나 표시할 수 없다.
- 비용: capability JSON · 라우트 · label 정리. 소유 창(`parent`)은 codex 가 재확인(성립).

## 데이터 캐시

| 이름 | 판 | 날짜 | 라이선스 | 이미 잠금 파일에? | 맞음 | 확신도 | 출처 |
|---|---|---|---|---|---|---|---|
| `@tanstack/react-query` | 5.105.0 | 2026-10-26 | MIT | 아니오 | `queryFn` 은 아무 Promise 나 됨 · 버스 푸시 때 `invalidateQueries`/`setQueryData` | 확실(수집자 + codex 재확인 + 메인 peer `react ^18\|\|^19` 확인) | npm registry · TanStack 문서 |
| Zustand + eventBus 손수 | — | — | — | 예 | TRD L563 이 거부 | — | TRD |
| SWR | — | — | — | — | 미검 | 불확실 | — |

- **확정(MAIN DECISION 없음 — TRD T1 이 이미 확정, 리뷰가 확인):** TanStack Query 는 선택이 아니다. 범위는 `libraryQueries.ts` 안으로 한정.
- ★`networkMode: 'always'`★ — 기본 `'online'` 은 오프라인일 때 질의를 멈춘다. 로컬 버스 질의는 HTTP 가 아니라 이 모드가 필수(tanstack.com network-mode 가이드).
- 그 밖: `QueryClientProvider` 필요 · 비 HTTP 라 창 포커스 재조회 · retry 를 끈다. 「두 번째 상태 소유자」라는 저장소 규칙(agentClient + Zustand)과의 긴장은 T1 이 이미 판정한 설계로 본다.

## 모달

| 이름 | 판 | 날짜 | 라이선스 | 이미 잠금 파일에? | 맞음 | 확신도 | 출처 |
|---|---|---|---|---|---|---|---|
| `@radix-ui/react-dialog` | 1.3.1 | 미기재 | 미기재 | 아니오 | 포커스 가둠 · aria · Esc · portal 컨테이너 | 가능성 높음 | 수집자 |
| 네이티브 `<dialog>` | — | — | — | — | WebView2(Chromium)에서 `showModal` 성립 | 확실(수집자 · 리뷰 반박 없음). 「닫힘 훅 · 애니메이션 없음」은 **contested** — HTML 명세에 `cancel`/`close` 사건이 있고 Chrome 블로그에 CSS 들어옴·나감 애니메이션이 있다 | HTML spec · Chrome blog |

- 저장소의 손으로 쓴 `role="dialog"`(`UsageSlot.tsx` ~991)는 대체 후보로만 기록.
- ★겹친 층 Esc★ — `radix-ui/primitives#4143`(DismissableLayer 1.1.19 가 겹친 두 다이얼로그를 함께 닫음). 설치 판을 확인하고 TRD D11 의 「나중에 연 것이 먼저 닫힘」을 시험한다. Popover 를 고르지 않으므로 해당은 모달 ↔ 모달 · 모달 ↔ 다른 Radix 층 조합.

## 마크다운 미리보기

- react-markdown ^9.1.0 재사용, `rehype-raw` 없이 — 원시 HTML 기본 이스케이프(확실 — 수집자 + codex 재확인). 최신 10.1.0(2025-03-07)은 선택 업그레이드. `rehype-sanitize` 6.0.0 은 원시 HTML 을 켤 때만. 발췌 = 문자열 자르기.
- ★정정★ 보안은 원시 HTML 하나로 닫히지 않는다 — 플러그인 · `components` · URL 변환도 믿음 경계다(react-markdown#security). **설정된 플러그인 묶음**(remark-gfm · remark-math · rehype-katex · rehype-highlight · 미리보기가 쓰는 `components` · 링크 URL 처리)을 따로 점검한다.

## 적대 리뷰 결과 (codex · effort high · FIX)

| # | 심각도 | 지적 | 처리 |
|---|---|---|---|
| 1 | HIGH | Radix Popover 가 D11 사다리를 못 준다(밀기가 뒤집기 앞 · 교차축 밀기 끔 · 가용 크기 한계 미적용 — popper `dist/index.mjs:139`) | **반영** — DetailFlyout 은 `@floating-ui/react-dom` 직접. Radix 는 사다리 증명 시에만 |
| 2 | MED | TanStack Query 는 확정 T1 결정이지 선택이 아니다 | **반영** — 확정으로 서술 · `libraryQueries.ts` 한정 |
| 3 | HIGH(누락) | 로컬 버스 질의에 `networkMode: 'always'` 필요 | **반영** — 결론에 필수로 올림 |
| 4 | LOW | 네이티브 `<dialog>` 에 `cancel`/`close` · CSS 애니메이션 있음 | **반영** — 수집자 서술 정정 |
| 5 | MED | react-markdown 의 플러그인 · components · URL 변환도 보안 경계 | **반영** — 설정된 묶음 점검을 공백에 올림 |
| 6 | MED | Radix 겹친 층 Esc 회귀(#4143) | **남김** — 설치 판 확인 · 시험은 구현 파일럿 |
| — | — | 재확인: Promise `queryFn` · 원시 HTML 이스케이프 · 소유 Tauri 창(`parent`) | 일치 |

## 공백 · 실측 필요 (구현 파일럿)

- DetailFlyout 사다리(오른쪽 → 왼쪽 → 앵커 위 겹침 → 자르기)를 `@floating-ui/react-dom` 미들웨어 순서로 만들고 경계 케이스를 실제 창에서 잰다. Radix 를 쓰려면 같은 시험으로 증명.
- WebView2 실측 없음(모달 · 포털 · 창) · 번들 크기 미측정.
- 막힌 실행 창의 Tauri capability JSON 미검증.
- Radix Dialog 겹친 층 Esc(#4143) — 설치 판에서 시험.
- `networkMode: 'always'` 와 포커스 재조회 · retry 끄기를 `QueryClient` 기본값으로 박고 시험.
- 마크다운 플러그인 묶음의 보안 점검. SWR 미검(T1 이 확정이라 필요 낮음).

## 출처

- npm registry(`npm view` — 수집자 + 메인 재실행): `@radix-ui/react-popover` 1.3.1 → `@radix-ui/react-popper` 1.4.0 → `@floating-ui/react-dom` ^2.0.0 · `@tanstack/react-query` 5.105.0(peer react ^18||^19) · `@floating-ui/react-dom` 2.1.9 · `@floating-ui/react` 0.27.20 · `@radix-ui/react-dialog` 1.3.1 · react-markdown 10.1.0 · rehype-sanitize 6.0.0
- 저장소: `package.json` · `node_modules` · `node_modules/@radix-ui/react-popper/dist/index.mjs:139`(codex) · `UsageSlot.tsx` ~991
- TRD `docs/process/S25-preset-library/trd.md` L401 · 418–423 · 511 · 563
- tanstack.com network-mode 가이드 · react-markdown#security · radix-ui/primitives#4143 · HTML spec(`<dialog>` `cancel`/`close`) · Chrome 블로그(`<dialog>` 애니메이션) — URL 은 입력에 이름만 있어 전체 주소는 못 적는다
