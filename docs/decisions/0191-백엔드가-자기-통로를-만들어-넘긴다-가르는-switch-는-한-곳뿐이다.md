# ADR-0191: The backend builds its own channel and hands it over — there is only one switch that splits them

- 상태: 확정 (2026-09-10, basis: user decision + two-person adversarial review findings + existing precedent)
- 관련: Amends ADR-0189 (it decided where the channel lives but did not decide the creation path) · ADR-0004 (backend knowledge isolation) · ADR-0044/0030 (channel shape · caps injection) · ADR-0012 (module isolation harness) · `docs/process/S21-codex-backend/trd-phase2a.md` §5·§8 · step-log S21

## 맥락

ADR-0189 decided to place the codex app-server channel as an `AgentTransport` implementation inside `crates/engram-dashboard-agent/src/backend/codex/`. **But it did not decide who builds it.**

- The place that builds the channel today is `select_transport` at `crates/engram-dashboard-agent/src/manager.rs:79-99`, and with a two-branch match on `TransportShape` **the assembly point itself** builds `StdioTransport`/`PtyTransport`.
- ★**The comment on that function pins it down itself**★ — `manager.rs:75-76`: 「이 함수는 어느 backend 도 이름으로 모른다: 모양은 `backend::transport_shape` 가 신고한 것을 그대로 받는다(ADR-0004)」. Today the string `codex` appears **0 times** in that file.
- So adding a third branch here **breaks that invariant on the spot** — the branch would call a type in `backend/codex/` by name.

Each of the two adversarial reviewers pointed at this spot (blind = BLOCK 「교체성 추상화가 장식적이다」 · doc-aware = FIX 「격리 누수」).

## 결정

**The single switch that splits backends decides the channel too and injects it all at once.**

- **That switch already exists and is only one** — `backend_for(c)` (`crates/engram-dashboard-agent/src/backend/mod.rs:282`).
- **The real code that builds the channel lives inside each backend folder.** The assembly point only receives what was built and plugs it into the session, and ★**does not know its actual type**★.
- **Do not go through the same switch again for each aspect.** Today the assembly point asks separately for the shape, the encoder, the cleaner, the turn decider and the mail eligibility, going through the same switch several times (`manager.rs:1037-1043`). Fold this into one, and the channel is carried in that bundle. ★User decision — 「이왕 하나 있는 거 알뜰히 이용한다」★
- **Leave the values and the order of the claude path as they are** — zero behavior change is the acceptance condition of this change.

## 거부한 대안

- **Add a third branch to the central `TransportShape` match** — the change is smallest but **the splitting place divides into two** (`backend_for` + `select_transport`), and the assembly point comes to know a backend type by name, breaking the comment above and ADR-0004 on the spot. Every fourth backend would require fixing two central places. ★On top of that, codex has **two** channels★ — the Phase 1 PTY conversation screen stays alive and app-server is added (ADR-0189). The premise 「백엔드당 모양 값 하나」 (one shape value per backend) breaks right there, and the declaration-table tripwire (`backend/mod.rs:779`) takes the form of **one tuple per `AgentCommand::Codex` variant**, so a second mode never goes through that check at all (doc-aware review F4).
- **Add one more per-aspect delivery function** (adding `open_transport` exactly in the `output_decoder` pattern) — the direction is right and isolation is kept, but it **leaves today's shape of going through the same switch again.** Rejected by the user's decision — we fold it toward deciding and injecting once.

## 근거

- ★**The precedent already exists in this repo**★ — `pub fn output_decoder(c) { backend_for(c).output_decoder(c) }` at `backend/mod.rs:477`. Its doc says: 「판정도 decoder 실물도 `AgentBackend::output_decoder` 가 소유하고 이 함수는 dispatch 뿐이다 … **새 backend 는 자기 폴더에서 그 메서드를 구현하면 되고 이 함수는 손대지 않는다**(교체성)」. The same form exists in `resume_transcript_events` too (`backend/mod.rs:489`).
- CLAUDE.md 「백엔드 확장」 — 「에이전트 백엔드 전용 코드는 없앨 수 없고 **한 곳에 모을 수 있을 뿐이다 — 그 한 곳이 `backend`**」.

## 영향 / 불변식

- **The 「조립점 주입」 (assembly-point injection) wording of ADR-0189 is satisfied by this form** — the injection still happens, and only **the party that makes the injected value** moves to the backend.
- ★**It is still forbidden for a channel implementation to hard-code `output.structured`**★ — `manager.rs:70-73` · `transport/stdio.rs:47-50` (ADR-0044/0030). This ADR **does not remove that value but moves the place that makes it**. (This decision closes what doc-aware review F5 pointed out, that 「caps 를 누가 채우나가 네 갈래로 적혀 있다」 (who fills caps is written in four ways).)
- **The caps ownership split (ADR-0030) is kept** — the division in which the transport reports its own and the backend reports its own does not change.
- **Zero claude behavior change** — the existing tripwire (`backend/mod.rs:779`) and the existing tests are the net.
- **Trigger to reopen this decision** — if another tool comes to use the same family of protocol and the channel implementation is no longer tied to a particular tool (the same event as the reopen trigger of ADR-0189), that implementation moves up to `transport/`, and then how this injection path receives it is reopened.
