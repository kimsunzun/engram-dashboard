# 조사 — 명부 스냅샷 브로드캐스트의 순서 보장

- **상태:** 완료(설계-결정 모드 · tier medium). **선택은 아직 안 됐다** — 적대 리뷰가 잠정 추천을 기각했고, 그 결과 요구가 셋으로 늘었다.
- **방법:** 3갈래 병렬 수집(직접 피어 · 메커니즘 선례 · 저장소 제약) → 메인 근거 검증 → cross-family 적대 리뷰 1회(effort high, 자체 웹서치 포함).
- **날짜:** 2026-08-26.
- **확신도 범례:** 확실(근거 검증 통과 + 원문 확인) / 가능성 높음(단일 출처 또는 정적 판독) / 불확실(미검).
- **연결:** `../tracking.md` T-32·T-16 · `../todo/replay-subscription.md` 「재생·구독」 · `../todo/roster-consistency.md`.

## 문제

데몬의 세션 맵을 **네 지점**이 각자 통째로 읽어 전체 스냅샷을 클라이언트에 보낸다(확실 — 전수 증명):

| # | 위치 | 계기 | 스레드 |
|---|---|---|---|
| ① | `crates/engram-dashboard-agent/src/manager.rs:1077` | spawn 꼬리 | 호출자 |
| ② | `crates/engram-dashboard-agent/src/reaper.rs:87` | reap 꼬리(락 밖) | 전용 reaper 스레드 |
| ③ | `crates/engram-dashboard-daemon/src/agent_conn.rs:204` → `connection_core.rs:1719` | 새 연결 인사 | 그 연결의 태스크 |
| ④ | `crates/engram-dashboard-daemon/src/connection_core.rs:909` | `ListAgents` 조회 응답 | 그 연결의 명령 처리 |

**전수 증명 방법:** 명부가 클라이언트에 닿는 wire variant 는 둘뿐(`AgentListUpdated`·`AgentList`)이고, 그 **생성 지점**을 저장소 전체에서 셌다(테스트·bin 제외 그 외 0건). 교차 확인으로 `core_agents_to_wire` 호출부와 `broadcast_text` 호출부를 각각 셌고 새 항목이 없었다. ★③은 `StatusSink` 를 통째로 우회하므로 sink 이름 축 grep 으로는 안 잡힌다★.

`manager.rs:1774-1780 list_agents()` 가 sessions 락을 잡아 복제하고 **놓은 뒤** AgentInfo 를 조립하며, 발신은 락을 전부 놓은 상태에서 일어난다. 이건 실수가 아니라 ADR-0006 이 요구하는 형태다. 그래서 **읽기와 보내기가 원자적이지 않고**, 옛 상태를 읽은 쪽이 새 상태를 읽은 쪽보다 늦게 보낼 수 있다. 프론트 `src/store/agentStore.ts:68 setAgents` 는 배열을 통째로 대입하므로 늦게 온 옛 스냅샷이 이긴다.

★프론트로 들어가는 문이 **둘**이다★ — store(`setAgents`)와 부착 상태기계(`src/api/protocolClient.ts` `observeRoster`, push `:661-666` / pull `:855-861`). ADR-0164 결정 3 이 둘 다 열려 있을 것을 불변식으로 박았다. **신선도 판정을 store 에만 넣으면 부착 경로가 여전히 옛 스냅샷을 먹는다**(확실).

## 후보

- **A** — 기존 발신자들을 락 뒤로 직렬화(스냅샷 읽기 + 발신을 한 임계구역에).
- **B** — 스냅샷에 단조 번호를 달고 프론트가 역행분을 버린다.
- **C** — 안 고친다.
- **D** — 발행을 단일 publisher 로 접고, 변경 지점은 사실만 통지.
- **E**(피어에서 발견, 원래 목록에 없었음) — 상태를 안 싣고 「바뀌었다」만 알린 뒤 클라이언트가 권위를 재조회.

## 발견 1 — 직접 피어: 이런 형태를 하는 곳이 없다

**조사한 어느 피어도 「여러 변경 지점이 각자 전체 집합을 읽어 독립 브로드캐스트」하지 않는다**(가능성 높음 — 부정 명제라 전수는 불가). 셋으로 수렴한다:

1. **부트스트랩 스냅샷 1회 + 이후 델타, 연결당 단일 발신 지점** — wezterm mux(`dispatch.rs` 가 알림·쓰기·읽기를 한 루프에서 순차 처리하는 단일 `item_tx`) · herdr(`session.snapshot` → `events.subscribe`) · Kubernetes list+watch · DAP.
2. **페이로드 없는 무효화 + 클라이언트 재조회** — tmux control mode(`%sessions-changed` 가 세션 목록을 **안 싣는다**; `control-notify.c` 가 생성·종료 양쪽에 같은 문자열을 낸다) · DAP `invalidated` · JupyterLab.
3. **전체 스냅샷을 만드는 주체를 하나로 고정** — zellij(단일 백그라운드 잡 폴링) · JupyterLab `SessionManager`(★`startNew()`·`shutdown()` 이 자기 스냅샷을 내지 않고 `refreshRunning()` 을 부른다★).

**유일한 반례가 결정적이다.** Zed collab 은 변경마다 room 집합체 전체를 참가자에게 push 하는데, **그 갱신이 끝날 때까지 room 락을 쥐고 있다.** 주석 원문(확실 — 메인이 원문 대조):

> "room_transaction runs the block in a transaction. It returns a RoomGuard, that keeps the database locked until it is dropped. This ensures that updates sent to clients are properly serialized with respect to database changes."

`TransactionGuard<T> { data, _guard: OwnedMutexGuard<()>, _not_send }` + `rooms: DashMap<RoomId, Arc<Mutex<()>>>` 로 구현이 주석과 일치함을 확인했다.

→ **「전체 스냅샷 브로드캐스트」 + 「보내기 전에 락 놓기」 조합은 조사 범위에 선례가 없다**(가능성 높음).

## 발견 2 — 판정 규칙

| 조건 | 정답 |
|---|---|
| 단일 소유자를 세울 수 있다(같은 프로세스) | 소스 직렬화 |
| 못 세운다(생산자가 독립·선점·네트워크 격리) | 버전 스냅샷 + 수신자 거부 |
| 스냅샷이 변경률 대비 크고 **채널이 무손실·순서보장** | 델타 |
| **채널이 드롭한다** | ★델타 금지★ — 드롭된 스냅샷은 다음 것이 덮지만 드롭된 델타는 조용히 영구 손상 |

**우리 채널은 드롭한다**(`crates/engram-dashboard-net/src/ws.rs:135-145` bounded `try_send`, 실패 시 warn 만 — 느린 소비자용 **설계된** 경로). 따라서 전체 스냅샷 + 마지막쓰기승리는 드롭에 대해서는 이미 옳고, 결함은 순서 쪽이다.

부수 발견 셋:

- **액터 모델은 그 자체로 이걸 안 고친다**(확실). Erlang 이 순서 보장을 **쌍 단위로만** 규정한다 — 한 개체가 같은 목적지로 여러 신호를 보내면 순서가 보존되지만, 여러 발신자가 한 프로세스로 보내는 것 사이엔 보장이 없다. 효력은 「액터」가 아니라 **「단일 발신자」**에서 나온다. (리뷰 지적: 그러나 Erlang 자신이 공통 프로세스를 두면 그 순서를 세울 수 있다고 설명한다 — 그게 곧 D 다.)
- **버전은 읽기와 같은 임계구역에서 배정해야 한다**(확실). Kubernetes #59848 이 정확히 그 실패이고(stale read → 같은 pod 가 두 노드에서), KEP-2340 이 그 수정이며 실패 이름을 "going back in time" 이라 부른다. **발신 시점 배정은 경쟁을 옮기기만 한다.**
- **Kubernetes 는 `resourceVersion` 을 「불투명·동등비교만」에서 뒤집었다**(확실 — 메인이 원문 대조): *"Resource version strings are orderable as monotonically increasing integers within the same resource type…"* + *"Starting with Kubernetes 1.35, orderability … is included in Certified Kubernetes requirements."* ★단 같은 리소스 타입 안에서만이고, `api-conventions.md` 는 여전히 불투명 취급을 요구해 **상류 문서 둘이 어긋나 있다**★. 인용할 땐 어느 쪽인지 밝힐 것.

## 발견 3 — 저장소 제약

- **`StatusSink` 의 논블록·비재진입 계약은 `agent_list_updated` 를 안 덮는다**(확실). `crates/engram-dashboard-agent/src/types.rs:711-733` 에서 그 계약 문단은 `turn_ended` 에만 달려 있고 `agent_list_updated`(`:715`)엔 주석이 없다. 「블록 금지 경로」라는 진술의 실제 거처는 코어가 아니라 데몬 주석(`messaging_host.rs:391-394`)이다.
- **ADR-0028 「단일 push」는 여기 구속력이 없다**(확실). `status_fanout.rs:16-18` 이 그 오독을 미리 못박는다 — 푸시 **갈래**가 하나라는 결정이지 전송이 한 번이라는 보장이 아니다.
- ★**저장소가 같은 판단을 이미 두 번 했고, 두 번 다 「락을 든 채 보낸다」쪽이었다**★(확실). `messaging_host.rs:396-406` 이 그것을 ADR-0006 으로부터의 **의도된 이탈**로 도장 찍고 「락 놓고 send 로 되돌리지 말 것」을 명시한다(안전 근거 = 재진입 불가 + 논블록, 그리고 논블록은 채널 쪽에만 성립한다고 스스로 한계를 밝힌다). ADR-0071 도 「IO 를 락 밖에」를 같은 경쟁 서사로 기각했다.
- **그 선례의 보호가 프론트 broadcast 엔 안 닿는다**(확실) — `messaging_host.rs:864` 가 락 밖에서 위임한다. `tracking.md:310` 이 이걸 갭으로 기록해 뒀다.
- **D 의 고유 충돌**(가능성 높음): `MessagingFlushSink::agent_list_updated`(`messaging_host.rs:862-865` → `:445-506`)가 **모든 스냅샷을 diff** 해 파킹 메일 flush 를 건다. publisher 가 스냅샷을 합치면 중간 등장이 사라져 그 이름 앞 메일이 24h TTL 까지 갇힌다(`:448-456` 이 조건을 좁히지 말라고 경고).
- **B 의 비용**(확실): wire 양쪽 variant + 커밋되는 바인딩 재생성 + ★Tauri 홉이 배열만 날라 형제 필드를 버린다★(`src-tauri/src/daemon_client/events.rs:78,110` → `src/api/tauriTransport.ts:196-201` 재조립 — 3곳을 함께 고쳐야 한다) + 프론트 타입은 손으로 미러링돼 게이트가 안 잡는다(`src/api/types.ts`). 바인딩 동기 게이트 = `.github/workflows/ci.yml:278-286`.
- **B 는 직전 거부 사유를 해소하지 못한다**(가능성 높음). T-16 시도가 리뷰 2인에게 거부된 뿌리가 「필터가 명부를 과신한다」였고 근거가 `ws.rs` 의 유실이 **설계된 경로**라는 것이었다. B 는 순서만 고친다.
- **런타임 관측 기록이 없다**(확실). T-32 미착수, T-16 조사도 정적 판독뿐(`tracking.md:293`).

## 적대 리뷰 결과 — ★잠정 추천 기각★

리뷰 판정 = **REJECT.** "D 단독은 정당화되지 않는다. 방어 가능한 설계는 단일 권위 publisher + 종단 간 세대 방벽 + 명시적 유실 복구이며, 메시징 flush 전이는 GUI 명부 상태와 분리해야 한다."

핵심 적출(심각도 순):

1. ★**프론트에서 문이 다시 갈린다 — 메인의 「wire 변경 불필요」 논거가 깨졌다**★(치명). 데몬 쪽 연결당 채널이 순서를 지켜도, Tauri 경계에서 브로드캐스트는 이벤트로, 조회 응답은 명령 반환값으로 간다(`src/api/tauriTransport.ts:191-201` vs `:429-445`). Tauri 는 Channel 의 순서는 문서화하지만 **이벤트와 명령 반환 사이의 순서는 규정하지 않는다.** → 생산자만 직렬화해서는 부족하다.
2. **순서와 유실은 별개 요구인데 하나의 축으로 후보를 줄 세웠다**(치명). A·D 도 「마지막 브로드캐스트가 드롭되면 복구 계기가 없다」를 남긴다. 어느 후보를 고르든 주기적 재동기·수신 확인·최신상태 보장 전달 중 하나가 **따로** 필요하다.
3. **「D 는 합치지만 않으면 된다」는 부정확하다**(치명). 사실이 publisher 앞에 쌓이면 「변경마다 한 번 읽기」로도 중간 상태가 빠질 수 있다. 반대로 메시징 flush 는 무손실 내부 전이 스트림으로 먹이고 GUI 는 최신값 슬롯으로 미는 분리가 가능하다.
4. **A 에 미검토 데드락 위험**(높음). ③은 `send(...).await` 로 보내는데, 그 연결의 writer 는 `on_connect` 가 반환된 **뒤에** 뜨고 큐가 차면 그 await 가 영영 안 돌아올 수 있다고 코드가 직접 적어 뒀다(`crates/engram-dashboard-net/src/ws.rs:359-383`). 전역 명부 뮤텍스를 그 await 너머로 쥐면 모든 발행자가 멈춘다.
5. ★**경쟁 상대가 하나 더 있다 — 스냅샷 vs `status_changed`**★(높음). `AgentInfo.status` 는 sessions 락을 놓은 뒤 읽힌다(`manager.rs:1774-1780` → `:1883-1895`). 더 새 `status_changed` 가 적용된 뒤 더 옛 전체 스냅샷이 그것을 덮을 수 있다. 게다가 프론트 콜백은 넘어온 화신 표식을 버리고 store 를 갱신한다(`src/store/eventBus.ts:151-160`). **A·B·D 모두 스냅샷끼리만이 아니라 스냅샷↔상태이벤트 사이의 규칙을 정의해야 한다.**
6. **「전역 카운터라 gap 감지가 함정 → 그래서 B 가 약하다」는 전제는 맞고 추론이 틀렸다**(높음). B 는 `version < last` 를 버리기만 하면 되고 연속값을 요구하지 않는다. Kubernetes 도 비연속이라 410 을 내는 게 아니라 **보존 창을 벗어났을 때** 낸다.
7. **「채널이 드롭하면 델타 금지」는 과장**(높음). etcd·Kubernetes 는 끊기는 스트림 위에서 스냅샷+델타를 쓴다 — 삭제를 명시 이벤트로 싣고, 재접속은 리비전으로 재개하며 안 되면 relist 로 떨어진다. 「키별 상태로는 제거를 표현 못 한다」도 tombstone 으로 반박된다.
8. ★**Kleppmann 인용이 과전이됐다**★(높음). 그 논변은 **만료되는 분산 리스**와 지연된 네트워크 요청을 전제한다 — 멈춘 클라이언트의 리스가 만료되고 다른 클라이언트가 락을 얻은 뒤 옛 요청이 뒤늦게 저장소에 닿는 구조다. **같은 프로세스의 만료 없는 뮤텍스를 enqueue 까지 쥐면 다른 발행자가 통과하지 못한다.** 즉 「생산자가 복수인 한 수신자 거부는 필수」는 성립하지 않는다 — 단 위 1번(Tauri 분기)을 못 없애면 다시 유효해진다.
9. 「액터 모델은 안 고친다」는 표현이 과하다(낮음) — 여러 발신자와 하나의 소유 액터를 구분한 것이지 액터 일반의 부정이 아니다.
10. Kubernetes 가 규약을 "뒤집었다"는 서술은 부정확(낮음) — 상류 문서 스큐 + 좁은 스코프 제한이지 클라이언트 파싱의 일반 승인이 아니다.

## 쟁점·한계

- ★**선택이 안 정해졌다.** 리뷰가 요구를 셋으로 늘렸고(순서·유실 복구·상태이벤트 경쟁), 그 셋을 한 설계로 묶을지 나눌지가 다음 결정이다.★
- **선결 조건이 하나 있다** — `../todo/replay-subscription.md` 가 적어 둔 *「명부 조회가 원래 읽기였는데 부착 권위로 승격됐다 — 정합 작업 때 이 구조부터 정할 것」*.
- **런타임 재현이 한 번도 없다.** 전부 정적 판독이다.
- **수집 공백(기권):** herdr 라이선스·소스·유실 복구 · orca 의 명부 동기 메커니즘 · paseo · Docker events 규약 · systemd D-Bus · wezterm 의 구독 끊김 후 복구 · Zed `RoomUpdated` 페이로드가 정말 전체인지 · `AgentEvent` serde 의 unknown-field 정책 · `PROTOCOL_VERSION` bump 필요 여부 · Saltzer 원문의 순서 관련 구절.
- **적대 리뷰는 1회(medium)다.** 통과 못 한 것을 잡았을 뿐 남은 게 없다는 증명이 아니다.

## 출처

- [Kubernetes API Concepts](https://kubernetes.io/docs/reference/using-api/api-concepts/) · [KEP-2340](https://github.com/kubernetes/enhancements/blob/master/keps/sig-api-machinery/2340-Consistent-reads-from-cache/README.md) · [k8s#59848](https://github.com/kubernetes/kubernetes/issues/59848) · [API conventions(불투명 쪽)](https://github.com/kubernetes/community/blob/main/contributors/devel/sig-architecture/api-conventions.md)
- [Envoy xDS protocol(ADS)](https://www.envoyproxy.io/docs/envoy/latest/api-docs/xds_protocol) · [etcd data model](https://etcd.io/docs/v3.5/learning/data_model/) · [etcd Watch API](https://etcd.io/docs/v3.7/learning/api/)
- [Erlang — processes / signal ordering](https://www.erlang.org/doc/system/ref_man_processes.html) · [Erlang message passing](https://www.erlang.org/blog/message-passing/)
- [Kleppmann — distributed locking](https://martin.kleppmann.com/2016/02/08/how-to-do-distributed-locking.html) · [Thompson — single writer principle](https://mechanical-sympathy.blogspot.com/2011/09/single-writer-principle.html)
- [Zed collab db.rs](https://raw.githubusercontent.com/zed-industries/zed/main/crates/collab/src/db.rs) · [wezterm mux](https://github.com/wezterm/wezterm/blob/main/mux/src/lib.rs) · [tmux control-notify.c](https://raw.githubusercontent.com/tmux/tmux/master/control-notify.c) · [zellij background_jobs.rs](https://raw.githubusercontent.com/zellij-org/zellij/main/zellij-server/src/background_jobs.rs) · [JupyterLab session manager](https://github.com/jupyterlab/jupyterlab/blob/main/packages/services/src/session/manager.ts) · [herdr socket API](https://herdr.dev/docs/socket-api/)
- [DAP specification](https://microsoft.github.io/debug-adapter-protocol/specification) · [Tauri — calling frontend](https://v2.tauri.app/develop/calling-frontend/) · [tokio watch](https://docs.rs/tokio/latest/tokio/sync/watch/index.html) · [Phoenix.Tracker](https://hexdocs.pm/phoenix_pubsub/Phoenix.Tracker.html)
- [LSP#1706](https://github.com/microsoft/language-server-protocol/issues/1706) · [ZOOKEEPER-1277](https://issues.apache.org/jira/browse/ZOOKEEPER-1277)
