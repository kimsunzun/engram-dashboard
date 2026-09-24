# TRD — 첫 대화 뒤에만 이어받는다 (S21)

> 상태: **개정 4판(2026-09-24) — 리뷰 4회차 지적 넷 반영**(사용자 종료 중의 제출을 보내지도 않는다 · 메모 A 문구 셋). 3판 = 리뷰 3회차(FIX) 지적 여섯 반영. 2판 = 리뷰 2회차(FIX, 리뷰어 셋) 반영 + 사용자 결정 Q1·Q2·로딩 배치. 코드는 아직 한 줄도 안 바뀌었다. 판독 기준 = master `93f0f9f`(운영 코드는 `e0af391` 과 같다). `file:line` 은 그 시점에 직접 확인한 것이다. 리뷰 지적의 코드 주장은 전부 다시 읽어 확인했고, **코드와 어긋난 1회차 지적 하나(RichSlot 비우기 창)는 반영하지 않았다** — 근거는 §3-5(2회차 리뷰어 셋이 각자 그 반박을 확인했다). 2회차의 Phase B 몫 지적은 §3-4 끝 「B 라운드 착수 전 반영할 리뷰 지적」에 모았다.
>
> **이 문서가 스스로 고른 것**(D3 로 구현에 위임된 자리 · 사용자 체감이 없는 내부 구현)은 본문에 「고름」으로 표시한다. 사용자 결정이 필요한 것은 §7 에만 둔다.
>
> **두 덩이로 나뉜다 — Phase A(D1+D2+D5) · Phase B(D4).** 어느 쪽이든 혼자 착지해도 빌드가 서고, 한쪽만 착지했을 때의 동작은 §6-3·§6-4 가 적는다. 절 제목의 [A]/[B] 가 그 소속이다. 나눌지·어느 쪽 먼저인지는 §7 Q5 — **결정됨: A 를 이번 라운드, B 를 다음 라운드**(사용자 2026-09-24).
>
> 앵커: ADR-0001(kill 2동사·join 상한) · ADR-0006(락 순서) · ADR-0008 · ADR-0012(시험대) · ADR-0019(등록 순서·reaper 단일 소비자·런타임 재시작 없음) · ADR-0038(시간 대신 신호) · ADR-0076 · ADR-0077(선례) · ADR-0082 · ADR-0083 · ADR-0145 · ADR-0172 · ADR-0185 · ADR-0201 · ADR-0202 · ADR-0204 · ADR-0216 · ADR-0217 · ADR-0218.

---

## 1. 문제

**P1 — codex 이어받기에서 챗 슬롯이 1.2~2.2초 동안 「새 에이전트」 첫 화면을 보였다 사라진다.** 실측 4/4(GUI). claude 이어받기는 안 깜빡이고, 새 codex 는 첫 화면이 맞다.

- 시간선(재활성화 기준 ms): 명부 공표 32–183 → Subscribe → replay 0 프레임으로 `ReplayComplete` +10 → **첫 화면** → `initialize` 341–587 → `thread/resume` 1393–2243 → 첫 라이브 프레임 = `Usage`(seq 0) → 첫 화면 사라짐 → `thread/items/list` +20–70 → 이력 렌더. 지배항은 codex 자신의 `thread/resume` 왕복이다.
- 인과 사슬:
  1. `manager.rs:1447` 이 `spawn_session` 직후 명부를 공표한다(등록 `:1890` → `start_pump` `:1902` → `session.rs:153` `transport.start`).
  2. 프론트가 재부착 → 데몬이 링을 동기 replay 하고 `ReplayComplete` 를 낸다(`connection_core.rs:1773` · `:1821`).
  3. claude 는 이력을 공표 **전에** 링에 심는다(`manager.rs:1375` `resume_transcript_events` → `:1834` `core.seed`). codex 는 **통로가 공표 뒤에** 넣는다 — `transport.rs` `writer_loop` 가 예산을 잡고(`:1441`) 핸드셰이크·`thread/resume`(`:810`)→ id 기록(`:1455`) → `thread/items/list` 로 이력(`:931`, 넣는 자리 `:1483`) → 게이트(`:1493`).
  4. `RichSlot.tsx:190` `replayDone = state==='live'` · `:290` `showEmpty = replayDone && !hasSent && items.length===0` — 「복원 끝 + 0건 = 새 에이전트」.
- ★ADR-0145 의 전제(「복원 끝 + 0건」이 새 에이전트를 정확히 가른다 — `0145:37`)를 **ADR-0204 가 말없이 깼다**★. ADR-0204 는 첫 화면·`ReplayComplete`·ADR-0145 를 언급하지 않는다(그것이 수용한 대가는 `0204:45-53` 의 다른 것들이다). 2026-09-22 인계 메모의 「ADR-0204 가 알고 수용했다」는 틀렸다.
- 프론트만 고치는 시도 둘은 실패해 되돌렸다(`32a76c9`/`bdcf086`, 기록 = `.claude/handoff/history/20260922-120942-v030-release-and-followups.md:62-74,127,165`): ① 「codex 이고 id 가 있으면 첫 화면 숨김」 → 새 codex 가 첫 화면을 잃었다(id 가 스폰 몇 초 뒤 기록된다) ② 부착 시점 래치 → 그 시점엔 프로필이 아직 스토어에 없다.

**P2 — 한 마디도 안 하고 끈 에이전트는 다시 켤 수 없다.** 매번 「이어받을 대화가 없습니다」(`ko.ts:91-92`)로 실패한다.

- codex app-server: `thread/resume` → `-32600 "no rollout found"` → `codex/mod.rs:711` `resume_failure_kind` = `NoConversationToResume` → `manager.rs:2133` `LinkFailed` 팔 → `tear_down_failed_activation`. 새 대화로 넘어가지 않는다(ADR-0082).
- claude: 스폰 때 우리가 `--session-id` 를 발급·**영속**하는데(`manager.rs:1238` → `profile.rs:674` `new_session_id`) claude 는 첫 교환이 있어야 그 대화를 만든다(ADR-0172 맥락).
- codex 터미널: 락 회수가 대화 없이 몇 초 안에 id 를 잡아 영속한다(ADR-0218 · `codex/mod.rs:1087-1095` — 훑기는 스폰 뒤 **적어도** 0.5·1.5·3.5·7.5 s … + 훑기 시간, §3-2-4 · GUI 실측 한 번은 약 6초 — `step-log.md:2455`). 입력 없이 닫으면 재개가 exit 1(`step-log.md:2442`).
- 사용자가 2026-09-21 에 「그냥 새 세션」 방향으로 고치기로 했다(`step-log.md:2442`).

---

## 2. 사용자 결정 (2026-09-23/24 — 그대로 구현한다)

- **D1.** 세션 id 는 **첫 대화 뒤에만** 영속한다 — 모든 백엔드·모드 공통(claude·codex × JSON·터미널). claude 는 스폰 때 뽑은 id 를 **메모리에만** 두고 첫 대화 때 영속한다. 「저장된 id 가 있다 ⟺ 이어받을 대화가 있다」가 한 규칙이 된다. 모드 선택은 이미 id 유무로 갈린다(`can_resume_profile` = 이어받기 축 ∧ id 존재, `backend/mod.rs:704`; 입구 = `manager.rs:1942` · `commands.rs:574` · `connection_core.rs:1385`) — 거기는 안 바꾼다. id 없음 → Fresh.
- **D2.** 챗 슬롯은 「이 화신이 대화를 이어받았나」(예/아니오 — id 자체가 아니다)를 **구독 응답**에서 받는다. id 가 생길 때의 broadcast 가 아니다. 사유: 구독 응답은 replay·`ReplayComplete` 와 **같은 순서 스트림**을 타고, 값은 스폰 때 화신마다 고정된다. `SubscribeAck.action` 의 `Resume`(seq 이어받기)과 이름이 겹치지 않게 한다.
- **D3.** 백엔드·모드별 영속 방아쇠는 구현에 위임(코드상 최선을 골라 문서화). 「저장했는데 백엔드가 대화를 안 만들었다」는 D4 가 흡수하므로 괜찮다.
- **D4.** 이어받기가 **「이어받을 대화가 없다」 부류**(`NoConversationToResume`)로 실패하면 **자동으로 새 대화를 연다.** 다른 실패는 ADR-0082 그대로(멈추고 보고). 루프 금지. ADR-0082 를 이 부류에 한해 개정하는 ADR 이 필요하다.
- **D5.** 이어받은 화신이 이력을 기다리는 동안 챗 슬롯은 **새 공용 「로딩 패널」**(로딩 아이콘 + 선택적 텍스트 칸)을 그린다. 이번 용도는 **아이콘만, 텍스트 없음**. e-ink·reduced-motion 에서는 정지 아이콘. 시간 기반 지연 없음(ADR-0038 · `0145:26`). 대기가 끝나는 때 = 첫 이력 항목 도착 · 사용자 입력 · 화신 종료(지금의 막) · D4 재시작(새 화신 → 표식 거짓 → 첫 화면).

---

## 3. 설계

### 3-1. 한눈에

| 결정 | 무엇이 새로 생기나 | 자리 |
|---|---|---|
| D1 [A] | 화신마다 **첫 제출 래치**(`SessionIdLatch`) — 「id 를 알았다」와 「첫 턴이 제출되려 한다」가 둘 다 일어난 첫 순간에 한 번, **비교-교체** 동사로 영속한다 | agent crate 새 모듈 + registry 새 동사 + `session.rs` 입력 두 동사 + `manager.rs` 조립(필수 인자) |
| D2 [A] | 화신 사실 `continues_conversation: bool` → **구독 응답이 replay 와 같은 화신에서** 표식과 함께 읽는다 → `SubscribeAck` 칸 → 셸 마커 flag bit2 → 프론트 `'live'` 통지 | agent · daemon · protocol · src-tauri · src/api |
| D3 [A] | 방아쇠 = **입력 경계**(백엔드 중립). 표 = §3-2-5 | 위와 같음 |
| D4 [B] | `resume_no_fallback` 의 실패 팔 셋에서 그 부류면 — **예약을 쥔 채** 거두고 → reaper 가 「그 화신이 다 거둬졌나」를 답하는 동기화 → 사용자 종료·셧다운 재확인 → **같은 예약으로** 새 대화 | `manager.rs` + `reaper.rs` |
| D5 [A] | `LoadingPanel` 공용 컴포넌트 + RichSlot 게이트 둘 + 대기 꼬리 판정 정렬 | `src/components/ui/` · `RichSlot.tsx` |

**통로(transport)와 백엔드의 계약은 바뀌지 않는다** — 둘은 여전히 「id 를 알게 되면 `SessionIdSink` 를 부른다」만 안다. 바뀌는 것은 조립점이 그 자리에 무엇을 꽂느냐다(오늘 = 곧바로 영속하는 sink, 이후 = 래치의 수령 손잡이). 그래서 `transport.rs` 의 기록 순서 시험(`:4123`·`:4150`·`:4169`·`:4763`)과 `codex/mod.rs` 의 수령 시험(`an_app_server_spawn_hands_the_thread_id_to_the_sink` · `a_recording_failure_ends_the_session`)은 **그대로 유효하다**(§4-2).

### 3-2. D1 — 첫 제출 래치 [A]

#### 3-2-1. 동작

화신 하나에 래치 하나. 입력 둘, 출력 하나.

- **`offer(id)`** — 이 화신의 세션 id 가 알려졌다.
  - claude(두 모드): ★**래치를 만드는 그 자리에서, `open_spawn` 보다 먼저**★ 넣는다 — Fresh 는 메모리에서 뽑은 값, Resume 은 spawn 이 읽은 저장값. 판정 축은 backend 이름이 아니라 **발급 축**(`assigns_session_id` — spawn 이 `sid` 를 쥐었나)이라 `if let Some(s) = sid { latch.offer(..) }` 한 줄로 백엔드 중립이다.
    - ★늦게 넣으면 안 되는 이유(리뷰 지적 반영)★: 세션은 `spawn_session` 안의 명부 등록(`manager.rs:1887-1890`) 순간부터 입력을 받을 수 있고, 뒤이은 명부 공표(`:1447`)가 파킹된 우편의 flush 를 부른다(`messaging_host.rs:450-490`). 그보다 늦게 offer 하면 첫 제출이 id 를 모르는 채 나가고 commit 이 턴보다 늦는다.
  - codex app-server: 통로 라이터가 핸드셰이크 응답에서 받은 id 를 기존 자리(`record_session_id`, `transport.rs:1455`)에서 넣는다.
  - codex 터미널: 락 회수 스레드가 기존 자리(`thread_lock` — `codex/mod.rs:1087-1095`)에서 넣는다.
- **`note_submission()`** — 이 화신에 첫 사용자 턴이 **제출되려 한다**(상대에게 나가기 직전).
- **`commit(id)`** — 기존 기록 포트(`session_id_sink`, `manager.rs:494` — uuid 해석·로그)는 그대로 두고, 그 안에서 부르는 동사만 **비교-교체**로 바꾼다: `ProfileRegistry::commit_session_id(id, epoch, expected, new)`(§3-2-3). 화신 가드 + 「칸이 **이 화신이 시작할 때 본 값**(`expected`)이거나 이미 `new` 일 때만」 쓰고 즉시 영속한다. `expected` = spawn 이 release 뒤 본 값(Fresh = `None` · Resume = spawn 이 읽은 저장값)이고, commit 이 한 번 성공하면 포트가 `expected` 를 그 값으로 옮긴다 — ★방어 절이다★: 아래 규칙대로 포트는 화신당 많아야 한 번 불리므로 운영에서는 옮긴 값이 다시 쓰이지 않는다(ADR-0216 과의 관계 논증은 첫 offer 위에 선다 — 끝의 메모 A). 새 기록 경로를 만들지 않는다 — 같은 포트의 동사 교체다.
  - ★덮어쓰기 동사(`observe_session_id` — `profile.rs:709-722`)를 쓰지 않는 이유(리뷰 지적 반영)★: 그 동사는 무조건 교체라, 첫 제출 **전에** 다른 기록자가 칸을 바꿨으면 commit 이 스폰 때 값으로 **되감는다.** 그 기록자가 실재한다 — ADR-0008 추적기가 `observe_session_id(id, None, new)` 로 대조 없이 쓴다(`daemon/src/lib.rs:289`). 예: claude 이어받기 화신이 기동하며 id 를 바꾸면(추적기가 있는 이유가 그 표류다 — ADR-0202 근거 절) 추적기가 Y 를 적고, 뒤이은 첫 제출의 commit(S)이 S 로 되돌리며 Y 를 이력으로 민다.
  - codex 의 `thread/resume` 가 저장값과 다른 id 를 돌려주는 경우(관측된 적 없다)는 칸 == `expected`(S)라 오늘처럼 교체된다 — 비교-교체가 오늘의 codex 동작을 바꾸는 자리는 없다.
- **규칙:** `offer` 와 `note_submission` 이 **둘 다** 일어난 첫 순간에 `commit` 한 번. 순서는 상관없다 — 제출이 먼저 왔으면 뒤이은 `offer` 가 그 자리에서 `commit` 한다. **제출 없이 화신이 끝나면 아무것도 영속되지 않는다** — D1 이 원하는 결말이다.
  - ★**offer 는 화신당 많아야 한 번이다**★(리뷰 지적 반영): 기록 포트의 계약이다(`backend/mod.rs:189` 「호출은 그 id 를 실제로 받은 뒤 정확히 한 번」). 네 칸이 모두 지킨다 — claude = 래치 생성 때 한 번(claude `open_spawn` 은 sink 를 버린다 — `claude/mod.rs:408`) · codex app-server = 라이터의 핸드셰이크당 한 번(`transport.rs:1455`) · codex 터미널 = `Found` 에서 sink 를 부르고 루프를 끝낸다(`thread_lock.rs:444-446`).
  - ★**그래서 규칙은 하나다 — 「포트는 화신당 많아야 한 번 불린다. 한 번 부르기 시작했으면(성공·거절·패닉 무엇으로 끝나든) 다시 부르지 않는다」**★. 계약 밖의 둘째 offer 는 언제 오든 버린다(debug 로그). 거절·패닉 뒤의 처분은 §3-2-2.
- 이어받기 화신은 `offer` 값이 이미 저장된 값과 같으므로 `commit` 이 no-op(비교-교체의 같은 값 갈래 — 디스크 쓰기 없음)이다.

#### 3-2-2. 자리

- **새 모듈** `crates/engram-dashboard-agent/src/session_id_latch.rs`. `backend::SessionIdSink` 타입(`backend/mod.rs:203`)과 로그 필드용 `types::AgentId` 만 알고 backend·transport·profile 을 모른다(uuid 해석·`expected` 추적은 commit 포트 안 — manager 쪽이다).
  ```rust
  pub(crate) struct SessionIdLatch { agent: AgentId, epoch: u32, commit: SessionIdSink, settled: AtomicBool, state: Mutex<LatchState> }
  struct LatchState { pending: Option<String>, submitted: bool, committed: bool }
  impl SessionIdLatch {
      pub(crate) fn new(agent: AgentId, epoch: u32, commit: SessionIdSink) -> Arc<Self>; // agent·epoch = 로그 필드만
      pub(crate) fn offer_sink(self: &Arc<Self>) -> SessionIdSink; // backend·통로가 받는 것
      pub(crate) fn offer(&self, raw: &str);
      pub(crate) fn note_submission(&self);
  }
  ```
  - ★**빠른 길의 표식은 「commit 이 돌아왔다」다 — 「제출이 있었다」가 아니다**★(리뷰 지적 반영): `settled` 는 첫 commit 이 **돌아온 뒤에만** 선다(Release 로 쓰고 Acquire 로 읽는다). 제출 표식을 빠른 길로 쓰면 두 제출자가 다른 스레드일 때(우편 flush 스레드의 `submit_input_observed` · 연결의 사용자 입력) 둘째가 첫째의 commit 이 끝나기 **전에** 빠른 길로 빠져 `send_input` 한다 — 첫 턴이 영속 전에 나간다(claude 두 모드 · codex 터미널). settled 전의 note 는 전부 뮤텍스를 잡고, 진행 중인 commit 뒤에 줄을 선다.
  - offer 가 아직 없는 동안(codex 핸드셰이크 중 · 락 회수 전)에는 commit 할 것이 없어 settled 가 안 선다 — 그동안의 note 는 경합 없는 뮤텍스 한 번씩이다(고름: 「commit 이 영영 없을 화신」을 따로 판정하지 않는다 — 판정할 신호가 없고 비용이 뮤텍스 한 번이다).
  - ★**id 가 제출보다 먼저 오면 info 로그 한 줄**★(리뷰 지적 반영): `offer` 가 제출 전에 오면 `tracing::info!(agent = %self.agent, epoch = self.epoch, "세션 id 를 받았다 — 첫 제출 전이라 영속을 보류한다")`. 사유: A 뒤에는 「id 가 왔다」를 알리는 줄이 commit 포트의 반영 로그(`manager.rs:514-518`)뿐이라 **0턴 화신은 아무것도 안 남긴다** — GUI 시행 7(§4-4)의 기다릴 사건이 없고 현장 진단도 그 줄을 잃는다. 레벨·모양은 `docs/reference/logging-conventions.md`(정상 수명주기 = info · 식별자는 필드)이고 그 기존 로그처럼 **id 값은 싣지 않는다**. offer 가 화신당 한 번이라(§3-2-1) 화신당 많아야 한 줄이다. 제출 뒤에 온 offer 는 곧바로 commit 하므로 이 줄 대신 포트의 반영 로그가 남는다.
  - ★**commit 이 거절·실패·패닉해도 규칙은 하나다 — 「포트는 화신당 많아야 한 번 불린다. 한 번 부르기 시작했으면(성공·거절·패닉 무엇으로 끝나든) 다시 부르지 않는다」(고름)**★(리뷰 지적 반영 · §3-2-1 의 그 규칙): 래치는 commit 포트를 부르기 **전에** `committed` 를 세운다. 포트가 무엇으로 끝나든 재시도는 없다. 포트는 반환이 없어 결과가 래치에 돌아오지 않는다(`SessionIdSink` doc — `backend/mod.rs:199-200`). commit 을 부르는 자리는 넷이다 — WS 도착순 줄의 `WriteStdin`(`connection_core.rs:1087`) · 우편 flush(`messaging_host.rs:753` 의 `spawn_blocking` → `:135` `submit_stdin_observed`) · codex 라이터의 `record_session_id`(`transport.rs:1355`) · codex 터미널 회수 스레드(`thread_lock.rs:445`). 넷 모두 아래 셋으로 끝난다.
    1. **거절**(`commit_session_id` 가 `false` — 표식 불일치 · 다른 기록자가 먼저 바꿈 · 프로필 부재, 또는 uuid 해독 실패): 포트가 로그를 남기고 돌아온다 → settled → **턴은 그대로 나간다.** 재시도하지 않는다(같은 입력이면 답도 같다).
    2. **디스크 IO 오류**: `ProfileStore::save` 가 삼킨다(`persistence/mod.rs:102-106` — error 로그뿐). `bool` 은 메모리 기준이라 포트에는 성공으로 보인다 → 1과 같이 settled · 턴 나감. 다음에 성공하는 저장(어느 프로필 변경이든 명부를 통째로 쓴다)이 디스크를 고친다. 오늘의 `observe_session_id` 와 같은 등급이다(§3-6).
    3. **패닉**: 포트 아래 패닉원은 poison 가드의 `expect` 뿐이다(레지스트리 맵 `profile.rs:395`·`:406` · 저장소 쓰기 락 `persistence/mod.rs:101`). 그러니 **남의 패닉이 먼저 그 락을 오염시켰을 때만** 선다(codex 쪽의 같은 분석 — `transport.rs:1312-1318`). 래치 자신은 패닉원을 만들지 않는다(인덱싱·`unwrap` 없음).
       - ★**릴리스에서는 이 갈래가 없다**★: 워크스페이스 `[profile.release]` 가 `panic = "abort"` 다(`Cargo.toml:35`). 첫 패닉이 데몬을 그 자리에서 끝내므로 오염된 락이 생기지 않는다. 결말은 데몬 종료다. note 에서 난 패닉이면 `send_input` 앞이라 그 턴은 안 나갔다. offer 쪽이면 codex 라이터는 큐에 선 턴과 함께 끝나고, 회수 스레드는 첫 턴이 이미 나간 뒤다(§3-2-4 예외와 같은 등급). 디스크는 마지막 저장 그대로이고, 재기동 뒤에는 영속된 id 로만 복원한다(§3-6 「데몬 재기동」). WS 줄의 프로필 CRUD 가 오늘 지는 `.expect` 와 같은 등급이다 — 새 종류의 위험이 아니다.
       - ★그래서 래치에 `catch_unwind` 를 두지 않는다(고름)★. 릴리스에서는 아무것도 잡지 않는 죽은 코드다(`transport.rs:1319-1322` 가 자기 봉쇄를 그렇게 적는다). unwind 빌드(개발·시험)에서의 이득도 「남의 패닉 위에 겹쳐 죽지 않는다」 하나다.
       - unwind 빌드에서의 결말: 래치 뮤텍스에 독이 든다. 뒤이은 offer·note 는 `into_inner` 로 회수한다. `committed` 가 이미 참이라 settled 를 세우고 통과한다 — 포트를 다시 부르지 않는다(위 규칙 그대로 · 시험 ⑨). 패닉 자체는 그 호출을 타고 올라간다 — note 에서 났으면 **그 한 번의 입력은 안 나간다.** 받는 것은 호출자 쪽의 기존 처분이다: WS = 그 연결의 도착순 소비 태스크가 패닉으로 끝난다(그 뒤 연결 처분은 확인하지 않았다) · 우편 = `DeliveryDone` 의 「언와인딩됐다」 경고(`messaging_host.rs:716-719`) · codex 라이터 = `catch_unwind` → 연결을 내린다 · 회수 스레드 = 그 스레드가 끝난다(영속은 이미 시도됐다).
- **`InputEncoder::submits_turn(&self, bytes: &[u8]) -> bool`**(`backend/mod.rs`, `submit_sequence` `:917` 옆). 무엇이 턴 제출인가는 backend 지식이라 여기 둔다(ADR-0004).
  - `ClaudeStreamJson` · `TransportFramed` → 항상 `true` — 「호출 1회 = 완결된 유저 턴 1개」 계약(`session.rs` `write_input` doc FIX 6a · codex `send_input` 이 쓰기마다 `Job::Turn` 하나, `transport.rs:2490-2512`).
  - `Raw` → `bytes` 에 제출 바이트(CR)가 들어 있으면 `true`.
  - ★**CR 판정은 프론트의 키 인코딩에 매인다**★(리뷰 지적 반영): 터미널 Enter = CR 은 xterm 이 기본 키 인코딩으로 보내는 바이트다. 설치본 `@xterm/xterm` 6.0.0 의 타입 정의에 kitty 키보드·win32-input-mode 같은 인코딩 전환 옵션이 없다(확인). 그런 모드를 켜는 변경이 오면 Enter 가 CR 이 아닌 시퀀스로 와 이 판정이 **조용히** 거짓이 된다 — 제출이 안 세어져 그 모드의 이어받기가 전부 사라진다. 그 자리에 주석을 달고 시험 이름에 가정을 박는다(`raw_submission_is_cr_under_default_xterm_encoding`). 같은 가정에 선 `submit_sequence`(`:917`)와 짝이다.
- **`AgentSession`**(`session.rs`): 래치를 들고, 입력 두 동사가 `send_input` **앞에서** 부른다.
  - `write_input_observed`(`:195`) — `self.encoder.submits_turn(bytes)` 면 아래 `UserKill` 확인 + `note_submission()` 후 `send_input`.
  - `submit_input_observed`(`:251`, 우편 배달) — ★**제출 CR 바로 앞에서 센다**★(리뷰 지적 반영): `if let Some(submit)` 갈래 안, 대기(`:271`) 뒤·제출 `send_input`(`:277`) 바로 앞에서 아래 `UserKill` 확인 + `note_submission()`. 제출 CR 이 `write_input_observed` 를 거치지 않고 `transport.send_input` 으로 직접 나가기 때문이다. `Raw` 는 언제나 `Some(CR)` 이고, JSON 인코더 둘은 `None` 이라 그 갈래가 없다(`backend/mod.rs:917-921`) — 그쪽은 본문이 곧 턴이라 `write_input_observed`(`:252`) 안에서 이미 본문 **앞**에 센다. 그래서 「첫 턴 전에 영속」은 `Raw` 에서도 그대로다(턴을 여는 것은 CR 이다).
    - 옛 자리(맨 앞)를 버린 이유: 거기서 세면 본문 쓰기 + 본문 착지 확인(`confirm_written` — 상한 `INPUT_FLUSH_BUDGET` 5 s, `session.rs:70`) + `SUBMIT_PACING` 0.5 s 가 CR 앞에 끼어, 그 창의 kill 이나 본문 쓰기·착지 확인 실패가 **턴 없는 영속**을 남긴다(A 만이면 다음 활성화가 P2 의 실패). 파킹 우편은 명부 공표 직후 flush 되므로(`manager.rs:1447` → `messaging_host.rs:450-490`) 스폰 직후의 kill · 부팅 복원 도중의 `shutdown_all` 이 바로 그 창에 닿는다.
    - 본문 자체에 CR 이 든 우편(보낸 쪽이 CR 을 실었을 때 — 드물다)은 `write_input_observed` 의 판정에서 본문 앞에 먼저 세어진다. 붙여넣기 행과 같은 부류다(§3-6).
  - ★**사용자 종료 중이면 세지도 보내지도 않는다**★(리뷰 지적 반영 — 4회차가 「보내지도」를 더했다): 래치가 있는 세션에서 두 동사는 note 자리에서 먼저 `termination_intent() == UserKill`(`session.rs:145`)을 본다. 섰으면 note 도 그 자리의 `send_input` 도 하지 않고 곧바로 `Err(PtyError::WriteFailed(..))` 를 돌려준다 — 세는 자리의 **턴을 내는 쓰기**(CR 이 든 키 입력 조각 · JSON 턴)는 본문째 안 나가고, `Raw` 우편은 제출 CR 이 안 나간다. CR 없는 `Raw` 키 입력은 세는 자리가 아니라 kill 창에서도 입력 큐가 닫힐 때까지 그대로 나간다(턴을 열지 않으므로 무해하다). `Raw` 우편의 본문은 kill 전에 쓰였을 때만 이미 나갔다 — 보통은 `UserKill` 뒤 수 µs 안에 큐가 닫혀 본문 쓰기 자체가 같은 `WriteFailed` 로 먼저 실패한다. ★거절에는 제 로그 줄과 사용자 종료를 명시하는 제 오류 문구를 둔다★(변형 `WriteFailed`·`Err` 모양은 그대로): 기존 「본문은 썼으나 제출 write 실패」 갈래(`session.rs:277-284`)의 warn 문구(「재시도가 그 위에 덧쓴다」)를 재사용하지 않는다 — 재시도는 새 화신의 PTY 로 가서 덧쓰이는 것이 없고, 메시징 커널이 오류의 Display 문구를 배달 실패 관측으로 적으므로(`service.rs:1763` → `:2120`) 일반 문구면 전송 결함으로 읽힌다. 확인은 settled 뒤에도 한다(고름: 원자 읽기 하나이고, settled 로 가르면 판정이 둘로 갈린다). 사유: `kill_agent` 는 `UserKill` 을 세운 뒤(`manager.rs:2715`) 통로 종료와 join 을 최대 5 s 기다리고(`:2722`), 세션은 reaper 가 거둘 때까지 명부에 남는다(`reaper.rs:51-60`). 그 창에 온 `WriteStdin`·우편 flush 는 표식 가드를 통과한다(kill 은 표식을 안 바꾼다). 세고 보내면 대화 없는 id 가 영속되고 턴은 `send_input` 에서 떨어진다 — A 만 착지했으면 다음 활성화가 P2 의 실패를 그대로 맞는다. ★세지만 않고 보내면 반대쪽이 샌다★: `UserKill` 과 입력 큐 닫기(`session.kill` → `shutdown` 1b — `transport/pty.rs:412-419` · `transport/stdio.rs:402` · codex 는 `closed` 표식 `transport.rs:2584`) 사이에 받아들여진 CR·턴을 라이터가 자식이 죽기 전에 넘길 수 있고, 그러면 **영속되지 않은 id 의 대화**가 생길 수 있다.
    - ★오늘과 같은 결말 부류다★: 큐가 닫힌 뒤의 `send_input` 은 이미 `Err(WriteFailed)` 다(`transport/pty.rs:368-378` → `input_queue.rs:112-113` · stdio 도 같은 큐 · codex `transport.rs:2494-2498`). 이 규칙은 같은 오류를 `UserKill` 순간으로 앞당길 뿐이다(고름: 변형도 같은 `WriteFailed` — 호출자가 가를 새 어휘를 만들지 않는다). 받는 쪽 처분도 닫힌 뒤의 `Err` 와 같다 — WS 는 그 요청에 `Error` 답장(`connection_core.rs:1087` → `reply` `:978-987`) · 우편은 inject 실패 팔이 그 수신자의 남은 배치를 무손실 재파킹하고(pending 유지 — 다음 등장에 재시도) 실패 관측을 남긴다(`messaging_host.rs:135` → 메시징 커널 `service.rs:1726-1771`). 오늘과 달라지는 것은 그 창에서 받아들여져 죽어가는 자식에게 「배달됨」이 될 수 있던 우편이 이제 재파킹된다는 것 하나다.
    - 래치 없는 기본 세션(시험 · A1 까지의 운영)에서는 이 확인도 무동작이다 — A1 의 「동작 불변」이 그대로다.
    - 데몬 셧다운도 같은 길로 덮인다 — `shutdown_all` 은 세션마다 `kill_agent` 를 불러(`manager.rs:2771`) 같은 `UserKill` 을 세운다. 셧다운 중의 입력이 영속도 턴도 남기지 않는 것이 맞는 결말이다.
    - `tear_down_failed_activation`(`manager.rs:2215`)은 `UserKill` 을 세우지 않는다(종료 분류가 거짓이 되므로 — `:2207-2209`). 무해하다: 연결이 거절된 codex 에는 offer 가 없고(id 는 핸드셰이크가 성공해야 온다 — 백스톱을 넘긴 뒤 늦게 선 핸드셰이크는 §3-6 「영속했는데 제출 write 가 실패」 행과 같은 부류), B 가 claude 에 쓰게 돼도 claude Resume 의 offer 는 저장값이라 비교-교체가 no-op 이다.
    - ★남는 창 둘(닫지 않는다)★: ① intent 를 본 **뒤**·commit 전에 kill 이 서는 경주 — 그 입력은 오늘처럼 `send_input` 까지 간다(큐가 아직 열려 있으면 받아들여진다). 폭은 판정 한 번이다(우편도 확인이 CR 직전이라 본문 쓰기·착지 확인·`SUBMIT_PACING` 이 이 창에 들지 않는다). ② kill **전에** 세어진 제출에 kill 창 안의 offer 가 닿는 경우 — 확인은 제출 쪽에만 있으므로 이 offer 쪽 창은 안 덮인다. codex app-server 핸드셰이크 중이면 `record_session_id` 의 종료 확인(`transport.rs:1344-1348`)이 `UserKill` 과 통로의 `shutdown` 표식 사이를 못 본다. codex 터미널 회수는 첫 턴이 이미 나갔으므로 영속이 맞다. ①·② 모두 「영속했는데 제출 write 가 실패」 행과 같은 부류다(§3-6).
  - `AgentSession::new` 서명은 **안 바꾼다** — 운영 밖 호출자가 16곳이다(운영 `manager.rs:1859` 까지 17 — agent 단위·통합 시험, daemon 시험, `drain_latency` dev bin). 그중 crate 밖(agent `tests/` · daemon)이 있어, 필수 인자로 만들면 래치 타입을 `pub` 로 열어야 한다. 대신 빌더 둘을 더한다 — `with_incarnation(continues_conversation: bool)` 과 `with_session_id_latch(Arc<SessionIdLatch>)`, 둘 다 `pub(crate)`(고름: 운영 밖에서 부를 일이 없고, 시험은 빌더가 아니라 실제 spawn 경로로 표식을 세운다 — §4-1). 기본값(빌더 안 부름)은 「이어받기 아님 · 래치 없음」.
  - ★**화신 표식 칸은 하나다 — 기존 `epoch`(`session.rs:28`)**★(리뷰 지적 반영): 빌더는 `continues_conversation` 만 받는다. 표식을 함께 받는 모양(`Incarnation{epoch, ..}`)은 `AgentSession.epoch` 옆에 두 번째 표식을 만든다. 그 칸은 이미 `WriteOutcome`(`:222`)·`agent_info`·`write_stdin_observed_if_epoch` 가 읽는다. 둘이 갈리면 어느 쪽이 화신인지 모르게 된다 — 소유권 분할(`session=id/cwd/epoch/…`) 위반이다. 구독 응답의 `Incarnation` 은 값 타입이다 — 응답을 만들 때 `Incarnation { epoch: self.epoch, continues_conversation: self.continues_conversation }` 로 조립한다(§3-3).
  - ★**운영 조립점에서는 필수다**★(리뷰 지적 반영): 운영에서 세션을 만드는 곳은 `spawn_session`(`manager.rs:1791`, 운영 호출자 하나 `:1423`) 하나다. 그 함수가 `SpawnIncarnation { continues_conversation, latch }` 를 **`Option` 이 아닌 필수 인자**로 받아 빌더를 부른다. ★빠뜨리면 무슨 일이 나나★: 제출이 한 번도 안 세어져 commit 이 영영 안 난다 → 어느 화신도 id 를 영속하지 못해 **모든 이어받기가 조용히 사라진다**(다음 활성화가 전부 Fresh — 오류가 없고, 남는 것은 offer 의 「영속 보류」 info 로그뿐이다). 그래서 기본값이 운영에 닿는 길을 컴파일러로 막고, 남는 배선은 소스 구조 시험이 잰다(§4-1).
- **`manager.rs` `spawn_agent_watching_link`:** 래치를 만들고(commit = 비교-교체 포트 — `session_id_sink(profiles, id, epoch, expected)`), claude 면 **그 자리에서** 뽑은 값/저장값을 `offer` 하고, `open_spawn` 의 sink 자리(`manager.rs:1412` `Some(session_id_sink(...))`)에 `latch.offer_sink()` 를 넘기고, `spawn_session` 에 `SpawnIncarnation` 으로 같은 래치를 싣는다.

#### 3-2-3. registry 동사 (claude 쪽이 실제로 바뀌는 곳)

- **Fresh:** `new_session_id`(뽑아서 **영속**, `profile.rs:674`)를 걷는다. 대신
  - 옛 값을 이력으로 밀고 빈 칸을 영속하는 **release** 동사 — 기존 `clear_session_id`(`:737`)와 같은 일이지만 「프로필이 사라졌다」를 `None` 으로 구별해 돌려준다(`profile_vanished_mid_spawn` 끊기를 보존해야 한다 — `manager.rs:1240`). 옛 값은 `old_session_ids` 로 간다 — ADR-0202 가 금한 파괴가 아니다.
  - 맵을 안 건드리는 `ProfileRegistry::mint_session_id() -> Uuid` 연관 함수 — 발급 단일점은 registry 에 남긴다(H-1.4 · `profile.rs:363`). 값은 여전히 매번 새 uuid 라 「Session ID already in use」 봉인(ADR-0076)은 그대로다.
- **새 비교-교체 동사** `commit_session_id(&self, id: AgentId, incarnation: u32, expected: Option<Uuid>, new: Uuid) -> bool` — 프로필 뮤텍스 안에서: 표식 불일치 → `false` · 칸 == `Some(new)` → `false`(쓰기 없음) · 칸 == `expected` → 옛 값을 이력으로 밀고 `new`, 영속, `true` · 그 밖 → `false`(다른 기록자가 먼저 바꿨다 — debug 로그). `observe_session_id` 는 추적기 몫으로 **그대로 남는다**.
  - ★**화신 가드와 이력 밀기는 `observe_session_id` 와 한 도우미를 쓴다**★(리뷰 지적 반영): 그 동사 본문의 표식 비교(`incarnation.is_some_and(|e| e != p.epoch)`)와 「옛 값 → `old_session_ids`, 새 값, `last_active`」 갱신(`profile.rs:711-719`)을 private 도우미 하나로 꺼내 두 동사가 함께 부른다. 새 동사가 가드를 따로 적으면 ADR-0217 결정 6 이 금한 「우회로를 새로 만든다」가 된다(ADR-0218 결정 6 이 승계). 새 동사가 더하는 것은 `expected` 비교 한 줄뿐이다.
- **`fresh_spawn_release_session_id`(`manager.rs:470`):** 조건 2(`!assigns_session_id`)를 뺀다 → `Fresh && (assigns_session_id || supports_control_channel || declares_link)`. 사유: 발급하는 backend 도 이제 칸을 비우고 시작하고, 그 칸을 다시 채우는 경로(래치 commit)가 있다. doc 의 「3번 열거는 손으로 관리된다」 목록에 「우리가 뽑아 첫 제출에 적는다」를 더한다.
- **Resume:** `ensure_session_id`(`:656`, 없으면 뽑아서 **영속**)를 운영 경로에서 걷는다 — 저장값을 읽기만 한다.
- ★**손잡이는 spawn 안에서 한 번 읽고, 그 읽기가 권위다**★(리뷰 지적 반영): 오늘 손잡이는 세 자리에서 따로 읽힌다 — 입구의 모드 판정(스냅샷 — `connection_core.rs:1385` 등) · `resume_no_fallback` 머리(`manager.rs:1999-2005`) · spawn 안(`stored_handle`, `:1350-1358` — 표식 확정 뒤). claude 의 argv 조립은 `sid` 가 `None` 이면 플래그를 **말없이 뺀다**(`claude/mod.rs:160` 터미널 · `:184` JSON). ensure 를 걷은 뒤 읽기 사이에 칸이 비면 claude 가 `--resume` 도 `--session-id` 도 없이 떠 **우리가 모르는 id 의 새 대화**를 열고, 판정은 `Alive` → `Resumed` 로 보고한다. 그래서:
  - 발급 축 backend 의 Resume `sid` = spawn 이 읽은 그 값이다(`stored_handle` 읽기를 발급 자리로 올려 한 번만 읽는다 — 「표식 확정 뒤」라는 그 읽기의 조건은 그대로다).
  - **Resume 인데 그 읽은 값이 없고 backend 가 저장 손잡이로 이어받는 쪽(`can_resume_stored_session`)이면 프로세스를 띄우기 전에 거절한다** — `PtyError::SpawnFailed("이어받을 손잡이가 spawn 도중 사라졌다")`. `resume_no_fallback` 의 `Err` 팔이 `SpawnFailed`(재시도 가능)로 기록하고, 다음 활성화는 id 가 없어 Fresh 로 간다. shell(`can_resume_stored_session` 거짓)은 옛 길 그대로다.
  - 아래 머리의 우회는 흔한 경우를 싸게 거르는 앞문이고, 이 거절은 두 읽기 사이의 경합에만 닿는 뒷문이다. **같은 술어**를 쓴다.
- **손잡이 없는 Resume → 새 대화 — ★고름이 아니라 사용자 결정 D1 의 귀결이다★**(리뷰 지적 반영). D1 이 「저장된 id 가 있다 ⟺ 이어받을 대화가 있다 · id 없음 → Fresh」로 정했고, 오늘 codex 우회가 같은 판단을 이미 「사용자 결정」으로 적어 두었다(`manager.rs:1989-1999`). ★**사용자·LLM 이 보는 결말이 바뀐다**★ — 손잡이 없는 claude 프로필에 `SpawnProfile{resume:true}` 가 오면 오늘은 `Failed` + `NoConversationToResume`(claude JSON — 터미널이 그 분류에 닿는지는 §3-4 도달성의 미확인과 같다), A 뒤에는 `Started`(새 대화)다. 그래서 개정 ADR 셋이 걸린다 — ADR-0076 결정 첫 항(「명시적 `resume=true` 는 그대로 존중」 — `0076:14`) · ADR-0082 결정 첫 항(「임의로 새 대화(fresh)를 만들지 않는다」 — `0082:16`)을 「이어받을 손잡이가 없는 이어받기 요청」에 한해 `0082:21` 의 「이어받을 게 없는 … 정상 생성」으로 읽는다 · 「영향」 `0082:37` 의 Fresh 동사 이름(`new_session_id`). ADR-A 가 개정한다(§5 · 끝의 메모 A). 구현: `resume_no_fallback` 의 `opens_a_new_conversation`(`:1999`)에서 `!assigns_session_id` 항을 빼고(→ `can_resume_stored_session ∧ 명부 손잡이 없음`), 참이면 `spawn_fresh_settled` 로 위임한다(결말 **낱말**은 오늘 codex 가 내는 `Started` 와 같다 — 그러나 codex 에서도 경로가 바뀌어 기록·판정 창이 달라진다. 목록과 죽은 갈래 처분은 §6-3). 사유: WS `SpawnProfile{resume:true}`(`connection_core.rs:1384-1385`)로 id 없는 claude 가 오면 지금은 `ensure_session_id` 가 쓰레기를 영속하고 `--resume <새 값>` 으로 죽는다 — D1 을 깨는 마지막 영속 경로다. shell 은 옛 길 그대로다(ADR-0082 회귀 시험대가 shell 대역으로 Resume 을 태운다 — `tests/activation.rs:102-128,167`).

#### 3-2-4. 순서·동시성

- **`note_submission` 은 `send_input` 앞이다** → 「첫 턴이 상대에게 가기 전에 영속」이 **codex 터미널을 뺀 세 칸**에서 선다(ADR-0185 Phase 2 요구 `0185:32`). claude 두 모드는 offer 가 래치 생성 때 들어가 있어 첫 note 가 그 자리에서 commit 한다. codex app-server 는 한 겹 더 단단하다: 핸드셰이크 전에 제출이 오면 commit 은 라이터 스레드의 `record_session_id` 안에서 일어나고, 그것은 `open_gate`(`:1493`) 앞이며, 큐에 선 턴은 게이트 뒤에만 나간다(`take_turn_locked`).
- ★**codex 터미널은 예외다**★(리뷰 지적 반영): 그 칸의 id 는 락 회수로만 온다 — 첫 훑기는 스폰 0.5 s 뒤, 빈손이면 대기를 두 배씩 15 s 상한까지 늘리며 시한 없이 돈다(`thread_lock.rs:242` · `:254` · `:396-450`). PTY 는 제출을 곧바로 자식에게 넘긴다. 그래서 회수 전에 Enter 가 오면 **첫 턴이 먼저 나가고** commit 은 회수 순간에 난다. 그 사이 데몬이나 자식이 죽으면 대화가 있어도 손잡이가 없다. ★회귀는 아니다★ — 오늘도 그 칸의 id 는 회수 순간에만 생긴다(ADR-0218). ADR-0185 Phase 2 요구는 이 칸에서 성립하지 않는다.
  - ★**「대개는 offer 가 먼저」가 아니다**★(리뷰 지적 반영 — 옛 문장을 고쳤다): 락은 0.7–1.9 s 에 생기지만(ADR-0218 근거) 훑기는 **대기한 뒤에** 돈다 — 스폰 뒤 **적어도** 0.5 · 1.5 · 3.5 · 7.5 s … 에 시작하고(`FIRST_DELAY` `thread_lock.rs:242` · 두 배 백오프 `:466` · 상한 `MAX_DELAY` `:254`), 한 바퀴는 후보 락 파일 **하나마다** Restart Manager 세션을 하나 연다(`:246`). 그래서 이 수치는 하한이고 실제 시각은 거기에 훑기 시간이 더해진다. 첫 훑기(0.5 s)는 실측 최소 생성 지연(0.735 s)보다 앞이라 빈손이고, 회수는 실측 생성 지연 안이면 둘째나 셋째 바퀴 — **적어도 1.5 / 3.5 s + 훑기 시간** — 에 난다. GUI 실측 한 번은 약 6초였다(그때 폴더에 남의 codex 가 쥔 락이 10개 — `step-log.md:2455`). 재현되지 않은 21 초 생성 사례도 기록돼 있다(`MAX_DELAY` doc — `thread_lock.rs:252-253`). ★**파킹된 우편이 있으면 첫 턴이 언제나 먼저다**★ — flush 는 명부 공표 순간(`manager.rs:1447` → `messaging_host.rs:450-490`)에 불리고, 제출 CR 은 본문 뒤 `SUBMIT_PACING`(0.5 s) 만큼 뒤에 나간다 — 1초 안이다(코드 판독 — 실측은 아니다). 사람의 첫 Enter 가 회수 전에 와도 같다. 회귀가 아닌 것은 그대로다.
- **래치 뮤텍스는 commit 호출 동안 쥔다**(고름) — commit 둘이 겹쳐 디스크 순서가 뒤집히는 일과 빠른 길 경합(위)을 구조로 막는다. 락 순서 = `latch → 포트의 expected 칸 → profiles → store write_lock` 단방향(리뷰 지적 반영 — 포트는 `expected` 를 읽고·비교-교체를 부르고·옮기는 동안 그 칸을 쥐고, 그 칸을 잡는 곳은 포트 하나뿐이다). 래치를 잡는 호출자는 모두 다른 락을 안 쥔다: `write_input_observed`(`get_session` 이 sessions 락을 놓은 뒤 — ADR-0006) · `submit_input_observed`(우편 flush 스레드, 같은 조건) · codex `record_session_id`(상태 락을 놓은 뒤 부른다 — `transport.rs:1344-1355`) · 락 회수 스레드(락 없음) · `spawn_agent`(offer — 락 없음). registry 는 밖을 부르지 않는 잎이다.
- **빠른 길:** `settled` 를 `AtomicBool` 로 먼저 본다 — 첫 commit 이 **돌아온 뒤**의 키 입력은 뮤텍스를 안 잡는다(위 3-2-2).
- **첫 제출에 `agents.json` 쓰기 한 번이 입력 경로에 붙는다** — 화신당 1회. 같은 도착순 줄이 이미 프로필 CRUD 의 동기 IO 를 돌린다(`connection_core.rs:170-176` ㉯) — 새 종류의 블로킹이 아니다. 그 한 번 동안 다른 스레드의 첫 제출은 기다린다(그것이 요점이다).
- **ADR-0172 §영향의 경고와 겹치지 않는다** — 그것이 금한 것은 **턴 종료 신호**(구조화 출력에서만 나오고, 화신 검사가 없고, `StatusSink` 논블록 계약) 위에서 프로필 락을 잡는 것이다. 이 방아쇠는 **입력 명령 처리 스레드**이고, 두 모드 모두에서 나오고, commit 이 화신 표식을 싣는다.

#### 3-2-5. D3 — 백엔드 × 모드별 방아쇠 (고름)

| | 방아쇠 | 근거 |
|---|---|---|
| **claude JSON** | 첫 `write_input` 호출(= 첫 유저 턴) — NDJSON 줄이 큐에 들어가기 전. offer 는 래치 생성 때 이미 들어가 있다 | 1 호출 = 1 턴 계약(`session.rs` FIX 6a). claude 는 첫 교환 뒤에 대화를 만든다(ADR-0172 맥락) |
| **claude 터미널** | CR 이 든 첫 입력 조각, 또는 우편의 제출 CR(본문 뒤 · §3-2-2) — PTY 큐에 들어가기 전 | `Raw` 제출 = CR(`backend/mod.rs:917-919`) · 키 인코딩 가정(§3-2-2). 우편은 CR 을 따로 쓴다(`session.rs:251-300`) |
| **codex JSON** | 첫 `write_input` 호출. id 가 아직 없으면(핸드셰이크 중) 핸드셰이크가 id 를 주는 자리에서 — 둘 다 첫 `turn/start` 쓰기보다 앞 | 쓰기마다 `Job::Turn`(`transport.rs:2510`) · `turn/start` 는 게이트 뒤(`:1689`) · **rollout 은 첫 사용자 메시지부터 생긴다**(`docs/reference/backend-capabilities.md` §1, codex 0.154.0 자기 시험) |
| **codex 터미널** | CR/우편 제출과 락 회수 둘 중 **늦은 쪽**. ★회수가 늦으면 첫 턴이 영속보다 먼저 나간다(§3-2-4 예외)★ | 락은 0.7–1.9 s 에 생기지만 회수는 적어도 1.5·3.5 s + 훑기 시간이다(GUI 실측 한 번 ≈ 6 s) — 파킹 우편이 있으면 첫 턴이 언제나 먼저다(§3-2-4) · 입력 없이 닫은 재개는 exit 1(`step-log.md:2442`) |

**거부한 방아쇠(고름의 근거):** 오케스트레이터 원안의 「codex `writer_loop` `Job::Turn` 팔에서 `turn/start` 직전」은 codex JSON 한 칸만 덮는다. claude 두 모드와 codex 터미널의 통로(stdio·PTY)는 턴을 모르는 바보 파이프라 같은 자리가 없고, 결국 입력 경계에 방아쇠를 하나 더 세워야 한다 — 방아쇠가 둘이 된다. 입력 경계 하나는 네 칸을 같은 코드로 덮고 통로를 안 건드린다.

#### 3-2-6. claude `/clear` 와의 관계 (★결정 — 사용자 2026-09-24: §7 Q1 = (a)★)

- `/clear` 추적기(`daemon/src/lib.rs:289` → `observe_session_id(id, None, new)`)는 **그대로 둔다** — 즉시 영속.
- ★**이것이 D1 의 유일한 의도된 예외다**★ — `/clear` 순간의 새 id 에는 아직 대화가 없는데도 곧바로 영속한다. 사유(사용자): `/clear` 는 옛 대화를 끝내는 사용자의 명시적 행위다. 대가: `/clear` 하고 입력 없이 끄면 다음 활성화가 실패한다 — A 만이면 오늘과 같은 실패, B 뒤에는 새 대화로 열린다.
- 이 결정(Q1 = (a) — 사용자)이 D1 과 부딪히는 자리는 하나다: `/clear` 직후 한 마디도 없이 끄면 대화 없는 새 id 가 저장돼 있다 → 다음 활성화가 이어받기에 실패 → **D4 가 새 대화로 연다**(Phase A 만 착지했으면 오늘과 같은 실패 — §6-3). 사용자가 지운 대화를 되살리지 않으므로 결말은 맞고, 대가는 헛시도 한 번(claude 실측 약 1초, ADR-0201)이다.
- 순서 안전은 **이제 순서가 아니라 구조로 선다**: 래치 commit 은 비교-교체라 추적기가 먼저 쓴 값을 덮지 못한다(§3-2-1). 그리고 첫 제출(그것이 `/clear` 자신이어도)은 **보내기 전에** 민팅 값을 commit 하고 추적기의 새 값은 claude 가 `/clear` 를 처리한 **뒤에야** 관측되므로, 정상 순서에서는 비교-교체가 거절할 일도 생기지 않는다.
- 전제 확인(구현 전): 추적기는 **기준값(민팅 값)과 다를 때만** `Changed` 를 낸다. 비교는 `backend/claude/session_file.rs:210`(`current != self.last_seen_sid`)이고, 기준값은 `:152` `last_seen_sid: expected_sid` 가 받는다(넘기는 자리 = `manager.rs:1437` 의 `session_id_source(..., s)`). 첫 폴링이 민팅 값을 그대로 보고하면 D1 이 claude 에서 샌다 — 그 자리를 시험으로 박는다(§4-1).

#### 3-2-7. 이 결정이 ADR-0172·ADR-0008 의 기각을 다시 여는 것이 아닌 이유

- ADR-0172 가 버린 것은 **「우리 관측을 새 칸에 영속해 활성화 전에 판정한다」**다 — 새 칸이 필요하고, 옛 항목에 도장이 없어 소급이 안 된다. D1 은 **새 칸이 없다** — 이미 있는 `backend_session_id` 를 **언제** 적느냐만 바꾼다. 활성화는 여전히 id 가 있으면 이어받기를 **시도**하고(`can_resume_profile` 불변), 실패는 여전히 시도한 자리에서 관측한다(ADR-0172 결정 1). 도장 없는 옛 항목(쓰레기 id)은 사전 판정이 아니라 **D4 의 반응형 처리**가 받는다.
- ADR-0008 이 금한 claude 파일 위의 기능 확장도 없다 — 무엇도 claude 파일을 읽어 판정하지 않는다.
- ADR-0185 의 「크래시 창」(응답 수령 → 영속 사이, `0185:21`)은 **codex 터미널을 뺀 세 칸에서 녹아 없어진다** — 첫 턴 전의 스레드는 rollout 이 없어 잃을 대화가 없고(backend-capabilities §1), 첫 턴은 영속 **뒤에** 나간다. codex 터미널은 회수가 첫 턴보다 늦은 경우 창이 남는다(§3-2-4 — ADR-0218 과 같은 등급).

### 3-3. D2 — 구독 응답의 「이어받는 화신」 표식 [A]

- **이름: `continues_conversation`**(고름) — `SubscribeAction::Resume`(seq 이어받기, `messages.rs:591`)과 겹치지 않게 「resume」 낱말을 뺐다. 뜻 = **「이 화신은 저장된 대화를 이어받으려고 떴다(스폰 때 이어받을 손잡이를 실었다)」 — 이어받기의 성공 여부가 아니다.**
- **계산(백엔드 중립):** `spawn_agent_watching_link` 의 `resume_session_id.is_some()`. `resume_handle_for`(`manager.rs:487-492`)가 「손잡이가 있다 ⟺ 이어받기 화신」을 이미 성립시키는 자리라 새 판정이 아니다. claude 도 같은 칸을 탄다(Resume 이면 `stored_handle` 을 모든 backend 에서 읽는다 — `:1350-1358`).
- ★**표식과 replay 는 같은 화신에서 나와야 한다**★(리뷰 지적 반영): 오늘 `handle_subscribe` 는 `agent_epoch`(`connection_core.rs:1747`)로 표식을 읽고, `subscribe_from`(`manager.rs:2558-2568`)이 `get_session` 을 **다시** 해 replay 한다. 두 조회 사이에 화신이 갈리면(D4 가 바로 그 교체를 만든다) Ack 의 표식·이어받기 표식이 **replay 한 것과 다른 화신**을 말한다. 오늘은 표식 한 칸의 TOCTOU 였지만(그 자리 주석이 `Err` 갈래로만 인정한다 — `:1778-1781`), 이어받기 표식이 얹히면 첫 화면/로딩 판정이 그 틈을 탄다. 그래서 한 번의 조회로 합친다:
  - **agent:** `types.rs` 에 `pub struct Incarnation { pub epoch: u32, pub continues_conversation: bool }` 와 `pub struct SubscribeReply { pub outcome: SubscribeOutcome, pub incarnation: Incarnation }`. `AgentSession` 은 `continues_conversation: bool` 을 화신 불변 칸으로 싣는다(§3-2-2 빌더 — 표식은 기존 `epoch` 칸 그대로). `Incarnation` 은 세션 칸이 아니라 응답 값이다 — `Incarnation { epoch: self.epoch, continues_conversation: self.continues_conversation }` 로 그 자리에서 조립한다. `AgentSession::subscribe_from` 이 `requested_epoch: Option<u32>` 를 받아 **자기 표식과 대조해** `epoch_matches` 를 스스로 계산한 뒤, core 의 outcome 에 자기 `Incarnation` 을 덧대 `on_ready` 와 반환 **둘 다**에 같은 값을 넘긴다. 고름: core 는 이어받기 표식을 모르므로 `SubscribeOutcome` 에 칸을 늘리지 않고 session 층에서 감싼다(소유권 분할 — core 는 replay/seq, session 은 epoch). `AgentManager::subscribe_from(agent_id, sink, after_seq, requested_epoch, on_ready)` 는 그대로 위임한다.
  - **daemon:** `handle_subscribe` 는 `agent_epoch` 선조회를 걷고, Ack 의 `current_epoch`·`continues_conversation` 과 `ReplayComplete.epoch` 를 **그 reply 하나**로 채운다. 「없는 에이전트」 거절은 기존 `Err` 갈래 하나로 합쳐진다(문구 `subscribe failed: …` — 거절 문구를 단언하는 daemon 시험은 없다, 확인). `agent_epoch` 의 다른 호출자(`tests/ws_e2e.rs`)는 그대로.
  - `on_ready` 가 replay 전송 직전·subscribers lock 보유 중 1회라는 계약(`session.rs:360` · `subscribe_from_err_never_invokes_on_ready` — `manager.rs:4232`)은 그대로다 — 인자 모양만 바뀐다.
- **protocol:** `AgentEvent::SubscribeAck { …, #[serde(default)] continues_conversation: bool }`(`messages.rs:374-387`) + `crates/engram-dashboard-protocol/bindings/AgentEvent.ts` 재생성(CI 동기 게이트).
- **셸(src-tauri):** `replay_flight.rs` — `InFlight` 에 칸 · `on_ack`(`:206`)가 받아 각인 · `Marker`(`:76`)에 칸 · `encode_marker_frame`(`:364`) flags **bit2 = `0x04`**(길이 30 불변). 실패 마커(마감 초과·거절)는 `false`. `connection.rs` `apply_replay_event` 의 `SubscribeAck` 팔(`:1443`)이 넘긴다. 순수성 게이트 영향 없음(bool 하나).
- **프론트:**
  - `wsFrame.ts` `MARKER_FLAG_CONTINUES = 0x04` 해독(`:16-19`·`:55-72`) · `transport.ts` `replayBoundary` 에 `continuesConversation` · `tauriTransport.ts:324-337` 정규화 · `wsTransport.ts:417` `observeReplayWire`(직결 경로 — `SubscribeAck` 칸을 in-flight 에 기억해 경계에 싣는다).
  - `protocolClient.ts` — `HeldMarker` 에 칸 · `evalMarker`(`:328`) · `flushToLive`(`:371`)가 `st.onState?.('live', { continuesConversation })`.
  - `agentClient.ts` — `onState?: (state: ViewPhase, info?: ReplayLiveInfo) => void`, `export interface ReplayLiveInfo { continuesConversation: boolean }`. `info` 는 `'live'` 에만 온다. 없으면 `false` 로 읽는다.
- **broadcast 없음.** 프론트에 id 를 읽는 곳이 없다 — `src/` 에서 `backend_session_id` 는 `src/api/types.ts:151` 선언 하나뿐(확인). D1 의 commit 도 `ProfileListUpdated` 를 안 낸다(오늘의 `observe_session_id` 와 같다).
- **PROTOCOL_VERSION 은 안 올린다(= 5 유지).** 올리는 기준은 모양이 아니라 **해로운 조용한 오작동**이다(`protocol/src/lib.rs:83-99`). ★아래 두 방향 판단은 새 칸 `continues_conversation` 의 doc 에 적는다★ — `lib.rs:99-103` 이 「기준의 집은 이 자리, 그 판단의 집은 변형 쪽」으로 정해 두었다(선례 = `TurnEnd`). 두 방향 모두 **오늘의 동작**으로 떨어진다: 신데몬+구셸 → 구셸이 모르는 칸을 버린다(`deny_unknown_fields` 없음) → 첫 화면 깜빡임 그대로. 구데몬+신셸 → `#[serde(default)]` = `false` → 첫 화면 그대로. 무해하다. 셸↔웹뷰 마커 bit 는 한 빌드로 함께 나가므로 버전 축이 없다.

### 3-4. D4 — 「이어받을 대화가 없다」면 새 대화로 [B]

- **자리:** `resume_no_fallback` 의 실패 팔 셋 — `Terminal`(`manager.rs:2069` — 단 `Killed` 는 사용자 취소라 제외, `:2080`) · `Diagnosed`(`:2107`) · `LinkFailed`(`:2133`). 공통 판정 = 그 팔이 이미 계산하는 `kind == NoConversationToResume`.
- ★**선례 — 같은 사용자 결정을 한 번 구현했다가 되돌렸다**★(리뷰 지적 반영): ADR-0077 이 「스폰만 하고 한마디도 안 한 건 그냥 새로 시작」을 fresh-fallback 으로 구현하고 수동 활성화까지 넓혔다. 그 확장(a4aac1a)이 이중 spawn 가드의 「already running」 `Err` 를 이어받기 실패로 **오인해 멀쩡히 돌던 에이전트를 죽였고**, 그것이 ADR-0082(fresh-fallback 폐지)의 직접 원인이다. 이 설계가 그 부류를 **구조로** 피하는 근거:
  1. 폴백은 **이 호출이 띄운 화신**(`SpawnOutcome::Started(spawned)`)에 대한 판정 팔 셋 안에서만 착수된다. `Moot`(남이 띄웠거나 띄우는 중 — `:2033-2039`)과 spawn `Err`(`:2013-2025`)는 판정 전에 돌아가고 폴백이 없다. 「already running」 은 오늘 `Err` 가 아니라 `Moot` 이다(`SpawnOutcome` doc — `:83-95`).
  2. 분류는 **이 화신 자신의 증거**(진단·종점 꼬리·연결 거절 사유)를 backend 가 읽은 `NoConversationToResume` 하나다 — 오류 문자열의 모양으로 가르지 않는다.
  3. 거두기는 화신 가드다 — `tear_down_failed_activation` 은 표식이 다르면 손대지 않는다(`:2223-2232`).
  4. 산 에이전트 재활성화는 그보다 앞에서 걸러진다 — `activate_profile` 의 in-flight 가드(`:1512`)와 「이미 실행 중」 갈래(`:1522-1532`).
  5. 재진입이 없다(아래 「루프 금지」).
- ★**ADR-0019 결정 2(「런타임 자동 재시작 없음」)를 어기지 않는 이유**★(리뷰 지적 반영): 그 결정이 금한 것은 **떠 있던 화신이 죽었을 때 시스템이 되살리는 것**이다. D4 는 사람·LLM·부팅 복원이 **요청한 활성화 안에서**, 그 활성화의 이어받기 시도가 「이어받을 대화가 없다」로 끝났을 때만 **같은 요청의 결말**을 새 대화로 바꾼다. 한 번 성립한 화신이 나중에 죽으면 D4 는 관여하지 않는다(reaper 처분 그대로).
- **절차(새 도우미 하나, 고름):**
  0. **예약을 쥔다 — 놓지 않는다**(리뷰 지적 반영). 연결 축 backend 는 판정이 들고 있던 `LinkWatch` 의 예약(`:406-410`)을 꺼내 넘긴다(`LinkWatch::into_reservation`). 연결 축이 없는 backend(claude)는 예약이 spawn 과 함께 이미 풀렸으므로 **여기서 새로 잡는다** — 못 잡으면(다른 요청이 활성화 중) 폴백하지 않고 오늘처럼 포기한다(기록 + `Failed`). ★놓으면 무슨 일이 나나★: 수거를 기다리는 동안(reaper 의 `apply_disposition` 은 `agents.json` 통째 쓰기다) 다른 활성화가 끼어 시체를 돌려받거나, 예약을 차지하고 **아직 저장된 쓰레기 id 로 Resume 을 또 띄운다** — 스폰이 셋이 된다.
  1. **실패 화신의 Arc 를 잡아 둔다**(표식 일치할 때) — 5단계 재확인용.
  2. **거둔다** — `tear_down_failed_activation(id, epoch)`(`:2215`, 화신 가드 · 이미 없으면 no-op) + **`tracker.unwatch(id)`**. `kill_agent` 는 추적 해제를 하지만(`:2725`) `tear_down` 은 안 한다 — 지금까지는 추적기가 안 붙는 codex 에만 쓰였고, D4 가 처음으로 claude 에 쓴다. 새 대화가 뜨면 `watch` 가 항목을 갈아치우지만(`session_tracker.rs:115`) 새 대화가 실패하면 옛 PID 를 보는 항목이 남는다. `LinkFailed` 팔은 이미 거둔다. ★`Diagnosed` 팔의 처분은 §7 Q6 에 매인다★ — 거기서는 자식이 아직 살아 claude 종료 훅을 돌리는 중이라(진단 +2.2 s · 종료 +6.4 s — `:42-49`) 지금 거두면 그 훅까지 함께 죽는다(`TerminateJobObject` 는 나무째 끝낸다). 안 거두면 새 스폰이 옛 세션을 「떠 있다」로 본다(`spawn_agent_watching_link` 의 조회 `:1169`).
  3. **reaper 동기화** — `ReaperCmd::Barrier { id, epoch, reply: Sender<bool> }` 를 새로 두고(`reaper.rs:30`), 보내고 답을 기다린다. reaper 는 앞선 명령을 다 처리한 뒤 **자기 스레드에서** 「그 `(id, epoch)` 가 명부에 없나」를 답한다.
     - ★왜 reaper 가 답해야 하나(리뷰 지적 반영)★: 명부에서 세션을 지우는 것은 reaper 하나뿐이고, `reap_one` 은 **맵에서 먼저 지우고** revoke·처분·목록 통지를 그 뒤에 한다(`reaper.rs:51-60` 대 `:77-87`). 그 한 건은 reaper 스레드에서 끝까지 돈 뒤에야 다음 명령을 본다. 그래서 reaper 스레드의 「없다」는 **그 화신의 수거가 목록 통지까지 끝났다**는 뜻이다. manager 가 기다린 뒤 명부를 직접 보면 수거 **도중**의 「없다」를 볼 수 있고, 그 늦은 목록 통지가 새 화신의 통지보다 뒤에 도착해 트리를 되돌린다.
     - 상한 = **새 상수 `REAP_BARRIER_BACKSTOP`**(고름). 값은 kill 인과의 `join_pump` 상한 5 s(ADR-0001)를 그대로 쓴다 — `INPUT_FLUSH_BUDGET`(`session.rs:70`)이 같은 숫자를 같은 사유(「이쯤이면 끝났어야 한다」)로 쓴 선례. `LINK_RESOLUTION_BACKSTOP` 을 재사용하지 않는 이유: 그 doc 이 값의 근거를 「통로의 핸드셰이크 상한보다 커야 한다」로 핸드셰이크에 묶어 두었다(`manager.rs:68-70`) — 축이 다르다.
     - 성립 근거: `tear_down` 의 `session.kill` 은 `join_pump` 를 기다리고, 세 통로 모두 pump 가 `core.finish`(ReapMsg 송신 포함 — `output_core.rs:442-449`) **뒤에** 완료 신호를 보낸다(`transport/pty.rs:353-358` · `stdio.rs:341-344` · `codex/transport.rs:2473-2474`). join 이 5 s 에 끊겼으면 ReapMsg 가 barrier **뒤에** 올 수 있는데, 그때 답은 「있다」라 4단계에서 포기로 간다.
  4. 답이 「있다」 · 시한 초과 · reaper 끊김(`Disconnected`) → **무조건 포기** — 오늘과 같다: `NoConversationToResume` 기록 + `Failed`.
  5. ★**사용자 종료·셧다운 재확인**★(리뷰 지적 반영): 1단계의 Arc 에서 `termination_intent() == UserKill` 이거나 `shutting_down` 이 섰으면 폴백하지 않는다 — 오늘의 `Killed` 팔처럼 기록 없이 `Failed`. 사유: 사용자 취소를 보는 자리는 판정 하나뿐이라(`:2080`) 판정 **뒤**의 kill(`kill_agent` 가 옛 세션에 `UserKill` 을 세운다 — `:2715`)을 D4 가 못 보면 **사용자가 끈 에이전트를 되살린다** — 그것도 `spawn_session` 이 `auto_restore=true` 를 켜서 다음 부팅에도 뜬다(`:1900`). `shutdown_all` 은 id 를 한 번만 뜨고(`:2763-2766`) spawn 경로는 `shutting_down` 을 안 본다 — 셧다운 도중 새 대화가 뜨면 아무도 안 죽인다.
  6. **같은 예약으로 새 대화** — `spawn_fresh_settled` 의 예약 인계판(아래 서명). Fresh 이므로 D1 이 쓰레기 id 를 이력으로 밀고 빈 칸에서 시작한다(옛 프로필 청소가 여기서 저절로 된다 — 이력 밀기라 ADR-0202 의 파괴가 아니다).
  7. **결말:** 성공 → `(RestoreOutcome::Started, Some(info))`. 새 대화 갈래가 `Moot` 이면(예약을 쥐었는데도 명부에 세션이 있다 — ADR-0082 열린 항목 ③ 의 선재 창) 기록 없이 `resume_no_fallback` 의 `Moot` 팔처럼 보고한다(`(RestoreOutcome::Resumed, info)` — `:2033-2039`). 실패 → 새 대화 갈래가 자기 규율로 기록한 것과 `Failed` 를 그대로.
- **서명(메인이 먼저 못 박는다 — Phase A 와 같은 함수를 만지는 접점, §6-5):** `fn spawn_agent_watching_link(&self, profile, mode, reservation: Option<SpawnReservation>)` — `None` = 오늘처럼 여기서 잡는다. ★이 인자는 **B2 가 들인다**★ — Q5 = (a)라 A 가 먼저 착지하고, A2 는 이 서명을 안 바꾼다. `spawn_fresh_settled(profile)` 는 `spawn_fresh_settled_with(profile, Option<SpawnReservation>)` 의 얇은 껍데기가 된다. 인계받은 예약은 새 대화의 `LinkWatch` 가 이어 쥐고 그 판정에서 풀린다.
- ★**예약 수명이 늘어나는 것은 안전하다**★ — `SpawnReservation` doc(`:591-608`)이 경고한 것은 수명을 **줄이는** 변경이다. 늘리면 그동안 같은 id 의 다른 활성화는 in-flight 가드에서 「다른 요청이 활성화 중」 `Err` 를 받는다(최악 아래 45 s).
- **기록(ADR-0172):** 폴백이 착수되면 이번 시도의 `NoConversationToResume` 을 **기록하지 않는다** — 실패가 정상 신규 생성으로 바뀐 것이다. 포기 갈래에서만 오늘처럼 기록한다. 원인은 로그(info)로 남긴다 — 버린 id 는 `old_session_ids` 에도 남는다.
  - ★**이전 시도가 남긴 기록(ADR-0202)**★(리뷰 지적 반영): claude(연결 축 없음)는 새 대화 갈래의 `note_spawn_result` 가 지운다. codex(연결 축)는 ADR-0202 대로 안 지운다 — `Ready` 는 기록을 건드리지 않는다(`spawn_fresh_settled` 의 `Ready` 팔). 그래서 D4 이전에 실패가 기록된 codex 프로필은 새 대화가 떠도 그 기록을 달고 있고, 끄면 「이어받을 대화가 없습니다」가 보인다(도는 동안은 ADR-0173 이 가린다). 제어 LLM 은 그 낡은 기록을 도는 에이전트 옆에서 그대로 읽는다(ADR-0202 §영향). 고름: ADR-0202 를 그대로 둔다 — 이 부류에 지움 예외를 새로 파면 ADR-0202 를 개정하는 것이라 §7 Q3 의 LLM 신호 선택지와 함께 결정한다.
- **루프 금지 = 구조:** 폴백은 `spawn_fresh_settled_with` 만 부르고, 그 함수는 이어받기를 시도하지 않으며 실패 분류를 `Other` 로 고정한다(`manager.rs:1681-1686`). `resume_no_fallback` 재진입 경로가 없다. 시험으로 박는다(스폰 최대 2회).
- **`RestoreOutcome::FreshFallback` 은 안 쓴다**(고름) — `new_sid` 가 필수인데(`profile.rs:114-118`) D1 로 그 시점엔 영속 id 가 없다(codex 는 아예 모른다). 「새 대화를 열었다 = `Started`」는 이미 사용자 결정 어휘다(`connection_core.rs:1370-1376` 주석). 소비자는 콘솔 로그뿐이다(`src/store/eventBus.ts:162-164`).
- **거부한 구현(고름):**
  - 명부를 100ms 로 들여다보며 기다리기 — 시각 기반 대기라 ADR-0038 결과 어긋나고, 사건(수거 완료)이 이미 있다.
  - 기다린 뒤 manager 가 명부를 직접 본다 — 수거 도중의 「없다」를 본다(3단계).
  - 예약을 놓고 기다린다 — 스폰이 셋이 된다(0단계).
  - `LINK_RESOLUTION_BACKSTOP` 재사용 — 근거가 다른 축에 묶인 값이다(3단계).
  - ★**파일로 판정한다**★(리뷰 지적 반영): 2026-09-21 라운드가 미뤄 둔 계획은 「재개 실패 폴백 — 기록 파일이 없을 때만 새 세션 + 고지 · **문구가 아니라 파일로 판정한다**」였다(`step-log.md:2458` — codex 에서 「그 id 를 담은 rollout 파일이 있나」가 재개 가능 여부와 네 경우 모두 일치함을 실측). 기각(고름): ① 활성화 **전에** 우리 관측으로 가르는 것은 ADR-0172 가 기각한 사전 판정이고, spawn 전에 백엔드 저장소를 들여다보는 것은 ADR-0077·ADR-0082 가 이미 거부한 대안이다(백엔드 내부 포맷 결합 · ADR-0004 격리 침해) ② claude 쪽 짝(claude 의 대화 파일)은 ADR-0008 이 금한 claude 파일 위의 기능 확장이라 백엔드 중립 규칙이 못 된다 ③ codex 의 오류문이 **같은 파일을 보고** `no rollout found` 를 낸다는 그 실측이 곧 「시도한 자리의 반응형 판정으로 충분하다」의 근거다. ★**단 codex 터미널의 문구가 아직 미측정이다(아래 도달성)**★ — B0 측정이 분류 가능한 문구를 못 찾으면 그 칸만 파일 판정이 남은 후보가 되고, 그것은 위 기각을 그 칸에 한해 다시 여는 것이라 사용자 결정이다. 그 계획의 「+ 고지」 절반은 §7 Q3 이 받는다.
- **교차 요청:** 예약을 쥐고 있으므로 폴백 사이(0~6단계)의 같은 id 활성화는 in-flight 가드에서 돌아간다. 남는 창은 ADR-0082 열린 항목 ③ 의 선재 창(예약 취득과 명부 조회 사이)뿐이다.
- **판정 뒤·새 화신 등록 전의 kill:** 옛 세션이 수거된 뒤(3단계 이후) 도착한 `kill_agent` 는 `NotFound` 로 돌아가고 5단계 재확인에도 안 잡힌다 — 그 창(수거 완료 ~ 새 세션 등록, 프로세스 기동 한 번 폭)에 끈 요청은 새 대화를 못 막는다. 결과는 보인다(kill 이 오류로 돌아오고 에이전트가 뜬다). 닫지 않는다(고름: 닫으려면 kill 이 예약을 보게 해야 하고, 그것은 kill 인과의 변경이다).
- **부팅 복원도 같은 길이다**(`restore_one` `:1939` → `resume_no_fallback`).
- ★**최악 블로킹**★(리뷰 지적 반영): 활성화 스레드가 폴백 전체를 한 호출 안에서 진다. codex app-server ≈ **45 s** = 이어받기 백스톱 15 + 거두기 `kill` 5 + barrier 5 + 새 대화 백스톱 15 + 그 거두기 5. claude ≈ **13 s** = 판정 창 3 + 거두기 5 + barrier 5(새 대화는 연결 축이 없어 기다리지 않는다 · Q6 에서 「자연 종료 대기」를 고르면 +최대 5). `activate_profile` doc 의 「최악은 백스톱(15s) + teardown 의 `session.kill(5s)`」(`manager.rs:1483-1484`)을 이 값으로 고친다. 호출자 의무(blocking 풀 — 오늘 운영 호출자 넷이 지킨다)는 그대로다.
- **백엔드 × 모드 도달성:**
  - claude JSON = 실측 — 문구 `No conversation found`(약 1 s — ADR-0201 근거) · 진단 스트림으로 +2.2 s 에 `Diagnosed`(`manager.rs:42-49` · `claude/mod.rs:361-366`).
  - ★**claude 터미널 = 미확인**★(리뷰 지적 반영): PTY 세션은 stderr 가 콘솔에 섞여 진단 버퍼가 비어 있으므로 `Diagnosed` 가 서지 않고, 분류는 **3 s 창 안에 죽었을 때만**(`Terminal` 팔의 콘솔 꼬리) 된다(`manager.rs:2292-2308`). ADR-0201 의 실측은 문구가 **언제 뜨나**이지 언제 **죽나**가 아니고, JSON 모드는 +6.4 s 에 죽는다(SessionEnd 훅). 터미널도 3 s 를 넘겨 죽으면 `Alive` → `Resumed` 로 보고되고 기록까지 지운 뒤 시체가 된다 — 오늘과 같은 결말이고 D4 는 그 칸에서 안 걸린다. 살아 있는 세션의 콘솔 링을 증거로 쓰는 것은 금지다(이어받은 대화 본문 오탐 — `:2242-2247`). B0 에서 잰다.
  - codex app-server = 실측 `no rollout found for thread id`(`codex/mod.rs:711-716`).
  - ★**codex 터미널 = 미확인**★ — `codex resume <rollout 없는 uuid>` 가 PTY 에 무엇을 찍는지·몇 초에 죽는지 잰 적이 없다. 분류가 안 되면 그 칸만 D4 가 안 걸린다(오늘과 같은 실패). B0 에서 잰다(§6-2).

#### B 라운드 착수 전 반영할 리뷰 지적 (2차 — 2026-09-24)

지금 재설계하지 않는다. B 라운드 TRD 개정의 입력이다.

1. **판정 뒤 Arc 취득이 reaper 와 경주한다**(보통). 1단계의 「실패 화신의 Arc」를 판정이 **끝난 뒤** `get_session` 으로 잡는다. 그런데 판정 창 안에 죽은 화신은 `core.finish` 의 훅이 곧바로 `ReapMsg` 를 보내고(`manager.rs:1845-1856`), reaper 가 맵에서 지운다. 100 ms 폴링 판정(`:2293-2311`)이 돌아온 뒤에는 흔히 `NotFound` 다. 1단계에 「Arc 가 없을 때」 규칙이 없다. 방향: `spawn_session` 이 만든 Arc(`:1423`)를 `Started` 결말과 함께 들고 나오거나, 판정이 쥔 Arc(`:2272`)를 넘겨 5단계 재확인을 그 Arc 로 한다. `LinkFailed` 팔은 거두기(`:2145`) **전에** 잡는다.
2. **5단계와 6단계 사이의 셧다운**(보통 — codex 리뷰어). 5단계의 `shutting_down` 확인과 6단계 새 화신 등록 사이에 셧다운이 시작되면, `shutdown_all` 이 한 번 뜬 id 목록(`manager.rs:2764-2767`)에 새 화신이 없다. §3-6 은 이것을 선재 창으로 적었지만, D4 가 그 창에 닿는 경로를 하나 더 만든다. 선택지: 셧다운 수락과 등록을 한 울타리에 넣는다 · 셧다운이 진행 중인 활성화를 기다린 뒤 한 번 더 비운다. 시험은 확인과 등록 사이에서 멈추게 해 잰다.
3. **`unwatch` 가 후임을 끊는다**(낮음). 2단계 `tracker.unwatch(id)` 는 AgentId 만 키로 쓴다(`session_tracker.rs:131-136`). 거두기는 화신 가드다(`manager.rs:2223-2231`). 가드가 거절했는데(후임이 섰다) unwatch 가 돌면 **후임의** 추적이 끊긴다. 거두기가 표식 일치로 실제 돌았을 때만 unwatch 한다.
4. **표류 id 가 대화보다 먼저 알려질 때**(불확실). claude 가 표류한 id Y 를 Y 의 transcript 가 생기기 **전에** 알리면(추적기가 적는다) 다음 `--resume Y` 가 「대화 없음」으로 끝난다. 그러면 D4 가 S 의 대화까지 버린다. Q3 오탐 목록에 넣거나 실측으로 배제한다.
5. **동시 claude 활성화의 보고**(낮음). 6단계 새 대화 갈래가 `Moot(Some(e3))`(다른 요청이 띄운 화신)이면 7단계가 `Resumed` 로 보고한다. 그 e3 는 곧 죽을 화신일 수 있다. §3-6 에 적거나 Q7 (a)(예약을 판정 창까지 늘린다)에 묶는다.
6. **LLM-우선**. 「이 화신이 이어받았다」 사실은 지금 구독 응답의 표식(D2)에만 있다. Q3 (d)를 고르면 새 칸을 짓지 말고 이 화신 사실(`continues_conversation` — 폴백이면 새 화신에서 거짓)을 `agent.spawn` 결과에 재사용한다.

### 3-5. D5 — 로딩 패널과 RichSlot 게이트 [A]

- **`src/components/ui/LoadingPanel.tsx` + `loading-panel.css`**(옆 css 규약 — `scroll-area.css` 선례).
  - props: `label?: ReactNode`(**보이는** 선택 텍스트 칸 — 이번엔 안 넘긴다) · `className?`. aria 이름과는 별개다 — `label` 을 넘기면 그것을 그리고, 안 넘기면 보이는 텍스트가 없다.
  - 아이콘 = lucide `LoaderCircle`(`lucide-react` 는 이미 의존성). 색 = 테마 토큰(`text-muted`).
  - 회전은 css keyframes. `@media (prefers-reduced-motion: reduce)` 와 `:root[data-theme='e-ink']` 에서 **정지**(고름: Tailwind `motion-safe:` 는 reduced-motion 만 덮고 e-ink 는 못 덮어 규칙이 두 곳으로 갈린다).
  - `role="status"` · `aria-busy="true"` · 화면 밖 이름 `aria-label={t('common.loading')}` — **보이는 텍스트는 없다.** `data-loading-panel="1"`(GUI 실측 관측점).
  - ★**i18n 키는 새로 만든다**★(리뷰 지적 반영): `common.loading`(`ko.ts` 의 `common` 묶음 `:143` 에 더한다). 기존 `window.loading`(`:48` — 「창 로딩 중… (label: {label})」)은 뜻과 인자가 달라 재사용하지 않는다.
- **RichSlot(`src/components/slot/RichSlot.tsx`):**
  - 새 상태 `continuesConversation` — `'live'` 통지의 `info` 로 세우고, 비우기 콜백(onReset)과 구독 effect 초기화에서 `false` 로 내린다.
  - `awaitingHistory = replayDone && continuesConversation && !hasSent && !items.some(isRenderedItem) && !agentUnavailable`
  - `showEmpty = replayDone && !continuesConversation && !hasSent && items.length === 0` — 기존 식에 한 항만 더한다.
  - `isRenderedItem` = `StructuredTextView` 의 `rowKindOf(item) !== 'skip'` 을 export(ADR-0051 의 null 반환 규칙과 한 몸 — `StructuredTextView.tsx:377`). ★**`items.length > 0` 으로 끝내지 않는 이유**★ — 실측상 이어받기의 첫 라이브 프레임이 `Usage` 인데 그것은 행을 안 그린다(`StructuredTextView.tsx:385`). 그걸로 끝내면 이력이 오기 전 20–70ms 빈 목록이 비친다.
  - ★**대기 꼬리도 같은 판정으로**★(리뷰 지적 반영): `streaming = awaiting || (!turnDone && items.length > 0)`(`RichSlot.tsx:283`)는 `Usage` 만 온 창에서 참이 되어 대화 영역 끝에 대기 꼬리(`WaitRow` — `StructuredTextView` 의 `showTail = streaming`, `:517`)를 붙인다 — 로딩 패널과 대기 꼬리가 함께 보인다. `items.length > 0` 을 `items.some(isRenderedItem)` 으로 바꾼다. 달라지는 것은 행을 그리는 항목이 **하나도 없는** 창뿐이고, 그 창이 이 결정이 새로 다루는 창이다.
  - **배치(★Q2 = (a) 선택으로 정해짐(사용자, 2026-09-24 — (가)/(나)를 Q2 와 함께 제시)★):** 대화 영역 가운데. 하단 입력창은 **그대로 활성**이다 — 이력이 끝내 안 오는 경우에도 사용자가 빠져나갈 길이 입력이다(§3-6). ★Q2 = (a) 가 이 배치에 기댄다★ — 슬롯 전체를 덮는 로딩이면 이력 0건 화신에서 빠져나갈 길이 없다. 입력창(textarea)의 부모 자식 자리를 밀지 않는다(ADR-0145 · `RichSlot.tsx` 막 주석 — ScrollArea 안에 넣거나 마지막 자식으로).
  - 관측 속성 `data-rich-awaiting-history="1"`.
  - 타이머 없음.
  - ★**비우기 콜백에서 `replayDone` 을 내리지 않는다 — 리뷰 지적을 반영하지 않은 자리**★: 지적은 「onReset 이 `replayDone` 을 참으로 둔 채 표식을 거짓으로 내리면 다음 `'live'` 까지 `showEmpty` 가 참 → 그 사이 페인트가 깜빡임을 되살린다」였다. 코드상 그 창이 없다. ① 비우기는 `flushToLive` 안에서만 불리고(`protocolClient.ts:377-381` — `restartPending` 일 때만) 같은 함수 끝에서 **동기로** `'live'` 가 따라온다(`:398`) — 한 틱의 상태 변경은 React 가 묶어 한 번에 그리므로 사이에 페인트가 없다. ② `restartPending` 을 세우는 자리는 `startBuffering` 하나이고(`:450`) 그 함수가 먼저 `'buffering'` 을 통지한다(`:459`) — 비우기 시점엔 `replayDone` 이 **이미 거짓**이다. 기존 주석(「replayDone 은 내리지 않는다 — 바로 뒤 같은 틱에 'live' 가 따라온다」, `RichSlot.tsx:198-199`)이 그 순서에 기대고 있으므로, 그 순서를 시험으로 박는다(§4-1).
- **대기가 끝나는 넷(D5)이 식의 항에 하나씩 대응한다:** 첫 이력 = `isRenderedItem` · 입력 = `hasSent` · 화신 종료 = `agentUnavailable`(막이 이긴다) · D4 재시작 = 새 화신의 onReset + `'live'`(`false`) → 첫 화면.

### 3-6. 실패 모드

| 경우 | 결말 |
|---|---|
| 다른 사유의 이어받기 거절(`EarlyExitAfterResume`·`Other` 등) | ADR-0082 그대로 — 멈추고 기록 |
| [B] D4 가 연 새 대화도 실패 | 새 대화 갈래 자신의 기록·`Err`. 재진입 없음(스폰 최대 2회) |
| 이어받는 중(판정 전) 사용자 kill | `Terminal{Killed}` → D4 안 탐, 기록 안 함(오늘 규율) |
| [B] 판정 뒤·폴백 중 사용자 kill | 옛 화신에 `UserKill` 이 섰으면 5단계에서 멈춘다 — 새 스폰 없음 · 기록 없음. 옛 화신 수거 뒤·새 등록 전의 kill 은 `NotFound` 로 돌아가 못 막는다(보이는 잔여 — §3-4) |
| [B] 폴백 중 데몬 셧다운 | 5단계에서 멈춘다. 재확인 뒤·등록 전에 `shutdown_all` 이 id 를 뜨면 새 화신이 그 목록 밖에 남는 창이 있다 — 모든 활성화가 셧다운과 겹칠 때의 선재 창과 같은 부류 |
| [B] 폴백 중 같은 id 의 다른 활성화 | 예약을 쥐고 있어 in-flight 가드가 `Err` 로 돌린다 |
| [B] barrier 시한 초과 · reaper 없음 · 수거 안 됨(join 상한 초과) | 오늘처럼 포기(기록 + `Failed`) |
| [B] claude 판정 창(3 s) 안의 두 번째 활성화 | 「이미 실행 중」 `Ok`(곧 D4 가 거둘 화신) — claude 는 그 창에 예약이 없다. 오늘도 같은 창이다(그 화신이 스스로 죽는다) |
| [A] 첫 답을 받는 중 kill | id 는 **보내기 전에** 영속됐다 → 다음엔 이어받기. codex 는 rollout 이 턴 시작부터 있어 이어진다. claude 가 아직 대화를 안 만들었으면 D4 가 새 대화로(A 만이면 오늘과 같은 실패). ★codex 터미널은 락 회수가 첫 턴보다 늦었으면 아직 영속 전일 수 있다(아래)★ |
| [A] codex 터미널: 회수 전 Enter, 회수 전 크래시 | 대화가 있어도 손잡이 없음 → 다음 활성화 Fresh. ADR-0218 과 같은 등급(회귀 아님 — §3-2-4) |
| [A] 추적기가 첫 제출 전에 id 를 바꿈 | commit 이 비교-교체라 추적기 값이 남는다(§3-2-1) |
| [A] 두 읽기 사이 손잡이 소실 | 프로세스를 띄우기 전에 `SpawnFailed`(재시도 가능) — 모르는 id 의 새 대화가 뜨지 않는다(§3-2-3) |
| [A] 영속했는데 제출 write 가 실패(note 바로 뒤의 `send_input` — 키 입력의 CR 조각 · 우편의 CR · JSON 본문) | 쓰레기 id 가 남는다 → 다음 활성화에서 D4 가 흡수(사용자 수용 · A 만이면 오늘과 같은 실패). `Raw` 우편의 본문 쓰기·착지 확인 실패는 note **앞**이라 영속이 없다(§3-2-2) |
| [A] kill 창(`UserKill` 뒤 · 수거 전 — 데몬 셧다운 포함)에 온 입력·우편 | 세지도 보내지도 않고 `Err` — 큐가 닫힌 뒤와 같은 `WriteFailed` 라 WS 는 오류 답장, 우편은 재파킹 → 영속도 턴도 없음(§3-2-2). 남는 창 둘(intent 확인 뒤의 kill · kill 전 제출에 닿는 늦은 offer)은 바로 위 행과 같은 부류 |
| [A] commit 거절·패닉 | 포트를 다시 부르지 않는다(§3-2-1 의 한 규칙) · 거절이면 턴은 나간다 · 패닉은 릴리스에서 데몬 종료(`panic = "abort"`), unwind 빌드에서는 note 에서 난 그 한 번의 입력만 안 나간다(§3-2-2) |
| 저장소가 IO 오류를 삼킴 | 오늘과 같은 등급 — 래치는 성공으로 보고 턴은 나가며, 다음 성공한 저장이 디스크를 고친다(`observe_session_id`·`commit_session_id` 의 bool 은 메모리 기준 — `profile.rs:704-705`) |
| [A] 선재: 거둔 claude 화신의 추적기 항목이 남아 있다가 옛 값을 적음(reaper 는 `unwatch` 하지 않는다 — `kill_agent` 만 한다, `manager.rs:2725`) | 다음 화신이 `watch` 로 갈아치우기 전에 그 값이 적히면 새 화신의 첫 commit 이 거절된다(칸 ≠ `expected`). 남는 값(옛 화신의 마지막 id)은 오늘과 같다 — 오늘은 스폰 때 영속한 값을 그 쓰기가 덮는다 |
| 데몬 재기동 | 메모리의 미영속 id 는 사라진다 — 대화가 없었으므로 잃을 것이 없다(codex 터미널의 회수 전 크래시는 위 행). 복원은 영속된 id 로만 |
| 옛 프로필의 쓰레기 id | 다음 활성화에서 이어받기 실패 → D4. 도달성: claude JSON·codex app-server 실측 · claude 터미널·codex 터미널은 B0 결과에 매인다. A 만이면 오늘과 같은 실패(§6-3) |
| 터미널 붙여넣기(여러 줄) | CR 이 들어 있어 제출로 센다 → 대화 없이 영속될 수 있음 → D4 가 흡수(사용자 수용 — 저장 시점 정밀도는 무관) |
| 우편 주입 턴 | `submit_input_observed` 가 셈 — 대화로 친다 |
| 신뢰 모달·안내 화면에서 Enter | 제출로 센다 → 위와 같은 흡수 |
| [B] 죽을 화신이 받은 입력·우편 | 폴백으로 사라진다(codex 는 경고 로그 한 줄 · 우편 영수증은 「배달됨」) — §7 Q7 |
| [B] 대화가 있는데 backend 가 「없다」고 답함(cwd·`CODEX_HOME` 변경) | D4 가 그 대화를 버리고 새 대화를 연다(옛 id 는 `old_session_ids` 에 남는다) — §7 Q3 |
| [B] codex 프로필에 이전 시도의 `NoConversationToResume` 기록 | 폴백이 성공해도 남는다(ADR-0202) — §3-4 · §7 Q3 |
| 차가운 UI 기동(화신이 이미 이력을 받음) | 구독 응답이 표식을 다시 싣고 replay 에 이력이 있어 곧바로 대화 |
| ★이어받았는데 이력 조회가 0건(예산 초과 · 조회 실패 · claude transcript 판독 실패)★ | 로딩 아이콘이 **입력할 때까지** 남는다. 입력창은 살아 있다. ★결정 — 사용자 2026-09-24: §7 Q2 = (a) 수용★ |
| ★PRE-EXISTING★ 챗 슬롯에서 보낸 입력이 거부됨(`WriteStdin` `Err`) | 입력창이 이미 비워졌고 화면 오류가 없다 — 오늘도 입력 큐가 닫힌 뒤 같은 경로로 같다. `Exiting` 동안 RichSlot 입력이 살아 있고(`agentGone` 은 Exited·Killed·Failed 만 — `src/components/slot/RichSlot.tsx:103-108`), 거부 처리는 콘솔 기록뿐이다(`:249-277`). 이번 설계 범위 밖 — 백로그 |
| codex 핸드셰이크 중 사용자 입력 | `hasSent` 로 로딩 종료 · 에코가 복원 이력 위에 그려지는 것은 ADR-0204 의 알려진 잔여 그대로 · 그 이어받기가 폴백으로 끝나면 입력은 사라진다(Q7) |
| 구데몬 + 신셸 / 신데몬 + 구셸 | 오늘의 첫 화면 동작으로 떨어진다(§3-3) |

---

## 4. 테스트 계획 (TDD)

### 4-1. 새로 쓰는 것 — 무엇을 증명하나

**agent crate [A]**
- `session_id_latch` 단위(프로세스 없음): ① offer → note 에 commit 1회, 값 일치 ② note → offer 에도 commit 1회 ③ note 없으면 commit 0 ④ note 두 번·offer 뒤 note 반복에도 1회 ⑤ commit 이 성공한 뒤 들어온 둘째 offer(계약 밖)는 포트를 부르지 않는다 — ⑤·⑨·⑩ 이 한 규칙 「포트는 화신당 많아야 한 번 불린다. 한 번 부르기 시작했으면(성공·거절·패닉 무엇으로 끝나든) 다시 부르지 않는다」(§3-2-1)의 세 갈래다 ⑥ 두 스레드가 `Barrier` 로 동시에 offer·note — 반복 N 회 모두 정확히 1회 ⑦ ★**commit 이 진행 중일 때 다른 스레드의 note 는 그 commit 이 돌아오기 전에 돌아오지 않는다**★(commit 포트를 채널로 붙잡아 재는 수법 — 빠른 길 경합의 회귀망) ⑧ offer 없이 note 만 반복하면 settled 가 안 서고 commit 0 ⑨ commit 이 패닉해도 뒤이은 note·offer 가 멈추지 않고 **포트를 다시 부르지 않는다**(독 회수 + `committed` 선행 — unwind 빌드에서만 도는 시험이다. 릴리스는 abort, §3-2-2) ⑩ 포트가 아무것도 적지 않고 돌아와도(거절) settled 가 서고 포트를 다시 부르지 않는다 ⑪ commit 전에 온 둘째 offer 도 버린다 — 첫 값이 commit 된다.
- registry `commit_session_id`: 칸 == `expected` → 교체 + 이력 · 칸 == `new` → 쓰기 없음 · 칸 == 다른 값 → 안 씀 · 표식 불일치 → 안 씀. ★「추적기가 첫 제출 전에 쓴 값을 commit 이 되감지 않는다」★를 이름으로 박는다. ★「이어받기 화신의 첫 offer 가 저장값과 다른 id 면 교체된다」★(칸 == `expected` == 저장값 — ADR-0216 이 기각한 「빈 칸에만」이 아님을 재는 짝. `0216:61` 의 논증이 서는 자리가 첫 offer 다)도 박는다. 포트가 `expected` 를 옮기는 방어 절(§3-2-1)은 포트 단위 시험 하나로만 잰다(성공한 commit 뒤 같은 포트의 둘째 호출이 새 값을 교체한다). 두 동사가 같은 가드 도우미를 부르는지는 소스 구조 시험으로 잰다(§3-2-3).
- `InputEncoder::submits_turn`: Raw 키 조각(CR 없음) 거짓 · CR 포함 참 · JSON 인코더 둘 항상 참. 이름에 키 인코딩 가정(`raw_submission_is_cr_under_default_xterm_encoding`).
- `AgentSession`(기존 `CapturingTransport` 하네스, `session.rs` tests): CR 없는 쓰기는 안 셈 · CR 있는 쓰기는 **`send_input` 보다 먼저** 셈(공유 사건 기록으로 순서 단언) · 우편 제출: ★`Raw` = 본문 쓰기 **뒤**·제출 CR `send_input` **앞**에 셈 · JSON = 본문 쓰기 **앞**에 셈★(§3-2-2) · `Raw` 우편의 본문 쓰기·착지 확인이 실패하면 안 셈 · 래치 없는 기본 세션은 무동작 · ★`UserKill` 이 선 세션(래치 있음)은 두 동사 모두 `Err` 를 돌려주고, 확인 자리에서 아무것도 안 쓰며(CR 이 든 키 입력 조각·JSON 턴 = 기록 0 · CR 없는 키 입력은 그대로 기록됨 · `Raw` 우편 = 본문 뒤 CR 없음 · 거절 문구가 사용자 종료를 명시), note 도 안 센다(래치 포트 호출 0)★(kill 창 — §3-2-2).
- ★**배선 구조 시험**★(이 저장소의 소스 분할 시험 방식 — `manager.rs:3845` `fresh_arm` 선례): `spawn_agent_watching_link` 본문에서 ① `open_spawn` 의 기록 포트 자리가 `Some(latch.offer_sink())` 이고 `Some(session_id_sink(` 이 없다 ② `latch.offer(` 가 `backend::open_spawn(` 보다 **앞**이다(claude offer 시점) ③ 같은 `latch` 가 `spawn_session` 으로 간다 · `spawn_session` 본문이 `.with_incarnation(` **과 `.with_session_id_latch(` 를 둘 다** 부르고, 그 인자가 `SpawnIncarnation` 에서 꺼낸 래치다(리뷰 지적 반영) · 운영 구획에 `ensure_session_id(` 가 없다.
  - ★③ 의 `.with_session_id_latch(` 단언을 빼지 말 것★: 필수 인자는 래치를 함수까지만 끌고 오고 **세션에 싣는 것은 강제하지 않는다** — `SpawnIncarnation { latch: _, .. }` 로 버려도 컴파일되고, 이름으로 해체한 뒤 안 써도 경고일 뿐이다(`ci.yml`·워크스페이스 `Cargo.toml` 에 경고를 오류로 올리는 설정이 없다 — 확인). 그 빌더가 빠지면 아무것도 영속되지 않아 모든 이어받기가 조용히 사라지는데(§3-2-2 의 최악), 그것을 잡는 다른 시험은 미확인인 PATH 수법(Q4) 하나뿐이다. 그래서 구현은 `spawn_session` 머리에서 `let SpawnIncarnation { continues_conversation, latch } = incarnation;` 로 이름째 해체하고, ③ 이 두 빌더 호출을 잰다.
- D1 claude(기존 「존재하지 않는 cwd 로 스폰 실패」 수법 — `tests/activation.rs:739` 선례): Fresh 스폰 뒤 `backend_session_id` 가 `None` · 옛 값은 이력으로 · 디스크(`store.load()`)에도 없음.
- D1 argv: `build_command_spec(Claude, Fresh, Some(minted), …)` 가 `--session-id <minted>` 를 싣는다(순수 함수 — 「발급 지점에 닿았다」를 영속 없이 증명).
- 추적기 전제: claude `session_id_source` 의 첫 폴링이 기준값과 같으면 `Unchanged`(§3-2-6). 시험 자리 = `backend/claude/session_file.rs` 의 시험 모듈(`:228`) — 비교(`:210`)가 거기 있다.
- 손잡이 단일 읽기: 프로필의 id 를 비운 뒤 공개 `spawn_agent(profile, Resume)` 를 직접 부르면(머리의 우회를 지나지 않는 입구라 경합을 결정론으로 재현한다) claude·codex 모두 **프로세스 없이** `SpawnFailed` · shell 은 옛 길.
- D2: `AgentSession::subscribe_from` 이 자기 표식으로 `epoch_matches` 를 계산한다 · `on_ready` 가 받은 `Incarnation` == 반환의 `Incarnation` · 이어받기 스폰의 표식 참 / Fresh 거짓(`resume_handle_for` 와 같은 동치 — 아래 daemon 항목과 같은 셸 수법).
- 손잡이 없는 Resume(claude): `spawn_fresh_settled` 로 가고 `Started`, 쓰레기 영속 없음. shell 은 옛 길(`tests/activation.rs:167` 이 그대로 초록이어야 한다).

**agent crate [B]**
- D4 구조(소스 분할 시험): 세 팔 모두 「예약 인계/취득 → 거두기 → `unwatch` → barrier → 재확인(`termination_intent`·`shutting_down`) → 예약 인계 새 대화」 순서 · 폴백 안에 `resume_no_fallback` 호출 없음 · `Killed` 팔·`Moot` 팔·spawn `Err` 팔에는 폴백 없음 · barrier 상한이 `LINK_RESOLUTION_BACKSTOP` 이 아닌 `REAP_BARRIER_BACKSTOP`.
- `reaper` 단위: `Barrier` 가 앞선 `Reap` 처리(목록 통지 포함) 뒤에 답한다(FIFO) · 명부에 `(id, epoch)` 가 있으면 `false` · 없으면 `true` · 같은 id 의 다른 표식만 있으면 `true`.
- ADR-0082 회귀: `reactivate_running_agent_leaves_it_alive_epoch_unchanged`(`tests/activation.rs:229` — ADR-0082 본문의 회귀 ①: 산 에이전트 재활성화 시 kill·재spawn 없음·표식 불변)가 그대로 초록.

**D4·D1 런타임 통합(실 프로세스 — `-- --test-threads=4`)**
- 가짜 `claude.cmd` 를 임시 폴더에 두고 프로필 env 의 `PATH` 앞에 붙인다(claude 는 `cmd.exe /c claude …` 로 PATH 해석 — `transport/stdio.rs:77` 주석). 가짜는 `--resume` 이면 `No conversation found with session ID: …` 를 찍고 exit 1, `--session-id` 면 살아 있다. 증명: 스폰 2회(한 번은 `--resume`, 한 번은 `--session-id`) · 결과 `Ok`·Running · 쓰레기 id 가 이력으로 · `last_failure` 없음 · 첫 `write_stdin`(CR) 뒤에야 민팅 값이 디스크에 앉는다.
- 같은 가짜로 [B] 재확인: 가짜가 `--resume` 에서 문구를 찍고 몇 초 살게 한 뒤 판정 뒤 kill → 새 스폰 없음(스폰 1회)·기록 없음 · 폴백 중 `shutdown_all` → 새 스폰 없음 · 폴백 중 두 번째 활성화 → 「다른 요청이 활성화 중」 `Err`·스폰 최대 2회.
- 같은 수법의 codex 판(`codex/mod.rs:2190-2240` 의 가짜 app-server 스크립트를 가짜 `codex.cmd` 가 띄운다): `thread/resume` 거절 → 새 스레드 · 첫 턴 뒤에만 thread id 영속.
- ★**이 수법이 서는지는 미확인이다**★(env 의 PATH 가 `cmd /c` 의 해석에 먹는지). A2 착수 때 파일럿 1건으로 먼저 잰다. 안 서면 manager 에 스폰 주입 seam 이 필요하고 그것은 **사용자 결정**이다(ADR-0012) — §7 Q4.

**daemon · protocol [A]**
- `handle_subscribe`: 이어받기 표식이 든 세션 → ack 에 `continues_conversation: true`, ack → replay → `ReplayComplete` 순서 불변 · Ack 의 표식 == `ReplayComplete` 의 표식 == replay 프레임의 화신.
  - ★**그 세션을 만드는 법 — 새 운영 seam 이 필요 없다**★(리뷰 지적 반영): 셸 프로필에 손잡이를 심고(`backend_session_id = Some(..)`) `core.manager.spawn_agent(&profile, SpawnMode::Resume)` 를 부른다. spawn 은 Resume 이면 backend 와 무관하게 명부의 손잡이를 읽는다(`manager.rs:1350-1358`). 셸은 그 값을 버린다(`build_spec` 의 `_resume_session_id` — `backend/shell/mod.rs:52` · 기본 `open_spawn` 의 `let _ = (..)` — `backend/mod.rs:465`). 그래서 셸이 그대로 뜨고 표식은 참이다. 새 id 는 등록의 신규 갈래가 스냅샷째 넣으므로(`manager.rs:1104` → `profile.rs:530`) 심은 손잡이가 명부에 선다. 기존 시험 `subscribe_emits_ack_then_replay_then_complete_in_order`(`connection_core.rs:2597`)가 이미 셸을 `spawn_agent` 로 띄운다 — 같은 수법이다. 대조군 = 같은 프로필 Fresh → `false`. (확인 = 코드 읽기 — 아직 돌려 보지 않았다.) `SessionIdLatch`·빌더가 crate 전용이어도 이 길은 막히지 않는다.
- ★구독 도중 재스폰(리뷰 지적 반영)★: 두 조회 사이의 화신 교체는 dispatch 로 결정론 재현이 안 되는 TOCTOU 라(기존 주석 `connection_core.rs:1778-1781` 과 같은 판단) 구조로 잰다 — `handle_subscribe` 본문에 `agent_epoch(` 가 없고 Ack·`ReplayComplete` 의 표식이 reply 에서 온다. 짝으로 agent 쪽 단위(위 D2)가 「한 Arc 에서 나온다」를 잰다.
- codec: 칸 없는 옛 `SubscribeAck` JSON 이 `false` 로 읽힌다(하위 호환 단언).

**src-tauri [A]**
- `replay_flight`: ack 의 표식이 성공 마커 bit2 로 · 실패 마커(마감·거절)는 0 · 길이 30 불변.
- `tests/daemon_client_replay.rs`: `apply_replay_event` 가 `deliver` 로 넘기는 바이트에 bit2.

**프론트(vitest) [A]**
- `LoadingPanel`: `role=status` · 아이콘 · 기본 텍스트 없음 · aria 이름 = `common.loading` · `label` 을 주면 그린다 · `data-loading-panel`. (e-ink·reduced-motion 정지는 jsdom 이 css 미디어를 못 돌려 GUI 실측으로 잰다.)
- `wsFrame`: bit2 해독 · `tauriTransport`: 정규화에 실림 · `wsTransport`: `SubscribeAck` 칸 → 경계 · `protocolClient`: 보관 마커(myGen 미확정) 경로에서도 실림 · `'live'` 에만 `info` · ★onReset 은 언제나 `'buffering'` 뒤, 같은 `flushToLive` 안에서 `'live'` 직전에만 불린다(순서 핀 — §3-5 의 반영하지 않은 지적이 기대는 성질)★.
- `RichSlot`: ① 표식 참 + 0건 → 첫 화면 없음·로딩 있음 ② 행을 그리는 첫 항목 → 로딩 사라짐·첫 화면 없음 ③ `Usage` 만 온 동안 로딩 유지 ④ 입력 → 로딩 사라짐 ⑤ 에이전트 부재 → 막·로딩 없음 ⑥ onReset + `'live'`(거짓) → 첫 화면(D4 재시작) ⑦ 표식 거짓 → ADR-0145 시험 전부 그대로 ⑧ `info` 없는 `'live'` → 거짓으로 읽음 ⑨ ★`Usage` 만 온 동안 대기 꼬리(`WaitRow`) 없음★.

### 4-2. 바뀌는 기존 시험

- `tests/activation.rs:739` `we_do_not_mint_a_session_id_for_a_backend_that_mints_its_own` — 대조군 「claude 프로필에 sid 가 있다」가 D1 로 뒤집힌다. 대조군을 「claude spec 이 `--session-id` 를 실었다」(위 argv 시험)로 옮기고, 이 시험의 claude 단언은 「영속 안 됨」으로 바꾼다.
- `manager.rs:2967-2968` — `assert!(!fresh_spawn_release_session_id(&claude, SpawnMode::Fresh))` 가 뒤집힌다(claude Fresh 도 이제 비우고 시작한다 — Resume 단언 `:2968` 은 그대로). 그 위 주석 「우리가 발급하는 쪽 — `new_session_id` 가 같은 밀기를 이미 한다」도 함께.
- `profile.rs` 의 `new_session_id`·`ensure_session_id` 시험(`:900-1060` 부근) — 동사 교체에 맞춰 다시 쓴다. `manager.rs:3082` 의 `ensure_session_id` 사용도. 기록 포트 시험(`releasing_the_handle_does_not_disturb_a_transport_that_refills_it` — `:2972` 등)은 포트가 비교-교체를 부르게 된 만큼 `expected` 를 준다.
- ADR-0082 회귀 — `activate_resume_early_exit_ends_failed_no_fresh_fallback`(`tests/activation.rs:167`, 파일 주석은 「①」이지만 ADR-0082 본문의 회귀 ②다 — 번호가 서로 어긋나 있어 이름으로 부른다) — shell 대역이라 분류가 `None` → 그대로 초록이어야 한다. 문구 「새 대화 안 생김(spawn 정확히 1회)」 옆에 「단 `NoConversationToResume` 부류는 예외(ADR-B)」를 단다 — [B] 몫이다.
- `subscribe_from_err_never_invokes_on_ready`(`manager.rs:4232`) — 인자 모양(`requested_epoch`·reply)만.
- `connection_core.rs` 의 ack golden(`:4755` 부근)과 순서 시험(`:2652`·`:2671`) — 칸 추가.
- src-tauri 의 `Marker`·`on_ack` 리터럴/호출 — 컴파일이 이끈다.
- 프론트 `wsFrame.test.ts:85-94` · `tauriTransport.test.ts:144-163` 마커 조립 도우미 — bit2 선택지.
- ★**바뀌지 않는 것**★(오케스트레이터 목록에 있었으나): `transport.rs` `:4123`·`:4150`·`:4169`·`:4763`, `codex/mod.rs` 의 두 시험 — 통로가 sink 를 부르는 계약·순서가 그대로이기 때문이다. 바뀌는 것은 sink 뒤쪽이다.
  - ★**단 재는 뜻이 줄어든다**★(리뷰 지적 반영): `the_session_id_is_recorded_before_the_gate_opens`(`transport.rs:4763`) · `a_recording_failure_ends_the_session`(`codex/mod.rs:2716`)은 코드는 그대로 초록이다. 그러나 이제 핀하는 것은 「게이트 전에 sink 에 **건넸다**」까지다 — sink 가 래치의 offer 가 되어 영속은 그 뒤 제출에 매이기 때문이다. 「첫 턴 전에 **영속**」은 셋이 함께 진다: 래치 시험 ⑦ · `AgentSession` 순서 시험(note 가 제출 write 앞 — §4-1) · 배선 구조 시험(③ 의 `.with_session_id_latch(` 단언 포함 — 그것이 없으면 필수 인자만으로는 세션에 래치가 안 실린 배선을 못 잡는다). 이 시험들의 doc 에 그 사실을 적는다.
- 손잡이 읽기 자리를 재는 구조 시험 `the_resume_handle_is_read_from_the_roster_after_the_incarnation_tag_is_stamped`(`manager.rs:5657`)는 앵커 줄 `let stored_handle = match mode {` 를 그대로 두면 초록이다 — A2 가 읽기를 발급 자리로 올려도 표식 확정 뒤·spec 조립 앞이다. 앵커 줄을 바꾸면 그 시험의 `only(..)` 도 함께 바꾼다.

### 4-3. 새 운영 seam

**없다(계획상).** 래치는 순수 API 라 직접 시험되고, `AgentSession::with_incarnation`·`with_session_id_latch` 는 crate 전용 조립 도우미이며, `ReaperCmd::Barrier` 는 운영 동사다. daemon 쪽 구독 표식 시험도 셸 수법으로 선다(§4-1) — `test-harness` 기능에 훅을 새로 열지 않는다. 단 §4-1 의 PATH 수법이 안 서면 manager 스폰 주입 seam 이 필요해진다 → §7 Q4.

### 4-4. GUI 실측 (`/qa full` 바인딩 절차 · 격리 인스턴스)

- 지난 측정 도구를 다시 쓴다(스크래치패드 `flicker-probe/` — `instrument.js` 가 `subscribeOutput` 콜백을 감싸고 `[data-rich-live]` 에 `MutationObserver` 를 걸어 시각을 찍는다). 관측 키에 `loading = !!el.querySelector('[data-loading-panel]')` 과 대기 꼬리 유무를 더한다.
- 격리: 전용 데이터 디렉터리·CDP 포트, 런처(`scripts/launch-detached.ps1` / `run-*.bat`) — 셸에서 직접 띄우지 않는다.
- 시행(각 3회 이상):
  1. [A] codex JSON 이어받기(kill → 재활성화) — **첫 화면이 한 번도 안 선다** · 로딩이 `'live'` 부터 첫 이력 행까지 · 대기 꼬리 없음 · 그 길이 ≈ `thread/resume` 왕복.
  2. [A] 새 codex JSON — 첫 화면 섬 · 로딩 없음.
  3. [A] 새 claude JSON — 첫 화면 섬 · 로딩 없음.
  4. [A] claude JSON 이어받기 — 첫 화면 없음 · 로딩 없음(또는 1프레임 이하).
  5. [A] ★새 claude JSON 을 띄우고 입력 없이 kill → `agents.json` 의 `backend_session_id` 가 `null` 로 남는다 → 재활성화 argv 가 `--session-id`★.
  6. [A] D1 claude 터미널: 띄우고 Enter 없이 kill → `null` → 재활성화 argv 가 `--session-id`.
  7. [A] codex 터미널: ★시간이 아니라 사건을 기다린다(ADR-0038)★ — 데몬 로그에 그 에이전트·화신의 래치 offer 로그(「세션 id 를 받았다 — 첫 제출 전이라 영속을 보류한다」, §3-2-2)가 찍힌 것을 본 뒤 입력 없이 kill → `null` → 다음 활성화가 Fresh. 그 줄 없이 `null` 을 보면 회수가 안 된 것과 구별이 안 돼 공허하게 통과한다. 조건 둘: **신뢰된 폴더**에서 띄운다(처음 보는 폴더는 신뢰 모달이 회수를 막았다 — `step-log.md:2455`) · 데몬 로그를 info 이상으로 켠다(기본 warn — 켜는 법은 `/qa` 바인딩 §full).
  8. [A] 데몬 재기동 뒤 codex 이어받기(지난 B1 시행) — 첫 화면 없음.
  9. [A] e-ink 테마 · reduced-motion — 아이콘 정지(스크린숏).
  10. [B] D4: codex JSON 을 띄우고 입력 없이 kill → 재활성화 — 실패 문구 없음 · 새 화신 · 첫 화면 · `backend_session_id` 가 첫 메시지 전 `null`, 후 값.
  11. [B] 옛 쓰레기 프로필: 데몬을 내린 상태에서 `agents.json` 에 무작위 uuid 를 심는다 → 재활성화 → D4. **모드마다 따로 잰다** — claude JSON · codex app-server · claude 터미널 · codex 터미널(뒤 둘은 B0 결과에 따라 D4 또는 오늘의 실패).
  12. [B] 폴백 중 보이는 순서(Q8 의 근거) — 챗 슬롯(로딩 → 막 → 첫 화면)과 트리(도는 중 → 시체 → 도는 중)의 각 구간 길이를 찍는다.

---

## 5. 영향

- **CLAUDE.md 「핵심 불변식」**
  - 소유권 분할: `session=id/cwd/epoch/cols/rows` → `session=id/cwd/epoch/continues_conversation/cols/rows + 세션 id 래치(제출 쪽 손잡이 — 수령 쪽은 backend·통로가 sink 로 쥔다)`.
  - 새 줄(제안): **세션 id 영속 = 첫 제출 래치 한 지점, 비교-교체로** — 스폰 때 발급해 영속하는 경로를 되살리지 말 것. 예외 = claude `/clear` 추적(ADR-0008 best-effort, §3-2-6 — 사용자 결정 Q1 = (a)). ★「첫 턴 전에 영속」은 codex 터미널에서 성립하지 않는다(id 가 락 회수로만 온다 — ADR-0218)★.
- **문서 갱신**
  - `docs/reference/architecture-overview.md` 「세션 복원 / 활성화」(`:319-` · 요약 `:782`) — [B] 「fresh fallback 폐지」에 D4 예외 · [A] 「persist 반쪽」을 첫 제출 영속으로 · [A] 「새 대화 표식이 wire 에 없다」를 구독 응답 표식으로.
  - `docs/reference/structure/agent-backend.md:167` `assigns_session_id` 행 — 「발급해 프로필에 영속」→ 「발급해 첫 제출 때 영속」.
  - `docs/reference/backend-capabilities.md` §1 — 0턴 세션의 id 가 `agents.json` 까지 내려간다는 종단 실측 문단(`:43-45` 부근)은 **그때의 사실로 남기고** 「D1 이후 0턴 세션은 영속되지 않는다」를 덧붙인다 · 「codex 에만 있는 위험」과 §4 크래시 창 항목에 「id 쪽은 첫 제출 전 영속으로 닫혔다 — codex 터미널 제외(메시지 내용의 내구성은 여전히 없다)」.
  - 코드 doc: `backend/mod.rs:207-211`(`assigns_session_id` — 「프로필에 영속」) · `profile.rs:159-160`(`backend_session_id` — 「최초엔 우리가 생성」) · `manager.rs:1219-1233`(sid 발급 규칙 주석) · `fresh_spawn_release_session_id` doc · ★2차 리뷰가 더한 다섯★ — `thread_lock.rs:412-425`(`run_capture` doc: 「kill 과 기록 사이」의 「기록이 나간다」가 이제 「제출이 있었으면 영속」이고, `:421` 의 `clear_session_id` 는 release 동사로 바뀐다) · `manager.rs:1327-1345`(손잡이 읽기 주석: `ensure_session_id` 로 명부를 거친다는 비교와 「`session_id_sink` 를 건네기 전까지」가 래치 `offer_sink` 로) · `profile.rs:508-526`(`merge_preserving_live`: 「발급 backend 는 spawn 이 `ensure/new_session_id` 로 확정」 서술) · `manager.rs:2966`(시험 주석 — §4-2 와 같은 자리) · protocol 의 버전 판단은 `continues_conversation` 칸 doc 에(§3-3) · [B] `activate_profile` doc 의 최악 블로킹(`:1483-1484` — §3-4 의 45 s / 13 s) · [B] `tear_down_failed_activation` doc 의 「`Terminal`·`Diagnosed` 갈래에서는 부르지 않는다」(`:2213-2214`)를 D4 예외와 Q6 의 결정으로 · [B] `SpawnReservation` doc(수명이 폴백까지 늘어난다) · [B] `reaper.rs` 헤더 불변식에 `Barrier` 한 줄(「수거 여부는 reaper 스레드가 답한다」).
  - `// ADR-NNNN` 앵커: 래치 모듈 · `submits_turn` · 세션 두 동사 · `commit_session_id` · D4 도우미 · `ReaperCmd::Barrier` · `continues_conversation` 계산 자리 · 구독 reply · RichSlot 게이트(D4 도우미·`Barrier` = ADR-B, 나머지 = ADR-A).
- **dev bin:** `saturation_pilot.rs:258-260` 이 스폰 직후 `agent_backend_session_id` 로 transcript 를 찾는다 — D1 로 그 시점엔 `None` 이라 추정 폴백으로 떨어진다. 첫 전송 뒤로 읽기를 옮긴다.
- **PROTOCOL_VERSION:** 안 올린다(§3-3).
- **LLM 제어 표면:** 새로 필요한 것 없음 — D5 는 표시 전용이고, D4 는 기존 활성화 동사(`agent.spawn` · `SpawnProfile` · 부팅 복원) 안에서 일어나 그 결과(`AgentSpawnOk`)가 그대로 돌아온다. 새 전역 핸들 없음. ★단 §7 Q3 에서 LLM 신호를 고르면 `agent.spawn` 결과에 칸이 는다(명령 버스 `#[since]` 판올림)★.
- **ADR: 둘로 나눈다**(리뷰 지적 반영 — Phase A 가 개정 안 된 ADR 본문 여럿과 정면으로 어긋난다). 담을 것은 끝의 「ADR 초안 반영 메모」 A·B. 채번·개정 도장·링크는 `/adr`.
  - **ADR-A = D1 + D2 + D5 — A5 에서 쓴다.** 개정(Amends): ADR-0008 결정 첫 항(spawn 때 발급·persist — `0008:10`) · ADR-0076 결정 첫 항(「명시적 `resume=true` 는 그대로 존중」 — `0076:14` → 손잡이가 없으면 Fresh, D1 의 귀결 — §3-2-3)과 결정 둘째 항 · 「영향」의 `ensure_session_id` = Resume 전용 불변식(Fresh = `new_session_id` 로 발급·영속) · ADR-0082 결정 첫 항(`0082:16` — 「이어받을 손잡이가 없는 이어받기 요청」에 한해 `0082:21` 의 「이어받을 게 없는 … 정상 생성」으로 읽는다)과 「영향」 `0082:37` 의 Fresh 동사 이름 · ADR-0185 결정 2(codex 수령 지점 = `observe_session_id`)와 「영향」의 수령 단일점(`0185:36`)·Phase 2 요구사항(첫 턴 전 persist — 이제 codex 터미널 밖에서 구조로 선다) · ADR-0216 결정 7 · ADR-0217 결정 7 · ADR-0218 결정 6(덮어쓰기 값 정책 → 화신 자기 commit 기준의 비교-교체) · ADR-0145 결정 1(`0145:14` 「복원 완료 신호 + 0건」 → 이어받는 화신이 아닐 때만 빈 상태 · 이어받는 화신이면 로딩)과 근거(`0145:37` 「프로토콜·백엔드 변경이 필요 없다」). 관련(링크): ADR-0083 · ADR-0172 · ADR-0204.
  - **ADR-B = D4 — B3 에서 쓴다.** 개정: ADR-0082(「이어받을 대화가 없다」 부류에 한해 새 대화). 관련: ADR-0077 · ADR-0202 · ADR-0019 · ADR-0172 · ADR-0201 · ADR-0001 · ADR-A.
- **고아 방지:** 이 문서는 step-log 항목과 두 ADR 에서 가리킨다(`docs/README.md:47`).

---

## 6. 구현 순서

각 조각 끝에서 빌드·시험이 초록이어야 한다. **중간 어디서 멈춰도 빌드가 서게** 순서를 짰다 — 새 타입·동사를 **아무도 안 부르는 채로 먼저 깔고** 뒤에서 배선한다.

### 6-1. Phase A — D1 + D2 + D5

| # | 조각 | 파일 | 끝났을 때 |
|---|---|---|---|
| A1 | 순수 조각 + 구독 reply: 래치 모듈 · `submits_turn` · `commit_session_id`·`mint_session_id`·release · `Incarnation`·`SubscribeReply` · `AgentSession` 빌더(기본 거짓·래치 없음)와 입력 두 동사의 note(래치 없으면 무동작 · 우편은 제출 CR 직전 · `UserKill` 이면 세지도 보내지도 않고 `Err` — 래치가 없어 동작 불변) · `subscribe_from` 이 reply 를 돌려준다 · daemon `handle_subscribe` 가 reply 로 표식을 채운다(wire 칸은 아직 없음) | `session_id_latch.rs`(새) · `backend/mod.rs` · `profile.rs` · `types.rs` · `session.rs` · `manager.rs`(위임만) · daemon `connection_core.rs`(한 함수) | **동작 불변** — 운영이 빌더를 안 부른다 |
| A2 | **D1 배선**: `spawn_session` 필수 `SpawnIncarnation` · 래치 생성 + claude offer(생성 자리) · `open_spawn` 에 `offer_sink` · claude 민팅 무영속 + Fresh release · 손잡이 단일 읽기 + 띄우기 전 거절 · 머리의 손잡이 없는 우회(+ `Ready`·`Alive` 팔의 죽은 삼항 걷기 — §6-3) · `ensure/new_session_id` 걷기 · 배선 구조 시험 · 시험 갱신(§4-2) · dev bin. PATH 수법 파일럿 1건을 여기서 먼저 잰다(Q4) | `manager.rs` · `profile.rs` · `tests/activation.rs` · daemon `bin/saturation_pilot.rs` | 0턴 세션이 영속 안 됨. 표식은 세션에 서지만 wire 에 아직 없다 → 화면 불변 |
| A3 | wire + 셸: ack 칸(`serde(default)`) · 바인딩 재생성 · 데몬이 reply 의 표식을 ack 에 싣는다 · flight/마커 bit2 | `messages.rs` · `bindings/` · `connection_core.rs` · `replay_flight.rs` · `connection.rs` | 표식이 웹뷰까지 온다. 프론트는 아직 무시 |
| A4 | 프론트: 마커 해독 → `'live'` info · `LoadingPanel` · RichSlot 게이트 · `isRenderedItem` · 대기 꼬리 판정 · `common.loading` | `src/api/*` · `src/components/ui/` · `RichSlot.tsx` · `StructuredTextView.tsx` · `src/i18n/ko.ts` | **P1 해소**(codex 이어받기 = 로딩) |
| A5 | **ADR-A 작성**(D1+D2+D5 — 개정·링크는 §5 와 끝의 메모 A, `/adr`) · A 몫 문서·CLAUDE.md·앵커 · GUI 실측 [A] 시행(§4-4 1–9) | `docs/decisions/` · docs | ADR-A 확정 · 개정 대상 ADR 에 도장 |

### 6-2. Phase B — D4

착수 전 사용자 결정: **Q6**(Diagnosed 처분 — B2 의 모양을 바꾼다) · **Q7**(코드가 붙을 수 있다) · **Q3**(LLM 신호를 고르면 wire 가 는다). Q8 은 B3 의 GUI 실측 뒤에 정해도 된다. 착수 전 TRD 개정: §3-4 끝 「B 라운드 착수 전 반영할 리뷰 지적」 여섯.

| # | 조각 | 파일 | 끝났을 때 |
|---|---|---|---|
| B0 | **측정**: codex 터미널 `codex resume <rollout 없는 uuid>` 의 PTY 문구·종료코드·시각 · ★claude 터미널 `--resume <대화 없는 uuid>` 의 문구 시각과 **종료 시각(3 s 창 안인가)**★. 잰 문구만 `resume_failure_kind` 에 더한다(ADR-0172 규율). 창 밖에서 죽으면 그 사실을 문서에 남기고 그 칸은 D4 밖으로 둔다(콘솔 링 판정은 금지 — §3-4) | `codex/mod.rs` · (문구가 다르면) `claude/mod.rs` | 분류 시험 초록 |
| B1 | `ReaperCmd::Barrier { id, epoch, reply }` + reaper 단위 | `reaper.rs` | 동작 불변(아무도 안 보낸다) |
| B2 | **D4**: 예약 인계(`spawn_agent_watching_link` 의 `reservation` 인자 · `LinkWatch::into_reservation` · `spawn_fresh_settled_with`) · `REAP_BARRIER_BACKSTOP` · 세 팔의 폴백 도우미(거두기·`unwatch`·barrier·재확인·새 대화) · doc(최악 블로킹·`tear_down`·`SpawnReservation`) · 시험 | `manager.rs` | **P2 의 옛 프로필·부정확 방아쇠 흡수** |
| B3 | **ADR-B 작성**(D4 — ADR-0082 개정, 메모 B) · B 몫 문서 · GUI 실측 [B] 시행(§4-4 10–12) | `docs/decisions/` · docs | ADR-B 확정 · ADR-0082 에 도장 |

### 6-3. Phase A 만 착지했을 때 — D4 가 흡수할 경우들

★결론 = **손잡이가 있는 이어받기에서는 활성화 결말·기록·처분이 오늘과 같고, 챗 슬롯이 막 전까지 첫 화면 대신 로딩을 그리는 것만 다르다.**★ 손잡이가 **없는** Resume 은 결말이 바뀐다 — 바로 아래 첫 항목(D1 의 귀결 — §3-2-3). 확인한 근거: A 는 `resume_no_fallback` 의 판정 팔 다섯(`Terminal`·`Diagnosed`·`LinkFailed`·`Ready`·`Alive` — `manager.rs:2068-2196`)의 **동작**을 안 바꾼다(`Ready`·`Alive` 에서 늘 거짓이 되는 삼항을 걷는 것뿐 — 아래 「손잡이 없는 Resume」). A2 가 바꾸는 것은 spawn 안(래치·손잡이 단일 읽기·release)과 머리의 손잡이 **없는** 우회뿐이고, 아래 경우는 전부 손잡이가 **있는** 이어받기라 그 우회에 안 걸린다.

- ★**손잡이 없는 Resume 이 `spawn_fresh_settled` 로 옮겨 가며 달라지는 것**★(리뷰 지적 반영 — §3-2-3 의 「보고 낱말이 같다」는 낱말만 맞다). 오늘 이 조합에 드는 것은 codex(WS `SpawnProfile{resume:true}` · 손잡이 없음)이고, A2 뒤에는 claude 도 든다. 보고 낱말(`Started`)은 같지만 경로(`manager.rs:1999-2066` → `:1687-1700`)가 바뀌어 넷이 달라진다:
  1. **연결 축이 없는 통로**(codex 터미널 · claude): 3 s 조기종료 창(`early_activation_verdict`)을 안 거친다. 창 안에 죽는 경우가 오늘은 `Failed` + 기록인데, 옮긴 뒤에는 곧바로 `Started` 다(`spawn_fresh_settled` 의 let-else → `note_spawn_result`, `:1695-1698`). 그 대신 활성화가 3 s 빨리 돌아온다.
  2. 같은 갈래에서 `note_spawn_result` 가 **곧바로** `last_failure` 를 지운다. 오늘은 창을 넘긴 `Alive` 에서만 지운다. 연결 축 codex app-server 는 양쪽 다 `Ready` 에서 안 지운다(같다).
  3. **연결 실패 분류**: 오늘 `resume_failure_kind(..)`(`:2134`) → 옮긴 뒤 `Other` 고정(`:1732-1738`). `thread/start` 거절에 「이어받을 대화가 없다」 어휘는 거짓이라 옮긴 쪽이 맞다.
  4. **Fresh release 가 돈다** — 칸이 이미 비었으므로 no-op 다.
  - ★**죽은 갈래를 걷는다(고름)**★: 위임 뒤 `resume_no_fallback` 의 나머지 경로에서 `opens_a_new_conversation` 은 늘 거짓이다. 그래서 `Ready`(`:2175`)·`Alive`(`:2189`) 팔의 삼항이 죽는다. A2 가 두 팔을 `Resumed` 단일로 줄이고, 그 변수는 머리의 위임 판정으로만 남긴다. 그 팔을 재는 구조 시험 `an_optimistic_success_does_not_clear_the_failure_record`(`:3993`)는 삼항을 안 보므로 그대로 초록이다.

- **옛 프로필의 쓰레기 id**(D1 이전에 영속된 0턴 id): 같은 argv(`--resume <쓰레기>` / `thread/resume`)로 뜨고 같은 팔에서 끝난다 — `NoConversationToResume` 기록 + 시체(claude JSON 은 진단 뒤 스스로 죽고, codex app-server 는 거둔다). 래치: claude 는 offer(S)·commit(S) 가 같은 값이라 no-op, codex 는 거절이라 offer 가 없다 — 디스크가 오늘과 같다. 화면만 다르다: 스폰이 손잡이를 실었으므로 구독 응답의 표식이 참이라 챗 슬롯이 **로딩 아이콘 → 막**을 그린다(오늘 = 첫 화면 → 막). 로딩 길이 = codex app-server 는 거절 왕복만큼, claude JSON 은 종료까지(+6.4 s).
- **A 가 새로 만드는 쓰레기**(제출로 셌는데 대화가 없는 경우 — 신뢰 모달의 Enter · 붙여넣기 · 영속 뒤 `send_input` 실패 · 첫 답 전 kill 인데 claude 가 아직 대화를 안 만든 경우 · `/clear` 후 말없이 끔): 다음 활성화가 위와 같은 길 — 오늘과 같은 실패. 단 오늘은 **입력 없이 끈 모든 세션**이 이 실패였고, A 뒤에는 이 부분집합만 남는다.
- **회복 수단:** 오늘과 같다 — 그 프로필은 삭제·재생성 말고는 못 연다(명시적 새 대화 요청도 입구가 저장 id 유무만 보므로 같은 실패 — `step-log.md:2442`).

### 6-4. Phase B 만 착지했을 때

B 는 A 의 코드에 기대지 않는다 — 폴백의 새 대화는 오늘의 Fresh 경로(claude = `new_session_id` 가 옛 값을 이력으로 밀고 새 값을 스폰 때 영속 · codex = release)를 탄다. 그래서 0턴 세션은 여전히 id 를 남기고, **그 에이전트를 다시 켤 때마다** 이어받기 헛시도 한 번 + 폴백이 붙는다(결말은 맞다 — 새 대화가 뜬다). 비용 = claude 약 1 s 헛시도 + 거두기·barrier · codex 거절 왕복 + 거두기·barrier, 그리고 매번 Q8 의 화면 순서. 표식(D2)이 없으니 codex 이어받기 깜빡임(P1)은 남는다.

### 6-5. 나눌 자리·조각 사이 계약

- **나눌 자리(파일 겹침 기준):**
  - **코더 A(agent — A1 → A2 순차, 한 사람):** 둘 다 `manager.rs`·`session.rs` 를 만진다. 겹치므로 쪼개지 않는다. A1 의 daemon 한 함수도 여기(컴파일이 이끄는 위임 교체).
  - **코더 B(wire+셸 — A3):** A1 의 reply 하나만 기대므로 A1 뒤에 A2 와 병렬(A2 = agent crate + `bin/saturation_pilot.rs` · A3 = protocol·daemon `connection_core.rs`·src-tauri — 파일이 안 겹친다).
  - **코더 C(프론트 — A4):** 아래 계약만 기대므로 A1~A3 과 병렬 가능(목 데이터로 시험). 착지는 A3 뒤.
  - **Phase B 코더 하나(B0 → B1 → B2 순차):** B2 가 `manager.rs` 를 크게 만진다. A2 와 **같은 함수**(`spawn_agent_watching_link`·`resume_no_fallback`)를 만지므로 A2 와 동시에 돌리지 않는다 — Q5 = (a)라 B2 가 착지한 A2 위에 앉는다.
- **조각 사이 계약(메인이 먼저 못 박는다):**
  - agent: `pub struct Incarnation { pub epoch: u32, pub continues_conversation: bool }` · `pub struct SubscribeReply { pub outcome: SubscribeOutcome, pub incarnation: Incarnation }` · `AgentManager::subscribe_from(&self, AgentId, Arc<dyn OutputSink>, after_seq: Option<u64>, requested_epoch: Option<u32>, on_ready: impl FnOnce(&SubscribeReply)) -> Result<SubscribeReply, PtyError>`.
  - 세션: `pub(crate) fn with_incarnation(self, continues_conversation: bool) -> Self` · `pub(crate) fn with_session_id_latch(self, Arc<SessionIdLatch>) -> Self`. ★표식 인자는 없다★ — 화신 표식은 `AgentSession.epoch` 하나이고, `Incarnation` 은 응답을 만들 때 `Incarnation { epoch: self.epoch, continues_conversation }` 로 조립하는 값 타입이다(§3-2-2).
  - 래치: 위 §3-2-2 서명 · `InputEncoder::submits_turn(&self, &[u8]) -> bool` · `ProfileRegistry::commit_session_id(&self, AgentId, incarnation: u32, expected: Option<Uuid>, new: Uuid) -> bool`(화신 가드 도우미는 `observe_session_id` 와 공유) · `fn spawn_session(…, incarnation: SpawnIncarnation)`(필수 — `SpawnIncarnation { continues_conversation: bool, latch: Arc<SessionIdLatch> }`).
  - ★A2·B2 접점★: `fn spawn_agent_watching_link(&self, profile: &AgentProfile, mode: SpawnMode, reservation: Option<SpawnReservation>)` — ★**B2 가 들인다**★(Q5 = (a) — A2 는 이 서명을 안 바꾼다). `None` = 오늘처럼 여기서 잡는다.
  - wire: `AgentEvent::SubscribeAck { …, #[serde(default)] continues_conversation: bool }`.
  - 마커: flags bit0=truncated · bit1=failed · **bit2(0x04)=continues_conversation** · 길이 30.
  - TS: `replayBoundary` 에 `continuesConversation: boolean` · `onState?: (state: ViewPhase, info?: { continuesConversation: boolean }) => void`(`'live'` 에만 info).
  - reaper: `ReaperCmd::Barrier { id: AgentId, epoch: u32, reply: Sender<bool> }` — `true` = 그 화신이 목록 통지까지 수거됐다.

---

## 7. 열린 질문 (사용자 결정)

- **Q1. [A] claude `/clear` 뒤의 새 id** — ★**결정(사용자 2026-09-24): (a)**★ — 추적기 경로 불변. D1 의 유일한 의도된 예외로 적는다(`/clear` 순간의 새 id 에는 아직 대화가 없다). 사유 = `/clear` 는 옛 대화를 끝내는 사용자의 명시적 행위다. 대가 = `/clear` 하고 입력 없이 끄면 다음 활성화가 실패한다(A 만이면 오늘과 같다 · B 뒤에는 새 대화) — §3-2-6. 선택지는 (a) 오늘처럼 곧바로 영속한다(추적기 경로 불변). `/clear` 후 말없이 끄면 한 번 헛시도한 뒤 D4 가 새 대화로 연다(A 만이면 오늘과 같은 실패) ← **권고**(바뀌는 코드 0, 결말이 맞다 · 래치 commit 이 비교-교체라 추적기 값이 되감기지도 않는다) · (b) 래치로 돌린다: `/clear` 를 보면 저장된 id 를 비우고 새 id 는 다음 제출 때 영속. 헛시도가 없어지는 대신 추적기 콜백을 화신의 래치까지 잇는 배선(데몬 조립점이 manager 를 늦게 묶어야 한다)이 붙는다. — D1 을 「화신이 태어날 때 받은 id」로 읽느냐, 도중에 바뀐 id 까지로 읽느냐의 문제다.
- **Q2. [A] 이어받았는데 이력이 0건으로 온 화신** — ★**결정(사용자 2026-09-24): (a)**★ — 새 「이력 끝」 신호를 만들지 않는다. 사유 = 품이 적고 드문 경우다. 이 결정은 입력창이 살아 있는 배치(§3-5 — Q2 = (a) 선택으로 정해짐(사용자, 2026-09-24 — (가)/(나)를 Q2 와 함께 제시))에 기댄다. 선택지: 로딩 아이콘이 입력할 때까지 남는다(드물다: 이력 조회 예산 초과·실패, claude transcript 판독 실패). (a) 수용 — 입력창은 살아 있다 ← **권고** · (b) 「이력을 다 넣었다」를 알리는 화신별 신호를 다시 연다 — D1+D2 가 대체한 설계(명부 표식·스트림 내 표식)의 재론이다.
- **Q3. [B] 폴백을 누가 어떻게 아나 — 그리고 대화가 있는데 「없다」고 답하는 경우**(옛 Q3 「알림」을 넓혔다) — 세 축이다.
  - **사람:** (a) 로그만(원인 + 버린 id; `old_session_ids` 에도 남는다) · (b) 화면에 한 줄. (2026-09-21 step-log 의 미룬 계획은 「새 세션 + 고지」였다 — D4 문구에는 고지가 없다.)
  - **제어 LLM:** 오늘 `agent.spawn` 의 결과(`AgentSpawnOk` = id·이름·상태·created)는 이어받기와 새 대화를 **가르지 못하고**, D4 는 `last_failure` 를 쓰지 않는다. ADR-0082 가 fresh-fallback 을 버린 사유 중 (2) 원인 은폐 · (3) LLM 이 판단 못 함이 바로 이 축이다. (c) 지금처럼 없음 · (d) 결과에 「새 대화로 열었다」 칸을 더한다(명령 버스 판올림) · (e) 활성화 기록 칸에 정보성 기록을 남긴다 — ADR-0202 개정이 따르고, §3-4 의 「codex 쪽 낡은 기록」 잔여가 함께 풀린다.
  - **오탐 부류:** 대화가 실재하는데 backend 가 「없다」고 답하는 경우 — claude 는 작업 폴더 기준으로 대화를 찾으므로 프로필 cwd 가 바뀌면 없다고 답한다(가능성 높음 · 미실측) · codex 는 프로필 env 로 `CODEX_HOME` 이 바뀌면 같다. D4 는 그 대화를 **조용히 버리고** 새 대화를 연다(옛 id 는 이력에 남아 손으로는 되찾을 수 있다). (f) 수용 · (g) 「id 를 저장한 뒤 cwd·env 가 안 바뀌었을 때만 폴백」 — id 옆에 지문 칸이 필요하다(새 칸 — ADR-0172 가 기각한 모양에 가깝다).
  - **권고: 사람 (a) · LLM (d) · 오탐 (f).** 사유: 잃는 것이 없는 게 보통이라 사람에겐 로그로 충분하지만, LLM 이 메인 조작 주체라(CLAUDE.md 「LLM-우선 제어」) 「이어받았다」고 믿고 이전 맥락을 전제한 지시를 내리지 않으려면 결과에서 갈려야 한다 — (d) 는 칸 하나다. 오탐은 cwd·`CODEX_HOME` 을 바꾸는 흐름이 드물고 id 가 이력에 남는다.
- **Q4. [A·B] (조건부) 시험 seam** — §4-1 의 PATH 수법이 서면 불필요하다(A2 착수 때 파일럿 1건으로 잰다). 안 서면: (a) 런타임 통합 시험 없이 단위 + 구조 시험 + GUI 실측으로 간다 — ★그러면 **D1 의 종단 영속**(실 spawn → 래치 → 명부 → 디스크)과 D4 의 kill·셧다운 재확인이 **GUI 로만** 검증된다★(구조 시험은 배선이 있다는 것까지 재고 실행 순서는 못 잰다) · (b) manager 에 스폰 주입 seam 을 운영 코드에 둔다(ADR-0012 사용자 결정 사항) ← **권고(조건부)** — D1 배선이 망가졌을 때의 결말이 「모든 이어받기가 조용히 사라진다」(§3-2-2)라 GUI 만으로 지키기엔 값이 크다.
- **Q5. [A·B] 단계 나누기** — ★**결정(사용자 2026-09-24): (a)** — Phase A 를 이번 라운드에 착지하고 Phase B 는 다음 라운드★. 선택지는 (a) Phase A 를 한 라운드로 먼저 착지하고 Phase B 는 별도 라운드 ← **권고** · (b) 한 라운드에 둘 다 · (c) B 먼저 였다.
  - 권고 사유: 리뷰 지적 21건 중 11건(구현 7 · 결정 4)이 B 몫이고, 결정 대기(Q3·Q6·Q7·Q8)가 전부 B 에 몰려 있다. A 만으로는 회귀가 없다 — D4 가 흡수할 경우들은 오늘과 같은 결말로 남는다(§6-3). (c) 는 쓰레기 id 에 막힌 프로필을 먼저 풀지만 0턴 세션마다 헛시도와 폴백 화면이 붙는다(§6-4).
  - 묶임: 이 답이 Q3·Q6·Q7·Q8 의 시한을 정한다 — (a) 면 B 라운드 착수 전까지 미룰 수 있고, (b) 면 지금 필요하다.
- **Q6. [B] claude `Diagnosed` 팔에서 거두는 시점** — 그 팔은 자식이 살아 종료 훅(SessionEnd)을 도는 중에 온다(진단 +2.2 s · 종료 +6.4 s — `manager.rs:42-49,135-137`). 오늘 그 팔은 아무것도 죽이지 않고(`:2100-2103`), `tear_down_failed_activation` doc 도 「`Terminal`·`Diagnosed` 갈래에서는 부르지 않는다 — claude 가 종료 훅을 도는 중」이라 적는다(`:2213-2214`). `TerminateJobObject` 는 나무째 끝내므로 훅까지 죽는다. (a) 곧바로 거둔다 — 빠르다(진단 직후 새 대화), 대가 = 사용자 훅이 중간에 끊긴다 · (b) 자연 종료를 기다리고(사건 = pump 종료 · 상한 = join 상한 5 s, ADR-0001) 그래도 살아 있으면 거둔다 — 약 4 s 늦다, 훅은 끝까지 돈다 ← **권고**: 쓰레기 id 에만 닿는 드문 길이라 4 s 가 싸고, kill 은 되돌릴 수 없으며, 오늘 코드의 두 주석과 ADR-0082 「관측된 종점을 기다린다」에 맞는다. 고르지 않은 쪽은 ADR 의 거부한 대안에 적는다.
- **Q7. [B] 죽을 화신이 받은 입력·우편** — 이어받기 화신이 판정 전에 받은 것이 폴백으로 사라진다. codex 는 핸드셰이크 실패 시 큐의 입력을 비우고 경고 로그 한 줄만 남긴다(`transport.rs:1515-1525`). 우편 영수증은 「배달됨」이다 — codex 통로는 착지 확인 수단이 없어 `flush_input` 이 `Unsupported` → 통과로 접히고(`session.rs:315-319`), 그 값이 영수증이 된다(`messaging_host.rs:124-140`). flush 는 이어받기 화신이 명부에 오를 때 불리고(`messaging_host.rs:450-490`) 그 공표는 핸드셰이크 **전**이다(`manager.rs:1447`). claude 는 파이프에 착지한 뒤 자식이 죽는다 — 영수증은 참이지만 턴은 없다. (a) 활성화가 결말을 내기 전(예약 보유 중)인 화신에는 우편을 flush 하지 않고, 결말 뒤 재시도 계기를 만든다(`activation_in_flight` 가 이미 있다 — 단 claude 는 판정 창 동안 예약이 없어 그 판정이 안 선다 → 창까지 예약을 늘려야 한다) · (b) Resume 연결이 서는 중인 화신은 입력을 거절한다(사용자 입력은 오류로 보이고 우편은 재파킹된다) · (c) 폴백이 잃은 건수를 보고만 한다 · (d) 수용(문서화). **권고: 우편 (a) · 사용자 입력 (d).** 사유: 우편은 보낸 LLM 이 배달됐다고 믿는 **조용한** 손실이라 막을 값이 있고, 사용자 입력은 새 화신의 비우기가 에코를 지워 **보이는** 손실이다(다시 치면 된다).
- **Q8. [B] 폴백 중 보이는 순서** — codex 챗 슬롯 = 로딩(거절 왕복만큼 — 미측정, 성공 왕복은 1.4–2.2 s) → 막(옛 화신 수거 — `agentGone`) → 첫 화면(새 화신) · 트리 = 도는 중 → 시체 → 도는 중(reaper 가 수거 뒤 목록을 낸다). (a) 수용 ← **권고**(쓰레기 id 에만 닿는 드문 길 — §4-4 12 로 실제 길이를 잰 뒤 재론) · (b) 매끄러운 교체를 설계한다 — 예: 「폴백 중」 상태를 wire 에 싣고 프론트가 그동안 막·시체 표시를 억제한다(새 wire 칸·새 상태 — LLM 도 같은 상태를 읽어야 한다). (옛 화신의 수거 통지를 새 화신이 설 때까지 미루는 길은 reaper 단일 소비자 순서(ADR-0019)를 건드려 선택지에서 뺐다.)

---

## ADR 초안 반영 메모 (오케스트레이터가 ADR 을 쓸 때 담을 것)

두 ADR 이다(§5). 채번·개정 도장·링크·인덱스 = `/adr`. 개정 도장은 **개정당한 ADR 쪽에도** 박는다(옛 ADR 만 읽는 세션이 죽은 조항을 따라가지 않게).

### A. ADR-A — D1 + D2 + D5 (A5 에서 쓴다)

- **범위:** 세션 id 는 첫 제출 뒤에만 영속한다(화신별 첫 제출 래치 · 비교-교체 동사) · 손잡이 없는 이어받기 요청은 새 대화로 연다(D1 의 귀결 — §3-2-3) · 챗 슬롯은 구독 응답의 「이어받는 화신」 표식으로 첫 화면과 로딩을 가른다 · 공용 로딩 패널.
- **개정(Amends) — 조항까지 적는다:**
  - ADR-0008 결정 첫 항(`0008:10` 「spawn 시 … 발급·persist」) → 발급은 spawn, 영속은 첫 제출. 결정 둘째 항(`/clear` 추적 즉시 persist)은 그대로다(Q1 = (a)) — D1 의 유일한 의도된 예외로 적는다.
  - ADR-0076 결정 첫 항의 「명시적 `resume=true` 는 그대로 존중」(`0076:14`) → 저장된 손잡이가 없으면 Fresh 로 연다. ★작성자의 고름이 아니라 사용자 결정 D1(「저장된 id 없음 ⟺ 이어받을 대화 없음 → Fresh」)의 귀결이다★ — 오늘 codex 우회가 같은 판단을 「사용자 결정」으로 적어 두었다(`manager.rs:1989-1999`). 보이는 결말이 바뀐다: 손잡이 없는 claude 의 `resume:true` 가 오늘은 `Failed` + `NoConversationToResume`, 이후에는 `Started`(§3-2-3). 「세션 존재 → Resume」 모드 유도는 그대로다.
  - ADR-0076 결정 둘째 항(Fresh = `new_session_id` 로 새 uuid 발급·영속)과 「영향」의 「`ensure_session_id` = Resume 전용」 → Fresh 는 메모리에서 뽑고(`mint_session_id`) 칸은 release 로 비운다 · 운영 경로에서 `ensure_session_id` 를 걷는다. 「Fresh 는 항상 새 sid」·「발급 단일점」은 그대로다.
  - ADR-0082 결정 첫 항(`0082:16` 「이어받을 세션이 … 시스템이 임의로 새 대화(fresh)를 만들지 않는다」) → **「이어받을 손잡이가 없는 이어받기 요청」에 한해서** 그 요청을 같은 ADR 결정 여섯째 항의 「이어받을 게 없는 새 프로필 … 정상 생성」(`0082:21`)으로 읽는다(위 ADR-0076 항목과 같은 D1 의 귀결). 손잡이가 **있는** 이어받기의 실패는 그대로다(그 부류의 개정은 ADR-B). 「영향」의 「`new_session_id` 로 새 sid 발급하는 명시적 신규 생성은 유효」(`0082:37`)는 동사 이름만 바뀐다(`mint_session_id` + release — 뜻은 그대로).
  - ADR-0185 결정 2(codex 수령 지점 = `observe_session_id`) → 수령은 래치 offer, 기록은 `commit_session_id`. 같은 개정으로 「영향」의 「수령 단일점이 `observe_session_id`다」(`0185:36`)도 낡는다. 「영향」의 Phase 2 요구사항(첫 턴 전 persist)은 이제 codex 터미널 밖에서 구조로 선다.
  - ADR-0216 결정 7 · ADR-0217 결정 7 · ADR-0218 결정 6(값 정책 = 덮어쓰기) → 화신 자기 commit 기준의 비교-교체(아래 문단).
  - ADR-0217 결정 6(ADR-0218 결정 6 이 승계)은 **개정이 아니라 준수**다 — 새 동사가 `observe_session_id` 의 화신 가드 도우미를 함께 쓴다(「우회로를 새로 만들지 않는다」 — §3-2-3).
  - ADR-0145 결정 1(`0145:14` 「빈 상태 판정 = 복원 완료 신호 + 0건」) → 규칙이 바뀐다: `'live'` + 0건 + **이어받는 화신이 아님** → 빈 상태 · 이어받는 화신이면 행을 그리는 첫 항목(또는 입력·화신 종료)까지 로딩 패널(§3-5 의 두 식). 근거만이 아니라 결정 본문이 개정 대상이다.
  - ADR-0145 근거(`0145:37` 「프로토콜·백엔드 변경이 필요 없다」) → ADR-0204 가 말없이 깬 그 전제를 wire 칸 하나(`SubscribeAck.continues_conversation`)로 복구한다.
- **관련(링크):** ADR-0083(시체 보존 — 입력 없이 끈 시체가 이제 다음 활성화에서 새 대화로 열린다) · ADR-0172(사전 판정 기각을 재론하지 않는 이유 — §3-2-7) · ADR-0204(ADR-0145 의 전제를 깬 쪽 — §1).
- ★**비교-교체가 ADR-0216 이 기각한 「빈 칸에만 쓴다」(`0216:61`)가 아닌 이유**★: 기각된 규칙은 「칸이 비었을 때만 쓰고 값이 있으면 거절」이었다. 그 고장은 새 id 가 나오는 분기에서 손잡이가 낡아 두 번째 재개부터 **조용히** 깨지는 것이었다. 비교-교체의 `expected` 는 빈 칸이 아니라 **이 화신이 시작 때 본 값**이다 — release 뒤 본 값(Fresh = `None` · Resume = 저장값). 그래서 이어받기 화신의 **첫 offer** 가 저장값과 다른 id 를 내면(재개가 다른 id 를 돌려주는 경우) 칸 == `expected` 라 **교체된다** — 0216 이 지키려던 분기가 그대로 착지한다. ★이 논증은 첫 offer 위에 선다★ — offer 는 화신당 한 번이고 포트도 화신당 많아야 한 번 불린다(§3-2-1). 「commit 이 성공하면 `expected` 가 그 값으로 따라온다」는 절은 방어용이라 이 논증을 떠받치지 않는다. 거절하는 것은 시작 때 본 뒤·유일한 commit 전에 **다른 기록자**가 칸을 바꾼 경우뿐이다(claude 추적기의 표류·`/clear`) — 포트가 화신당 많아야 한 번이라 그보다 앞선 commit 이 없고, commit 뒤의 추적기 쓰기에는 래치가 반응하지 않는다(그 값이 그대로 선다). 거기서 덮어쓰기를 하면 추적기 값을 스폰 때 값으로 **되감는다**(§3-2-1). codex 에는 추적기가 안 붙으므로(`assigns_sid` 게이트) 오늘의 codex 동작은 바뀌지 않는다.
- **codex 터미널 예외:** 「첫 턴 전 영속」이 그 칸에서만 성립하지 않는다 — id 가 락 회수로만 오고, 파킹 우편이 있으면 첫 턴이 언제나 먼저다(§3-2-4). ADR-0218 대비 회귀는 아니다.
- **commit 실패 규율:** 「포트는 화신당 많아야 한 번 불린다. 한 번 부르기 시작했으면(성공·거절·패닉 무엇으로 끝나든) 다시 부르지 않는다」 · 거절이면 턴은 나간다 · 패닉은 릴리스에서 데몬 종료(`panic = "abort"`)라 래치에 `catch_unwind` 를 두지 않는다 · 사용자 종료(데몬 셧다운 포함) 중의 제출은 세지도 보내지도 않는다 — 큐가 닫힌 뒤와 같은 `Err` 를 앞당긴다(§3-2-2).
- **거부한 대안** — 출처를 항목마다 적는다(CLAUDE.md 「결정 날조 금지」): 「사용자 결정」 = 사용자가 고르거나 버린 것 · 「리뷰 지적·고름」 = 리뷰 지적을 받아 또는 D3·내부 구현 위임 안에서 작성자가 고른 것.
  - commit 에 덮어쓰기 동사(`observe_session_id`)를 쓴다 — 추적기 값을 되감는다(§3-2-1). 「리뷰 지적·고름」
  - id 생성 broadcast(D2) — replay·`ReplayComplete` 와 같은 순서 스트림을 타지 않는다(§2 D2). 「사용자 결정」(D2)
  - 화신별 「이력 끝」 신호 — 명부 표식·스트림 내 표식(D1+D2 가 대체한 설계 — §7 Q2) · Q2 (b) 로 그것을 다시 여는 것. 「사용자 결정」(D1·D2 · Q2 = (a))
  - Q1 (b) `/clear` 를 래치로 돌린다 — 사용자가 (a) 를 골랐다(§7). 「사용자 결정」
  - 슬롯 전체를 덮는 로딩 — 「대화 영역 가운데 + 입력창 활성」 배치가 이긴 대안이다. 이 TRD 가 적은 귀결: 슬롯 전체를 덮으면 이력 0건 화신에서 빠져나갈 길(입력)이 없고, Q2 = (a) 가 그 배치에 기댄다(§3-5). 「사용자 결정」(Q2 = (a) 선택으로 정해짐(사용자, 2026-09-24 — (가)/(나)를 Q2 와 함께 제시). (가) = 가운데 아이콘 + 입력창 활성 · (나) = 슬롯 전체 덮기)
  - `writer_loop` `turn/start` 방아쇠(D3) — codex JSON 한 칸만 덮는다(§3-2-5). 「리뷰 지적·고름」(D3 위임 안의 고름 — 오케스트레이터 원안을 작성자가 기각)
  - 세션에 두 번째 화신 표식 칸(`Incarnation` 을 세션 칸으로) — 소유권 분할 위반(§3-2-2). 「리뷰 지적·고름」
  - 래치에 `catch_unwind` — 릴리스에서 죽은 코드다(§3-2-2). 「리뷰 지적·고름」
- **사용자 결정 기록:** D1·D2·D3·D5(2026-09-23/24) · Q1 = (a) · Q2 = (a) · 로딩 배치 = 대화 영역 가운데 + 입력창 활성(Q2 = (a) 선택으로 정해짐(사용자, 2026-09-24 — (가)/(나)를 Q2 와 함께 제시)) · Q5 = (a) — 뒤 넷은 2026-09-24.

### B. ADR-B — D4 (B3 에서 쓴다)

- **범위:** D4 — 「이어받을 대화가 없다」 부류의 이어받기 실패는 같은 활성화 안에서 새 대화로 연다. ADR-0082 를 **이 부류에 한해** 개정한다(다른 실패는 ADR-0082 그대로). ADR-A 를 링크한다 — D1(영속 시점) 없이는 D4 의 「잃는 것이 없다」 근거가 약하다.
- **링크(개정·관련):**
  - ADR-0077 — 같은 사용자 결정(「스폰만 하고 한마디도 안 한 건 새로 시작」)을 fresh-fallback 으로 구현한 선례(폐기됨).
  - `a4aac1a` — 그 확장이 「already running」 `Err` 를 이어받기 실패로 오인해 산 에이전트를 죽였다 → ADR-0082 의 직접 원인.
  - ADR-0082 — 개정 대상. 회귀 가드 ①(산 에이전트 재활성화 — kill·재spawn 없음·표식 불변)은 그대로 강제된다. 회귀 가드 ②(스폰 정확히 1회)는 「`NoConversationToResume` 부류 제외」로 좁혀진다.
  - ADR-0202 — release 는 이력 밀기라 파괴가 아니다 · 성공이 기록을 안 지우므로 codex 쪽 이전 기록이 남는다(§3-4 — Q3 결과에 따라 개정 여부).
  - ADR-0019 — 결정 2(런타임 자동 재시작 없음)와 겹치지 않는 이유 · reaper 단일 소비자 위에 선 `Barrier`(수거 여부를 reaper 스레드가 답한다).
  - ADR-0172 — 사전 판정 기각을 재론하지 않는 이유(D1 은 새 칸이 없고, 판정은 여전히 시도한 자리) · 잰 문구만 분류에 더한다.
  - ADR-A(영속 시점 — 위 A) · ADR-0201(claude 실패 문구 시각) · ADR-0001(barrier 상한의 출처).
- **가드 논증(a4aac1a 부류를 구조로 피하는 이유):** 이 호출의 `Started` 뒤 판정 팔 셋에서만 · `Moot`·spawn `Err` 에는 없음 · 분류는 이 화신 자신의 증거 · 화신 가드 거두기 · 재활성화 가드가 앞 · 재진입 없음(스폰 최대 2회) · 예약을 폴백 끝까지 쥔다 · 판정 뒤 사용자 종료·셧다운 재확인.
- **D4 도달성:** B0 측정 결과를 적는다(codex 터미널 문구 · claude 터미널의 종료 시각도 함께).
- **거부한 대안:**
  - **파일로 판정한다**(rollout 파일 존재 — `step-log.md:2458` 의 미룬 계획) — 사전 판정(ADR-0172) · 백엔드 내부 저장소 결합(ADR-0077·0082 가 거부 · ADR-0004) · claude 짝은 ADR-0008 위반 · codex 오류문이 같은 파일을 보므로 반응형으로 충분. 단 codex 터미널 문구가 측정으로 안 나오면 그 칸에 한해 재론(사용자 결정).
  - 명부 폴링 대기 · 밖에서 명부 확인 · 예약을 놓고 기다리기 · `LINK_RESOLUTION_BACKSTOP` 재사용 · `RestoreOutcome::FreshFallback` 어휘.
  - Q6 에서 고르지 않은 쪽(곧바로 거둔다 / 자연 종료를 기다린다) — U2 요구대로 ADR 에 적는다.
- **B 착수 전 반영:** §3-4 끝 「B 라운드 착수 전 반영할 리뷰 지적」 여섯의 처분을 ADR 본문(결정 또는 알려진 잔여)에 싣는다.
- **사용자 결정 기록:** D4(2026-09-23/24) · Q3·Q6·Q7·Q8 의 답(나오는 대로).
- **근거 수치:** claude JSON 진단 +2.2 s / 종료 +6.4 s · claude 실패 문구 약 1 s(ADR-0201) · codex 거절 즉시 · 최악 블로킹 codex ≈ 45 s / claude ≈ 13 s · B0 측정값.
