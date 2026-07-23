# ADR-0102: 부팅 레이스 방지 — managed state는 build 전 등록 + 프론트 부팅 pull 재시도 (main 창 무한 로딩 근절)

- 상태: 확정 (2026-07-23, 근거: 부팅 순서 조사 + 사용자 결정)
- 관련: ADR-0057(레이아웃/창 상태) · ADR-0100(release 패키징 — 이 fix 반영 위해 release/ 재빌드) · `src-tauri/src/lib.rs`(manage 위치) · `src/components/layout/WindowLayout.tsx`·`src/store/viewStore.ts`(부팅 pull) · `src/store/eventBus.ts`

## 맥락
release exe 실행 시 main 창이 프론트 로딩 플레이스홀더("창 로딩 중… (label: main)")에 **영구 고착**하는 현상. 진단 결과:
- 백엔드 레이아웃은 정상 — main 창은 부팅 시 `ViewManager::new()`가 항상 시드하고 디스크 영속이 없다. 재실행하면 `list_tabs('main')`가 즉시 OK를 반환하고 실 UI가 뜬다(일시적). 로그에 panic/poison 0 → 뮤텍스 poison·lock 교착은 배제.
- 근본(가능성 높음) = **Tauri v2 부팅 레이스**: webview가 `builder.build()` 중 프론트 JS/React를 로드해 `setup()`의 `app.manage(LayoutState)` 완료 *전에* `invoke('list_tabs')`를 조기 발화할 수 있다. 그 좁은 창에 걸리면 Tauri가 managed state 미존재 에러를 반환한다.
- 이 일시 실패가 **영구 고착**이 되는 이유(확실) = 프론트가 부팅 pull 실패를 `console.warn`만 하고 **재시도하지 않으며**, main 창은 이후 `window:tabs-updated`(탭 변경 시에만 발화)를 못 받아 **복구 경로가 없다**.
- release에서 주로 관찰된 이유(불확실·가설): release는 프론트가 exe 임베드 미니파이 번들(`tauri.localhost`, 인-프로세스)이라 빨리 로드→invoke 조기 발화, dev는 Vite 개발서버(`localhost:1420`) HTTP 첫 로드가 느려 `setup`이 대개 먼저 끝남. 원래 스파크의 정확한 에러는 그 순간 캡처 못 함(디버그 포트·로그 없는 인스턴스였고 재실행 시 사라짐).

## 결정
1. **결정적 managed state는 build 전에 등록.** `LayoutState`(런타임 불필요·결정적)를 `setup()` 안의 `app.manage(...)`에서 **`builder.manage(LayoutState::new())`(`.build()` 이전)로 이동** → webview가 뜨기 전에 상태가 반드시 존재해 조기 invoke가 "state 없음"을 만날 수 없다(레이스 클래스를 by-construction 제거).
2. **프론트 부팅 pull은 재시도 + 표면화.** `WindowLayout` mount pull과 `initMainWindowFromBackend`의 `list_tabs`(및 뒤이은 `get_view`)를 **bounded 재시도/백오프**로 감싸고, 최종 실패는 삼키지 말고 표면화한다(콘솔 경고 이상 — 사용자 가시/진단 가능). 다른 조기 invoke(런타임 의존 상태 등)의 일시 실패도 자기복구시키고, 비-일시적으로 실패하면 정확한 에러를 남겨 근본을 잡는 계측이 된다.
3. **확신도:** (1)은 원래 스파크가 이 레이스든 아니든 유효하다(실재하는 레이스 클래스를 구조적으로 없앰). 재시도(2)는 남은 부팅 일시 실패의 안전망 + 진단.

## 거부한 대안
- **`visible:false` 후 setup 끝나고 `show()`** — 버림. 더 무겁고(창 표시 지연·깜빡임), main 창만 가릴 뿐 다른 조기 invoke(다른 managed state)는 여전히 노출된다. build-전 `manage`가 더 근본적·범용.
- **프론트 재시도만(백엔드 그대로)** — 버림. 증상만 가리고 레이스는 상존한다. by-construction 제거(1)를 우선하고 재시도는 방어로 병행.
- **`DaemonClient`까지 build 전 이동** — 불가/비목표. DaemonClient는 tokio 런타임이 필요해 `setup()`에서 생성해야 한다. 그 상태에 닿는 조기 invoke의 일시 실패는 프론트 재시도(2)가 커버한다.

## 근거
- 부팅 순서 조사(read-only): `app.manage(LayoutState)`가 `setup()` 안(`lib.rs`)이고, Tauri v2는 webview를 `build()` 중 로드해 조기 invoke가 setup을 앞지를 수 있음을 확인. lock 교착·panic-poison은 코드·로그로 배제.
- 모든 관찰과 정합: 일시적 · 재실행 성공 · poison/panic 로그 0 · 캡처 실패(부팅 첫 순간) · release-prone(빠른 임베드 프론트 로드).

## 영향 / 불변식
- **★부팅 조기 invoke가 닿는 결정적 managed state는 `.build()` 전에 등록한다★** — `setup()` 안으로 되돌리면 이 레이스가 재발한다. 이동 지점에 `// ADR-0102` 앵커 주석 필수(다음 세션의 rot·회귀 방지).
- **main 창 상태는 이벤트 복구 경로가 없다** → 부팅 pull은 one-shot 금지(재시도 필수). `window:tabs-updated`는 탭 변경 시에만 발화하므로 초기 실패를 메우지 못한다.
- **release/ 재빌드 필요:** 이 fix는 소스 변경이라, 배포용 `release/` exe에 반영하려면 `scripts/build-release.ps1`로 재빌드해야 한다(ADR-0100). 재빌드 전 release exe는 여전히 옛 코드다.
- 검증: 코드 게이트 + 프론트 재시도 단위테스트(실패 N회 후 성공→win 채워짐) + release 재빌드 후 exe 부팅이 실 UI에 도달하는지 실측.
