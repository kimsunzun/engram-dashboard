# ADR-0071: persistence registry mutate 락 규율 — store.save를 map 락 보유 중 실행 (동시 rename stale-snapshot race fix)

- 상태: 확정 (2026-07-12, 근거: cross-family(Codex) 적출 → fix → 재검증 PASS · 동시성 회귀 테스트)
- 관련: ADR-0006(락 순서 — **별개 도메인**) · ADR-0070(§5 rename 노출이 동시성 창을 염) · ADR-0061 · `crates/engram-dashboard-core/src/agent/preset.rs`(`PresetRegistry::mutate`) · `.../agent/profile.rs`(`ProfileRegistry::mutate`·`mutate_if`·`observe_session_id`) · step-log

## 맥락

`PresetRegistry`/`ProfileRegistry` 의 공통 변경 경로 `mutate` 는 원래 **"map lock 안에서 변경 → 스냅샷만 뜨고 → lock 해제 → lock 밖에서 `store.save`"** 였다(디스크 IO 를 lock 밖으로 빼려는 의도, 기존 주석 설계). 단일 호출자(사람 UI — 한 번에 하나)만 가정하면 무해하다.

그러나 ADR-0070 이 rename 을 §5 command(`RenamePreset`/`RenameProfile`)로 노출하면서 LLM/오케스트레이터가 create/rename/delete 를 **프로그래밍적으로 동시·연속** 호출할 수 있게 됐다 — 사람 손으로는 못 여는 인터리브 창이 실제 경로가 된다.

## 결정

`mutate` 와 `store.save` 를 **한 임계구역**으로 묶는다 — lock 을 풀기 **전에** save. 저장 스냅샷은 방금 커밋한 최신 맵이라 `persisted == observed` 가 보장된다. 변경 없는 경로(`observe_session_id`)를 위해 조건부 변형 `mutate_if`(클로저가 `true` 를 반환할 때만 lock 보유 중 save)를 신설해 no-op 디스크 쓰기 절약을 유지한다.

**같은 racy 패턴 3곳을 수정**: `PresetRegistry::mutate` · `ProfileRegistry::mutate` · `observe_session_id`(옛 코드는 lock 밖 `list()` + save → `mutate_if` 위임). create/delete/rename/set-autorestore 도 전부 이 공통 경로를 경유하므로 함께 안전해진다.

## 거부한 대안

- **IO-바깥-락 (기존 설계 — lock 밖 save)** — race 를 연다. 두 mutation 이 겹치면 `A 스냅샷 → unlock → B 스냅샷 → unlock → B save → A save` 순서로 인메모리·broadcast 는 최신(B)인데 디스크는 stale(A)로 남아, 재시작 시 옛 값이 로드된다(`persisted ≠ observed`). 최악은 A 스냅샷이 B 의 insert 를 못 봐 **엔트리 누락**. §5 동시 호출로 이 창이 실제로 열린다. lock-hold 시간 단축이라는 이점은 로컬 소형 파일 IO 라 무의미하다.

## 근거

cross-family 리뷰어(Codex, effort high)가 적출하고, 사용자가 §5 도달성(LLM 동시 호출)으로 확정했다.

- **데드락 없음 (ADR-0006 무관):** `store.save` 는 store 내부 leaf mutex(`write_lock`)만 잡고 registry 로 재진입하지 않는다 → 락 순서는 `presets|profiles → write_lock` 단방향, 순환 없음. registry lock 은 세션(sessions/core/status) 락 도메인과 **분리**라 ADR-0006 순서에 얽히지 않는다(reaper·manager 는 sessions guard 를 해제한 뒤 registry 를 호출).
- **advisory:** save 중 map read 가 블록되지만 로컬 소형 파일이라 무시 가능.
- **회귀 봉인:** `concurrent_mutations_persisted_equals_final_map`(4 스레드 × 50 create+rename = 200 엔트리 → 디스크 == 인메모리 id 집합·내용 일치) + `save_writes_current_map_not_stale_snapshot`.

## 영향 / 불변식

- **`mutate`/`mutate_if` 의 `store.save` 를 lock 밖으로 다시 빼지 말 것** — 정확히 거부한 대안(옛 코드)으로 되돌아가는 것이고 stale-overwrite race 가 재발한다. 코드 앵커 `// ADR-0071`.
- registry 변경 경로(create/delete/rename/set-autorestore/observe_session_id)는 전부 이 공통 경로를 경유한다 — 새 변경 API 는 반드시 `mutate`/`mutate_if` 를 거친다(직접 lock + save 금지).
- 이 락 도메인은 ADR-0006(sessions → 내부) 순서와 **별개**다(store leaf lock). 두 도메인을 한 경로에서 겹쳐 잡지 않는다.
