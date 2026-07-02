# 터미널 렌더 (xterm.js on WebView2) — 렌더러/글리프 클리핑 + "검은 화면" 근본원인

> 상태: 조사 완료 + 실측 검증. 날짜: 2026-07-02. 방법: /research medium (Claude Sonnet 2갈래 BLIND + Codex 독립 교차) + 실제 앱 cdp 실측.
> 확신도 범례: **확실**(1차 출처+실측 수렴) / **가능성높음** / **불확실**.

## 배경 (조사 발단)

claude Code 같은 전체화면 TUI를 xterm.js 슬롯에 띄웠을 때, 블록/박스드로잉 글리프로 그린 로고 **첫 행 상단 픽셀이 시각적으로 깎임**(cosmetic). 우리 스택: `@xterm/xterm`(당시 6.0.0) + `@xterm/addon-fit`만 로드 = **기본 DOM 렌더러**, WebView2(Chromium) on Windows, 분수 DPI(devicePixelRatio 비정수, rowHeight 15.2px).

## ★ 이번 세션의 가장 큰 교훈 — "검은 화면"의 진짜 원인은 렌더러가 아니었다 ★

디버깅 중 터미널이 통째로 검게 나온 걸 **렌더러 문제로 오진**하고 webgl/xterm-bump 삽질을 크게 했으나, **실제 원인은 데몬 상태 오염**이었다(사용자가 "claude 실행이 안 되는 것"이라고 정확히 지적).

- 앱을 세션 내내 여러 번 재시작 → **orphan claude 프로세스 18개 누적** + **재attach로 엉킨 데몬**(engram-dashboard-daemon 단일 인스턴스가 재시작 간 persist). 출력 라우팅(구독/replay)이 깨져 슬롯이 빈 채 커서만 떴다.
- **해결:** 데몬 프로세스만 kill(자식 agent는 Job Object로 정리) → baseline로 clean 재기동 → fresh claude가 **즉시 정상 출력·렌더**. **확실**(실측).
- **운영 수칙:** 렌더가 빈 슬롯/커서만이고 에이전트가 "Running"인데 출력이 없으면 → **렌더러를 의심하기 전에 데몬/orphan 프로세스부터 확인**(`Get-Process claude` 개수, 데몬 살아있는지). 반복 재시작이 누적 오염을 만든다. `claude.exe` 무차별 kill 금지(내 세션·사용자 다른 세션까지 죽음) — **engram 데몬만** 죽여 Job Object 정리.

## 렌더러/글리프 클리핑 — 조사 결론 (cross-family 수렴)

| 클레임 | Claude | Codex | 판정 |
|---|---|---|---|
| DOM 렌더러는 `customGlyphs` **미지원** → 블록/박스드로잉을 폰트에 위임 → 분수 DPI에서 상단 클리핑 | 확실 | 확실 | **수렴·확실** |
| canvas/webgl은 이 글리프를 **직접 벡터로 그림**(customGlyphs 기본 true) → 폰트·DPI 의존 끊음 = 클리핑 제거 | 확실 | 가능성높음 | **수렴** |
| 성숙 OSS(ttyd·VS Code·Hyper)는 DOM 안 씀 — canvas/webgl 사용 | 확실 | 확실 | **수렴** |
| 지속 클리핑은 PTY 크기 문제가 아니라 렌더러 문제 | 확실 | 가능성높음 | **수렴**(실측 일치) |

관련 이슈: xterm.js #2409(box/block pixel-perfect→customGlyphs), #967(lineHeight≠1 + DPR≠1 행 잘림), #3807(DOM 글리프 상단 잘림), #4813(DOM emoji 클리핑, "webgl은 안 잘림"). 분수 DPI fix = v5.0.0 PR #3926/#4009/#4105(canvas/webgl 전용).

## ★ 클리핑 정식 해결이 이 환경에서 막힌 지점 (실측) ★

조사대로 canvas/webgl이 정답이지만, **우리 환경에서 셋 다 벽에 막힘** — 그래서 이번엔 클리핑을 **미해결로 두고 baseline(DOM, cosmetic clip) 유지**:

1. **WebGL** — `@xterm/addon-webgl` 로드 시 이 WebView2에서 **텍스트 글리프(텍스처)를 못 그림**(커서 사각형만, 글리프 검음). fresh claude로도 재현 → claude 실행 배제된 순수 webgl 텍스처 실패. **확실**(실측). GPU/ANGLE 텍스처 경로 제약 추정. *단, 이 실측은 아래 3번(beta 승격)과 뒤섞였을 수 있어 재검증 필요 — 아래 do-not.*
2. **폰트 함정(별개 실버그, 확인됨):** `new Terminal({ fontFamily: 'var(--font-terminal)' })` — canvas/webgl은 글리프를 canvas 2D `ctx.font`로 rasterize하는데 **canvas는 CSS `var()`를 해석 못 함**(실측: `13px var(--font-terminal)` → `10px sans-serif`로 폴백). DOM 렌더러만 CSS cascade로 var를 풀어줘 됐던 것. **canvas/webgl 쓰려면 생성 시점에 실제 폰트 문자열로 해석해 넘겨야 함**(`getComputedStyle(root).getPropertyValue('--font-terminal')`). **확실**. (단 폰트 픽스해도 1번 webgl 텍스처 실패는 남았음.)
3. **Canvas 애드온** — `@xterm/addon-canvas`는 stable 0.7.0/beta 0.8.0-beta.48 **둘 다 peer `@xterm/xterm ^5.0.0`** = xterm 6에선 사실상 **dead-end**(6.x 대응 릴리스 없음). CDP 캡처는 되는데(2D) 버전이 막음.
4. **xterm beta 승격 = regression** — 6.0.0 → 6.1.0-beta.288로 올렸더니 **DOM 렌더러로도 렌더 깨짐**. beta로 점프하지 말 것. (되돌려 baseline 6.0.0 복원함.)

**CDP 실측 한계:** `scripts/cdp.mjs`의 `Page.captureScreenshot`은 **WebGL(GPU) canvas를 캡처 못 함**(DOM·canvas는 잡음). webgl은 커서조차 스샷에 안 찍힘 → webgl 검증은 사람 눈 필요. **canvas/DOM은 CDP 자가검증 가능.** ⇒ 자율 작업엔 CDP로 잡히는 렌더러(canvas/DOM)를 선호.

## 초기 PTY winsize (별개 latent 개선)

OSS 표준 = **client-first**: 프론트에서 실제 cols/rows 확정 → spawn 시점에 크기 주입. Rust `portable_pty::openpty(PtySize{rows,cols,..})`를 `spawn_command` 전에 호출(두 호출 분리돼 있음). ttyd(protocol.c는 클라 {cols,rows} 수신 후 spawn)·VS Code·node-pty 모두 client-first. 우리는 현재 스폰 후 늦은 1회 resize. alt-screen TUI는 초기 크기 불일치가 **일시적 첫 프레임 글리치**로 끝나고 SIGWINCH로 복구되므로 *지속 클리핑의 원인은 아님*. **가능성높음**.

## 권고 (다음에 클리핑 재도전 시)

- CDP로 자가검증 가능한 **canvas 렌더러**가 최선이나 xterm 6에선 버전 막힘 → (a) canvas 애드온의 xterm6 대응 릴리스 등장 대기, 또는 (b) webgl을 쓰되 **폰트 해석 픽스 + 사람 눈 검증** 전제로 재시도(단 이 WebView2 텍스처 실패가 폰트 때문인지 GPU 때문인지 3번과 분리해 재검증 필요), 또는 (c) DOM 유지 + cosmetic clip 수용(사용자 "대충 OK").
- **beta xterm 승격 금지.** 렌더러 바꿀 땐 baseline에서 한 조각씩, CDP 캡처로 검증하며.

## do-not / 재검증 필요
- webgl 텍스처 실패 실측이 **폰트 버그 + beta 승격과 뒤섞여** 있었음 → "webgl이 이 WebView2에서 근본 불가"는 **미확정**. 깨끗한 baseline(xterm 6.0.0 stable) + 폰트 해석 픽스 상태에서 webgl 단독 재검증 필요(단 스샷 캡처 불가라 사람 눈).

## 출처 (핵심)
- xterm.js: #2409, #967, #3807, #4813 · release 5.0.0/5.1.0 노트 · ITerminalOptions(customGlyphs/lineHeight/letterSpacing) 문서
- ttyd Client-Options wiki(rendererType) · ttyd src/protocol.c(client-first) · VS Code PR #84440·issue #106202(webgl 전환) · Hyper term.tsx · portable-pty docs.rs
