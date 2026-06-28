# Research — async subscribe cleanup race (등록 await 중 dispose 누수)

- **상태:** 완료 (cross-family medium: Claude Sonnet 팬아웃 3갈래 + Codex blind 1 → opus 교차/적대)
- **날짜:** 2026-06-28
- **동기:** S14 레이아웃 슬라이스의 `subscribeViewEvents` dispose-during-await 가드가 **dead branch**임이 적대 검증으로 확정됨. 실제 누수(등록 `await` 중 정리가 오면 리스너 영구 누수)를 OSS 관행에 맞게 닫기 위한 설계 근거.
- **확신도 범례:** (확실) 1차/공식 출처 다수 수렴 · (가능성 높음) 합의하나 1차 출처 일부 · (불확실) 단일/지식기반.

## 문제

비동기 구독 등록(`subscribe`/`listen`이 `Promise<Unlisten>` 반환)이 `await`로 끝나기 **전에** cleanup/unmount/dispose가 호출되면, 호출자는 아직 unlisten 핸들을 못 받아 → 등록이 뒤늦게 완료된 리스너가 **영구 누수**.

우리 코드(`src/store/viewStore.ts` `subscribeViewEvents` + `src/store/eventBus.ts:75` `await subscribeViewEvents()`)가 이 형태. 기존 `if (disposed)` 가드는 disposer가 `await` *이후* 반환되어 `disposed`를 set할 경로가 없으므로 dead.

## 발견 (확신도·출처)

- **동기 teardown 핸들 반환이 업계 표준.** RxJS `Subscription`(unsubscribe idempotent·"늦은 unsubscribe는 무해"), zustand `subscribe`(동기 unsub), Node `EventEmitter`(동기 on/off), React `useSyncExternalStore`(subscribe가 unsub 동기 반환 요구) — 모두 동기. 근거: 등록~첫 emission 사이 *취소 불가 window* 제거. (확실)
  - https://rxjs.dev/guide/subscription · https://react.dev/reference/react/useSyncExternalStore · https://github.com/tc39/proposal-observable/issues/163 (`Promise<Subscription>` 안티패턴 기각: 무한 버퍼 + 취소 불가 window)
- **React 표준 race 해법 = `ignore`/`cancelled` 플래그(클로저 로컬).** `isMounted` ref는 안티패턴(동시성 모드 충돌). (확실) https://react.dev/learn/synchronizing-with-effects · https://react.dev/reference/react/useEffect
- **단, 구독 해제는 fetch 결과 무시(ignore)와 다름 — 늦게 온 unlisten은 cancelled여도 반드시 호출.** 안 부르면 누수. (가능성 높음) https://github.com/tauri-apps/tauri/issues/8913
- **Tauri `listen()`은 등록 전 unmount race를 내부적으로 막지 않음.** `invoke('plugin:event|listen')` resolve 후에야 UnlistenFn 생성(packages/api/src/event.ts). 공식 cleanup 예제는 `promise.then(fn => fn())`이나 race window는 남음. (확실) https://v2.tauri.app/develop/calling-frontend/ · https://github.com/tauri-apps/tauri/issues/8913
- **`@tauri-apps/api` 2.6.0+(PR #13306): unlisten 호출 시 리스너 즉시 동기 해제.** 이전 v2.0.x는 unlisten이 비동기 IPC라 별도 race. (이건 우리 등록-await race와 *별개* 층위지만 의존성 버전 점검 가치.) (확실) https://github.com/tauri-apps/tauri/pull/13306 · https://github.com/tauri-apps/tauri/issues/8916
- **AbortController는 "취소 의도 전달"이지 "정리 완료 보장"이 아님.** `abort()` 반환 ≠ 정리 완료. Tauri `listen()`이 signal을 안 받아 래핑 시 내부 ignore-flag 로직이 결국 동일하게 필요 → AbortSignal 도입의 추가 이득 없음. (확실) https://developer.mozilla.org/en-US/docs/Web/API/AbortController
- **`Symbol.asyncDispose`/`await using`(TS 5.2+)은 비동기 정리의 현대 표준** — 단 "취소 핸들을 await 없이 즉시 쥐어야"만 충족하면 별도 적용 불필요. (가능성 높음)

## 교차검증표 (Claude ↔ Codex)

| 클레임 | Claude | Codex | 판정 |
|---|---|---|---|
| 동기 teardown 핸들이 표준 | ✓ | ✓ | 수렴(확실) |
| Tauri listen 내부 race 미방어 | ✓(#8913) | ✓(event.ts 구현) | 수렴(확실) |
| cancelled여도 unlisten 호출해야 | ✓(Claude B) | ✓(`disposed?fn():…`) | 수렴 |
| `{dispose, ready}` = 표준 시그니처? | 아니오(개념적 유사만) | 아니오 | 수렴 |
| `{dispose, ready}` = 우리 케이스 적절? | 조건부 적절 | "Tauri listen 래핑엔 깔끔" | 수렴 |
| AbortSignal이 더 표준? | 한계 동일 | 새 API면 우선, 래핑엔 불요 | 수렴 |

## 우리 적용 (결정 근거)

`subscribeViewEvents`를 `{ dispose: () => void, ready: Promise<void> }` 동기 반환으로. eventBus는 `dispose`를 `await` 없이 즉시 `unlistenFns`에 push(등록 중 정리가 와도 누수 0) + `ready`를 await 후 `initFromBackend`(F-listen 유지).

**필수 계약(함정 회피, 양 family 합의):**
1. `dispose` idempotent (double-dispose noop).
2. `dispose`가 `ready` 전에 불리면, 늦게 도착한 unlisten 핸들을 **즉시 호출**(cancelled 무관 — 안 부르면 누수).
3. `ready`는 hang 금지 (dispose/등록실패에도 정상 종료).
4. `ready` reject 시 부분 등록분 정리(누수 0).

## 공백 / 한계

- 우리 누수의 **실제 트리거 확률은 낮음**(dev HMR + `import.meta.hot.accept` 부재 → 보통 full reload·prod엔 경로 없음). 그러나 설계결함이라 수정 대상.
- `@tauri-apps/api` 현재 버전(package.json) 미점검 — 2.6.0 미만이면 unlisten IPC race(#8916)가 별도로 존재. 우리 수정 범위(등록-await race)와는 다른 층위.
