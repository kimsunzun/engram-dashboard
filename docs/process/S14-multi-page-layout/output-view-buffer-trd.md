# TRD — S14 모듈① 출력 평면 재설계: 출력 단위 = View 독립

**상태:** 리뷰 반영 (`/review trd full` FIX 반영) — 코더 대기(동시성-치명 → `/review code deep` 게이트)
**작성:** 2026-06-30
**근거:** ADR-0040(출력 단위=View 독립) · PRD `output-view-buffer-prd.md` · ADR-0036/0037(전송 중계·의미론 Rust 단독)·ADR-0006(락 순서)·ADR-0007(epoch)·ADR-0001/0005(kill·finalize 데몬 소유)·ADR-0012(seam)
**조사:** Explore 출력 평면 정밀 맵(2026-06-30).
**리뷰:** `/review trd full` = Codex(Designer doc-aware) + opus(Architect-breaker doc-aware) **둘 다 FIX, BLOCK 없음**. FIX 5건 반영: ① 재구독/read 두 축 분리 ② 락 밖 send + 단일 락 ③ epoch 태깅(락 아님) ④ terminal 시 버퍼 정리 ⑤ 공통 추출=데이터 구조만.

---

## 현재 구조 (출발점 — Explore 회수)

- **데몬 ReplayBuffer** (`crates/engram-dashboard-core/src/agent/output_core.rs:415–461`): `VecDeque<OutputChunk{seq:u64, data:Vec<u8>}>` + `total_bytes`. 이중 상한 **`max_bytes=2MB` OR `max_events=4096`** 초과 시 `pop_front` evict(oldest_seq↑). `subscribe_from(after_seq)`(:316–394): epoch 불일치/None→`FromOldest`; `Some(s)`→버퍼 빔=`Resumed,0`/`s<oldest`=`Truncated`(oldest부터)/`s>=oldest`=`Resumed`(tail). `subscribe_from`은 subscribers 락 보유 중 on_ready→replay send(C4, :374–391) — finalize(ADR-0005)와 얽힘.
- **src-tauri 출력 흐름** (`connection.rs:880–912`): `decode_frame→OutputFrame{agent_id,epoch,seq,data}`→`decide_epoch`(Deliver/DropEpochMismatch; `st.epoch=None`이면 통과)→`router.targets→Vec<WindowLabel>`→`fan_out_per_window`(창별 dedup + **락 1개(registry)만 보유 중 동기 send**, :1060–1110 — ADR-0006 미위반 확인).
- **재연결** (`connection.rs:788–832`): `router.current_agents()`→`min_render_seq`→`resubscribe_params`→wire `Subscribe{epoch,after_seq}`. SubscribeAck epoch 변경 시 `reset_all_windows_for_agent`(:862–870). subs.retain(:832)이 router 기반 정리.
- **main_loop = 단일 actor**(select! task) — Binary arm·Text(SubscribeAck) arm·cmd_rx arm이 **직렬**. (epoch 정정의 근거.)
- **프론트** (`tauriTransport.ts`): `outputChannel.onmessage(bytes)`→decode→messageCb. **dedup 없음** — Rust 단독 진실원(ADR-0037).

---

## 새 구조 — 공유 버퍼 + per-view cursor (★두 축 분리)

src-tauri에 **에이전트당 공유 출력 버퍼**를 신설. "frame → 각 창 Channel(창별 dedup)"을 **"frame → 공유 버퍼 append → 각 창이 자기 cursor부터 read → (락 밖) Channel send"**로 전환. 삽입점 = `main_loop` Binary arm의 epoch 처리 직후, fan-out 전.

**★핵심: 두 축을 분리한다(리뷰 FIX 1).**
- **축 A — 데몬↔클라 동기화:** 데몬 재구독 `after_seq` = **공유 버퍼 최신 seq**. 데몬은 클라 버퍼에 없는 것만 보내 버퍼에 이어 append(비중복).
- **축 B — 창별 read 무손실:** 각 창은 **per-view cursor**로 공유 버퍼를 `cursor`부터 read. cursor가 버퍼 [oldest~최신] 안이면 그 구간을 무손실로 읽는다.
- 이 둘은 **다른 축** — 합치지 않는다. (초안이 "버퍼 최신" 하나로 합쳐 미렌더 창 유실/중복 모순을 낳았다 — opus·Codex 수렴 지적.)

```
[데몬]  단일 무손실 스트림 (에이전트당 1구독, after_seq=버퍼 최신)
   ▼ OutputFrame{seq, epoch, data(원본 binary frame bytes)}
[src-tauri 공유 버퍼]  AgentBufferStore
   · content:  HashMap<AgentId, BoundedSeqLog>   — 콘텐츠 에이전트당 공유 1벌 (core 순수)
   · cursors:  HashMap<SlotId, {agent_id, cursor}> — 보는 단위(slot) 1차 키 (core 순수)
   · epoch 태그
   ▼ [락 안] 그 agent 보는 slot 중 cursor<최신인 것의 (slot, bytes) snapshot → [락 밖] Channel send
[main/popup/tree창의 각 slot=xterm]  순수 렌더러 (raw → xterm)
```

### 1. 공유 버퍼 모델 + 락 규율 (결정점 1·3 — FIX 2)

- **자료구조(★cursor 키 = 보는 단위, 콘텐츠 = 에이전트 공유 — 사용자 교정):** core 순수 두 조각 —
  - `BoundedSeqLog<Chunk>` = **콘텐츠, 에이전트당 공유 1벌**(VecDeque + 이중상한 `min(2MB,4096)` + evict + oldest/latest seq; **데이터 구조만**).
  - `SlotCursorMap` = **보는 단위(slotId) 1차 키 → `{agent_id, cursor}`**(ViewManager leaf와 1:1). 같은 에이전트를 N slot이 보면 cursor N개 + 콘텐츠 1벌.
  - 둘 다 **Tauri 무관**(slotId/agentId generic — core에 Tauri 타입 누출 0, ADR-0012 / Codex FIX). 저장 단위 = **원본 binary frame bytes**.
- **소유:** src-tauri `AgentBufferStore` = `Arc<Mutex<{ content: HashMap<AgentId, BoundedSeqLog>, cursors: HashMap<SlotId, ViewCursor{agent_id, cursor}> }>>`. **콘텐츠·cursor를 한 락 안**에 둔다(별도 맵 분리 시 순서 역전).
- **fan-out 역조회:** agent 출력 도착 → 그 agent를 보는 slot들(`cursors`에서 agent_id 매칭, 또는 `OutputRouter`의 agent→targets 활용 — ADR-0036)을 찾아 각 cursor부터 send.
- **★락 규율(ADR-0006, FIX 2):** 버퍼 락 구간에서는 **append + cursor 갱신 + 보낼 `(WindowLabel, bytes)` snapshot 수집까지만**. **Channel `send`는 락을 푼 뒤**(데몬 `output_core` C4 패턴과 동형). → 버퍼 락 ⊃ registry 락 중첩이 사라져 fan-out 경로와 subscribe 경로의 **락 순서 역전 데드락 차단**. registry(Channel 보관)는 send 시점에만 짧게.

### 2. 새 창 mount → 버퍼 replay (결정점 2 / 화면 동일성)

- `subscribe_output` invoke(slot이 에이전트를 배정받음): 버퍼 락 안에서 — 그 agent `content` 버퍼가 없으면 신설(데몬 `subscribe_from` FromOldest로 채움), 있으면(이미 다른 slot이 봄) 재사용 + `cursors.insert(slot_id, {agent_id, oldest_seq})`(처음부터 = PRD §3-1) + **replay할 [oldest~최신] bytes snapshot** → 락 밖에서 그 slot Channel로 send. 이미 보던 에이전트면 데몬 재요청 0.
- raw bytes 순서 replay → xterm 렌더+scrollback. **화면 동일성**(PRD §5-6): 데몬과 같은 raw 청크 순서라 ANSI/부분 UTF-8/CR 보존이 **현 fan-out과 동형**(악화 없음).
- **재연결 대기 중 새 창**(수용기준 5): 버퍼 보존 → 끊긴 상태서도 즉시 replay(빈 화면 0).

### 3. 재연결 (결정점 3 — ★급소, FIX 1)

- **축 A:** 버퍼 보존(끊겨도). 재연결 시 데몬 `after_seq` = **버퍼 최신 seq** → 데몬이 그 이후만 보내 append(비중복).
- **축 B:** 각 창 cursor는 **재연결과 무관하게 보존**. 미렌더 창(cursor=oldest)이 데몬 replay를 못 받고 끊긴 뒤 재연결해도, 그 창은 **클라 버퍼의 oldest부터 자기 cursor로 read** → 무손실(버퍼에 있으므로). = 현 `should_deliver` "미렌더 재시도"(T7b)가 cursor read로 자연 보존.
- **gap(Truncated):** 끊김이 길어 `버퍼 최신 < 데몬 oldest`면 데몬 `Truncated` → 데몬 oldest부터 받아 버퍼를 그 기준으로 재구성(oldest↑), **모든 창 cursor를 `max(cursor, 새 oldest)`로 클램프**. 그 사이 구간은 데몬도 evict한 불가피 유실(PRD §3-1 "보관 하한" 정의 안, 잘림 미표시).
- `min_render_seq`(가장 뒤처진 창 합산) **폐기** — 재구독 기준이 창이 아니라 버퍼 최신(축 A).

### 4. eviction · 버퍼 생명주기 (결정점 4 — 사용자 확정 + FIX 4)

- **evict:** 데몬 미러 상한(2MB/4096) `pop_front`, oldest↑ → 클라 보관 하한.
- **★버퍼 생명 = 어느 창엔가 배정된 동안만(사용자 확정).** 클라 버퍼는 "표시 캐시"고 데몬이 원본을 보유(에이전트 생존 동안)하므로, **현재 어느 View엔가 배정된 고유 에이전트당 1벌**만 둔다. 같은 에이전트를 N창에서 보면 버퍼 1벌 + cursor N개. **모든 창이 그 에이전트를 놓으면(close/재배정/창 닫기) 버퍼 폐기** → 다시 배정되면 데몬 `subscribe_from`(FromOldest)로 replay받아 새 버퍼(로컬 loopback 한 홉, 무손실). 메모리는 **동시에 보는 고유 에이전트 수**로 자연 상한(에이전트가 수십이어도 화면에 띄운 것만). PRD/ADR "데몬=장기 원본 / 클라=뷰 채우기 캐시" 분담과 정합.
- **terminal 정리(FIX 4 흡수):** 위 "안 보면 폐기" 규칙이 누수를 근본 차단한다 — 에이전트가 terminal(Killed/Exited)이면 어느 창에도 live 배정이 없어 버퍼가 폐기된다(죽은 에이전트 누수 자동 해소, opus 지적). 폐기 트리거 = **View 배정 해제**(close_slot/재배정/창 닫기) — 마지막 cursor가 빠지면 버퍼 drop. 구현은 `ViewCursorSet`가 비면(cursor 0개) 그 에이전트 버퍼 entry 제거.

### 4b. epoch 전환 (FIX 3 — 락 아님, 태깅)

- **정정:** `main_loop`가 단일 actor(select! task)라 SubscribeAck arm과 Binary arm은 **이미 직렬** — "버퍼 락 직렬화"는 무의미(opus 지적). 락으로 막을 race가 아니다.
- **태깅:** 공유 버퍼가 **epoch 태그**를 든다. Binary frame의 `frame.epoch ≠ 버퍼 epoch`이면 → **버퍼 리셋(새 스트림 seq 0) + 모든 창 cursor 리셋 후 append**(SubscribeAck를 기다리지 않고 frame.epoch 기준). 현 `decide_epoch`(st.epoch=None 통과)과 정합.
- **확인 필요(코더 spike):** 데몬 wire가 epoch 전환 직후에도 `SubscribeAck → replay → frame` FIFO를 보장하는지(opus 미확인). 태깅이 이 순서에 무관하게 안전하나, 보장되면 reset 트리거를 Ack에 둘 수도 — spike로 확정.

### 5. output_window_seq.rs 재작업 (결정점 5)

- 현 per-window `render_seq`(`should_deliver`/`mark_rendered`) → `SlotCursorMap`의 slot별 cursor로 흡수. "미렌더 재시도"(T7b)는 cursor=oldest read로 보존(§3 축 B). `min_render_seq` → **폐기**(§3 축 A). `reset_all_for_agent` → epoch 태깅 시 cursor 리셋(§4b)으로 흡수.
- **코더 분리물(보류분) 처리:** 직전 세션 `output_window_seq.rs`(min 모델 + 테스트 14개)는 이 재작업으로 **대체**(min 합산 폐기). 신규 = `BoundedSeqLog` + `ViewCursorSet` headless 단위테스트.

### 6. resize 위험 (PRD §7 carry — 신규 결정 아님)

단일 공유 버퍼 raw bytes ∩ 크기 다른 창 → escape 충돌 가능. **현재도 raw 청크 동일 fan-out이라 동일 미해결**(악화 없음). viewport별 버퍼 분기 필요 시 retrofit(ADR-0040 §영향 위험). resize 정책은 별도 열린사항.

### 프론트 (무변경 — ADR-0037)

순수 렌더러. src-tauri가 cursor/인덱스 전부 관리, 프론트엔 raw bytes Channel만. 새 창 replay도 같은 Channel → 프론트 코드 무변경.
- **데드락은 프론트 무관(load-bearing):** WebView JS는 단일 스레드 이벤트 루프라 락이 없다 → 데드락(락 역전)은 src-tauri Rust 전담(§1 락 규율), 프론트는 방어 대상 아님. 프론트의 동시성 위험은 데드락이 아니라 비동기 race/순서/stale(Channel 콜백 겹침·재구독·죽은 연결 부활)이고, 기존 `tauriTransport.ts` 가드(세대 가드·멱등 등록·연결 상태 단일 진실원, T7c)를 **유지**한다(새로 추가·삭제 금지). 신경 쓸 유일한 점 = replay→live 순서, 단일 Channel FIFO로 보장(프론트는 받은 순서대로 write).

---

## 모듈 경계 (DDD — FIX 5: 공통 추출은 데이터 구조만)

- **core** — `BoundedSeqLog<Chunk>`(VecDeque+이중상한+evict+oldest/latest seq, **데이터 구조·순수 메서드만**) + `SlotCursorMap`(slotId→{agent_id, cursor}, generic). Tauri 무관, headless 단위테스트. **데몬 `ReplayBuffer` struct 시그니처·동기화·on_ready·C4 호출 구조는 비공유**(데몬/클라 각자 소유) — 공통 추출이 데몬 emit 핫패스·C4 TOCTOU(:374) 회귀시키지 않도록 **데이터 구조만** 공유(데몬이 `BoundedSeqLog`를 채택할지는 시그니처 안전 확인 후 옵션, 강제 아님).
- **src-tauri** — `AgentBufferStore`(BoundedSeqLog+ViewCursorSet 조립, epoch 태그, Arc<Mutex>), `main_loop` Binary arm 삽입, `subscribe_output`/재연결 배선, **Channel send는 락 밖**(§1).

---

## 동시성 불변식 (★`/review code deep` 대상 — ADR-0001/0005/0006/0007)

- **락 순서(ADR-0006, FIX 2):** 버퍼 락 안 = 데이터(append/cursor/snapshot)만, Channel send는 락 밖 → 버퍼 락 ⊃ registry 락 중첩 0, 역전 데드락 0. ViewManager 락(ADR-0035)도 fan-out 핫패스에서 미보유(라우팅 snapshot, ADR-0036).
- **epoch(ADR-0007, FIX 3):** 단일 actor 직렬 + frame.epoch 태깅. fresh fallback(ADR-0008 새 sid)도 SubscribeAck epoch 변경 → 태깅 리셋으로 흡수.
- **생명주기(ADR-0001/0005, FIX 4 + 사용자 확정):** 데몬 reaper/finalize는 데몬 소유. 클라 버퍼는 **View 배정 해제 시 폐기**(cursor 0개 → 버퍼 drop) — terminal 에이전트는 배정이 사라져 자동 폐기(누수 0). 재배정 시 데몬 replay로 새 버퍼.
- **데몬 핫패스 비회귀(FIX 5):** 공통 추출은 데이터 구조 한정 — 데몬 emit/C4 불변.

---

## 수용 기준 (PRD §5 + 구현)

PRD §5 1~8 + 구현 게이트: `cargo test`(루트, `BoundedSeqLog`·`SlotCursorMap` 단위테스트 포함) + core `rg "use tauri"` 0 + `npm test`/`tsc` + `cdp.mjs` 멀티뷰 실측(같은 에이전트 2·3창, 새 창 늦게 열기, **재연결 중 새 창**, **미렌더 창 재연결 무손실**, terminal 후 버퍼 정리).

---

## 결정점 요약 (사용자 보고 / 갈림길)

| # | 결정점 | 처리 | 갈림길? |
|---|---|---|---|
| 1 | 인덱스↔버퍼 | seq 단위, `BoundedSeqLog` (데이터구조만 core) | 내부(보고) |
| 1b | **cursor 키 단위** | **slot(보는 단위) 1차 키** + 콘텐츠 에이전트 공유 — ViewManager leaf 연동 | 사용자 확정 |
| 2 | xterm↔버퍼 | raw replay = 현 fan-out 동형, 화면 동일성 | 내부(보고) |
| 3 | 재연결 | **두 축 분리** — 데몬 동기화(버퍼 최신) ≠ 창 read(per-view cursor). gap=클램프 | 내부(급소, deep 리뷰) |
| 4 | 버퍼 생명 | **어느 창엔가 배정된 동안만**(cursor 0개면 폐기, 재배정 시 데몬 replay) — 캐시/원본 분담, 누수 자동 차단 | 사용자 확정 |
| 4b | epoch | 단일 actor 직렬 + frame.epoch 태깅(락 아님) | 내부(보고, wire 순서 spike) |
| 5 | output_window_seq | min 폐기 → cursor 흡수, 보류분 대체 | 내부(보고) |
| 6 | resize | 현재와 동일 미해결, retrofit 위험 명기 | 열린사항 |
| 락 | 락 규율 | send 락 밖 + 단일 락 → 역전 데드락 0 | 내부(보고, deep 리뷰) |

**#4 버퍼 생명 = 사용자 확정** — 어느 창엔가 배정된 동안만(cursor 0개면 폐기, 재배정 시 데몬 replay). 클라=캐시/데몬=원본 분담 정합 + 메모리 자연 상한(동시에 보는 고유 에이전트 수) + 죽은 에이전트 누수 자동 차단. 나머지는 무손실 정확성·격리 정답이라 내부 설계.
</content>
