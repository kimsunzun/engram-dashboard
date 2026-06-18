# ADR-0025: UI 부팅 1회 데몬 ensure 유지 — ADR-0024 C3("UI ensure 금지") 폐기

- 상태: 확정 (2026-06-18, dashboard7 세션 — 사용자 결정)
- 관련: ADR-0024(C3 폐기 대상)·ADR-0023(토폴로지)·ADR-0021(on-demand·무재시작)·CLAUDE.md §5 · `src/api/clientFactory.ts`(bootstrapDaemonIfNeeded)·`src/App.tsx`
- 범위: daemon 모드에서 "UI가 데몬을 켜도 되나"의 결정. ADR-0024의 C3만 번복한다(C1·C2·C4·데이터 위치는 유효).

## 맥락
ADR-0024 C3은 "데몬 desired-state는 owner(tray-host)가 소유, UI는 직접 ensure 금지, 연결만"으로 정하고 현 `bootstrapDaemonIfNeeded`(UI 부팅 시 데몬 ensure)를 제거하라 했다. 사용자 검토에서 이게 의도와 반대임이 드러났다 — **"UI는 켜졌는데 데몬은 없다"는 상태가 사용자 멘탈모델상 모순**이다(UI=모니터, 데몬=작업장; 모니터를 켜면 작업장은 당연히 있어야 함).

## 결정
- **UI는 부팅 시 데몬을 1회 ensure 한다(`bootstrapDaemonIfNeeded` 유지).** "UI 열기 = 데몬 자동 동반"이 사용자 의도다.
- **자동은 "부팅 1회"지 "사망 시 되살리기"가 아니다.** 데몬이 (외부 kill 등으로) 죽으면 UI는 자동 재시작하지 않는다 — 명령 경로(`ensureReady`)는 attach-only라 데몬을 못 깨운다(ADR-0021). 트레이 아이콘이 활성/비활성으로 데몬 생사를 표시하고, 부활은 명시 "데몬 켜기"/UI 재실행으로만. ← 사용자가 겪은 "막아도 리스타트"가 재발하지 않는 지점.
- **데몬 ensure 주체는 둘(tray-host "데몬 켜기" + UI 부팅).** 데몬은 싱글톤(lockfile, ADR-0024 C2)이라 누가 ensure하든 살아있으면 no-op → 다중 ensure가 안전하다. "desired-state 단일 owner"는 ADR-0024 C3이 우려한 다중 ensure race를 막기 위함이었으나, 싱글톤 lockfile이 그 race를 이미 무력화하므로 UI ensure를 *제거*할 필요가 없다.
- **종료 중 재spawn race는 C4로 막는다(C3로 막지 않는다).** 전체 종료 신호(reason=full_shutdown) 수신 시 UI가 reconnect/ensure를 차단한다(ADR-0024 C4 유지). 단계1에서는 전체 종료가 taskkill(둘 다 동시 종료)이라 race 자체가 없고, 정밀 가드는 단계3.

## 거부한 대안
- **ADR-0024 C3 원안(UI ensure 완전 제거, tray-host만 ensure):** UI 단독 실행 시 "데몬 꺼짐" 상태가 되어 사용자 의도(UI=데몬 자동 동반)에 정면 배치. C3의 명분이던 "다중 ensure race"는 데몬 싱글톤 lockfile(C2)로 무력화되므로, ensure를 통째로 제거하는 비용까지 치를 이유가 없다. → 폐기.
- **데몬 사망 시 UI 자동 재시작(supervision):** 사용자가 명시 거부("UI 한 번 띄울 때만"). 자동재시작 폐기(ADR-0021)와도 일치.

## 근거
- 사용자 직접 결정(dashboard7 세션). 현 코드(`bootstrapDaemonIfNeeded` + attach-only `ensureReady`)가 이미 의도대로 동작 → 코드 변경은 "제거하지 않음"이 결론.
- 싱글톤 보증은 ADR-0024 C2(lockfile{PID·port·generation} stale 검사)에 의존. 그 보증이 깨지면 다중 ensure race가 되살아나므로 C2는 단계3에서 반드시 구현.

## 영향 / 불변식
- **ADR-0024 C3은 폐기.** ADR-0024 본문 C3 줄에 폐기 표기됨.
- 단계1 계획에서 "UI bootstrap의 데몬 ensure 제거" 항목을 삭제한다(`bootstrapDaemonIfNeeded`·`App.tsx` 부팅 ensure는 유지). `clientFactory` 기본 모드 flip(embedded→daemon)은 그대로 진행.
- "막아도 리스타트 안 됨" 불변식은 **"UI ensure는 부팅 1회 + 명령 경로 attach-only"**에 load-bearing. 향후 명령 경로가 데몬을 깨우게 바꾸면 이 불변식이 깨진다.
- 종료 중 재spawn 차단(C4)은 여전히 필수 — 단계3에서 정밀 구현.
