# 사용량 조회 429 거절 기한 — 영속하나, 어디에 두나 (2026-10-08)

- 상태: 조사 완료(light) · 결정 대기
- 방법: `/research light` — 수집자 1(소스 직접 읽기) · 메인 grounding 스팟체크(orca `service.ts:91` · Claude-Code-Usage-Monitor `api_usage.py` 대조 — 지지) · cross-family 적대 리뷰 없음(light)
- 확신도 범례: 확실 = 코드로 확인 · 가능성 높음 = 코드 일부 확인/부재 근거 · 불확실 = 검색만

## 질문

데몬이 claude·codex 사용량을 조회하다 429(`Retry-After` 약 1시간 관측)를 받으면 거절 기한을 `daemon\state\usage_rejects.json` 에 따로 저장한다(`crates/engram-dashboard-daemon/src/usage_service/reject_store.rs` · TRD S21-usage-limit-slot §3 #30 — PRD 리뷰 2차의 「재시작 거절」 지적을 반영한 기본값이고 사용자 결정은 아니었다). 사용자 물음: 메모리만으로 충분하지 않나 · 영속한다면 기능마다 파일을 만들지 말고 한곳에 둬야 하지 않나.

## 결론

- **피어는 갈린다.** 사용량 엔드포인트를 부르면서 429 를 다루는 도구 넷 중 둘은 기한을 재시작 너머로 들고 가고(CodexBar · Claude-Code-Usage-Monitor) 둘은 메모리에만 둔다(orca · ClaudeBar). 아예 429 를 따로 다루지 않는 도구도 둘이다(paseo · t3code).
- **기한만 담는 전용 파일을 두는 곳은 없다.** 영속하는 둘은 ① 범용 키-값 저장소에 게이트당 키 하나(CodexBar — macOS UserDefaults) 또는 ② 마지막 값 캐시와 같은 파일에 칸 하나(Claude-Code-Usage-Monitor — `latest.json` 의 `retry_after_epoch`)로 둔다.
- orca 는 같은 1시간 창을 주석으로 알면서도(`src/main/rate-limits/service.ts:91`) 기한을 메모리에만 두고, 429 동안 옛 값을 최대 24시간 보여 주는 쪽을 택했다.

## 사례

| 도구 | 429 처리 | 기한 위치 | 작은 상태 저장 방식 | 확신도 | 근거 |
|---|---|---|---|---|---|
| CodexBar | 있음(없으면 5분) · 사용자 새로고침은 우회 | **영속** — UserDefaults 키(`claudeOAuthUsageRateLimitBlockedUntilV2.<토큰 해시>` 등) | 게이트 = UserDefaults 키 · 큰 스냅숏 = 기능별 JSON | 확실 | `ClaudeOAuthUsageRateLimitGate.swift:4-6,28-41` · `ClaudeCLIRateLimitGate.swift:4-5,13,30-34` (HEAD `5891c0b`) |
| Claude-Code-Usage-Monitor | 있음 → 캐시 값으로 답 | **영속** — 값 캐시 파일 `~/.claude-monitor/api/latest.json` 의 `retry_after_epoch`(tmp + `os.replace`) | 값 + 기한 = 파일 하나 | 확실 | `src/claude_monitor/output/api_usage.py:33,150-165,196` |
| orca | 있음 · 강제 조회는 우회 · 옛 값 24시간 유지 | **메모리** | 앱 상태 `orca-data.json` 하나 + 자주 바뀌는 캐시만 따로(`orca-github-cache.json` — 큰 파일 재기록을 피하려고) | 메모리 = 가능성 높음(영속 계층에 참조 없음) · 저장 방식 = 확실 | `src/main/rate-limits/service.ts:91,170-180` · `src/main/persistence/loading-store/user-data-path.ts:16,29-32` (`df7460af`) |
| ClaudeBar | 있음(없으면 5분) | **메모리** — `UsageMemory.retryAt` | 메모리 | 확실 | `Modules/DataSources/Sources/DataSource.swift:100-111,367-394` |
| paseo | 없음(401·403 만) | 없음 — 5분 메모리 캐시 | 메모리 | 확실 | `packages/server/src/services/quota-fetcher/providers/claude.ts:466-474` |
| t3code | 따로 없음(비2xx = 일반 오류) | 없음 | 조사 안 함 | 가능성 높음 | `apps/server/src/usage/cliproxyApi.ts:174-176` |
| ccusage · herdr · Claude Squad | 사용량 엔드포인트를 부르지 않음 | — | — | 가능성 높음 ~ 불확실(검색만) | — |

일반 HTTP 클라이언트·CLI 의 백오프(Google API 클라이언트 · gh CLI · Octokit throttling)는 거의 다 프로세스 메모리다 — 이번에 코드로 다시 확인하지 않았다(불확실 ~ 가능성 높음).

## 우리 선택지

| | 내용 | 대가 |
|---|---|---|
| **A. 메모리만** | `usage_rejects.json` 과 `reject_store.rs` 를 걷는다 | 거절 기한 안에 데몬을 재시작하면 조회가 한 번 더 나가 다시 429 를 받는다. 그 재조회가 거절을 늘리는지는 모른다. 데몬은 셸보다 오래 살아 재시작이 드물다 |
| **B. 현행 유지** | 기능별 파일 | 피어 선례 없음 · 작은 상태가 늘 때마다 파일이 는다 |
| **C. 한곳으로 통합** | 데몬의 작은 런타임 상태를 파일 하나(또는 키-값 하나)에 모으고 거절 기한은 그 안의 칸 하나 | 공용 저장 계층을 새로 세워야 한다 — 지금 그 파일에 들어갈 다른 입주자가 없다 |

## 공백

- 조기 재조회가 Claude 쪽 거절 기간을 늘리는지 — 피어 어디에도 실측이 없다.
- CodexBar · ClaudeBar 가 사용량 값 자체를 디스크에 캐시하는지 — 보지 않았다.
- orca 메모리 판정은 영속 계층에 참조가 없다는 데서 나왔고 기동 경로 전체를 추적하지 않았다.
