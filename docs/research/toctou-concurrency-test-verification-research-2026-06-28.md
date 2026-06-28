# TOCTOU/세대 가드 동시성 테스트 검증법 — 리서치 (cross-family)

**상태:** 완료 / 출처 단 보고서 (medium, 설계-결정 모드)
**날짜:** 2026-06-28
**방법:** Claude(Sonnet) 2갈래 + Codex 1, 전부 독립 BLIND → opus 교차대조 + 핵심주장 적대검증.
**확신도 범례:** 확실 / 가능성 높음 / 불확실.

## 동기 (왜 조사했나)

S14 daemon_client tracing backfill QA 중, 기존 스트레스 테스트 `toctou_stress_reconnect_no_stale_down_clobber`가 **간헐 실패**(baseline T2에서도 ~1/20)함을 발견. 직접 진단: 가드(`lifecycle::publish_if_current` — `std::sync::Mutex` 아래 `generation==my_gen` 비교 + `watch::send`를 한 critical section으로 원자화)는 정상이고, **테스트가 서버 hold(5ms) vs assert sleep(3ms) 매직 타이밍에 의존해 current task의 정상 Down을 stale clobber로 오판**하는 false positive였다. "매직넘버로 때우지 말고 OSS 사례로 풀라"는 사용자 지시에 따라 조사.

## 발견 (확신도·출처)

### 1. loom 직접 적용 — 불가(우리 async 케이스) · 확실
- loom은 `loom::model` 안의 동시성 코드를 DPOR로 **인터리빙 전수 탐색**, 확률적 stress를 대체하는 도구. (확실 — https://docs.rs/loom/)
- **intrusive**: `loom::sync`/`loom::thread`로 *계측된 코드만* 보인다. 평범한 `std`/서드파티 sync는 그 라이브러리도 loom 계측돼야 보임. (확실 — https://docs.rs/loom/#intrusive-implementation)
- tokio는 **내부적으로** loom 계측을 가짐(`crate::loom::sync`, loom CI `--cfg loom`). 그러나 **"tokio가 내부에서 loom을 쓴다" ≠ "사용자가 tokio async 코드를 그대로 loom 검증한다."** 사용자가 `tokio::sync::watch`/`mpsc`를 loom으로 보려면 **tokio를 `--cfg loom`으로 재컴파일**(patch override)해야 하고, `tokio::runtime` multi-thread를 `loom::model` 안에서 띄우면 패닉(loom #256). (확실 — loom #256, tokio loom CI, tokio/src/sync/watch.rs)
- `std::sync::Mutex` 부분만 `loom::sync::Mutex`로 cfg(loom) 교체하는 건 표준 패턴이고 가능. 단 그 안의 `tokio::sync::watch::send`는 loom 추적 밖.

### 2. 해법 = 단위 수준 + 결정론적 랑데부 · 확실 (양 family 수렴)
- **단위로 내리기:** 소켓/서버 제거, 가드 메서드를 직접 순서 호출로 검증 — stale generation → 미발행, current → 발행. compare+send 규칙을 네트워크 타이밍 없이 증명. (확실 — 양 family + axum testing 사례)
- **결정론적 랑데부:** `tokio::sync::Barrier`/`Notify`/`oneshot` 또는 `cfg(test)` pause-hook(seam)으로 "비교 통과 → 강제 preempt → 다른 스레드 bump → stale write" 순서를 *정확히* 재현. sleep 없이 stale-vs-current 인터리빙만 친다 → false positive 원천 차단. (확실 — tokio Barrier/Notify, turmoil barrier hooks, matklad "properly testing concurrent data structures")

### 3. 보조 도구 — 부분 적합 · 가능성 높음
- **tokio::time pause/advance** (`start_paused`): 벽시계 의존 제거에 유효하나, "어느 task가 Down을 쐈나" 순서 문제는 단독으로 못 풂. (확실 — tokio time docs; advance가 모든 타이머 처리 보장 안 함 경고)
- **shuttle(awslabs):** 스케줄러 제어 randomized 탐색(전수 아님), tokio 래퍼 있음. loom보다 저비용·확장적이나 완전성 보장 없음. (확실 — github.com/awslabs/shuttle)
- **turmoil(tokio-rs):** 네트워크 시뮬레이션(지연·파티션·barrier) — 분산/네트워크 테스트용. 우리 가드 단위검증엔 과함. (확실)

## 교차검증표 (Claude ↔ Codex)

| 클레임 | Claude(2) | Codex | 수렴 |
|---|---|---|---|
| loom intrusive — 계측된 코드만 본다 | ✓ | ✓ | 수렴(확실) |
| tokio::sync 사용자 loom 검증 = tokio 재컴파일 필요/비현실 | ✓ (#256) | ✓ (#256, CI) | 수렴(확실) |
| 해법=단위 내리기 + 결정론적 랑데부 | ✓ | ✓ | 수렴(확실) |
| loom 도입 = 지금 저ROI | ✓ | ✓ | 수렴 |
| tokio::time는 순서문제 못 풂 | ✓ | ✓ | 수렴 |
| shuttle=randomized 대안 | ✓ | ✓ | 수렴 |

불일치: 없음(상호보완 — matklad pause-hook 사례 vs turmoil barrier-hook 사례).

## 권고

1. **loom/shuttle 지금 도입하지 않는다** — 저ROI(tokio 재컴파일·cfg(loom) 듀얼빌드 vs, 떼어내면 랑데부/단위가 대부분 가치를 더 싸게 줌). T4(재연결·백오프로 동시성 표면↑) 합류 시 재검토 가치는 있음(lifecycle.rs 주석의 기존 메모와 일치).
2. **flaky 테스트를 다음으로 교체:**
   - (a) **가드 단위 테스트** — `Lifecycle::publish_if_current`/`store_cmd_if_current`를 stale vs current generation으로 직접 검증(소켓·sleep 0). 불변식의 깨끗한 증명.
   - (b) **결정론적 랑데부 테스트**(선택) — 통합 레벨 커버 원하면 Barrier/Notify 또는 cfg(test) pause-hook으로 stale-vs-current 인터리빙을 결정론적으로. 기존 sleep stress는 제거 또는 wiring-only로 강등.

## 공백·한계

- pause-hook(seam) 주입은 프로덕션 코드에 `cfg(test)` 지점을 추가 — 침습성 vs 결정성 트레이드오프(가드를 단위로 떼면 불필요할 수 있음).
- 이 스킬 자체가 단일 모델 대비 더 낫다는 대조검증은 아직(스킬 ⚠️ 검증상태).

## 출처 (1차)
loom: docs.rs/loom · github.com/tokio-rs/loom (#256) · tokio loom CI(.github/workflows). 랑데부: docs.rs/tokio Barrier·Notify·oneshot · docs.rs/turmoil(#barriers) · matklad.github.io/2024/07/05. 시간/대안: docs.rs/tokio/time · github.com/awslabs/shuttle · github.com/tokio-rs/turmoil.
