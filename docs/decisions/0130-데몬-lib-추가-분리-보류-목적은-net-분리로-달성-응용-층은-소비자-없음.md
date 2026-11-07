# ADR-0130: Further splitting of the daemon lib put on hold — the purpose is met by the net split, and the application layer has no consumer

- 상태: 확정 (2026-08-05, basis: user decision + measurement of the whole internal dependency graph of the daemon crate) · 부분 폐기 by ADR-0151 (criterion for deciding a split) · 부분 폐기 by ADR-0236 (resume condition 2 and the reopen trigger)
- 관련: Amends ADR-0129 (decisions 2 and 3 put on hold) · ADR-0020(boundary-erosion history — this decision narrows the range where that lesson applies) · ADR-0110(compiler-enforced wall idiom) · ADR-0029(agent host = daemon process) · step-log S18.24 · Amended by ADR-0151 (criterion for deciding a split) · Amended by ADR-0236 (resume condition 2 and the reopen trigger)

## 맥락

ADR-0129 decided to split the daemon lib into three layers (network lib · agent-system lib · thin assembly binary). **Decision 1 (the network lib) has been carried out** — `engram-dashboard-net` became independent with zero agent vocabulary, and the isolation gate in that crate's lib.rs header enforces this.

Before starting decisions 2 and 3 (extracting the application-layer lib + making the assembly thin), the user asked about the scope again: *"The net split I asked for is done. Why refactor the rest?"* To answer this, the whole internal dependency graph of the daemon crate was measured, and the result is the basis below. **The user's conclusion: "going that far is overkill"** — the daemon crate is not split any further.

## 결정

**Decisions 2 and 3 of ADR-0129 are put on hold.** `engram-dashboard-daemon` **keeps the application layer (connection core · connection adapter · status fan-out · messaging host · control plane) and the assembly in one crate**.

- Decision 1 and its isolation gate are **still valid as they are** — this ADR does not touch decision 1.
- The hold is **not a discard** — if any one of the resume conditions in §영향 below is observed, the target shape of ADR-0129 is taken out again. ★A note on wording★: the `부분 폐기 by ADR-0130` stamped on the ADR-0129 status line is the **standard stamp phrase** of the `/adr` script, and the substance of this relation is a hold. Do not read that stamp as "decisions 2 and 3 are dead".

## 거부한 대안

- **Extract only the control plane into a separate crate (option A)** — by measurement the cheapest. `control/` is a leaf with 0 edges going out to other daemon modules, so it comes off **without a cycle** (on the incoming-edge side the import paths have to be fixed — they come from three modules), and it does not touch `net`, so it sidesteps the `frame_port` feature problem. **Why it was dropped:** being cheap is not a reason to do it — there is no consumer that would use the control plane alone (§근거), and it differs from the shape ADR-0129 decision 2 drew (the whole application as one lib), so it adds one more decision. What it gets is only a preventive effect, and the preventive argument is weak for this chunk (§근거).
- **Complete ADR-0129 (option B)** — once the production cycles were refuted, the main risk of this option disappeared (it is a move, not a design change). **Why it was dropped:** the risk went down, but a need did not appear. The absence of a consumer stays as it was, and two decisions come attached — whether to move `frame_port` out of the `server` feature (a public API change, so it gets more expensive the longer it is postponed), and the proper way to keep the assembly part from holding both the registry and the fan-out.
- **Just stop with no record** — a later session that reads only ADR-0129 sees "decisions 2 and 3 not carried out" and runs the same investigation again from scratch. This ADR itself is the nail that stops that reopening.

## 근거

**① The original purpose was met and measured.** The objection in ADR-0129 was *"the daemon should be a relay and be reusable in other systems, but it became the whole backend"*. After the `net` split, the normal dependency closure of `discovery` shrank from 122 to 110 crates and the async-runtime family dropped to 0 lines (to re-check = the gate command in the comment of `crates/engram-dashboard-discovery/Cargo.toml`).

**② The other two chunks have no consumer.** The `net` split had real consumers (`discovery` and `src-tauri` needed only the network but were carrying everything). There is currently no consumer that would use the control plane or the relay layer separately, and none is planned.

**③ The boundary already stands without a wall (measured 2026-08-05).** The 2 module cycles that had been recorded are **refuted in the production build** — both come from fixtures inside `#[cfg(test)] mod tests` reaching out to a neighboring module, and the dependency graph of the default build is a one-way DAG. In particular `control/` has **0** edges going out to other top-level daemon modules.

The re-check command — **the result and the command are treated differently.** The *result* ("0 lines", 2026-08-05) is the basis of the decision, so it is right for it to be frozen with its date. But the *command* is a tool, so **whether it is right or wrong has nothing to do with the date** — if it has a miss, it is a defect no matter when it was written, and it is fixed (in fact the 2026-08-05 review caught false negatives caused by restricting the search to `crate::` and replaced the command below. It is not the original text from when the decision was made). The live copy is 「ADR-0130 재론 트리거」 of the `/qa` binding, and **the command lines of the two copies (regex · flags · paths) must be the same** — if they diverge, one of the two is wrong, so align them. The rule applies only to the command line (the explanatory comments have different readers and need not be the same):
```bash
# control/ 이 control 밖 데몬 모듈을 참조하는가 (기대: 0줄)
#   ★경로 접두를 crate:: 만으로 좁히지 말 것★ — control/mod.rs 에서 super:: 는 크레이트 루트라
#   `use super::connection_core::X` 가 같은 간선인데 빠져나간다(그 디렉토리가 실제로 super:: 형태를
#   쓴다). 좁은 판이 2026-08-05 리뷰에서 적출됐다 — 트리거는 과탐이 낫지 미탐은 조용해서 위험하다.
#   ★-U 를 빼지 말 것★ — rustfmt 가 폭 넘는 import 를 `use crate::{⏎ connection_core::A,⏎};` 로 쪼개면
#   접두와 모듈명이 다른 줄에 앉아 단일행 패턴은 못 문다(이 repo 엔 rustfmt.toml 이 없어 기본 래핑이 돈다).
#   매치가 나오면 그 줄이 그 파일의 `#[cfg(test)]` 시작 줄보다 앞인지 볼 것 — 뒤면 테스트 픽스처라 합법.
#   ★형제 모듈 이름은 lib.rs 의 모듈 선언에서 **파생한다** — 손으로 박지 마라★: 박아 둔 명단은 새 모듈이
#   생겨도 안 늘어 그 간선이 아예 안 보인다(실측 2026-08-19 — 박힌 판이 command_delivery·command_roster
#   두 간선을 놓친 채 0줄을 냈다). 명령은 도구라 이 미탐은 날짜와 무관한 결함이고, 발견 즉시 고친다.
#   양성 대조의 한계: 경로를 src/ 로 넓히면 실간선이 잡히지만 그 간선은 **전부 crate:: 형태**라
#   super:: 갈래가 죽어도 이 대조는 통과한다. 접두를 좁히는 개정을 이 대조로 승인하지 말 것.
rg -U -n "(crate|(super::)+)::[^;]*\b($(rg -o '^(pub )?mod ([a-z_0-9]+);' -r '$2' crates/engram-dashboard-daemon/src/lib.rs | rg -v '^control$' | paste -sd'|'))\b" crates/engram-dashboard-daemon/src/control/
# connection_core 가 production 에서 옆 모듈을 참조하는가
#   (매치가 전부 그 파일의 `#[cfg(test)] mod tests` 시작 줄 이후면 production 순환 없음 — 줄 번호를 여기 박지 말 것)
#   접두·-U 주의사항은 위 명령과 동일하다 — connection_core.rs 도 크레이트 루트의 직계라 super:: 가 같은 간선이다.
rg -U -n "(crate|(super::)+)::[^;]*\b(status_fanout|messaging_host)\b" crates/engram-dashboard-daemon/src/connection_core.rs
```

★**Amendment marker (ADR-0236)**★ — the live copy (the binding trigger) has been withdrawn. This command remains only as a tool for measuring the edges when the control-plane split is reopened (cautions = ADR-0236 「영향」).

**④ The erosion argument is weak for these two chunks.** The reason ADR-0129 chose a crate wall was the erosion history of ADR-0020 — the connection core had been declared a carrier-neutral module, yet the control plane and messaging flowed into the same crate. **The cause of that erosion was that they were on the same axis** (while handling the connection, work to be done on top of the connection piled up next to it). The control plane is a **separate entrance** — it takes LLM requests from inside the agents through its own HTTP listener, not through the webview's WS (the `TcpListener::bind` point in `crates/engram-dashboard-daemon/src/control/mcp_server.rs`). It is hard for it to take the same erosion path.

**⑤ The size was an illusion.** The "about 8,000 lines" in the body of ADR-0129 is the total including tests. On the default build, the production code of the daemon crate is smaller than that, and nearly half of it is the control plane, which is a leaf (measured 2026-08-05 — the number is a point-in-time value, so it is not fixed here. To re-measure = split by the line where each file's `#[cfg(test)] mod tests` starts).

## 영향 / 불변식

- **Decision 1 of ADR-0129 is alive.** The isolation gate of `engram-dashboard-net` (source of truth = that crate's `src/lib.rs` header) is kept and run as before. This ADR does not relax that gate.
- **The mismatch between the `daemon` crate's name and its contents is recorded here.** The **implementation** of the daemon upkeep (the single-instance guard · portfile) was taken over by `net` through decision 1 — **but the order in which they are called is still owned by this crate** (`run()` holds the order guard acquisition → data_dir → stale check → bind → …. The actual call sites = `instance::acquire` · `portfile::read`/`is_stale`/`write_atomic` in `daemon/src/lib.rs`). What this crate holds now is the application layer + assembly + that startup order, and the daemon's entry point as a process is `main.rs`. **It is not renamed** — the binary name is accurate, and renaming the crate would mean fixing, in the same change, the points where `src-tauri`'s daemon-client tests call this crate name directly. Instead the crate header states this boundary in one line.
- **★Resume conditions (if any one is observed, take out the target shape of ADR-0129 again)★**
  1. A **consumer that would use the control plane or the relay layer separately actually appears** (not an assumption — a consumer that has been started).
     - ★**Amendment marker — ADR-0151 discarded this decision criterion**★ The new criterion is 「**독립적으로 쓸 수 있는가**」 (can it be used independently), and it **does not ask whether there is an actual consumer now** (ADR-0151 decision 1 · size is not a criterion either — decision 3). So do not read the wording of this clause as 「소비자가 생겼나」 (did a consumer appear) and apply it that way. **The hold conclusion itself is alive, but what holds it in place and the conditions for reopening are in the ADR-0151 impact section, which is the source of truth.**
  2. A **production edge appears going out from `control/` to another top-level daemon module** (= the leaf property collapses — the `/qa` binding's 「ADR-0130 재론 트리거」 loses its 0 lines. The same command is in §근거 ③, but the current decision uses that trigger).
     - ★**Amendment marker — ADR-0236 withdrew this condition and its trigger**★ — `control/` has met this condition since S20 (2026-08-19~20), and the user set down the leaf property without reopening the question (2026-09-27). The `/qa` binding no longer has a reopen trigger.
  3. A **cycle appears in the production dependency graph** (outside test blocks). ★Only this condition has no single command★ — the grep of condition 2 looks only at the outgoing edges of `control/`, so it does not fire on a cycle that arises elsewhere (e.g. `status_fanout → messaging_host`). How to measure = take each module's references inside the crate (`crate::` **and `super::` both** — in a top-level module the two point to the same place. Because of group imports, `rg -U` is required), **compare them with the line where that file's `#[cfg(test)]` starts** to keep only production edges, and then check the direction (the 2026-08-05 measurement got the DAG below this way: `agent_conn → {connection_core, control}` · `connection_core → control` · `status_fanout → connection_core` · `messaging_host → {status_fanout, control}` · `control → 없음`).
     - ★**Amendment marker — observed (ADR-0236)**★ — the `control/` ⇄ `connection_core` cycle has existed since 2026-08-20. ADR-0236 decision 2 covered that bundle by deciding not to reopen the question, and the condition itself is alive.
  - **Registration status (2026-08-05 — this line is the source of truth):** **Only ②** is registered in the `/qa` binding (`.claude/skill-bindings/qa.md`) as a **reopen trigger** — it is **not** an isolation gate that means *FAIL if broken*, it is a notice to "stop and reopen this ADR", so the decision rule is the opposite (do not treat a match as a regression and revert it). **① is not a target for mechanization** (a consumer appearing is a human judgment), and **③ is still unregistered — it does not fire automatically.** Do not read the 0 lines of the ② trigger as meaning ③ is clean too.
    - ★**Amendment marker (ADR-0236)**★ — the trigger for ② was withdrawn from the `/qa` binding on 2026-09-27. No reopen trigger is registered now.
- **A test-code sidestep is not a defect.** A `#[cfg(test)]` fixture using a sibling module is legal inside one crate, and under this decision there is no reason to fix it. But it is recorded that **when resuming, that code will cross a crate boundary, so it becomes a target for relocation** — read this paragraph first when resuming.
- **The decision on which feature `frame_port` belongs to is also put on hold.** This is a matter the ADR-0129 note left as "re-examine before slice 2", and since slice 2 is on hold, that deadline is pushed back with it. When resuming, decide it **before starting** (it is a public API change, so doing it later is expensive).
