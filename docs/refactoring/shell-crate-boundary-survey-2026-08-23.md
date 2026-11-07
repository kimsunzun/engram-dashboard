# Measured crate-split boundary of the shell (`src-tauri`) — 2026-08-23

- **Nature:** preliminary survey (measured). **Not a decision** — whether to split and in what shape is up to the user (undecided as of 2026-08-23: the user is reviewing the structure).
- **Source question:** "백엔드는 분리했는데 왜 셸은 안 됐나. 쪼갤 수 있으면 쪼개고 아니면 합치면 될 듯" (user, 2026-08-23) (the backend was split, so why not the shell? split it if it can be split, otherwise merge it).
- **Related:** `../tracking.md` T-27 (this survey is the material for its 「미결 ①」) · ADR-0151 (split decision criteria) · ADR-0035 (layout authority lives in the shell) · ADR-0012 (module isolation) · `../research/crate-granularity-survey-2026-08-17.md` (background survey for ADR-0151).

## Conclusion

**The boundary already stands.** This survey did not set it up; someone had already done the work of stripping out the Tauri dependency and left **only the package unmoved**.

- Of **10,637 lines** of real shell code, **8,286 lines (78%) have zero Tauri imports**.
- Those 8,286 lines are a **straight line with no cycles**, and there is **not a single place** that references the adapter layer backwards.
- What is actually tied to Tauri is only **2,351 lines** (click handlers · tray · boot wiring).

**So the answer to 「쪼갤 수 있나」 (can it be split) is already out, and what is left is 「어떤 모양으로」 (in what shape).**

## ★The motive changed (동기가 바뀌었다) — the existing tests are not the grounds★

**User decision 2026-08-23: 기존 테스트 파일에 미련을 갖지 않는다. 분리 가능한 단위에 초점을 맞춘다.** (Do not cling to the existing test files. Focus on the units that can be split.)

I state this explicitly because it is where this part departs from T-27's framing. T-27 names "written but nobody runs them" as the problem — there are **209** tests inside the shell source and none of them execute (the measurement is in the appendix below). In that frame, 「그 209개를 살린다」 (revive those 209) becomes the motive for the split, and a cheap alternative (adding one compile-only step line to CI) comes up as a competing option.

> ★**Factual correction (2026-08-24) — the factual premise of this section has collapsed. But the user decision above still stands.**★
> 「전부 실행되지 않는다」 (none of them execute) is no longer true. With `[lib] test = false` + `[[test]] name = "lib_unit"` (`path = "src/lib.rs"`) + `cargo:rustc-link-arg-tests=` in `build.rs`, the Windows manifest is now linked into the test exe too, and that unit suite **runs** with `cargo test -p engram-dashboard --test lib_unit` (decision · rejected alternatives = ADR-0174). ~~Out of 217~~ (★writing the count at all was a mistake — it went stale in a day★) most are green, and the remaining failures are **all on one axis, `daemon_client::tests::`** — the canonical source for the numbers and the CI registration status is the `lib_unit` line of CLAUDE.md 「빌드·검증 명령」, and it is not copied here. Those failures are not a product defect but pre-existing harness rot (the `app: None` short-circuit in `daemon_client::start_connection` — since commit `ffcd766`. The production path is `app: Some`, so it is unaffected). Because of those failures, **the whole suite** is not yet registered in CI.
> That is, **the condition of 「살릴 그물이 없다」 (there is no net to revive) has changed** — the net now runs locally and is mostly green. ~~**That said, this document does not overturn the decision above**: whether to throw it away is the user's decision, and here we only reveal **the fact that the decision now stands on a collapsed premise** (the same tension exists in `docs/tracking.md` T-27).~~ → ★**Attribution correction (2026-08-24) — the block below**★: what collapsed is not **the premise of the user's decision** but **the words in which this document transcribed that decision**. This document itself created the tension of 「버릴지 말지」 (whether to throw away or not).

> ★**Attribution correction (2026-08-24) — the one quoted line above is correct as it is. What is wrong is the words in which this document wrote it again.**★
> What the user actually said is **all in the one bold quoted line at the head of this section** (「사용자 결정 2026-08-23: …」 above). It is a **priority statement** — "do not put the split motive on the existing test files". But this document (and `../tracking.md` T-27) rewrote it as a **disposal decision, 「209개를 버린다」 (throw the 209 away)**, and used that stronger version as the **premise** of the later conclusions. ★**The user explicitly denied ever saying that (2026-08-24, verbatim: "내가 언제 버린다고 했어?")**★ — that is, 「폐기·처분」 (discard · disposal) is **the session's paraphrase**, not the user's decision.
> ★**A priority statement is not a disposal order**★ — "do not put the motive there" and "throw it away" are different statements, and nobody has ever decided the latter. **What to do with the 209 is still undecided** (that one line does not settle it). ★We do not decide it here on anyone's behalf★.
> **On top of that, the facts went the opposite way in the meantime:** that suite now runs (ADR-0174), and **work to fix the remaining failures is under way with the user's approval** (in the shape of moving the connection task's emission behind a port to remove the `app: None` short-circuit — the canonical source for the gap and the decision authority is `../tracking.md` T-27). That is, the framing of 「버려지는 테스트」 (tests being thrown away) stands on **neither the premise nor the facts**.
> **Every place below that is written as 「버리므로」 or 「폐기됐다」 came from this paraphrase** — each place got a correction attached, and nothing was deleted (what it believed when it reached that conclusion is the value of the record).

> ★**Follow-up factual correction (2026-08-24, later than the two blocks above) — everything those blocks wrote as 「아직」 (not yet) has closed.**★
> The remaining failures have **all been fixed.** So two things moved together: ① CI runs that unit suite **as a whole** (the name filter that picked only the export tests to avoid the red ones is gone) ② the workspace regression also dropped `--exclude engram-dashboard` and **runs this package too**. That is, shell logic is now inside the automatic regression net both locally and in CI.
> ★**So every place below that reads as 「알려진 실패가 남아 있다」, 「CI 미등재」 or 「수리가 진행 중이다」 is a record from before this point**★ — we leave them without deleting (what this survey saw when it judged is the value of the record). ★No count is written here — canonical source = the `lib_unit` line of CLAUDE.md 「빌드·검증 명령」★. The remaining gap and the decision authority are in `../tracking.md` T-27, and its scope also narrowed the same day (the verification gap is closed, and what remains is the split-shape decision and the disposal of the old tests).

~~**That motive has been discarded.** Since we decided to throw the 209 away:~~ → ★**Attribution correction (2026-08-24)**★ — **it was not discarded; it was only set down as a split motive.** The actual decision is "기존 테스트 파일에 미련을 갖지 않는다" = do not put the split motive on them, and **no disposal of the 209 has been decided.** So the status of the items below splits:

- ~~**The grounds for promoting a compile-only gate are gone** — the entire value of that step was 「이미 쓴 209개를 그물로 쓴다」 (use the 209 already written as a net). (T-27's 「미결 ③」 (the old latter half of 「필요한 것 ②」) = promoting `cargo check --all-targets` to a gate also loses its grounds with it.)~~
  - → ★**Attribution correction (2026-08-24) — this conclusion stood on the paraphrase.**★ The argument "the 209 are thrown away, so the value is 0" assumed **words the user did not say**, so it does not hold as it is. The value that step originally aimed at (「이미 쓴 209개를 그물로 쓴다」) is **alive** — only that, reading the verbatim decision honestly, we do not set it up *as the motive for the split*. **The value of the gate itself and the motive for the split are separate questions, and this document having glued the two together was the error.**
  - ★**And one separate reason has newly appeared — it calls for a judgment**★: that suite now **actually executes** with `--test lib_unit` (ADR-0174). The compile-only gate was a cheap **stand-in** for a suite that *executes*, so now that execution is possible, how much value the stand-in has left **has to be measured again.** ★This document does not make that judgment★ — whether to set up the gate or to put the executing suite on CI is the user's decision and is still open.
- **Two split motives remain:**
  1. **Tests written from now on run normally** and are automatically carried in the workspace regression (today anything put into the shell falls into the same blind spot).
  2. **The dependency direction is enforced by the compiler** — today inside the shell any module can call any module.

> ★**Factual correction (2026-08-24) — the premise of motive 1 is gone.**★ That blind spot was the shell package being **left out** of the workspace regression, and that exclusion has been lifted (「후속 사실 정정」 (follow-up factual correction) above). A `#[cfg(test)]` written in `src-tauri/src/**` from now on rides `lib_unit` into the workspace regression and CI **even without splitting**. So the split motive that remains is in effect **motive 2 alone**. ★Whether to split and in what shape is still the user's decision and this document does not decide it★.

~~★Whether the 209 currently compile was **not checked**★~~ → ★**An answer has come (2026-08-24)**★: they not only compile but **execute** — they all run with `--test lib_unit` (★no count is written here — canonical source = the `lib_unit` line of CLAUDE.md 「빌드·검증 명령」. The number that had been pinned in this place went stale in a day★). ~~The remaining failures come from one axis of harness rot (the `app: None` short-circuit) and are not a product defect.~~ → ★**Correction (2026-08-24, later the same day): those failures have also all been fixed and no known failure remains**★ (「후속 사실 정정」 above). ~~**The decision to throw them away is the user's, so we do not reverse it here**~~ → ★**Attribution correction (2026-08-24)**★: **there was no 「버리기로 한 결정」 (decision to throw them away)** (the §Attribution correction block above). It is not a user decision to reverse but a session paraphrase to strip out. — Only that an argument that takes 「상태를 모른다」 (we do not know the status) as its grounds no longer holds is still correct.

## Measurement — the deciding line is 「Tauri import가 실제로 있나」

★Comments are not counted★. Every "tauri" string attached to `layout/` **was in a comment** — a loose text search catches 92 hits, yet the actual imports are 0 lines. It is the same reason the core-isolation gate uses the line anchor `^\s*use tauri` (CLAUDE.md 「빌드·검증 명령」 — "since it is an import-line anchor, a comment quoting itself is not a false positive").

Reproduction command (from the shell package root `src-tauri/`):

```
for d in layout daemon_client commands tray; do echo "--- $d/"; grep -rn "^\s*use tauri" $d --include="*.rs"; done
grep -n "^\s*use tauri" output_router.rs
```

### Result

| Chunk | Real code | `use tauri` | Decision |
|---|---:|---|---|
| `layout/` (7 files) | 4,222 | **0 lines** | splittable — also 0 internal shell references |
| `output_router.rs` | 906 | **0 lines** | splittable — references only `layout` |
| `daemon_client/` (6 files excluding tests.rs) | 3,158 | **1 line** | splittable — that 1 line is the only obstacle |
| `commands/` (7 files) | 1,347 | 5 files directly tied | stays in the shell (adapter) |
| `tray/` (3 files) | 695 | 2 files directly tied | stays in the shell (adapter) |
| `lib.rs` + `main.rs` | 243 | assembly point | stays in the shell |
| `output_channel.rs` | 66 | **undetermined** (§Unchecked) | undecided |

- Shell source total 13,779 lines. Subtracting the 3,142 lines of `daemon_client/tests.rs` gives **10,637 lines of real code**.
- Split candidates total = 4,222 + 906 + 3,158 = **8,286 lines (77.9%)**.
- Adapters that stay = 1,347 + 695 + 243 + 66 = **2,351 lines**.

### That 1 line in `daemon_client/`

`daemon_client/connection.rs:41` — `use tauri::Emitter;`. It is for emitting events.

★There is nothing new to invent★ — `layout/apply.rs` **has already solved the same problem with a port** (that file's header: "all of them are free functions with `AppHandle` · `State<..>` taken out of the arguments — called standalone without a window"). The shape is that emission is received through a port and the shell owns the real adapter, and `tests/layout_apply.rs` is the physical form of that harness. The same method applies as is.

## Dependency direction — no cycles, a straight line

Reproduction command:

```
grep -rhoE "crate::[a-z_]+" layout --include="*.rs" | sort | uniq -c
grep -hoE  "crate::[a-z_]+" output_router.rs | sort | uniq -c
find daemon_client -name "*.rs" ! -name "tests.rs" -exec grep -hoE "crate::[a-z_]+" {} + | sort | uniq -c
```

Result:

```
layout/           → (셸 내부 참조 0)          ★완전 바닥★
output_router.rs  → crate::layout (8건)
daemon_client/    → crate::output_router (2건) · crate::output_channel (2건)
```

**None of the candidates references `commands` · `tray` · `lib` (= the adapter layer).** So those three come off downward as they are. Since the arrows all point one way, moving them into crates creates no cycle.

## Shape — two rather than three is recommended (a recommendation, undecided)

**Splitting into three is not recommended.** It is a straight chain, so there is no gain:

- The linear chain matklad's "Fast Rust Builds" points out = sequential compilation, so build time does not shrink (cited in the grounds section of ADR-0151).
- The middle piece (`output_router`, 906 lines) has only one consumer and **has no meaning on its own** — ADR-0151 decision 2 ("what cannot be used independently stays as a module inside the crate that uses it") is exactly this case.

### Recommended: 2 crates

1. **Window layout computation (4,222 lines)** — `layout/`. The true bottom that references nothing. It knows neither windows nor Tauri and only does tree · coordinate · split computation. By ADR-0151's criterion it clearly meets 「독립적으로 쓸 수 있는가」 (can it be used independently).
2. **Daemon communication + output delivery (4,064 lines)** — `daemon_client/` + `output_router.rs`. The router's only consumer is the daemon communication, and both share the same concern: "where do we send the received output?"

What stays in the shell = 2,351 lines of true adapters. Here we keep checking with integration targets as now. ★**The reason in parentheses has changed (2026-08-24)**★ — the old sentence was "the problem of the lib target dying with `0xc0000139` is confined to this layer, so we carry it as it is", but that instant death is resolved (`--test lib_unit`). The reason we now check with integration targets is **that what that layer measures is wiring that needs a real socket and a real `AppHandle`**, and ~~incidentally the unit suite is still not registered in CI as a whole (waiting on known failures), so the CI signal sits on the integration targets~~ → ★**Correction (2026-08-24, later the same day)**★: that latter clause is dead — the unit suite is also registered in CI **as a whole** and the workspace regression also runs this package (「후속 사실 정정」 above). That is, **the former clause alone** remains as the reason.

★This shape is only a recommendation★ — ADR-0151 decision 1 says "when in doubt, lean toward splitting", so 3 crates is not a criterion violation either. The user decides after seeing the future structure.

## Why it has not been done until now

**It was not left undone for a reason; since the criterion was set, nobody has applied it to the shell.**

`docs/decisions/` has **no decision record at all** on splitting the shell. The daemon has the three-layer split (ADR-0129) and the deferral of further splitting (ADR-0130) respectively, and messaging (ADR-0110) and the command tool (ADR-0155) too. Only the shell lacks one. ADR-0151 fixed the decision criterion on 2026-08-17, and there is no record of applying that criterion to the shell.

T-27 had already been registered as a **symptom** of that blind spot (layout authority is in the shell yet outside the automatic regression net — ADR-0035 × tests that are not run).

## Unchecked

- **The Tauri coupling of the 66 lines of `output_channel.rs`** — a loose text search caught 3 hits, but **we did not settle whether they are comments or actual type uses** (not re-checked with the `use` line anchor). If they are Tauri types, it stays in the shell, and the daemon-communication side receives it through a port (exactly the way `layout/apply.rs` did). ★**The boundary decision stands as above regardless of these 66 lines**★ — this file was not included in the 8,286 split-candidate lines.
- **`lib.rs`, 235 lines** — it is the assembly point, so using Tauri is natural, and we did not re-check it separately with the `use` anchor. The grounds for classifying it as staying is its role, not measurement.
- **Whether the 209 currently compile** — see §「동기가 바뀌었다」 (the motive changed) above. Not measured.
- **Cost of carrying out the split** — we have not actually moved anything. We measured only the import direction and the line counts. The workload of splitting `Cargo.toml` and visibility (`pub` promotion) is unmeasured.

## Appendix — measuring the shell test execution status

| Location | Count | Does it run |
|---|---:|---|
| inside `src-tauri/src/` (`#[test]`·`#[tokio::test(..)]`) | **209** | ✗ none executed → ★**correction (2026-08-24): it runs**★ |
| `tests/layout_apply.rs` | 49 | ✓ |
| `tests/layout_commands.rs` | 27 | ✓ |
| `tests/daemon_client_pending.rs` | 4 | ✓ |

Measurement command:

```
grep -rhoE "#\[(tokio::)?test[^]]*\]" src/ --include="*.rs" | wc -l
grep -rcE "^\s*#\[test\]|^\s*#\[tokio::test\]" tests/*.rs
```

★Why a loose pattern was used for the 209 count★ — closing with `\]` as in `^\s*#\[(test|tokio::test)\]` misses the 38 hits of `#[tokio::test(flavor = "multi_thread", ..)]` (only 171 are caught).

> ★**Factual correction (2026-08-24) — the 「✗ 전부 미실행」 (✗ none executed) in the table above is the state at the time of this measurement.**★ Those 209 are now **executed** with `cargo test -p engram-dashboard --test lib_unit` (history · decision = ADR-0174 · the canonical source for the numbers and CI registration status = the `lib_unit` line of CLAUDE.md 「빌드·검증 명령」). The table itself is left as it is — what this survey saw when it judged is the value of the table.

Distribution of the 209 unexecuted (top, by the strict pattern): `layout/manager.rs` 62 · `layout/tree.rs` 29 · `output_router.rs` 26 · `daemon_client/protocol_state.rs` 17 · `tray/core.rs` 10 · `layout/spatial.rs` 10.

`daemon_client/tests.rs` is a **3,142-line test-only file** and it does not run either.

> ★**Factual correction (2026-08-24) — this line in particular flipped.**★ `daemon_client/tests.rs` now not only runs, but **all the known failures that remain right now occur inside that file** (`daemon_client::tests::` — the canonical account of the cause is the comment on the `app: None` branch in `src-tauri/src/daemon_client/mod.rs`, and the remaining gap and the decision authority are in `docs/tracking.md` T-27).
>
> ★**Re-correction (2026-08-24, later the same day) — those failures have also all been fixed.**★ No known failure remains in this file, and as a result this suite was put on CI **as a whole** (「후속 사실 정정」 above).

### The original plan and where it leaks (history)

Non-execution is not an accident; it was **something started with eyes open**. The comment of commit `ffcd766` on 2026-07-01:

> ★Execution recovery constraint★: the src-tauri lib test is blocked from *running* by the WebView2 DLL link (0xc0000139), so only build/--no-run is recovered — we keep it as a compile-regression net (the store unit tests are the real version of semantic recovery).

The value aimed at = 「실행은 못 해도 컴파일은 되니, 참조하는 함수 모양이 바뀌면 컴파일이 깨져 알려준다」 (even if we cannot run it, it compiles, so when the shape of a referenced function changes the compile breaks and tells us). **Nothing calls that compile** — CI has no `--no-run` step, the QA procedure has none either, and `cargo build` does not compile test targets (CLAUDE.md states this). So even the one value that was planned was not recovered.

~~★But this history is **not a target for action because of 「동기가 바뀌었다」 (the motive changed) above**★ — the 209 are being thrown away, so there is no net to restore.~~ The record is kept as the grounds for "why it ended up in this state".

> ★**Attribution correction (2026-08-24) — the grounds of the previous sentence have fallen away on both sides.**★ ① 「209개를 버리므로」 (since the 209 are thrown away) is **not something the user said** (the attribution correction block above — the denial verbatim: "내가 언제 버린다고 했어?"). ② Before that paraphrase was stripped out, **the facts flipped first** — it is not that there is no net to restore, it is that **it is already running** (ADR-0174). That is, the conclusion "it is not a target for action" lost **both** its premise and the facts. ★**That said, we do not decide here that it is a target for action either**★ — how to treat this history (and whether anything is left to treat at all) is left open.

One point where the stated cause diverges: this commit wrote **the WebView2 DLL link** as the cause, and T-27 wrote "looks like a DLL entry-point failure but unconfirmed". Neither is a confirmed finding.

> ★**Factual correction (2026-08-24) — it has been identified. And both were wrong.**★ The cause was not the DLL itself but **the Windows manifest resource being linked only to the `[[bin]]` target** (so the test exe asked for comctl32 v5.82 and could not find the entry point), and the fix was to redeclare that source as an explicit `[[test]]` target so the `-tests` link argument reaches it. **The canonical source for the decision and the rejected alternatives = ADR-0174** — it is not written out again here.
