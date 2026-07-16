# ADR-0056: 탭 전환 렌더링 전략 — keep-alive(A) + 보이는 슬롯만 WebGL 좌석, 렌더모드(dom/xterm) 교체 레버

- 상태: 확정 (2026-07-09, 근거: 같은-모델(xterm.js) OSS 조사 + 실 WebView2 WebGL 좌석 실측 + 사용자 결정)
- 관련: PRD `docs/process/B-wezterm-tabs/PRD.md`(Phase 2 탭) · ADR-0046(뷰별 라우팅·replay — 콜드 탭 탈출구) · ADR-0055(command registry — 렌더모드 레버가 여기 얹힘) · CLAUDE.md §5(LLM 제어)·§2(슬롯이 capability로 렌더러 선택) · `src/components/slot/TerminalSlot.tsx`(WebglAddon + onContextLoss→DOM 폴백) · `src/store/eventBus.ts`(`__engramLayout.setRenderMode`/`toggleDomMode` — 렌더모드 기존 표면) · step-log 2026-07-09

## 맥락
WezTerm식 창>탭>슬롯(PRD B-tabs)에서 한 창은 활성 탭 하나만 보여준다. **안 보이는(숨은) 탭의 터미널을 어떻게 다루나** — 살려두나(즉시 전환·데이터 보존) vs 버렸다 되살리나(가벼움·전환 지연) — 를 정해야 한다. 우리 슬롯 터미널은 xterm.js + WebglAddon(GPU 렌더러)을 쓴다. 브라우저(WebView2/Chromium)는 **동시 활성 WebGL 컨텍스트를 하드 상한**으로 제한하므로, 터미널을 많이 살려두면 이 상한에 부딪힐 수 있다. 사용자 가정 최대 스케일 = **창 3 × 탭 2 × 슬롯 4** → 동시에 *살아있는* 터미널 최대 24개, 동시에 *보이는* 터미널 최대 12개(3창 × 활성탭1 × 4슬롯).

## 결정
- **탭 전환 = keep-alive(A).** 비활성 탭의 xterm 인스턴스(버퍼·데이터)를 **dispose하지 않고 살려둔다**(숨김만). 출력은 숨은 상태에서도 xterm 내부 버퍼에 계속 누적되고, xterm의 IntersectionObserver가 렌더 루프만 일시정지한다 → **전환 즉시·무손실, replay 불필요.**
- **WebGL 좌석은 "보이는 슬롯"에만.** GPU 컨텍스트(=희소 좌석)는 활성/보이는 터미널에만 부여하고, **탭을 숨길 때 그 슬롯의 WebGL은 반납**한다(데이터 버퍼는 유지, 좌석만 놓음). 숨은 탭은 어차피 렌더 안 하므로 좌석 반납에 시각적 손해가 없다. 보이는 것 최대 12 ≤ 좌석 16이라 **상한을 절대 안 넘는다.**
- **렌더모드(dom/xterm) 교체 = 레버로 노출.** 슬롯 렌더러를 dom(평문)/terminal(xterm)로 강제하는 오버라이드는 **이미 `__engramLayout.setRenderMode`/`toggleDomMode`로 존재**한다(프론트 전용 override). 이걸 Phase 1 command registry(ADR-0055)에 command로 감싸 사람·LLM이 동일하게 호출하게 한다(§5). 이 레버가 "일부 터미널을 일부러 DOM으로 돌려 좌석 아끼기" + "구조화/트리 콘텐츠는 DOM"의 수동 제어 지점이다.
- **사전 풀링/tiering은 지금 안 만든다.** 위 규칙(보이는 것만 좌석)만으로 사용자 최대 스케일에서 안전하므로, 정교한 렌더러 풀·콜드탭 replay tiering은 **실측으로 문제가 드러나기 전까지 구현하지 않는다**(YAGNI). 필요 시 탈출구는 이미 있다(ADR-0046 뷰별 replay = 콜드 탭 재구성 경로).

## 거부한 대안
- **B(replay 우선) — 비활성 탭 언마운트 후 전환 시 백엔드 버퍼에서 재구성:** 메모리·좌석 최소지만 전환마다 재구성 지연 + "항상 살아있음" 체감 상실. 같은-모델 앱(VS Code/Tabby/Hyper) 중 아무도 일상 탭 전환에 이 방식을 안 쓴다(reload/재연결 전용). 우리 스케일(≤12 보임)에선 A로 충분 → 거부. (단 콜드/대량 탈출구로는 남겨둠 = ADR-0046.)
- **사전 3-tier 렌더러 풀링(활성=WebGL / 웜=DOM / 콜드=백엔드):** 같은-모델 생태계 어디도 사전 풀링을 안 한다 — 전부 keep-alive + context-loss 반응형 폴백. 사용자 최대 스케일이 좌석 16 안에 드는데 미리 풀을 짜는 건 over-engineering(검증 안 된 최적화 = 껍데기만, CLAUDE.md §0) → 거부(실측 문제 시 재검토).
- **WezTerm 네이티브 모델 직이식(mux가 그리드 소유·GUI는 stateless 렌더러):** WezTerm은 네이티브 GPU 앱이라 파서(mux)와 렌더러가 분리돼 "상태만 유지, 그리기만 lazy"가 자연스럽다. xterm.js는 파서+렌더러가 한 `Terminal` 객체라 그 분리가 그대로 안 되고, 버퍼를 다른 데이터로 스왑하는 API도 없다 → 직이식 불가. (같은-모델 = xterm.js 앱을 봐야 한다는 근거.)
- **naive-A(숨은 탭도 WebGL 유지):** 숨겨도 WebGL 좌석은 반납 안 하면 그대로 카운트된다 → 24 살아있으면 16 초과 → 오래된 8개가 컨텍스트 소실(반응형 DOM 폴백은 되나 비결정적·churn). "숨길 때 좌석 반납"으로 결정적으로 회피 → naive-A 거부.

## 근거
- **실측(정본):** 실 WebView2에서 WebGL2 컨텍스트를 40개까지 생성 → **정확히 16개만 생존, 24개는 오래된 순으로 소실**(cdp eval, 2026-07-09). 우리 런타임의 GPU 좌석 = 16 확정. 보이는 것 최대 12 ≤ 16 → 항상 안전.
- **같은-모델 OSS 조사(xterm.js, WezTerm 제외):** VS Code 통합 터미널(`terminalInstance.ts`: hidden에도 `xterm.raw.write` 무조건, dispose 안 함), Tabby(`left:-1000%` 오프스크린·`*ngFor` 유지), Hyper(오프스크린 + IntersectionObserver), Theia(Phosphor visibility) — **전부 keep-alive(A)**. WebGL 상한은 모두 **반응형**(context-loss → canvas/DOM 폴백), 사전 풀링 없음. 우리 `TerminalSlot`도 이미 `onContextLoss(()=>webgl.dispose())`→DOM 폴백 보유.
- **"안 보이면 안 그린다":** xterm IntersectionObserver가 오프스크린/숨김 시 렌더 루프를 자동 일시정지 → 숨은 탭의 그리기 비용 ≈ 0. 그래서 살려둠이 싸고, 좌석만 반납하면 유일한 하드 자원(WebGL 좌석)도 안 걸린다.

## 영향 / 불변식
- **Phase 2 탭 구현 불변식:** 탭 숨김 시 xterm 인스턴스는 유지, **WebGL(WebglAddon)은 dispose**. 탭 표시 시 재부착 + `fit()`+`refresh()`. 보이는 슬롯 수는 설계상 항상 ≤ 좌석 16(창3×슬롯4=12) — 이 상한을 깨는 레이아웃(창/슬롯 최대치 상향)은 이 ADR 재검토를 요한다.
- **콜드/대량 탈출구:** 스케일이 이 가정을 넘어 실측 문제가 나면, 비활성 탭 언마운트 + ADR-0046 뷰별 replay로 재구성(B tiering)을 얹는다 — 지금은 미구현, 인프라만 존재.
- **렌더모드 레버 = command:** dom/terminal 강제는 `setRenderMode`(기존)를 ADR-0055 레지스트리 command로 노출(예: `slot.setRenderMode`). 슬롯 콘텐츠가 터미널이 아닌 것(에이전트 트리·diff·구조화 뷰)은 애초에 DOM이라 WebGL 좌석 0 — 슬롯 콘텐츠 종류 모델(후속 PRD)과 정합.
- **load-bearing 앵커:** `TerminalSlot.tsx`의 WebglAddon 로드/dispose 경로에 `// ADR-0056` 앵커를 단다(탭 가시성 연동 시).

## 구현 확정·검증 (2026-07-16)
Phase 2 가시성 연동을 실제 구현·검증했다(`TerminalSlot.tsx`). 설계(위)는 그대로, 구현 세부 2가지가 확정됐다:

- **좌석 반납은 `loseContext()`를 명시 호출해야 결정적이다(load-bearing).** xterm `WebglAddon.dispose()`는 canvas/layer만 떼고 `WEBGL_lose_context.loseContext()`를 부르지 않는다(소스 확인 · PixiJS #8215) → dispose만 하면 좌석이 GC까지 비결정적으로 점유된다. 그래서 dispose *이전에* 직접 `loseContext()`를 부른다.
- **GL 컨텍스트는 attach 시점에 캡처한다.** React는 passive `useEffect` cleanup *이전에* host ref를 null로 비우므로, 언마운트 cleanup에서 `containerRef`로 canvas를 찾으면 이미 null이라 좌석 반납이 실패한다 → attach(=보임) 순간 GL 컨텍스트를 ref에 잡아두고 release가 그걸 쓴다.
- **트리거 = IntersectionObserver.** 숨김(display:none) 전환에 IO가 발화함을 실측 확인(self + 조상 div 토글 = WindowLayout 방식 둘 다).

**실측(정본, cdp — 실 WebView2):** ① loseContext한 8개 반납 후 새 8개 생성 시 안 건드린 8개 전원 생존 = 좌석 결정적 반납 확인(상한 16 재확인). ② IO가 display:none 조상 토글에 발화. ③ E2E: 스폰 터미널 attach(WebGL 1) → 탭 숨김(canvas 0, 인스턴스 생존) → 복귀(WebGL 1 재부착) 무손실 사이클.

**구현 중 추가로 거부한 대안:**
- **detach-DOM-but-keep-instance(DOM만 떼고 인스턴스 유지):** 살아있는 Terminal이 노드를 강참조해 GC 안 됨 → 메모리 이득 ≈ 0인데 재attach 리스크만 짐. 대형 앱 선례도 없음 → 거부(H=display:none 유지).
- **dispose-instance-on-hide(숨길 때 인스턴스째 파괴):** warm 버퍼 상실 → 복귀 시 replay 재구성(데몬 캐시 상한까지만) + 깜빡임. keep-alive 즉시·무손실 이점 포기라 스케일 전용 → 거부.
