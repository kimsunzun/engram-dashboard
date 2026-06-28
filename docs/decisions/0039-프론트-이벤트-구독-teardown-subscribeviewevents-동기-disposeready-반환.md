# ADR-0039: 프론트 이벤트 구독 teardown — subscribeViewEvents 동기 dispose+ready 반환

- 상태: 확정 (2026-06-28, 근거: cross-family OSS 리서치 + opus/Codex 적대 리뷰)
- 관련: `src/store/viewStore.ts`(subscribeViewEvents) · `src/store/eventBus.ts`(initEventBus) · `docs/research/async-subscribe-cleanup-race-2026-06-28.md` · ADR-0035(레이아웃 권위=src-tauri) · CLAUDE.md "프론트 구조·제어 표면"

## 맥락
S14 레이아웃 슬라이스에서 `subscribeViewEvents`는 `async (): Promise<() => void>`였다 — `await Promise.all([listen(...), listen(...)])`로 두 emit(layout:updated/view:list-updated) 등록을 마친 *뒤에야* disposer를 반환한다. 호출자 `eventBus.initEventBus`는 `unlistenFns.push(await subscribeViewEvents())`로 그 disposer를 받았다.

문제: 등록 `await`가 pending인 동안(또는 HMR dispose 콜백이 등록되기 전) 정리(HMR `import.meta.hot.dispose` / 재-init)가 `unlistenFns.forEach(fn=>fn())`을 돌면, 그 시점 `unlistenFns`엔 viewStore disposer가 아직 없어(push 전) 정리에서 빠지고, await가 늦게 끝나 등록 완료된 리스너가 **영구 누수**된다. 기존에 둔 `if (disposed)` 가드는 **dead branch**였다 — `disposed`를 true로 만드는 코드가 반환된 disposer뿐인데 disposer는 await 후에야 호출자 손에 들어가므로, await 중엔 `disposed`가 절대 true가 될 수 없다. 즉 가드가 막으려던 race가 구조적으로 그 가드를 건드릴 수 없었다. (cross-family 적대 검증으로 확정: opus + Codex.)

실행 위험은 낮다(트리거가 dev HMR 한정 — `import.meta.hot.accept` 부재로 보통 full reload, prod 빌드엔 경로 없음). 그러나 거짓 주석("이 누수는 막힌다")이 다음 세션에 *해결됨*으로 오인을 유발하는 설계결함이라 닫는다.

## 결정
`subscribeViewEvents`를 **동기 함수** `(): { dispose: () => void; ready: Promise<void> }`로 바꾼다.
- 호출 즉시 `dispose`를 반환 → 호출자가 등록 `await` 없이 즉시 `unlistenFns`에 push(누수 차단의 핵심). 등록(`listen()` 2개)은 백그라운드로 시작.
- `ready`(Promise<void>)는 두 등록이 settle된 뒤 resolve → 호출자가 이걸 await한 다음 후속 init(F-listen 보존: 등록 완료 전 도착 emit 누락 방지).
- 내부 `adopt(fn)`가 갓 등록된 unlisten 핸들을 받아 — `disposed`면 *즉시 호출*(늦게 도착한 핸들 해제), 아니면 `handles`에 보관. `dispose`는 `handles`만 비우며 호출(idempotent).

호출자 `eventBus`도 함께: ① `dispose`를 ready await *전*에 `unlistenFns`에 push ② **`import.meta.hot.dispose` 콜백도 ready await 전에 등록**(ready pending 중 HMR에서도 dispose 호출 경로 보장 — 이게 빠지면 dispose를 일찍 push해도 호출할 주체가 없어 무효) ③ `await ready`를 try/catch로 감싸 reject 시 `viewSub.dispose()` 호출로 성공 부분분 정리 + agentClient 구독(ADR-0011 도메인)과 격리.

## 거부한 대안
- **async 반환 + `if (disposed)` 가드 (기존)** — dead branch. disposer가 await 후 반환되므로 await 중 `disposed`를 set할 경로가 없어 가드가 영원히 안 탄다. 누수를 못 막으면서 "막힌다"고 주석으로 거짓 주장 → 채택 불가.
- **AbortController/AbortSignal** — Tauri `listen()`이 signal 인자를 받지 않아 우리가 직접 ignore-flag 로직을 내부에 둬야 하므로 추가 이득이 없다. 게다가 `abort()` 반환은 "취소 의도 전달"이지 "정리 완료 보장"이 아니다(cross-family 합의). 새 구독 API를 설계한다면 우선이나, non-abortable Promise API(listen)를 *감싸는* 우리 경우엔 부적합.
- **async dispose (`dispose(): Promise<void>`, ready 없음)** — 취소도 되고 완료 대기도 되지만, "취소 핸들을 await 없이 *즉시* 손에 쥐어야 한다"(등록 await 중 누수를 막는 핵심 요구)를 못 채운다. dispose를 받으려 await하면 그 사이가 곧 누수 윈도다.

## 근거
- **OSS 표준(수렴, cross-family):** RxJS `Subscription`·zustand `subscribe`·Node `EventEmitter`·React `useSyncExternalStore`가 모두 teardown 핸들을 *동기* 반환한다 — 등록~첫 emission 사이 취소불가 window를 없애기 위함. TC39 Observable 제안은 `Promise<Subscription>`(async 반환)을 무한 버퍼 + 취소불가 window로 기각. 상세·출처 = `docs/research/async-subscribe-cleanup-race-2026-06-28.md`.
- **Codex 판정:** "non-abortable Promise API(Tauri listen)를 감싸면서 + 동기 취소 핸들 + 등록완료 await가 둘 다 필요한 경우 `{dispose, ready}`가 깔끔한 해결책."
- **적대 리뷰 적출:** 1차 — 기존 가드 dead branch 확정. 3차(FIX) — Codex가 "HMR dispose 콜백이 ready await *뒤*에 등록돼 ready pending 중 HMR이면 누수가 여전"임을 적출(opus는 못 봄, cross-family 가치). 수정으로 콜백을 ready 전으로 이동해 닫음.
- **실측:** 부팅 시 기본 View 1 빈 슬롯이 `ViewLayoutRenderer`로 정상 렌더(cdp `eval`로 `list_views` + 메인 캔버스 `Slot … — empty` 확인) → 구독→ready→init 흐름 무회귀.

## 영향 / 불변식
- `subscribeViewEvents` 시그니처 = `{ dispose: () => void; ready: Promise<void> }`(동기). 호출자는 **dispose를 await 없이 즉시 확보**해 정리 경로에 등록해야 한다.
- **계약 4개(어기면 누수/hang):** ① `dispose` idempotent(double-dispose noop) ② `dispose`가 `ready` 전에 불리면 늦게 도착한 핸들을 `adopt`가 즉시 호출(cancelled 무관 — 미호출=누수) ③ `ready` hang 금지(dispose 먼저·등록 reject에도 정상 종료) ④ `ready` reject 시 성공 부분분 정리는 **호출자의 dispose 호출 책임**.
- **eventBus 통합 불변식:** dispose push·`import.meta.hot.dispose` 콜백 등록 둘 다 **`await ready` 전**에 끝낸다. `await ready`는 try/catch로 격리(layout 구독 실패가 agentClient 구독을 막지 않음). F-listen(등록 완료 후 `initFromBackend`) 순서 유지.
- 어기면: dispose를 ready 뒤로 늦추거나 HMR 콜백을 ready 뒤에 등록하면 ready-pending-중-HMR 누수가 재발한다. ready를 await 안 하고 init하면 등록 전 emit 누락(F-listen 위반).
