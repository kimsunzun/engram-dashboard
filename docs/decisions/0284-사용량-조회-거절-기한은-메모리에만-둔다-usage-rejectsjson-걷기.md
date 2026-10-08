# ADR-0284: 사용량 조회 거절 기한은 메모리에만 둔다 (usage_rejects.json 걷기)

- 상태: 확정 (2026-10-08, 근거: 사용자 결정) · **구현 전**
- 관련: Amends ADR-0264 (결정 1 의 거절 기록 파일) · PRD `docs/process/S21-usage-limit-slot/prd.md` R24 · R29 · TRD `docs/process/S21-usage-limit-slot/trd.md` §3 #30 · 「거절 저장(R29)」 절 · TRD `docs/process/S21-storage/trd.md` §2-1 · 조사 `docs/research/usage-reject-persistence-2026-10-08.md` · `crates/engram-dashboard-daemon/src/usage_service/reject_store.rs`

## 맥락

데몬은 사용량 슬롯을 위해 claude · codex 사용량을 조회한다. 상류가 429 로 거절하면 거절 기한(`reject_until`)을 세우고, 그 기한 안에는 자동 · ⟳ 조회를 내지 않는다(R24 — claude 에서 `Retry-After` 약 1시간 관측). 지금은 그 기한을 데몬 재시작 너머로 들고 가려고 `daemon\state\usage_rejects.json` 에 따로 저장한다(`RejectStore` · `FileRejectStore` — TRD §3 #30).

그 영속은 사용자 결정이 아니었다. PRD 리뷰 2차가 「재시작하면 거절 기한을 잃는다」를 지적했고, 그것을 「리뷰 반영 기본값 — 확인 필요」로 넣은 것이다(PRD 리뷰 기록). 저장 관리 정리 중 사용자가 이 파일을 보고 과하다고 물었다.

## 결정

1. **거절 기한은 데몬 메모리에만 둔다.** `usage_rejects.json` 을 쓰지도 읽지도 않는다 — `reject_store.rs`(`RejectStore` · `FileRejectStore`)와 조립 · 완료 가드의 거절 저장 배선을 걷는다.
2. **데몬 재시작 뒤엔 거절 기한이 없는 상태로 시작한다.** 거절 기한의 그 밖의 규칙(R24 — 기한 안 자동 · ⟳ 조회 없음 · 숨은 거절 · 거절 끝 접기)은 그대로다.
3. **옛 파일은 지우지 않는다.** 이미 디스크에 있는 `usage_rejects.json` 은 읽지 않을 뿐 그대로 둔다 — 옛 데이터를 건드리지 않는 ADR-0264 결정 5 의 적용이다(메인 세션 판단 — 지우는 코드를 더하지 않는다).

## 거부한 대안

- **B. 현행 유지(기능별 전용 파일)** — 코드만 복잡하다(사용자). 전용 파일로 기한만 저장하는 피어 선례도 없다(조사).
- **C. 데몬의 작은 런타임 상태를 파일 하나로 모으고 거절 기한은 그 안의 칸으로** — 공용 저장 계층을 새로 세워야 하는데 지금 같이 들어갈 다른 상태가 없다(코드 사실 — 사용량 값도 저장하지 않는다). 사용자는 저장 자체가 필요 없다고 봤다.

## 근거

- 사용자 결정(2026-10-08): 「구지 저장까지 할 필요는 없다고 생각. 코드만 복잡함」.
- 조사(`docs/research/usage-reject-persistence-2026-10-08.md` · light): 사용량 엔드포인트를 부르며 429 를 다루는 피어 넷 중 둘은 메모리(orca · ClaudeBar), 둘은 영속(CodexBar — 범용 키-값 · Claude-Code-Usage-Monitor — 값 캐시 파일의 칸). 기한만 담는 전용 파일은 없다. orca 는 같은 1시간 창을 알면서도 메모리를 택했다(`src/main/rate-limits/service.ts:91`).
- 데몬은 셸보다 오래 살아 재시작이 드물고, 사용량 값 자체도 재시작 너머로 들고 가지 않는다.

## 영향 / 불변식

- **대가(수용):** 거절 기한 안에 데몬을 재시작하면 조회가 한 번 더 나가 429 를 다시 받을 수 있다. 그 재조회가 상류 거절을 늘리는지는 실측된 적이 없다.
- 데이터 폴더 `daemon\state\` 의 지킬 파일은 `agents.json` · `presets.json` 둘이 된다(ADR-0264 결정 1 의 거절 기록 파일 개정).
- 저장 방식을 다시 꺼낼 때는 대안 C(한곳으로)부터 본다 — 기능마다 파일을 늘리지 않는다(사용자).
