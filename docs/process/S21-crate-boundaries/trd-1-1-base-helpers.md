# TRD — Boundary refactoring 1-1: base general-purpose helpers (S21)

> Status: **Edition 5 — decisions applied (2026-10-04).** Not a single line of code has changed yet. All eleven choices in §4 are settled — **delegated by the user (2026-10-04 「알아서 진행해」) → the edition-4 recommendations were adopted.** The parts that conflicted with the confirmed ADR-0269 and ADR-0268 were amended in ADR-0275 (2026-10-04 · §4-2 — 0274 is used by the storage branch). Implementation starts from U1 in §6 (start checklist §6-1).
> **Revision log — 2026-10-04 · edition 5 — decisions applied:**
> - §4 was changed from 「사용자 선택 대기」 to a 「결정」 record — every option chosen is the same as the edition-4 recommendation (C5 (b) · T1 (b) · T2 (a) · C1 (a) · C2 (a) · C3 (a) · C4 (a) · S1 (a) · S2 (e) · N1 (b) · P1 (a)). The long per-option discussion was withdrawn; only the chosen option, a one-line reason, and one line per rejected option remain (the rejected options are material for ADR-0275 「거부한 대안」).
> - **S2=(e) changed the move rule** — the exclusion list of twelve locks (37 sites) where logging while held is forbidden is no longer the move rule. It remains briefly in §2-4 as 「(a) 를 고르지 않은 이유」, and the units move 83 operational sites (7 stay in place — §3-5). U2 13 → **17** · U3a 17 → **24** · U3b 16 → **42** (§5 M3 · §6). The instruction for the codex stdin source-count test (keep the expected value 2 · only widen the matcher) is unchanged.
> - **T1=(b)** — the two truncation functions were removed from `text`. The three copies are replaced in place in U1 with std `floor_char_boundary` · `ceil_char_boundary` (§3-2). **C4=(a)** — the fake clock `ManualClock` sits behind the `test-support` feature inside `time` (§3-3).
> - Conditionals tied to unchosen options (「S2=(b) 면 …」 · 「C5=a 일 때만」 · 「T1=b 면」 etc.) were withdrawn or reduced to footnotes. G3 (raising the transport dependency cap 1 → 2) is gone (§7).
> - §4-2 (the amendments to write into ADR-0275) and §6-1 (the U1 start checklist) were added.
> - The files widened by (e) (leftover cleanup · queued-input registry · pending-list table · latch · turn observation table · codex state lock · the four daemon ones) were checked again against the source-string tests — the only thing that breaks is still the codex stdin count test (§6 common check 5).
> - ★Edition 4 was never committed (this folder is untracked)★ — the details withdrawn in this edition (how the exclusion audit found things · full per-lock quotes from the docs · the exhaustive nesting check · the deadlock decision) survive only as a summary (§2-4).
>
> **Revision log — 2026-10-03 · round 3 review applied** (each finding was re-checked against the code before fixing):
> - **One more site missed by the nesting rule was removed — the `expected` field of the commit port of the first-submit latch (`manager.rs:550`).** The latch calls that port while holding the guard (`session_id_latch.rs:141-145`) — the documented lock order 「래치 → 포트의 expected 칸 → profiles → store write_lock」 (`session_id_latch.rs:23-25` · `manager.rs:523-524`). Excluded locks eleven · 36 sites → **twelve · 37 sites** · agent moved part 34 → **33** · U3a 18 → **17** · M3 47/48 → **46/47** (§2-4 · §5 M3 · §6).
> - **The nesting check was redone exhaustively** — for each of the twelve excluded locks, we read the documented lock order (`rg '락 순서|lock order'` 33 lines + CLAUDE.md 「락 순서」) and every callback and port called while holding the guard. Nothing more was caught. However, the codex head's 「상태 → 대기표」 (`transport.rs:21`) is only allowed by the docs, and no operational site actually takes the locks that way — we left it on the moving side and recorded it as a latent edge (§2-4 · §10).
> - The latch entry's 「`drop(state)` 뒤에 로그」 is true only for the latch's **own** logs — the commit port already emits `warn!` · `info!` · `debug!` while holding the latch (`manager.rs:541-569`). We rewrote it and raised it in §9.
> - Stale numbers were corrected — the 「48 대 49」 in §4 「묶인 것」 · the 「그 밖의 48곳」 in S2 (a) · 「제외 35곳」 (re-counted with grep over §0 · §1-1 · §2-4 · §3-5 · §4 · §5 · §6 · §7).
> - If 1-3 U1 moves the 「liveness 공유」 reason of discovery `Cargo.toml:12-14` to the platform line, no unit writes a new reason on the base line — we added it to U5's touch list.
> - **We added option (e) 「경고 없이 되찾기만」 to S2 and changed the recommendation from (a) → (e) (worker proposal — not a decision).** With (e) no exclusion list is needed and the move range goes from 46/47 → **83**. The cost = an amendment to confirmed ADR-0269 decision 4 (§4 S2 · §7).
>
> **Revision log — 2026-10-03 · round 2 review applied** (each finding was re-checked against the code before fixing):
> - **The warning-exclusion rule was swept again with broader wording** — the search terms of editions 1 and 2 (`쥔 채.{0,40}(로그|IO|emit)|로그도 놓은|잎이다`) missed other phrasings of the same discipline. Five newly included locks: codex state lock (26 recovery sites — `transport.rs:3012-3015` 「★상태 락을 쥔 채 찍지 않는다★」) · turn observation table (`turn.rs:18` 「잡은 채 외부를 부르지 않는다」 — found by reading the docs site by site, not by a search term) · daemon command registry · usage exit field · usage subscription registry (§2-4). The exclusions stay as they are (old form · no warning) — **main-session decision 2026-10-03**. A helper that 「놓고 경고하고 다시 잡는」 was rejected (§4 S2).
> - **The operational/test split was corrected** — two `#[cfg(test)]` functions (codex `Pending::len` `transport.rs:1348` · `input_queue.rs` `queued_bytes` `:262`) had been counted as operational. Operational 92 → 90 · test-only 25 → 27.
> - All numbers were recounted — move range 80/81 → **48/49** (§5 M3) · U3b 43 → **16** · U2 daemon 8 → **5** · U3a 20 → **18** · of the mixed locks, those caught by (b) six → **five** (the command registry dropped out as an exclusion). The exclusion rule was written into the S2 option text.
> - **The usage book lock (`usage_service/mod.rs:606` — this is the only recovery site of the book lock) was also moved to the exclusions — main-session decision 2026-10-03.** In the subscription swap `first_sheets` (`:281-291`) it is taken inside the excluded subscription-registry lock, so a warning would be printed while holding that outer lock. The exclusion rule was widened to 「제외한 자물쇠 안에서 잡히는 자물쇠도 제외(중첩)」. Exclusions 35 → **36** · M3 48/49 → **47/48** · U2 daemon 5 → **4** (§2-4 · §5 M3 · §6).
> - We wrote down in §3-3 and U5 that discovery's base dependency gains a real use after U5 (the `time::Clock` supertrait) — so that 1-3 does not remove that dependency (handed over).
> - The base description in the root `Cargo.toml:2` · `:12-13` was added to U1's touch list and the §7 document list — it is the same sentence as base `lib.rs:15` and CLAUDE.md, so it goes stale in the same change (1-3 fixes its own part again).
>
> **Revision log — 2026-10-03 · round 1 review (`/review trd full`) applied** (each finding was re-checked against the code before fixing):
> - **The S2 recommendation was changed from (b) → (a) (worker proposal — not a decision).** Clearing the poison (`clear_poison`) would also erase the signal that a neighbor using the same lock with `.expect` was watching — seven mixed locks are listed in §2-4 (four counted by the review + three measured in this edition: stdio `child` · `CommandDeliveries` · the `try_lock` of stdin). Variant (d) (once per process), which does not clear the poison, was added. The 「동작 그대로」 in §6 was rewritten as a claim tied to S2.
> - Shell `output_channel.rs` is already a warning + `clear_poison` (ADR-0231) and its test asserts `!is_poisoned` — U2's disposition was tied to S2 (in place unless (b)).
> - U3b got an instruction to widen the codex 「stdin 블로킹 락 개수」 source test (expected 2) — if the call shape changes it becomes 0 and goes red. The result of sweeping all source-string tests was also recorded (§6).
> - The lookup list of the usage query (`LookupGate::release`) is moved by 1-3 U5 to platform, so it was taken out of 1-1 and left in place (U3a).
> - The warning-exclusion rule was set as 「자물쇠 문서가 쥔 채 로그 · IO · 밖 호출을 금한 자리」 and three more were taken out (`QueuedInputs` · `InputsPendingTable` · the first-submit latch) — the deadlock decision was written together with it (§2-4). The move range shrank 85 → 80 (81 if S2=(b)).
> - G1's gate ③ copy sites went from 4 → 6 (two in `docs/testing-strategy.md`), and the base section of that file was added to the base document list.
> - C5 — we wrote that (b) is an amendment that defers confirmed ADR-0269 decision 3-2, and that (a)'s 「`windows` 를 끈다」 depends on the order with 1-3 U1, and added it to the §7 amendment candidates.
> - The 1-3 overlap pointers in §6 were fixed (daemon `lib.rs:109` · agent `manager.rs:77`) · `usage/process.rs` · `leftover.rs` were added to the overlap files.
> **Scope = the part of work order 1-1 that goes first** — bring the `text` · `time` · `path` · `sync` · `testing` helpers into base and replace the scattered copies (`docs/refactoring/architecture-discussion-2026-09-26.md` §10). Clearing the corrupt copies (`file`) is deferred until after storage P3 lands (§8).
> **How to read:** each item shows its source — **user decision (date)** · **user delegation (date)** (the user left the choice to us and the recommendation was adopted) · **main-session decision** · **worker proposal** (the proposal of the worker who wrote this document — not confirmed) · **measured** (the command that was run is written alongside). The eleven choices in §4 are confirmed by user delegation (2026-10-04). Tags in the body such as 「C5=b」 point to that decision.
> **Document placement rule:** the 「새 기능 **설계 착수** → `process/SN-name/` 새 폴더」 line of `docs/README.md` 「새 내용을 어디에 넣나」. The 1-3 (platform crate) TRD stands separately in the same folder.
> **Reference code:** `0ef6292` (branch `v0.3.3/refactor/crate-boundaries`). master `9406581` is a merge of this commit and `git log master..HEAD` = 0, so the trees are the same. For the storage branch we compared against `origin/v0.3.3/feat/storage` `5a2cf7f` (fetched 2026-10-03).
> Anchors: **ADR-0269** (the charter of this phase — base general-purpose helpers) · **ADR-0275** (the decision record of this TRD — written 2026-10-04. One ADR carrying the 1-1 part (amendments to ADR-0269 · ADR-0268 — §4-2) together with the sibling 1-3 part (amendments to ADR-0266 · ADR-0262)) · ADR-0268 (logs go straight through `tracing`) · ADR-0267 (command · messaging do not depend on base for now) · ADR-0266 (platform crate — the home of `write_atomic` · no base → platform edge) · ADR-0270 decision 5 (`normalize_cwd` → base) · ADR-0175 (tenancy condition) · ADR-0177 (transport — zero workspace dependencies) · ADR-0086 (token) · ADR-0231 (the poison-warning precedent of the output Channel registry) · ADR-0262 (the leaf lock of claude leftover cleanup) · `docs/research/clock-seam-placement-2026-10-03.md`.

---

## 0. Canonical summary of this edition

```
① 들이는 모듈 다섯 — 전부 상태 없는 공개 자유 함수(ADR-0269 결정 2 · S1=a), 예외는 시계 트레이트 하나(결정 3).
     text   = hex_lower                       (UTF-8 자르기는 base 에 안 둔다 — 사본 셋을 std floor/ceil_char_boundary 로 제자리 교체 · T1=b)
     time   = now_epoch_ms + Clock(now 하나 · Send + Sync — C1=a) + SystemClock
              + ManualClock(test-support 기능 뒤 — C4=a)
     path   = normalize_spelling(P1=a)
     sync   = 락 오염 복구(lock · read · write · wait_timeout) — 경고 없이 되찾기만, 독 표시는 남긴다(S2=e)
     testing(test-support 기능 뒤 — N1=b) = wait_until

② 실측이 ADR-0269 의 그림과 다른 곳 — 결정(§4)을 거쳐 ADR-0275 로 고친다(§4-2).
     - 시계 seam 은 넷이 아니라 여섯이다(+ agent LeftoverClock · CaptureClock) → LeftoverClock 은 합친다(C2=a · U6). storage 브랜치가 일곱째를 더한다(P3 뒤 D2).
     - transport 에 base 를 붙이면 transport 의 「워크스페이스 의존 0」 게이트가 깨진다 → transport 몫은 3단계로 미룬다(C5=b).
     - UsageClock 은 깔끔히 맞지 않는다 → 그대로 둔다(C3=a).
     - UTF-8 경계 자르기는 고정된 rustc 1.95 의 std 한 줄(floor/ceil_char_boundary)이다 → base 함수 없음(T1=b).
     - 가짜 시계를 testing 에 두면 입주자 무참조(게이트 ③)와 부딪힌다 → time 안 기능 플래그 뒤(C4=a).
     - hex 루프 셋은 전부 「옮기지 않는다」로 정한 함수(토큰 생성기 둘 · sha256_hex) 안에 있다 → 루프만 바꾼다(T2=a). sha256_hex 의 제외 사유(「sha2 를 끈다」)는 거짓이다.
     - 락 오염 복구 117곳(운영 90) · 운영 빌드는 panic = "abort" 라 그 갈래가 안 닿는다 · 오늘 116곳이 이미 말없이 되찾는다
       → 경고 없는 도우미(S2=e)로 운영 83곳을 옮기고 7곳은 제자리(§3-5). 경고를 달았다면 쥔 채 로그 · IO · 밖 호출을 금한
         자물쇠 열둘(37곳)을 빼야 했다(§2-4 역사 기록 — 그 목록이 리뷰마다 넓어진 것이 (a) 를 고르지 않은 사유다).
     - 같은 자물쇠를 한쪽은 되찾고 다른 쪽은 .expect(또는 try_lock)로 쓰는 자리가 일곱이다 → (e) 는 독을 걷지 않아 그 의미가 그대로다.

③ 단위는 「새 함수 먼저 → 호출부 → 옛 사본 삭제」 순서로, 어디서 멈춰도 빌드가 선다(§6). 동작은 로그까지 그대로다.
```

---

## 1. Scope

### 1-1. What we do

| Module | What | Absorbed targets (§2 measured) |
|---|---|---|
| `text` | bytes → lowercase hex | the three daemon loops (T2=a). ★The three UTF-8 truncation copies do not come to base — they are replaced in place with std (T1=b · U1)★ |
| `time` | current epoch ms · the 「지금 읽기」 trait · fake clock (behind a feature) | three agent `now_millis` copies · daemon `command_delivery` `Clock` (operational · both fakes) · the now part of discovery `Clock` · the `mono_now` of agent `LeftoverClock` (C2=a · U6) |
| `path` | pick the spelling of a typed path only | agent `normalize_cwd` (the shell borrows it) |
| `sync` | lock poison recovery — no warning · the poison mark is kept (S2=e) | **83** operational sites in §2-4 (operational 90 − 7 in place — §3-5 · §5 M3) |
| `testing` | `wait_until(timeout, cond) -> bool` | 11 copies of the same contract |

- **The base tenancy rule document is fixed too** — ADR-0269 impact: replace tenancy condition ① in the `lib.rs` header of base with the rule of decision 7, and rewrite the 「셋째 입주자를 받기 전에 ADR-0175 재론」 paragraph as answered by decision 1 (in the same change). The base item in CLAUDE.md 「백엔드 모듈 맵」 is the same. The two member comments in the root `Cargo.toml` (`:2` 「base(바닥 인프라 — 로깅·PID 판정 …)」 · `:12-13` 「로깅 + PID 판정만 담는 잎 crate … 셋째 입주자는 ADR 재론을 거친다」) are the same sentence and go stale together — fixed in U1 (round 2 review). ADR-0269 decision 8 (the wording that steers the next session to move new general-purpose helpers into base) may also be written here — whether and when to write it is for the orchestrator to decide.

### 1-2. What we do not do (reasons in §8)

- `file::set_aside_corrupt` unification — after storage P3 lands on master (user 「알아서」 2026-10-03 → main-session recommendation applied · memo §10).
- `write_atomic` — goes to platform, not base (ADR-0266 decision 8 · 1-3, also after P3).
- messaging's XML escape · the three lock poison recovery sites of the command harness (`src/testing.rs`) — the two crates do not depend on base for now (ADR-0267 decision 1).
- log wrapper — there is none (ADR-0268 decision 1).
- agent codex's `clip` · `truncate` (character-count truncation) — kept private on purpose behind the masking gate. Moving them would break through that gate (§2-1).
- transport clock and transport's base dependency — deferred to phase 3 (the transport attachment TRD) (C5=b · §8 D4). 1-1 changes transport's UTF-8 truncation to std (U1), but that does not make transport depend on base.
- daemon `UsageClock` — left as it is (C3=a · §8 D6).
- the 7 in-place lock poison recovery sites (§3-5) · the 24 test-only sites (§5 M3).

---

## 2. Current state, measured (`0ef6292`)

> Many line numbers in the memo (`architecture-discussion-2026-09-26.md`) have shifted — the values below are the current ones. The list of shifted ones is in §10.

### 2-1. `text`

**UTF-8 boundary truncation — three copies, two shapes** (measured: `rg -n "is_char_boundary" crates src-tauri/src`)

| Site | Side kept | Behavior | Returns |
|---|---|---|---|
| `crates/engram-dashboard-transport/src/ws.rs:147-156` `clamp_close_reason` | front | if `len <= 123` return as is, otherwise **go down** from 123 to a character boundary and cut | borrowed `&str` |
| `crates/engram-dashboard-daemon/src/experiment/record.rs:178-186` `cap_response` | front | same loop · cap 4096 | new `String` (only the `saturation_pilot` experiment binary calls it) |
| inside `crates/engram-dashboard-agent/src/output_core.rs:1059-1062` `push_diagnostic` | back | **go up** from `cut = len - CAP` to a character boundary and `drain(..cut)` — in-place edit | none (it modifies the `String`) |

- The other three `is_char_boundary` are not truncation — messaging `service.rs:3804-3807` (segment validation) · agent `backend/claude/mod.rs:5441` (test precondition).
- **★One std line gives the same behavior (measured)★** — `str::floor_char_boundary` · `str::ceil_char_boundary` compile without a feature flag on this workspace's pinned compiler (`rust-toolchain.toml` = 1.95.0) (compiled a scratch file with `rustc --edition 2021` · confirmed 4 → 3 · 6 in `"가나다"`, 2026-10-03). Keep-front = `&s[..s.floor_char_boundary(max)]`, keep-back site = `s.ceil_char_boundary(s.len() - max)`. The result is the same as the two loops above (`floor` returns `len` if `max >= len`). → **Decision T1=(b)**: replace in place with these two, with no base function (§3-2 · U1).
- **The keep-back form has only one copy** — ADR-0269 decision 7 (「지금 여러 곳에서 쓰이거나 복사돼 있으면」) does not establish a basis for tenancy. That the table in decision 2 lists `keep_tail_bytes` follows the user 「다 옮겨」 (2026-09-26) — with T1=(b), ADR-0275 removes the two functions from that table (§4-2).
- **What does not move (in place):** agent `backend/codex/decoder.rs:1479` `clip` → `:1488` `truncate` (`char_indices().nth` — character count · appends `…`) · `backend/codex/transport.rs:1537` `clip` (character count · `…(잘림)`). In both, a source-string test measures 「문이 정해진 둘뿐」 (`decoder.rs:3486` · `transport.rs:5028`). Agent `usage/process.rs:616` `tail_line` restores bytes lossily (`from_utf8_lossy`) and masks first — a different contract. The `terminal_tail` of `output_core.rs` · `session.rs` is `Vec<u8>`.

**bytes → lowercase hex — three loops, all inside 「옮기지 않는다」 functions** (measured: `rg -n '02x\}' crates src-tauri/src`)

| Site | Wrapping function | ADR-0269's decision |
|---|---|---|
| `crates/engram-dashboard-daemon/src/lib.rs:77-80` | `generate_token` (:73 — WS client token) | no sharing of token generators · only the loop is general-purpose |
| `crates/engram-dashboard-daemon/src/control/mod.rs:146-149` | `gen_token` (:142 — control channel token) | same |
| `crates/engram-dashboard-daemon/src/experiment/record.rs:192-195` | `sha256_hex` (:189) | excluded as 「`sha2` 를 끌고 온다」 — ★this is not true★ (below) |

- All three are the same `write!(s, "{b:02x}")` loop. The only consumer is daemon.
- **The text of ADR-0086 has no sentence forbidding sharing** — `grep -n "공용\|공유" docs/decisions/0086-*.md` → 0 lines. 「공용화 금지 · 혼용 금지(ADR-0086 §맥락)」 is in code comments (`daemon/src/lib.rs:584-585` · `control/mod.rs:140-141`), and what it points to is **do not mix the two tokens** (not the encoding loop). So keeping the two generators separate while only the loop inside them calls `hex_lower` does not conflict with that comment (worker judgment).
- **`sha256_hex` does not pull in `sha2`** — it is a hand-rolled SHA-256 using std only (`record.rs:247-251` comment 「워크스페이스에 sha2 가 없다」 · `grep -n sha2 crates/*/Cargo.toml src-tauri/Cargo.toml` → 0 lines). Even so, the function itself has one copy, so under decision 7 it does not move in — the exclusion stands but the reason changes (ADR-0275 — §4-2).
- → **Decision T2=(a)**: the three wrapping functions stay in place; only the loop inside moves to `hex_lower` (U1).

### 2-2. `time`

**Current epoch ms** (measured: `rg -n "UNIX_EPOCH" crates src-tauri/src`)

| Site | Shape | Used by |
|---|---|---|
| agent `profile.rs:20-25` `now_millis` | `i64` · before 1970 = 0 | `:244` · `:819` (`last_active`) |
| agent `persistence/mod.rs:25-30` `now_millis` | same | **only one** use: the corrupt copy name at `:89` (`agents.json.corrupt-<ms>`) |
| agent `persistence/presets.rs:22-27` `now_millis` | same | **only one** use: the corrupt copy name at `:81` |
| inside shell `src-tauri/src/settings/store.rs:122-125` `copy_aside` | `u128` · before 1970 = 0 | corrupt copy — ★storage P3 fixes this file → 1-1 does not touch it★ |
| storage branch `src-tauri/src/state/saver.rs` (`5a2cf7f`) `SystemClock::wall_ms` | `u64` · before 1970 = 0 | not on master yet |

- The remaining `UNIX_EPOCH` sites are not ms — nanosecond temp folder names (discovery `lib.rs:2620` · daemon `tests/ws_e2e.rs:2341` · shell `settings/tests.rs:24`) · seconds (daemon `bin/saturation_pilot.rs:1703,1713` · the signed floor seconds of `usage_service/clock.rs:51`) · base `logging/mod.rs:155` (log file name — it stays inside `logging`: no tenant references it).
- **The return type splits three ways** (i64 · u128 · u64). What 1-1 changes is only the three agent i64 ones.

**Clock seam — ADR-0269 counted four, now there are six** (measured: `rg -n "trait (Clock|UsageClock|\w*Clock)\b" crates src-tauri/src`)

| Site | Shape | Boundary | Operational implementation | Fake |
|---|---|---|---|---|
| daemon `command_delivery.rs:176` `Clock` | `now() -> Instant` | `Send + Sync` | `SystemClock` (:181) | `tests::ManualClock` (:2425 — `Mutex<Instant>` + `advance`) · `connection_core.rs:4403` also uses it |
| discovery `lib.rs:431` `Clock` | `now()` + blocking `sleep(Duration)` | none · received as `&dyn Clock` (:485) | `RealClock` (:1047) | `FakeClock` (:1742 — `RefCell<Instant>` · `sleep` pushes now · counts `slept`) |
| transport `clock.rs:20` `Clock` | `now()` + `sleep -> BoxFuture<'static, ()>` | `Send + Sync + 'static` · `Arc<dyn Clock>` | `SystemClock` (tokio sleep) | `testing::ManualClock` (:99 — wakes the sleepers in deadline order · oneshot) |
| daemon `usage_service/clock.rs:9` `UsageClock` | `mono() -> Duration` (holds the origin) + `wall() -> i64` (signed epoch seconds) | `Send + Sync + 'static` | `OsUsageClock` | `ManualUsageClock` (pushes the two axes separately) · `usage_service/mod.rs:1251` · `schedule.rs:370` |
| ★agent `backend/claude/leftover.rs:953` `LeftoverClock`★ | `mono_now() -> Instant` + blocking `sleep` + thread `spawn` | `Send + Sync` · `pub(super)` | `SystemClock` (:961) | `FakeClock` (:4664 — atomic integer · sleep pushes it) · `SkipClock` (:6586 — **real time** + the skipped share) |
| ★agent `backend/codex/thread_lock.rs:261` `CaptureClock`★ | `child_alive()` + blocking `sleep` — **there is no now-read (지금 읽기)** | none | `ChildClock` | `FakeClock` (:895) |

- `LeftoverClock` is the 「지금 읽기」 seam — the fifth one that ADR-0269 decision 3 did not count. `mono_now` is called at 22 sites (`rg -n "mono_now\(" crates/engram-dashboard-agent/src | wc -l`). 「★문 자물쇠 안에서도 부른다★ — 다른 자물쇠를 잡거나 기다리지 않는다」 is baked into that contract (:954) — it is the same as the leftover cleanup clause of the CLAUDE.md 「핵심 불변식」 lock order. → **Decision C2=(a)** — merge in supertrait form (U6).
- `CaptureClock` does not read time — there is nothing to merge (worker judgment, not a question).
- **The storage branch adds a seventh** — `src-tauri/src/state/saver.rs` (`5a2cf7f`) `pub trait Clock: Send + 'static { now() -> Instant; wall_ms() -> u64 }`. There is no `Sync`. It is the job of the unit after P3 lands (§8 D2). ★→ TRD A U8 merged it as a subtrait of the base `Clock` (`SaverClock` — base `Clock` carries the `Send + Sync`) (ADR-0291 decision 14 · `fa6cb2f3`)★
- **`UsageClock` does not fit cleanly** (the condition of ADR-0269 decision 3-4 — 「안 맞으면 그때 사용자에게 묻는다」):
  - `mono()` is not `Instant` but **`Duration` since the origin the clock holds**, and that value flows into the domain type of `usage_service/book.rs` (`book.rs:54` `pub mono: Duration` · `owes_at(mono)` · `reject_deadline(mono, …)` etc.). Tests use the `Duration::MAX` · `Duration::ZERO` end values (`book.rs:2133,2756` · `clock.rs:176`) — `Instant` cannot produce those end values.
  - `wall()` is signed epoch **seconds** (negative before 1970 · saturating at both ends), so it cannot be replaced by `now_epoch_ms()` (ms · 0 before 1970).
  - Usage: `.mono()` 11 sites · `.wall()` 13 sites (daemon src).
  - → **Decision C3=(a)** — leave as is (the answer to 「안 맞으면 그때 묻는다」 in decision 3-4 · the reason goes in the head of `usage_service/clock.rs` · U5 · §8 D6).

### 2-3. `path`

- Definition = agent `commands.rs:1143-1153` `pub fn normalize_cwd(raw) -> String` — if the first character is `"` or `'` and the string ends with the same character and the inside is not empty, strip **one layer only**, then `\` → `/`. It does not look at the filesystem.
- Callers = agent `commands.rs:1106` (common registration part) · shell `src-tauri/src/layout/commands.rs:38` (import) · `:1127` (`verb_spawn_into`). The shell test `tests/layout_commands.rs:2121` only mentions the name in a comment.
- Tests = agent `commands.rs:2091` (five stripping · flipping cases) · `:2106` (five non-stripping cases) — they can move to base together with the function. The 「두 문이 같은 철자」 tests after `:2114` measure the registration path, so they stay in agent.
- The doc comment speaks of the domain (「사람 · LLM 이 친 cwd」 · the empty-value check in front `reject_blanks` · ADR-0122) — the base version drops those words and keeps only the contract, and the domain reason moves to the comment at the agent call site (worker proposal).
- The shell → agent edge is not closed by this — `COMMAND_SPECS` · `llm_creation_refusal` remain (part of 2-1 · ADR-0270).

### 2-4. `sync` — lock poison recovery

**Total = 117 sites** (measured — how it was counted is in §10). The 94 of ADR-0269 「근거」 (`ebfdafc`) was 67 same-line closures + 27 `PoisonError::into_inner`, and now those two counts are the same 67 · 27 (line counts — one of those lines is a `output_channel.rs:18` comment). 117 is the number that includes forms spanning several lines and other forms.

| Form | Count |
|---|---|
| `.lock()` + `unwrap_or_else(\|x\| x.into_inner())` | 77 |
| `.lock()` + `unwrap_or_else(PoisonError::into_inner)` | 26 |
| `match m.lock() { Ok(g) => g, Err(p) => p.into_inner() }` | 6 |
| `.lock().unwrap_or_else(\|e\| { warn; clear_poison; e.into_inner() })` | 1 |
| `Condvar::wait_timeout(..).unwrap_or_else(\|p\| p.into_inner())` | 4 |
| `RwLock::read` / `write` + closure | 1 / 1 |
| the `TryLockError::Poisoned` branch of `try_lock` | 1 |

| crate | Operational | Test-only | Note |
|---|---|---|---|
| agent | 68 | 19 | of the operational ones `backend/codex/transport.rs` 42 · `transport/input_queue.rs` 7 · `backend/claude/leftover.rs` 3 · `transport/stdio.rs` 3 · `manager.rs` · `reaper.rs` (read 1 · write 1) · `session.rs` · `transport/pty.rs` 2 each · `inputs_pending.rs` · `queued_input.rs` · `session_id_latch.rs` · `turn.rs` · `usage/process.rs` 1 each |
| daemon | 8 | 5 | operational = `agent_conn.rs:213` · `command_delivery.rs:1008` · `command_roster.rs:401` · `usage_service/mod.rs:571,606,648` · `usage_service/reject_store.rs:276` · `usage_service/watch.rs:205` |
| shell | 13 | 0 | `view_commands.rs` 7 (:403,410,455,527,552,758,790) · `settings/mod.rs` 3 (:478,482,486) · `daemon_client/mod.rs:88` · `output_channel.rs:41` · `ui_settings.rs:768` |
| base | 1 | 0 | `logging/mod.rs:129` |
| command | 0 | 3 | `src/testing.rs` |
| messaging · net · protocol · discovery · transport | 0 | 0 | — |

- The 「운영/시험」 split is a script estimate (after the first `#[cfg(test)] mod … {` in the file · `tests/` · `testing.rs` · `fakes.rs`) plus hand corrections — daemon `usage_service/clock.rs:114` is a `#[cfg(test)]` fake, so it was counted on the test side. ★In the round 2 review we fixed the script to also look at the attributes of the wrapping function, and moved two more to the test side★ — codex `transport.rs:1349` (`#[cfg(test)] fn len` `:1348` — a test-only window of `Pending`) · `transport/input_queue.rs:263` (`#[cfg(test)] fn queued_bytes` `:262`). So operational 92 → 90 · test-only 25 → 27. Attributes attached to the wrapping `impl` or to a module were still checked by hand only, so room for error remains.
- **Two precedents that already exist:** agent `usage/process.rs:636`'s `fn lock<T>(&Mutex<T>) -> MutexGuard<T>` — exactly the free-function form (callers = three `StderrTail` `:696 · 985 · 1002` + the lookup list `LookupGate::release` `:352`). Shell `output_channel.rs:38-45` — **warning + `clear_poison`**, 「poisoning 한 번에 warn 한 번」 (ADR-0231 anchor). Its test `:145-148` asserts `!registry.is_poisoned()` after recovering — **it goes red if moved to a helper that does not clear the poison** → since the S2=(e) helper does not clear the poison, this site stays in place (§3-5 · U2). Every other site recovers **silently** and leaves the poison mark — 116 of the 117 sites (the basis for S2=(e)).
- **★In operational builds this branch is unreachable★** — the workspace `[profile.release]` is `panic = "abort"` (`Cargo.toml:34-35`), so a panic ends the process and no poison arises. A code comment already says so (`command_roster.rs:396` · `command_delivery.rs:1003`). So even if a recovery warning had been attached, it would have been a **signal of debug · test builds** — that is the basis for saying that is all S2=(e) loses (§4).
- **★The move rule of S2=(e) — move 83 of the 90 operational sites and leave 7 in place★** (list · reasons = §3-5). Because the helper does no logging, takes no other lock, and makes no outside call, we also move the sites of locks for which the docs forbid 「쥔 채 로그 · IO · 밖 호출」 (the two leaf locks of leftover cleanup — CLAUDE.md 「핵심 불변식」 lock order · queued-input registry · codex state lock, etc.) — the helper adds no behavior, so moving does not change the discipline of those locks (what to take and when stays with the caller).
- **Historical record — the exclusion list that would have been needed under the options that attach a warning (S2 (a) ~ (d)) (not adopted · the rejection reason in ADR-0275):** under those options the warning is printed **while holding the guard** (the guard can be returned only after the poisoned guard is recovered). So locks for which the lock docs forbid 「쥔 채 로그 · IO · 밖 호출」, and locks taken **inside** such locks (nesting), could not be moved and had to stay in the old form — twelve · 37 recovery sites (main-session decision 2026-10-03):
  - agent (33) — two leaf locks of leftover cleanup (`leftover.rs:402` `Recorder.inner` · `:1713` `GateCell.state` — CLAUDE.md lock order) · queued-input registry (`queued_input.rs:405` — 「★이 가드를 쥔 채 emit·IO·다른 락을 잡지 말 것★」) · pending-list table (`inputs_pending.rs:47` — 「명부·표는 잎」) · first-submit latch (`session_id_latch.rs:150` — head `:23`) · the `expected` field of its commit port (`manager.rs:550` — nesting · round 3 review) · turn observation table (`turn.rs:145` — head `:18-21` 「leaf」) · codex state lock 26 (`backend/codex/transport.rs` — `SharedState` `:1079` · discipline `:3010-3015` 「★상태 락을 쥔 채 찍지 않는다★」 · sites = `lock.lock()` 25 + writer `wait_timeout` `:2192`).
  - daemon (4) — command registry (`command_roster.rs:401` — `detach` doc `:313-314`) · usage exit field (`agent_conn.rs:213` — `usage_service/watch.rs:28-29`) · usage subscription registry (`usage_service/watch.rs:205` — `:146-147`) · usage book lock (`usage_service/mod.rs:606` — taken inside the subscription registry · nesting).
  - **The history of that list is the main reason we did not choose (a)** — edition 1 = two leaf locks of leftover cleanup → round 1 review = three more by wording of the same discipline → round 2 review = five more by other phrasing (bold text · English `leaf`) + the nesting rule (book lock) → round 3 review = the `expected` field the nesting missed. It had to be widened at every review and no gate guarded it — if 「쥔 채 로그 금지」 is newly written in a lock doc, or code is changed to take a moved lock inside an excluded lock, it silently drifts (one latent site = the 「상태 → 대기표」 that the codex head allows · §9). That discipline is not even fully kept in the code already (two in §9). It was not because of deadlock — the file sink of the operational subscriber (base `logging/mod.rs:417-421`) takes only its own `LOG_FILE` lock (code reading). What was at stake was consistency of discipline.
  - ★The 「놓고 경고하고 다시 잡는」 helper was rejected too★ — it empties that lock itself, but still prints under an outer lock held by the caller (e.g. codex `Announcer.order` → state lock — `transport.rs:1112-1115`).
  - The details edition 4 carried (the `rg` regex of how it was found · full per-lock quotes from the docs · the exhaustive nesting check for every lock · the four 「경계에 있지만 빼지 않는」 — `command_delivery.rs:1008` · `daemon_client/mod.rs:88` · `view_commands.rs` · `reaper.rs`) were withdrawn in this edition. The summary of the measuring commands is in §10. ★If we want to attach the warning again, this audit is redone from scratch over all 83 sites★ (written into ADR-0275).
- **Mixed locks — the same lock is recovered on one side and used as `.expect` (or `try_lock`) on the other** (four from round 1 review + three measured in edition 2. How found = a scratch script that matches the field name of each recovery site against `.lock()/.read()/.write()` + `.expect(`/`.unwrap(` in the same crate's operational code, followed by a hand check — other people's structs with the same field name were filtered out). Right now the poison mark **does not disappear** — so the behavior of 「다른 쪽」 below is independent of whether a recovery ran in between. ★The S2=(e) helper does not clear the poison either, so this independence stays the same after 1-1★ — this table is kept as the basis for rejecting (b) (`clear_poison`) (last column = if it had been (b)).

| Lock | Recovering side | Other side | If the poison had been cleared ((b) — not adopted) |
|---|---|---|---|
| agent PTY `child` | `transport/pty.rs:261-264` (watcher thread) · `:330-333` (pump) | `shutdown()` stage 2 `:422` `.expect("child poisoned")` — ADR-0001 kill causal path | Now, if poisoned, shutdown panics and skips stage 4 (Job terminate) and stage 5 (master drop). If cleared, whether it panics depends on whether the watcher's or pump's recovery ran first |
| agent stdio `child` | `transport/stdio.rs:378-381` (pump) | `shutdown()` `:482` `.expect("child poisoned")` | same shape (whether Job terminate is skipped depends on the order) |
| agent `sessions` RwLock | `reaper.rs:52-55` (write) · `:154-156` (read) | 7 sites of `.expect("sessions poisoned")` in the operational section of `manager.rs` (`:843 · 2011 · 2839 · 2921 · 2938 · 2954 · 2972` — the six in test module `:3060~` were left out) | one reaper pass erases the manager's fail-fast signal |
| daemon `CommandRoster.inner` | `command_roster.rs:400-405` `lock_for_cleanup` (「정리 경로 전용」) | `lock()` `:390-391` `.expect` — a pair split on purpose | one cleanup pass erases 「오염된 표를 계속 쓰지 않는다」 |
| daemon `CommandDeliveries.inner` | `command_delivery.rs:1003-1008` `lock_for_cleanup` (destructor only) | `lock()` `:990-991` `.expect` — the same pair as above (its comment 「표마다 따로 서야 한다」) | same — ★something the review could not count (edition 2 measured)★ |
| agent lookup list `LookupGate` | `usage/process.rs:352` `release` → `:636` helper | `claim` `:345-348` reads the poison as 「도는 중 → 모름」 (test `:1742`) | the `release` of one lookup that was in progress turns that decision back to 「평소」 — in place in 1-1 (U3a · 1-3 U5) |
| agent stdin (stdio · codex) | `stdio.rs:194` · codex `transport.rs:1555 · 2751-2753` | the `try_lock` of `shutdown` (`stdio.rs:496` · codex `:4246`) reads the poison as 「건너뜀」 | shutdown would take the stdin (harmless direction, but behavior changes) — edition 2 measured |

- **Files overlapping with storage P3:** shell `settings/mod.rs` (3 sites) — the storage branch fixes this file (`git diff <merge-base> origin/v0.3.3/feat/storage --stat` → `settings/mod.rs | 36`). The storage branch adds 1 recovery site (`state/` — `self.cell.lock().unwrap_or_else(PoisonError::into_inner)`).
- **`clear_poison` exists in this compiler** — `Mutex::clear_poison` · `RwLock::clear_poison` (shell `output_channel.rs:43` already uses it). The 1-1 helper does not use it (S2=e).
- **Sites where tests assert the poison state** (`rg -n "is_poisoned" crates src-tauri/src`): 9 sites — all are preconditions **before** recovery (「전제: 락이 poisoned」 · 「시험이 poison 갈래를 탔다」) or `!is_poisoned`. The S2=(e) helper does not clear the poison, so those preconditions and what they measure stay the same (read — not run). The `!is_poisoned` of shell `output_channel.rs:145-148` measures the poison clearing at that site (ADR-0231), so that site stays in place (§3-5 · U2). Under (b), assertions would not break but what they measure would shrink — e.g. `leftover.rs:3903` 「기록 자물쇠가 독에 걸려도 켜기 · 넣기 · 복사 · 끄기가 돈다」 would take the poison branch on the first operation only.

### 2-5. `testing` — waiting in tests

**Same contract `fn(timeout, cond) -> bool` (checks once more after the deadline) — 11 copies** (measured: `rg -n "fn (wait_until|poll_until|…)\b" crates src-tauri`)

| Site | Closure | Polling |
|---|---|---|
| agent `tests/activation.rs:53` · `transport_smoke.rs:65` · `session_smoke.rs:85` · `stdio_smoke.rs:135` · `headless.rs:79` | `Fn` | 50 ms |
| agent `tests/backend_contract.rs:703` | `FnMut` | 50 ms |
| agent `tests/reaper.rs:54` | `Fn` | 25 ms |
| daemon `tests/control_send.rs:76` · `tests/mcp_manager_lifecycle.rs:64` | `Fn` | 20 ms |
| daemon `tests/ws_e2e.rs:2404` `poll_until` (only the name differs · inside a module) | `FnMut` | 50 ms |
| daemon `src/messaging_host.rs:1153` (inside the `#[cfg(windows)]` test module) | `Fn` · **no deadline argument** (fixed 150 × 20 ms) | 20 ms |

- The 「9개 파일(agent 7 · daemon 2)」 in ADR-0269 is right — it is the first nine of the 11 copies above.
- **Five variants with a different contract (proposed to stay in place):** agent `platform/windows.rs:746` (returns `Option<T>` · panics after 10 s) · `transport/input_queue.rs:620` (5 s · panic) · `backend/codex/mod.rs:1251` (60 s · before panicking, writes first to stderr, which does not go through capture — the measured reason is in the comment) · daemon `usage_service/mod.rs:1417` (cap `BOUND` · panic) · agent `usage/process.rs:1056` `eventually` (5 s · panic). They are a label + panic + each one's own polling (5 ms etc.), so folding them into `assert!(wait_until(..), "{what}")` would change the polling.
- **Feature flag precedents:** `test-support` (command · transport — opens the harness helper module (하네스 도우미 모듈을 연다)) · `test-harness` (agent · daemon · messaging — opens a seam inside operational code (운영 코드 안의 seam 을 연다)). Both are turned on only as a dev-dependency, and the workspace is `resolver = "2"` (`Cargo.toml:9`), so features from the dev graph are not merged into operational builds. → **Decision N1=(b)** — base means the same thing, so `test-support`.
- **Command to check the operational graph (measured — base has no features now, so only `feature "default"` appears):** `cargo tree --locked -p engram-dashboard-daemon -e normal,features -i engram-dashboard-base --target all`. Running `-e normal,dev,features -p engram-dashboard-agent -i engram-dashboard-command` in the same form shows `feature "test-support"` only on the dev side (measured — the existing precedents split that way).

---

## 3. Design (decisions of §4 applied — details without a tag are worker proposals)

### 3-1. Placement · tenancy rule

- Modules = one file each: `src/text.rs` · `src/time.rs` · `src/path.rs` · `src/sync.rs` · `src/testing.rs`. A `// ADR-0269` in each head.
- **`testing` is `#[cfg(any(test, feature = "test-support"))] pub mod testing;`** — same as the transport precedent (`any(test, feature = "test-support")`). Then base's own tests compile that module without the feature, so the CI base test step (`ci.yml:276-278` — no feature argument) does not need to be fixed. Feature name = `test-support` (N1=b) — `time::ManualClock` is also behind the same feature (C4=a).
- **We keep the tenant no-reference rule (gate ③)** — none of `text` · `time` · `path` · `sync` · `testing` calls another tenant. The places that would trip it:
  - If the fake clock were put in `testing` it would have to implement `crate::time::Clock` and trip gate ③ → put it behind the same feature inside `time` (C4=a · §3-3). `testing` does not call `time`.
  - `sync` emits no logs (S2=e) — it calls neither `logging` nor `tracing`.
  - base `logging`'s own recovery site (`logging/mod.rs:129`) does not use `sync` (gate ③ · §3-5).
- **Zero new external dependencies** — all five new modules use only std. We bring in no serde · tokio · futures (ADR-0269 decision 6).
- **Error type:** none of the 1-1 functions can fail — there is no place to use `std::io::Result`. That rule is used in `file` (after P3).
- 1-3 moves the `platform` module out of base (ADR-0266 decision 4) — at that point `platform` drops out of the names in gate ③ (§7).

### 3-2. `text`

```rust
/// 바이트를 소문자 16진 문자열로 — 바이트 하나당 두 글자.
pub fn hex_lower(bytes: &[u8]) -> String
```

- Replace the three loops (§2-1) with this — the wrapping functions (the two token generators · `sha256_hex`) stay in place as they are (T2=a).
- **UTF-8 truncation is not placed in base (T1=b)** — we do not create `truncate_bytes` · `keep_tail_bytes`. The three copies are replaced in place with std in U1 (only the hand loop changes; the function name · signature · cap constant stay as they are):
  - transport `ws.rs:147-156` `clamp_close_reason` (keep front · borrowed) — `while … is_char_boundary` loop → `&reason[..reason.floor_char_boundary(MAX_CLOSE_REASON_BYTES)]`. Even with this replacement transport does not depend on base (C5=b).
  - daemon `experiment/record.rs:178-186` `cap_response` (keep front · new `String`) — same loop → `s[..s.floor_char_boundary(RESPONSE_CAP_BYTES)].to_string()`.
  - inside agent `output_core.rs:1059-1062` `push_diagnostic` (keep back · in-place edit) — the loop that raises `cut` → `let cut = buf.ceil_char_boundary(buf.len() - DIAGNOSTIC_CAP_BYTES);` followed by `buf.drain(..cut)` as is. The 「★문자 경계로 밀어 올린다★」 comment above it stays (why it is a boundary is still true).
  - All three give the same result as the hand loop (§2-1 — `floor` returns `len` if `max >= len`). The branch that returns early below the cap gives the same result whether kept or removed — we keep it (to shrink the changed lines · worker proposal).

### 3-3. `time`

```rust
/// 벽시계 epoch 밀리초. 1970 전이면 0 (agent 세 벌의 동작 그대로).
pub fn now_epoch_ms() -> i64

/// 「지금 읽기」 — 시험이 시간을 갈아 끼우는 seam(ADR-0269 결정 3-1).
pub trait Clock: Send + Sync {          // C1=a
    fn now(&self) -> Instant;
}
pub struct SystemClock;                 // now() = Instant::now()

/// 시험용 가짜 — 기점 + 원자 정수 오프셋, 자물쇠 없음(C4=a).
#[cfg(any(test, feature = "test-support"))]
pub struct ManualClock { /* … */ }      // new() · advance(d) · impl Clock
```

- The return type `i64` = the same type agent writes into persistent fields (`last_active` etc.). storage's `u64` · settings' `u128` are aligned in a unit after P3 (§8 D2).
- Whoever needs waiting **makes this a supertrait of its own trait** (ADR-0269 decision 3-2) — e.g. discovery `pub trait Clock: engram_dashboard_base::time::Clock { fn sleep(&self, d: Duration); }`. One fake implements both traits with the same state (keeping the linkage where sleep pushes now).
- **★discovery keeps its base dependency, and after U5 it really uses that dependency★ (round 2 review — consistency with 1-3).** Today (`0ef6292`) the only thing discovery calls from base is `platform::pid_alive_with_start_time` (`discovery/src/lib.rs:1043` · `engram-dashboard-base` in `Cargo.toml:15`). If 1-3 moves `platform` out of base that use disappears, but when U5 changes discovery `Clock` into a subtrait of `engram_dashboard_base::time::Clock`, the discovery → base edge gets a real use again. ★So 1-3 does not remove discovery's base dependency★ — if 1-3 lands before U5 it briefly becomes an unused dependency in between, but the workspace has no gate that catches unused dependencies (`rg 'unused_crate_dependencies|machete|udeps' Cargo.toml crates/*/Cargo.toml .github/workflows/ci.yml` → 0 lines), so build and CI stand as they are. The correction on the 1-3 TRD side is that document's job (handed over).
- **Whether the base trait must be `use`d when calling the supertrait's `now()` from `dyn Sub` was not confirmed by compiling** — if needed it is one `use engram_dashboard_base::time::Clock as _;` line in the calling file.
- The fake clock `ManualClock` (atomic integer offset, no lock · `new()` · `advance(d)`) sits behind `#[cfg(any(test, feature = "test-support"))]` inside `time` (C4=a) — putting it in `testing` would trip gate ③. The reason for the lock-free form = so that fakes can also fit the 「문 자물쇠 안에서도 부른다 — 다른 자물쇠를 잡지 않는다」 contract of `LeftoverClock` (those fakes are atomic integers now · worker proposal).
- **What merges:**
  - daemon `command_delivery` — use base `Clock` · `SystemClock` as they are, and change the test fake `tests::ManualClock` (`:2425` — `Mutex<Instant>`) to base `ManualClock` too (the tests of `connection_core.rs:4403` are the same). Decision 3-3 · U5. The line that turns on `test-support` in daemon's dev dependency is already added by U4.
  - discovery — per decision 3-2, `Clock: engram_dashboard_base::time::Clock` + blocking `sleep`. The fake `FakeClock` (`:1742`) keeps the linkage where sleep pushes now and the `slept` count, so it stays in its own place, and changes `RefCell` · `Cell` to a form that is `Sync` (atomic integer or `Mutex`) (C1=a). It does not carry base `ManualClock` — the intent is not to turn on a dev-dependency feature newly in discovery (worker proposal). U5.
  - agent `LeftoverClock` — `trait LeftoverClock: base::time::Clock { sleep; spawn }` · `mono_now` 22 sites → `now`. The fakes `FakeClock` · `SkipClock` stay in their own place as atomic integers (C2=a · U6).
- **What does not merge (one reason line in the head):** transport `clock.rs` (C5=b — 「3단계 transport 부착 때 base `time::Clock` 의 하위 트레이트로 — ADR-0275」 · U5) · daemon `usage_service/clock.rs` `UsageClock` (C3=a — the contract difference in §2-2 · U5) · agent `CaptureClock` (it has no now-read — no comment needed).

### 3-4. `path`

```rust
/// 사람이 친 경로의 철자만 고른다 — 감싼 따옴표 한 겹(안이 빈 짝 · 두 겹은 그대로)을 벗기고 `\` 를 `/` 로.
/// 파일시스템을 보지 않는다(실재 · 상대경로 판정은 부르는 쪽 몫).
pub fn normalize_spelling(raw: &str) -> String     // P1=a
```

- agent `normalize_cwd` is deleted — the agent registry and the shell both call base directly (ADR-0270 decision 1 — the shell does not know agent). Leaving a thin agent variant would open a path for the shell to keep calling that one.

### 3-5. `sync`

```rust
pub fn lock<T: ?Sized>(m: &Mutex<T>) -> MutexGuard<'_, T>
pub fn read<T: ?Sized>(l: &RwLock<T>) -> RwLockReadGuard<'_, T>
pub fn write<T: ?Sized>(l: &RwLock<T>) -> RwLockWriteGuard<'_, T>
pub fn wait_timeout<'a, T>(cv: &Condvar, g: MutexGuard<'a, T>, d: Duration)
    -> (MutexGuard<'a, T>, WaitTimeoutResult)
```

- Form = free functions (S1=a — `usage/process.rs:636` is already that form). Each body is one `….unwrap_or_else(PoisonError::into_inner)` line — **no warning · the poison mark is kept · it does not call `clear_poison`** (S2=e). `#[track_caller]` is not needed either (there is no log to carry the location).
- **Contract (written in the module head · `// ADR-0269` · `// ADR-0275`):**
  1. It does not clear the poison — it does not erase the signal seen by neighbors that use the same lock with `.expect` · `try_lock` (the mixed-lock table in §2-4).
  2. It does no logging, takes no other lock, and makes no outside call — the helper adds no behavior, so moving the sites of 「쥔 채 로그 · IO · 밖 호출 금지」 locks does not change the discipline of those locks (what to take and when stays with the caller · §2-4).
  3. Operational builds are `panic = "abort"`, so this branch is not reached — it is a recovery in debug · test builds.
  - ★If we want to add a warning, 2 breaks★ — the 「쥔 채 로그 금지」 audit of all 83 sites must be redone (§2-4 historical record · ADR-0275).
- **Forms that move:** of the forms in §2-4, closure · function pointer · the `match` branch · `wait_timeout` · `read` / `write`. The 6 sites of the `match` form (`pty.rs:263,332` · `stdio.rs:317,380` · `command_roster.rs:403` · `command_delivery.rs:1010`) connect the post-processing attached after the branch (e.g. `.take()` of `stdio.rs:317`) onto the helper result as is. The 3 operational condition-variable `wait_timeout` sites = `input_queue.rs:178 · 254` · codex `transport.rs:2192` (`:10176` is a test module).
- **The 7 sites that stay in place (operational 90 − 83):**

  | Site | Reason | 「왜 base `sync` 를 안 쓰나」 comment |
  |---|---|---|
  | base `logging/mod.rs:129` | calling `sync` of the same crate would trip the tenant no-reference rule (gate ③) | one line in U2 |
  | agent `leftover.rs:1662-1665` `GateCell`'s `Debug` (`try_lock`) | a different contract with a `WouldBlock` branch — it does not wait for the lock | one line in U3a |
  | agent `usage/process.rs:636` local helper (the share of lookup list `LookupGate::release` `:352`) | 1-3 U5 moves the lookup list to platform and platform does not know base (ADR-0266). The three that call `StderrTail` (`:696 · 985 · 1002`) move to `sync::lock`, and the remaining helper is folded into `release` (worker proposal) | one line in U3a |
  | shell `settings/mod.rs:478 · 482 · 486` (3) | storage P3 fixes this file — 1-1 does not touch it. After P3, in D2, all three move (with S2=e, the 「잎」 discipline of `:486` does not apply either) | none (the file is not touched) |
  | shell `output_channel.rs:38-45` | ADR-0231 — warning + `clear_poison` (「poisoning 한 번에 warn 한 번」) is the contract of that site. Moving it loses that behavior and the `!is_poisoned` of test `:145-148` goes red | one line in U2 — 「ADR-0231 — 독을 걷는 자기 도우미, base `sync` 는 독을 걷지 않는다」 |

- What else does not move: command `src/testing.rs` 3 sites (test-only · ADR-0267) · 24 test-only sites (agent 19 · daemon 5 — §5 M3).
- Comments are attached only at the in-place sites above. No separate comments are attached to the 83 moved sites — edition 4's 「제외한 자물쇠 정의에 한 줄」 instruction disappeared together with the exclusion list.

### 3-6. `testing`

```rust
/// `cond` 가 참이 될 때까지 20 ms 마다 본다. 시한이 지나면 마지막으로 한 번 더 보고 그 값을 돌려준다.
pub fn wait_until(timeout: Duration, cond: impl FnMut() -> bool) -> bool
```

- It is received as `FnMut` — the `Fn` copy also goes in as is.
- Polling 20 ms (§5 M2) — the copies split into 20 · 25 · 50 ms. The 50 ms copies (real PTY · real shell tests) check the condition more often — it does not start more child processes (unrelated to the `--test-threads=4` reason).
- `messaging_host.rs:1153` goes to `wait_until(Duration::from_secs(3), cond)` — a time instead of a count (150 × 20 ms ≈ 3 s).
- The crates that use it (agent · daemon) add `[dev-dependencies] engram-dashboard-base = { path = "…", features = ["test-support"] }` (writing the same crate as a normal dependency and a dev dependency with only the feature differing is exactly the command precedent — agent `Cargo.toml:41`).

---

## 4. Decisions (결정) (2026-10-04)

> **Source (common to all eleven): user delegation (2026-10-04 「알아서 진행해」) → recommendation adopted.** We picked the worker recommendations of edition 4 §4 as they were — after cross-checking following the delegation, all eleven are the same text as the edition-4 recommendations. The parts that conflict with the confirmed ADR-0269 are amended in ADR-0275 (§4-2). The rejected options are material for ADR-0275 「거부한 대안」. The long per-option discussion was in edition 4 (before commit); this edition carries only the conclusions.

### 4-1. Decision record

| id | Question | Decision |
|---|---|---|
| C5 | Do we attach base to transport? | **(b)** Leave it out this time and look at it in phase 3 (the transport attachment TRD) |
| T1 | Do we keep UTF-8 truncation as a base function? | **(b)** Replace in place with std `floor_char_boundary` · `ceil_char_boundary` |
| T2 | Do we change the three hex loops to `hex_lower`? | **(a)** Change them (the wrapping functions stay as they are) |
| C1 | The boundary of base `Clock` | **(a)** `Send + Sync` |
| C2 | Do we merge agent `LeftoverClock` too? | **(a)** Merge — as a separate unit (U6) |
| C3 | daemon `UsageClock` | **(a)** Leave as is |
| C4 | Where the fake clock goes | **(a)** Inside `time` · behind a feature flag |
| S1 | The form of `sync` | **(a)** Free functions |
| S2 | The unit of the recovery warning · do we clear the poison? | **(e)** Recover only, with no warning (the poison mark is kept) |
| N1 | The name of the `testing` feature | **(b)** `test-support` |
| P1 | The new name of `normalize_cwd` | **(a)** `normalize_spelling` |

**C5 = (b).** Reason: both are amendments to confirmed decisions, but (b) is a deferring amendment, so transport's gates and dependency graph stay as they are — transport has zero consumers and phase 3 reopens the dependency and gates anyway (in the same vein as ADR-0267). The truncation copy in `ws.rs` disappears without base through T1=(b).
- Rejected (a) attach — an amendment that changes transport gate ① (exactly 1 direct workspace dependency line · ADR-0177) to 2 lines, plus `tracing` · `tracing-subscriber` · `regex` (up to `windows` if before 1-3 U1) get loaded into a reusable lib with zero consumers.
- Rejected (c) attach only the clock and use std for truncation — it carries the whole cost of (a) and gains one clock.
- Accepted cost: the transport clock, which departs from 「시간은 공용이면 무조건 합친다」 (user 2026-09-26), remains until phase 3 — written in the head of transport `clock.rs` (U5 · §8 D4).

**T1 = (b).** Reason: it is one std line with nothing to wrap (the same reason ADR-0269 left out `dunce::canonicalize`) · the keep-back form is one copy, so it falls short of decision 7 · regardless of C5, up to `ws.rs` is cleaned up in one go.
- Rejected (a) base `text::truncate_bytes` · `keep_tail_bytes` — the name tells the meaning, but the body is one std line and the keep-back form has no basis for tenancy (decision 7).

**T2 = (a).** Reason: this is exactly the part ADR-0269 wrote down separately as 「인코딩 루프만 범용」, and it does not conflict with the code comment 「생성기를 섞지 말라」 (§2-1 — the text of ADR-0086 has no sentence forbidding sharing).
- Rejected (b) leave `hex_lower` out of 1-1 — the same three loops remain and it conflicts with the table in decision 2.

**C1 = (a).** Reason: most 「지금 읽기」 seams (daemon · leftover · transport) are already this boundary, and what changes is one discovery test fake.
- Rejected (b) no boundary — every use site would have to write a boundary, and a `dyn Clock` without a boundary could appear.
- Rejected (c) two reads (`now` + wall-clock ms) — it amends decision 3-1 (「지금 읽기 하나」), yet 1-1's consumers do not use the wall clock.

**C2 = (a), separate unit (U6).** Reason: it is covered by the user rule 「시간은 공용이면 무조건 합친다」, and all that changes is the supertrait and the name (`mono_now` → `now`). The fake stays in its own place as an atomic integer because of the 「자물쇠를 잡지 않는다」 contract.
- Rejected (b) leave it — a seam that departs from the rule would remain.

**C3 = (a).** Reason: it does not fit cleanly (§2-2 — the `Duration` of `mono` flows into the book domain and there are `Duration::MAX` end-value tests · `wall` is signed epoch seconds). This is the answer to the 「안 맞으면 그때 묻는다」 of decision 3-4. The reason is written in the head of `usage_service/clock.rs` (U5).
- Rejected (b) attach only the supertrait — it merges only a shell that nobody calls `now()` through.
- Rejected (c) `mono` to `Instant` — a contract change of ADR-0250 · usage TRD §3 #28 and a rewrite of the end-value tests, so it exceeds the 1-1 scope.

**C4 = (a).** Reason: gate ③ (tenant no-reference) stands as it is — putting it in `testing` would require implementing `crate::time::Clock` and trip the gate. Cost = one feature flag spans two modules (`time` · `testing`).
- Rejected (b) put it in `testing` and have gate ③ exclude `testing` — the gate gets an exception and the regex gets longer.
- Rejected (c) do not put it in base — it amends decisions 3-1 · 5 and daemon's `Mutex<Instant>` fake remains.

**S1 = (a).** Reason: the same as decision 2 (「상태 없는 공개 함수」) and `usage/process.rs:636` is already that form.
- Rejected (b) extension trait `m.lock_or_recover()` — one `use` line per file · a different name from std `lock` · out of keeping with ADR-0269 「거부한 대안」 (helpers behind a trait), so it needs explaining.

**S2 = (e).** Reason: today 116 of the 117 sites already recover silently, so the behavior stays the same down to the logs (pure de-duplication); attaching a warning would print while holding the guard and force us to exclude twelve locks (37 sites) where logging while held is forbidden, but that list widened at every review and no gate guards it (§2-4 historical record). What we lose = a signal that appears only in debug · test builds and that is absent today at 116 sites (operational is `panic = "abort"`). The move range widens from 46 → 83, which fulfills ADR-0269's purpose (scattered copies into one place) more fully.
- Rejected (a) a warning at every recovery (poison kept) — the wording of decision 4 as is, but it must carry the exclusion list (twelve · 37 sites), and once the poison is in, every lock pours out warnings.
- Rejected (b) once per poison (`clear_poison`) — at the seven mixed locks (§2-4) the behavior of the neighbors (`.expect` · `try_lock`) becomes tied to 「그 사이 복구가 돌았나」 (the shutdown of PTY · stdio `child` is the ADR-0001 kill causal path) · mixed with the excluded locks, within one crate some locks would clear and some would not.
- Rejected (c) once per site — it needs a macro and departs from decision 2 (「상태 없는 공개 함수」) · the exclusion list is still needed.
- Rejected (d) once per process, the first time — it holds a static flag, which is out of keeping with decision 2, and from the second poison on it is not visible at the default level · the exclusion list is still needed.
- Rejected 「놓고 경고하고 다시 잡는」 helper — it still prints under an outer lock held by the caller (§2-4).
- Accepted cost: it amends confirmed ADR-0269 decision 4 (ADR-0275) · to attach a warning later, the exclusion audit of all 83 sites is redone from scratch · shell `output_channel.rs` keeps ADR-0231's warning + poison clearing and stays in place (§3-5).

**N1 = (b).** Reason: command · transport use 「하네스 도우미 모듈을 연다」 as a name with the same meaning — the same name for the same meaning.
- Rejected (a) `testing` — the same as the module name, but it splits from the existing precedent.
- Rejected (c) `test-harness` — agent · daemon · messaging use 「운영 코드 안의 seam 을 연다」 with a different meaning.

**P1 = (a).** Reason: it is a draft name that ADR-0269 decision 2 and ADR-0270 decision 5 already wrote, so the wording of both ADRs still fits.
- Rejected (b) `normalize_typed_path` — the target shows in the name, but it conflicts with the wording of the two ADRs.
- Rejected (c) `tidy_typed_path` — same.

**Consequences of the tied items (묶인 것의 귀결):** C5=(b) · T1=(b) → transport does not depend on base, and only `ws.rs` changes to std (no G3). C1=(a) · C4=(a) → base `ManualClock` is an atomic-integer form that satisfies `Send + Sync`. S2=(e) → shell `output_channel.rs` stays in place and no disposition of the mixed locks is needed · 7 in-place sites instead of an exclusion rule (§3-5).

### 4-2. Amendments written into ADR-0275 (Amends ADR-0269 · ADR-0268)

> **Recorded (2026-10-04)** — ADR-0275 is **one ADR** carrying the 1-1 part (this table — the amendment of ADR-0269 + the amendment of the base lock poison recovery warning line in ADR-0268 「영향」) together with the sibling 1-3 part (amendments to ADR-0266 · ADR-0262 — TRD 1-3 §8). Number = **0275** (0274 is used by `origin/v0.3.3/feat/storage` — confirmed 2026-10-04 with remote `git ls-tree`. The last one on this branch is 0273). It was recorded by the orchestrator · `/adr` (no fabricated decisions — the rejected alternatives below · the reasons come from §4-1 and the measurements of this TRD). Basis for the status = user delegation 2026-10-04 (「알아서 진행해」 → recommendation adopted) + the measurements of this TRD (`0ef6292`). ★The body of ADR-0269 also gets an amendment link 「ADR-0275 가 결정 2 · 3 · 4 · 5 와 거부한 대안 한 줄을 고쳤다」★ — so that a session reading only the old ADR does not follow stale wording (warning log · transport subtrait · the fake in `testing`) (CLAUDE.md 「설계 결정 기록」 — status is bidirectional). Start order: record it before U1 (U1 leans on T1 · the name confirmation — §6-1).

| # | Place in ADR-0269 | What changes | Rejected alternatives | Reason |
|---|---|---|---|---|
| 1 | **Decision 4** (S2=e) | 「복구할 때 `tracing::warn!` 로 경고 로그를 남기는」 helper → **a helper that only recovers, with no warning (poison mark kept · no `clear_poison`)**. 「`sync` 가 `logging` 을 부르지 않아 무참조가 선다」 becomes even more true (there is no log at all). One exception = shell `output_channel.rs` (ADR-0231's warning + poison clearing — in place). The 「base 의 락 오염 복구 경고도 `tracing::warn!` 을 직접 부른다」 line of ADR-0268 「영향」 goes stale by the same decision (Amends ADR-0268) | (a) a warning at every recovery · (b) once per poison (`clear_poison`) · (c) once per site (macro) · (d) once per process · a helper that releases, warns, and takes the lock again | Operational builds are `panic = "abort"`, so the signal exists only in debug · test builds · a warning prints while holding the guard and forced us to exclude twelve locks (37 sites) where logging while held is forbidden, and that list widened with each of three reviews and no gate guards it · 116 of 117 sites already recover silently · (b) changes the meaning of the neighbors (`.expect` · `try_lock`) of the seven mixed locks · (c) · (d) depart from the 「상태 없는 함수」 of decision 2. Cost = to attach a warning again, the audit of all 83 sites is redone |
| 2 | **Decision 2 table, row `sync` · the 57 sites of 「근거」** | Absorbed targets = **83 operational sites** of the 117 measured (operational 90 · test-only 27). In place = 7 operational (base `logging` · `leftover.rs` `try_lock` · the lookup list helper · shell `settings/mod.rs` 3 (D2 after P3) · shell `output_channel.rs`) · command 3 (ADR-0267) · test-only 24 | — (number fixed — the value that 「근거」 deferred, saying 「1-1 착수 때 다시 센다」) | the count in §2-4 · §5 M3 |
| 3 | **The transport part of decisions 3-2 · 3-5** (C5=b) | We **defer to phase 3 (the transport attachment TRD)** the matter of transport having its own trait with the base trait as supertrait. In 1-1 transport does not depend on base and transport gate ① (exactly 1 line · ADR-0177) is also unchanged. Of the 「가짜 하나가 둘을 같은 상태로 구현」 in decision 3-2, only the discovery part stands in 1-1. Decision 3-5 (sleep with tokio · `tokio::time::pause` does not stop std `Instant`) remains as a fact and its application is in phase 3 | (a) attach now · (c) attach only the clock and use std for truncation | transport has zero consumers and phase 3 reopens the dependency and gates anyway · (a) is a gate expected-value change + the footprint of `tracing` · `tracing-subscriber` · `regex` (· `windows`) · in the same vein as ADR-0267 (connect only when needed) · transport's truncation is resolved with std, without base. Cost = a seam that departs from 「시간은 공용이면 무조건 합친다」 remains until phase 3 (written in the head of transport `clock.rs`) |
| 4 | **Decision 2 table, row `text`** (T1=b) | Remove `truncate_bytes` · `keep_tail_bytes` → `text` = only `hex_lower`. The three copies are replaced in place with std `floor_char_boundary` · `ceil_char_boundary`. The 「UTF-8 경계 자르기」 row of the 「맥락」 table remains as a fact | (a) two base functions (inside is one std line) | it is one std line of the pinned compiler 1.95, so there is nothing to wrap (the same reason as 「거부한 대안」 `dunce::canonicalize`) · the keep-back form is one copy and falls short of decision 7 |
| 5 | **Decision 3-1 · decision 5 · decision 2 table, row `testing`** (C4=a · N1=b) | Move the fake `Clock` from `testing` → **`#[cfg(any(test, feature = "test-support"))]` inside `time`**. The `testing` row is only `wait_until`. Feature name = `test-support` (the same meaning · same name as command · transport) | (b) put it in `testing` and make an exception in gate ③ · (c) do not put it in base / feature name `testing` · `test-harness` | Decision 5, decision 6, and gate ③ (tenant no-reference) collide — the fake in `testing` must implement `crate::time::Clock`. `test-harness` has a different meaning in this workspace: 「운영 코드 안의 seam 을 연다」 |
| 6 | **The seam list of decision 3 · the 「네 벌」 of the 「맥락」 table** (C2=a) | There are six seams (+ agent `LeftoverClock` · `CaptureClock`). **Merge `LeftoverClock` as a subtrait of base `Clock`** (`mono_now` → `now` · the fake stays an atomic integer). `CaptureClock` has no now-read, so there is nothing to merge. The storage branch's saver `Clock` is the job of D2 after P3 | (b) leave it | the user rule 「시간은 공용이면 무조건 합친다」 (2026-09-26) · all that changes is the name and the supertrait. Cost = it touches a file heavy with ADR-0262 invariants (leftover cleanup), even if only by a mechanical rename — a separate unit (U6) |
| 7 | **Decision 3-4** (C3=a) | `UsageClock` 「보류 — 1-1 때 깔끔히 맞으면 합치고 안 맞으면 묻는다」 → confirmed by measurement that it does not fit, **leave as is** (user delegation). The reason goes in the head of `usage_service/clock.rs` | (b) attach only the supertrait · (c) `mono` to `Instant` | the `Duration` of `mono` flows into a book-domain type and there are `Duration::MAX` · `ZERO` end-value tests · `wall` is signed epoch seconds · (b) is a shell that nobody calls `now()` through · (c) is a contract change of ADR-0250 · the usage TRD and exceeds the 1-1 scope |
| 8 | **「거부한 대안」 `sha256_hex` line** | Reason 「`sha2` 를 끌고 온다」 → **「사본이 하나라 결정 7 에 못 미친다」**. The conclusion (does not move in) stays. The hex loop inside calls `hex_lower` (T2=a) | — (factual correction) | it is a hand-rolled std implementation — `record.rs:247-251` comment · `grep -n sha2 crates/*/Cargo.toml src-tauri/Cargo.toml` → 0 lines |
| 9 | **Detail confirmation** (not an amendment — it fills in decision 2 「함수 이름은 가안」 · decision 3 「세부 모양은 1-1 착수 때 정한다」. Written as one line each in ADR-0275) | `Clock: Send + Sync` (C1=a) · `sync` = four free functions (S1=a) · names `hex_lower` · `normalize_spelling` confirmed (T2=a · P1=a) · the absorbed targets of `wait_until` are not 9 files but 11 copies (+ daemon `ws_e2e.rs` `poll_until` · `messaging_host.rs`) | C1 (b) no boundary · (c) two reads / S1 (b) extension trait / P1 (b) `normalize_typed_path` · (c) `tidy_typed_path` | each line of §4-1 |

- **What is not amended:** decisions 1 · 6 · 7 · 8 · 9 · 「영향」 (including the sentence that adds module names to gate ③) do not collide with these decisions. ADR-0177 (the transport gate) does not change because of C5=(b). ADR-0231 (the output Channel registry's warning + poison clearing) stands as it is and remains an exception to base `sync`.

---

## 5. Main-session decisions (2026-10-04 · ADR-0269 decision 7 — reported without asking)

> **Source: main-session decision (2026-10-04).** The five below fall under ADR-0269 decision 7 (the main session makes the tenancy decision and reports it to the user without asking), so the main session decided them rather than user delegation (§4). The proposals that were worker proposals up to edition 4 were adopted as they were — the content did not change.

- **M1 — test temp-folder helper:** 「태그 + PID + 시각으로 고유한 임시 폴더 경로」 has 9 copies (base `logging/mod.rs:448` · discovery `lib.rs:2619` · net `portfile.rs:94` · `instance.rs:349` · shell `settings/tests.rs:20` · `settings/store.rs:1193` · agent `backend/claude/session_file.rs:232` · `persistence/mod.rs:162` · `persistence/presets.rs:134`). Under decision 7 it is a tenancy target. **We do not put it in 1-1** (main-session decision 2026-10-04) — if net called `testing`, net's 「base 심볼 정확히 2줄」 gate would move (the test module is also inside `src/`), the two in the shell are files that storage fixes, and the copy in base `logging` cannot be used because of gate ③. Separately, after P3 · 1-3.
- **M2 — `wait_until` polling fixed at 20 ms** (the copies are 20 · 25 · 50 ms). It is not taken as an argument.
- **M3 — `sync` move range = 83 operational sites (S2=e).** Test-only ones (agent 19 · daemon 5) stay as they are — moving them gains nothing (there is no warning, so test output is the same too). If we decide to move them, add them to that unit — with no exclusion rule, nothing blocks it.
  - Count: 117 = operational 90 (agent 68 · daemon 8 · shell 13 · base 1) + test-only 27 (agent 19 · daemon 5 · command 3). Of the operational ones, 7 in place = base `logging` 1 · `leftover.rs:1664` `try_lock` 1 · the lookup list helper (`usage/process.rs:636`) 1 · shell `settings/mod.rs` 3 · shell `output_channel.rs` 1 (§3-5) → 90 − 7 = **83** (agent 66 · daemon 8 · shell 9).
  - Per unit: U2 17 (daemon 8 · shell 9) · U3a 24 (outside the agent codex link) · U3b 42 (codex `transport.rs` — including 26 state locks) · §6.
  - The numbers up to edition 4 (edition 1 85 → edition 2 80/81 → edition 3 47/48 → edition 4 46/47) are the record of the exclusion list of the warning-attaching option widening at every review. The difference of 37 between edition 4's 46 and 83 = the recovery sites of the twelve locks where logging while held is forbidden (agent 33 · daemon 4 — §2-4 historical record).
- **M4 — the five panic variants stay in place** (§2-5) — the label · panic · polling each have their own grounds.
- **M5 — the two persistence copies of `now_epoch_ms` are also changed in 1-1** — in those two files the only use is the corrupt copy name, so they would move again after P3 when `set_aside_corrupt` takes that time as an argument, but changing them now does not overlap with the storage branch (storage does not fix `crates/` — `git diff --stat <merge-base> origin/v0.3.3/feat/storage -- crates` → 0).

---

## 6. Work units

> **Common wording for the instructions (to the coder):** 「중간 어디서 멈춰도 빌드가 서게 순서를 짜라」 — **first add the new function to base (coexisting with the old copy) → replace call sites file by file → delete the old copy last.** The order that deletes the old data structure first is forbidden. For comments, inject the comment convention file of `/code-conventions`.
>
> **Point to roll back to:** the code tree is still the starting point `0ef6292` (the only untracked thing is this TRD folder). Before U1, make a local commit of this TRD to use as the starting point (§6-1), and after each unit's gates go green, leave a snapshot with a local commit.
>
> **Verification per unit (common):**
> 1. Run `cargo test --workspace -- --test-threads=4` **before and after** the unit and compare **both the `test result:` line count and the pass total** — a vanished target is silence, not failure (CLAUDE.md 「빌드·검증 명령」). Expected: line count = unchanged (new base tests are placed only as unit tests inside `src/` — adding a file to `tests/` gives +1) · passes = before + the number of new base tests (moved tests offset as −N on this side · +N in base). ★The starting figures were not measured★ — the last record is 59 lines · 3822 passed · 30 ignored on 2026-10-01 in CLAUDE.md (`cac9ee0`). Measure them when U1 starts (the command in §6-1).
> 2. `cargo test -p engram-dashboard-base -- --test-threads=4` · `cargo fmt --check` · base gates ①②③ (§7) · the `-p` tests of the changed crates (flag rules in CLAUDE.md 「병렬은 테스트 바이너리마다 걸린다」).
> 3. A unit that changes the shell runs `cargo test -p engram-dashboard --test lib_unit` and the relevant integration target (`--test layout_commands` etc.) separately.
> 4. Because source changes, `/qa` before commit (scope is set by the binding). **Behavior stays the same down to the logs** — the S2=(e) helper does not touch the poison mark and emits no logs, and the 83 sites we move already recovered silently (the only one of the 117 that emits a warning, `output_channel.rs`, stays in place). T1's std replacement gives the same result as the hand loop (§2-1). Operational builds are `panic = "abort"`, so they never reach the recovery branch. So there is no behavior change that needs a GUI measurement — except that U1's shell `layout/commands.rs` touches the cwd spelling of `agent.spawnInto` (behavior identical · regression net = the moved tests + `layout_commands`).
> 5. **Source-string tests (measured — we read all 21 places where `rg -n 'include_str!\(' crates src-tauri/src` reads source files and the one folder sweep of `settings/tests.rs:1321`):** what **breaks** under the 1-1 edits is the one `the_production_blocking_stdin_locks_are_counted` (`:7329-7375`) of codex `transport.rs` — U3b. What it touches but does not break: the order test of the handshake-failure branch of `manager.rs:3953` (the `:2751-2753` that U3b fixes is inside that branch — the needles it measures are only the order of `tracing::warn!(…headline())` and `deliver_link(`) · `manager.rs:6374` `squashed_production_body` (the needle `self.sessions.write()` is on the `.expect` side, so U3a does not fix it) · `profile.rs:1167` (U1's `now_millis` replacement does not touch the needles `p.epoch` · `replace_session_id(`). ★Edition 5 re-cross-check (the part widened by S2=e):★ among files with tests that read source, the only ones newly touched are codex `transport.rs` (26 state-lock sites) and `manager.rs` (`:550`), and the needles of those tests (`clip(` · `s.turn = TurnState::Idle` · `.close_turn_items(` · `ChildGuard(Some(child))` · `record_session_id(` · `hydrate_history(` · `HANDSHAKE_BUDGET` · `s.closed = true;` · `self.pending.close()` / `_reservation` of `manager.rs` · `spawn_fresh_settled` · `note_activation_result` · `.with_session_id_latch(` etc.) do not measure the lock recovery form. Leftover cleanup · queued-input registry · pending-list table · latch · turn observation table · the four daemon ones (`command_roster.rs` · `agent_conn.rs` · `usage_service/{watch,mod}.rs`) have no tests that read source. The `rg` gate of the CI · qa binding has no line that sweeps for the lock recovery form (`grep -n 'into_inner\|lock()\|poison' .github/workflows/ci.yml .claude/skill-bindings/qa.md` → 0).

| id | Scope (site count) | Files touched | Prerequisite |
|---|---|---|---|
| **U1** | base tenancy rule document (header · Cargo `description`) + `text::hex_lower` (three loops) + `time::now_epoch_ms` (three copies) + `path::normalize_spelling` (1 definition · 2 callers · 2 tests moved) + the three UTF-8 truncations to std (T1=b) | §6-1 is canonical (files · lines) — summary: base `lib.rs` · `Cargo.toml` · new `text.rs` · `time.rs` · `path.rs` / agent `commands.rs` · `profile.rs` · `persistence/{mod,presets}.rs` · `output_core.rs` / daemon `lib.rs` · `control/mod.rs` · `experiment/record.rs` / transport `ws.rs` / shell `layout/commands.rs` · `tests/layout_commands.rs` (comment) / gate ③ 6 places · CLAUDE.md · `docs/testing-strategy.md` · root `Cargo.toml` `:2` · `:12-13` | ADR-0275 recorded |
| **U2** | `sync` module (four functions) + daemon operational **8** + shell operational **9** = **17** · two in-place comments | base `lib.rs` · `Cargo.toml` (`description`) · new `sync.rs` · `logging/mod.rs` (in-place comment on `:129` — gate ③. ★Do not write the spellings `crate::` · `super::` + `sync` in that comment — gate ③ catches comments too★) / daemon `command_delivery.rs` (`:1008` — `lock_for_cleanup`) · `command_roster.rs` (`:401` — `lock_for_cleanup`) · `agent_conn.rs` (`:213` — `slot`) · `usage_service/mod.rs` (`:571` `save_lock` · `:606` `desk` · `:648` lookup guard field) · `usage_service/reject_store.rs` (`:276`) · `usage_service/watch.rs` (`:205` — `lock`) / shell `view_commands.rs` (7 — `:403 · 410 · 455 · 527 · 552 · 758 · 790`) · `daemon_client/mod.rs` (`:88`) · `ui_settings.rs` (`:768`) · `output_channel.rs` (in place + one 「ADR-0231 — 독을 걷는 자기 도우미, base `sync` 는 독을 걷지 않는다」 line comment) / gate ③ 6 places (`+sync`). ★The two pairs `lock_for_cleanup` (`command_roster` · `command_delivery`) keep the name · 「정리 경로 전용」 docs as they are and change only the body to `sync::lock` — they are pairs split on purpose from `lock()` on the `.expect` side, and (e) does not clear the poison, so that split still stands★ | U1 (base header) |
| **U3a** | agent `sync` — operational outside the codex link **24** · two in-place comments | `transport/input_queue.rs` 7 (including `wait_timeout` `:178 · 254` — `:263` is `#[cfg(test)] fn queued_bytes`, so it is not changed) · `transport/stdio.rs` 3 (`:194` stdin · `:317` · `:380`) · `transport/pty.rs` 2 (`:263 · 332` — `match` form) · `reaper.rs` 2 (`read` `:154-156` · `write` `:52-55`) · `session.rs` 2 · `manager.rs` 2 (the `expected` field of `session_id_sink` `:550` · `lock_name_allocation` `:1061`) · `backend/claude/leftover.rs` 2 (`Recorder.inner` `:402` · `GateCell.state` `:1713`) + 1 in-place comment (the `try_lock` of `:1662-1665` `Debug`) · `queued_input.rs` 1 (`:405`) · `inputs_pending.rs` 1 (`:47`) · `session_id_latch.rs` 1 (`:150`) · `turn.rs` 1 (`:145` `TurnObservations::lock`) · `usage/process.rs` (0 recovery sites — change the three `:696 · 985 · 1002` that call `StderrTail` to `sync::lock`, and fold the local helper `fn lock` (`:636`), which keeps only the lookup list's share, into `LookupGate::release` (`:352`) with a 「1-3 U5 로 간다 · platform 은 base 를 모른다(ADR-0266)」 one-line comment — worker proposal). ★Two leaf locks of leftover cleanup · queued-input registry · pending-list table · latch · turn observation table are 「쥔 채 로그 · 다른 락 · 밖 호출 금지」 locks — the (e) helper has no logging, no other lock, no outside call, so it stays within that discipline (§3-5 contract 2). The lock docs are not fixed★ | U2 |
| **U3b** | agent `sync` — `backend/codex/transport.rs` operational **42** + ★widening the source-count test★ | that one file. Operational section = before `:4284` (start of the test module). ★The 42 sites changed = state lock 26 (`SharedState` `:1079` — `lock.lock()` 25: `:499 · 1115 · 1144 · 1176 · 1572 · 2000 · 2175 · 2230 · 2348 · 2461 · 2638 · 2972 · 3176 · 3254 · 3334 · 3399 · 3524 · 3589 · 3645 · 3715 · 3921 · 3949 · 4079 · 4171 · 4227` + writer `wait_timeout` `:2192`) · `Announcer.order` 1 (`:1112`) · `Pending.entries` 5 (`:1298 · 1316 · 1328 · 1333 · 1338` — `:1349` is `#[cfg(test)] fn len`) · `stdin` 2 (`:1555 · 2752`) · `child` 2 (`:3889 · 4233`) · `stdout` · `stderr` · `open_params` · `link_sink` · `writer_handle` · `decoder` 1 each (`:3999 · 4009 · 4038 · 4052 · 4073 · 4092`)★. The 「★상태 락을 쥔 채 찍지 않는다★」 (`:3010-3015`) discipline is still kept — the helper does not print. ★`tests::the_production_blocking_stdin_locks_are_counted` (`:7329-7375`) counts the `.lock()` right after `stdin` and expects **exactly 2** (the deadlock guard — `write_line` `:1555` and the handshake-failure branch `:2751-2753`). If U3b changes those two to the new form it becomes 0 and goes red. **Widen the matcher to the new call form and keep the expected value 2 as it is** — match `stdin` after `sync::lock(`, skipping any `&` (in `write_line`, which takes a `&Mutex`, the call becomes `sync::lock(stdin)` · S1=a). Keep counting the old `.lock()` form too (if it comes back, that is blocking as well). 「숫자만 고치지 말 것」 is the instruction in that test's comment — lowering the expected value to 0 would make a blind guard that stays green even if a third blocking site appears. The comment of `:2741-2750` in the same file (「블로킹으로 이 락을 잡는 자리는 둘」) is still true★ | U2 |
| **U4** | `testing` module (`test-support` feature) + `wait_until` + 11 copies + dev dependency + operational graph gate (§7 G2) | base `lib.rs` · `Cargo.toml` (`[features] test-support = []`) · new `testing.rs` / agent `Cargo.toml` · `tests/{activation,transport_smoke,session_smoke,stdio_smoke,headless,backend_contract,reaper}.rs` / daemon `Cargo.toml` · `tests/{control_send,mcp_manager_lifecycle,ws_e2e}.rs` · `src/messaging_host.rs` / `ci.yml` · CLAUDE.md · qa binding · gate ③ 6 places (`+testing`) | U1 |
| **U5** | `time::Clock` (`Send + Sync`) + `SystemClock` + `ManualClock` (behind `test-support` · C4=a) + daemon `command_delivery` + discovery + head comments for the two clocks that do not merge | base `time.rs` · `Cargo.toml` (`description`) / daemon `command_delivery.rs` (delete `Clock` `:176` · `SystemClock` `:181` → base · test `ManualClock` `:2425` → base) · `connection_core.rs` (`:4403` tests) · `usage_service/clock.rs` (head comment — C3=a: 「base `time::Clock` 과 합치지 않는다 — `mono` 의 `Duration` 계약 · 부호 있는 epoch 초 · ADR-0275」) / discovery `lib.rs` (`Clock` `:431` → base subtrait + blocking `sleep` · the fake `FakeClock` `:1742`'s `RefCell` · `Cell` to a form that is `Sync` — C1=a) · `Cargo.toml:12-15` (base dependency comment — the reason as 「base `time::Clock` 상위 트레이트」. 1-3 U1 moves the 「liveness 판정 공유」 reason of that line to the platform line, so if this unit does not write it, the base line has no reason — if 1-3 lands first, add to it; if 1-1 is first, add it beside the 「liveness」 reason · round 3 review) / transport `src/clock.rs` (head comment — C5=b: 「3단계(transport 부착)에서 base `time::Clock` 의 하위 트레이트로 — ADR-0275」 · the code is not changed) | U4 (`test-support` feature) · U2 (`command_delivery.rs` overlap) |
| **U6** | make `LeftoverClock` a subtrait of base `Clock` (C2=a) — `mono_now` 22 sites → `now` | agent `backend/claude/leftover.rs` only (`LeftoverClock` `:953` · operational `SystemClock` `:961` · the fakes `FakeClock` `:4664` · `SkipClock` `:6586` stay in their own place as atomic integers — they implement base `Clock` as well). ★Rewrite the 「문 자물쇠 안에서도 부른다 — 다른 자물쇠를 잡거나 기다리지 않는다」 (`:954`) contract over to `now`★ | U5 |

- **Order:** U1 → U2 → (U3a ∥ U3b) → U4 → U5 → U6. The workspace builds and is green after stopping at any unit — because each unit adds the new function and then deletes the old copy within that unit.
- **Size (by site count — file overlap is the axis):** U3b is a mechanical replacement of 42 sites in one file + one test, so it is bundled into one worker (splitting would overlap on the same file). U3a is 24 sites over twelve files — the files do not overlap each other, so it could be split in two, but each part is small, so we propose bundling (worker proposal).
- **File overlap (for judging parallelism):**
  - base `lib.rs` · `Cargo.toml` · gate ③ lines (6 places) — U1 · U2 · U4 touch them and U5 touches only `description` → **serial.**
  - daemon `command_delivery.rs` — U2 (1 recovery site) · U5 (clock · fake) → serial.
  - agent `backend/claude/leftover.rs` — U3a (2 recovery sites · comment) · U6 (clock) → serial.
  - U3a and U3b do not overlap in files → can run in parallel (same crate, so simultaneous builds in one worktree block each other — whether to isolate worktrees is the orchestrator's call).
  - agent · daemon `Cargo.toml` — U4 only.
  - discovery `lib.rs` · `Cargo.toml` — U5 only. transport — U1 (`ws.rs`) · U5 (`clock.rs` comment).
- **Files overlapping with the sibling TRD (1-3 platform) — we recommend not running 1-1 and 1-3 at the same time:** base `lib.rs` · `Cargo.toml` · gate ③ (1-3 removes `platform`) · agent `transport/pty.rs` · `transport/stdio.rs` · `backend/codex/transport.rs` (1-3 = the Job handle branch · 1-1 = U3) · agent `usage/process.rs` (1-3 = U3 · U5 — the OS layer of the usage query, including the lookup list · 1-1 = the three `StderrTail` of U3a + the in-place comment) · agent `backend/claude/leftover.rs` (1-3 = U3 · 1-1 = U3a · U6) · agent `manager.rs` (1-3 = OS rule :77 · 1-1 = U3a) · discovery `lib.rs` (1-3 = `wmi_spawn` move · 1-1 = U5 — ★1-3 does not remove discovery's base dependency (`Cargo.toml:15`): after U5 `time::Clock` is its use · §3-3★) · discovery `Cargo.toml:12-15` (1-3 U1 = the 「liveness 공유」 reason moved to the platform line · 1-1 U5 = the `time::Clock` reason on the base line — round 3 review) · root `Cargo.toml` member comments `:2` · `:12-13` (1-1 = U1 fixes the tenancy rule sentence · 1-3 = fixes again the part left after `platform` drops out — handed over) · daemon `lib.rs` (1-3 = executable name branch :109 · 1-1 = U1 hex) · agent `persistence/{mod,presets}.rs` · daemon `usage_service/reject_store.rs` (1-3's `write_atomic` — but that side is after P3).

### 6-1. U1 start checklist

0. **Two preconditions.** ① Record ADR-0275 first — ★recorded on 2026-10-04 (one ADR with the 1-3 part)★ (orchestrator · `/adr` — §4-2. U1 leans on its #4 (T1 — remove the two truncation functions from the `text` table) and #9 (name confirmation), and since the base head points to ADR-0269, the amendment must stand first for the anchors to fit). ② A point to roll back to — this TRD folder is untracked (`?? docs/process/S21-crate-boundaries/`). Before starting, make a local commit to create the starting point (the code tree is still `0ef6292`).
1. **Starting figures (before touching code).** From the workspace root:
   ```
   cargo test --workspace -- --test-threads=4 > <scratch>/u1-before.txt 2>&1
   awk '/^test result:/{n++; for(i=1;i<NF;i++){if($(i+1)~/^passed/)p+=$i; if($(i+1)~/^failed/)f+=$i; if($(i+1)~/^ignored/)g+=$i}} END{print n" 줄 · "p" 통과 · "f" 실패 · "g" 무시"}' <scratch>/u1-before.txt
   ```
   Write down the result line count and pass total (+ failed · ignored) as they are. If they differ from the last record (CLAUDE.md 2026-10-01 `cac9ee0` = 59 · 3822 · 0 · 30), write only the difference — the starting point is master after that. If the terminal dies, run it with `scripts/run-detached.ps1` (qa binding 「분리 실행」).
2. **Files to touch (exactly):**
   - base — `crates/engram-dashboard-base/src/lib.rs` (head: add `text` · `time` · `path` to the tenant list · tenancy condition ① per ADR-0269 decision 7 · rewrite the `:15-19` 「셋째 입주자를 받기 전에 ADR-0175 재론」 paragraph as answered by decision 1 · `pub mod text; pub mod time; pub mod path;` · gate ③ `:40` · `// ADR-0269`) · `Cargo.toml:5` (`description`) · new `src/text.rs` (`hex_lower` + unit tests) · new `src/time.rs` (one `now_epoch_ms` — `Clock` is U5 · unit tests) · new `src/path.rs` (`normalize_spelling` + the two tests moved over from agent · a contract doc with the domain words removed).
   - agent — `src/commands.rs` (delete `normalize_cwd` `:1143-1153` · callers `:1106` → base · move tests `:2091` · `:2106` to base `path.rs` · the 「두 문이 같은 철자」 tests after `:2114` stay · domain reasons such as 「사람 · LLM 이 친 cwd」 · ADR-0122 go into comments at the call sites) · `src/profile.rs` (delete `now_millis` `:20-25` · `:244` · `:819` → `now_epoch_ms`) · `src/persistence/mod.rs` (delete `:25-30` · `:89`) · `src/persistence/presets.rs` (delete `:22-27` · `:81`) — if the `use std::time::{SystemTime, UNIX_EPOCH};` (`profile.rs:12` · `mod.rs:11` · `presets.rs:12`) of the three files lose their use, delete them together · `src/output_core.rs:1059-1062` (`ceil_char_boundary` — §3-2).
   - daemon — `src/lib.rs:77-80` (loop inside `generate_token` → `hex_lower`) · `src/control/mod.rs:146-149` (loop inside `gen_token`) · `src/experiment/record.rs:178-186` (`cap_response` → `floor_char_boundary`) · `:192-195` (loop inside `sha256_hex` → `hex_lower`). The 「공용화 금지 · 혼용 금지」 comments of the two token generators (`lib.rs:584-585` · `control/mod.rs:140-141`) stay as they are — the generators are not mixed.
   - transport — `src/ws.rs:147-156` (`clamp_close_reason` → `floor_char_boundary`). `Cargo.toml` is not touched (no base dependency — C5=b).
   - shell — `src-tauri/src/layout/commands.rs:38` (import → `engram_dashboard_base::path::normalize_spelling`) · `:1127` (`verb_spawn_into`) · `src-tauri/tests/layout_commands.rs:2121` (the name in a comment). The shell already depends on base (`src-tauri/Cargo.toml:61`).
   - docs · gates — gate ③ regex, 6 places, to `(crate|super)::(logging|platform|text|time|path)`: base `lib.rs:40` · `CLAUDE.md:249` · `.github/workflows/ci.yml:644` · `.claude/skill-bindings/qa.md:138` · `docs/testing-strategy.md:50` · `:155`. `CLAUDE.md` 「백엔드 모듈 맵」 base item (tenants · tenancy condition · 「셋째 입주자를 받기 전에 ADR-0175 … 다시 연다」 sentence) · 「빌드·검증 명령」 base test line description. `docs/testing-strategy.md:43` (unit test list) · `:46` · `:138`. root `Cargo.toml:2` · `:12-13` (base description · 「셋째 입주자는 ADR 재론」). ★CLAUDE.md · the qa binding · testing-strategy are load-bearing documents — attach the doc lens to the review of that unit★. ★Do not write, in new module heads or comments, a spelling of `crate::` · `super::` followed by a tenant name — the gate ③ regex catches comments too (e.g. if you write 「`crate::time` 을 부르지 않는다」 that line gets caught)★.
3. **Order (so the build stands even if we stop):** three new base modules + tests → call sites file by file (agent → daemon → transport → shell) → delete the old copies (three `now_millis` · `normalize_cwd`) → docs · gate ③.
4. **Gates (after U1):** base ① (`cargo tree -p engram-dashboard-base --depth 1 --prefix none -e normal,dev,build --target all --all-features | rg "^engram-dashboard" | sort -u` → exactly 1 line) · ② (`rg "^\s*use tauri" crates/engram-dashboard-base/src/` → 0 lines) · ③ (new regex → 0 lines · path existence first — qa binding 4b-pre) · transport gate ① (same form `-p engram-dashboard-transport` → exactly 1 line — we touched `ws.rs`, so look again) · `cargo fmt --check` · `cargo test -p engram-dashboard-base -- --test-threads=4` · `cargo test -p engram-dashboard-agent -- --test-threads=4` · `cargo test -p engram-dashboard-daemon -- --test-threads=4` · `cargo test -p engram-dashboard-transport` and `--all-features` (the same two combinations as CI transport gates 4a · 4b) · `cargo test -p engram-dashboard --test lib_unit` · `--test layout_commands`.
5. **Figures (after U1):** run the command in step 1 again and compare with `<scratch>/u1-after.txt` — expected: result line count = same as before · passes = before + the number of new base unit tests (the two moved `normalize_cwd` tests offset as agent −2 · base +2) · failures 0 · ignored the same. If the line count drops, a target vanished — stop even if green.
6. `/qa` (source change) → commit after the gates go green (`S21: refactor(base): …` — the step number continues the step-log · Co-Authored-By trailer at the end).

---

## 7. Gate · document change list (this TRD does not edit them — list only)

| id | What | Where | When |
|---|---|---|---|
| G1 | Add the new module names to the tenant no-reference regex — `(crate\|super)::(logging\|platform)` → `(…\|text\|time\|path)` (U1) → `+sync` (U2) → `+testing` (U4). The unit that creates a module adds its name (so the gate names only tenants that really exist). 1-3 removes `platform` | `crates/engram-dashboard-base/src/lib.rs:40` · `CLAUDE.md:249` · `.github/workflows/ci.yml:644` · `.claude/skill-bindings/qa.md:138` · `docs/testing-strategy.md:50` · `:155` (two copies found by the round 1 review — 6 places) | U1 · U2 · U4 |
| G2 | **New gate (proposal):** base's test feature is not in the operational graph — `cargo tree --locked -p engram-dashboard-daemon -e normal,features -i engram-dashboard-base --target all \| rg 'feature "test-support"'` → 0 lines, `-p engram-dashboard` (shell) the same. As a pair, to prevent a blind gate: `-e normal,dev,features -p engram-dashboard-agent` → 1 line or more (because if the feature name is changed the expectation of 0 passes silently — the same grounds as transport gate ⑤). The `time::ManualClock` behind the same feature (C4=a) is also covered by this gate | `ci.yml` new step · `CLAUDE.md` 「빌드·검증 명령」 · qa binding | U4 |
| — | **What does not change:** base gate ① (exactly 1 line) · ② (`use tauri` 0 lines) · transport gate ① (exactly 1 direct workspace dependency line — because of C5=b, edition 4's G3 (1 → 2 lines) is gone · the phase 3 TRD looks at it again) · net's base symbols, exactly 2 lines (net does not newly call base in 1-1) · the command · messaging cap gate (ADR-0267) · transport gate ③ (the three pure layers take the clock as an argument) · the base CI test step `ci.yml:276-278` (`testing` is `cfg(any(test, …))`, so no feature argument is needed) | | |

Documents (in the same change):

- base `lib.rs` header — tenant list · tenancy condition ① per ADR-0269 decision 7 · tidy the 「셋째 입주자 재론」 paragraph · gate ③ names · `// ADR-0269` anchor (U1).
- base `Cargo.toml` `description` (U1 · afterwards each unit adds its module).
- root `Cargo.toml` member comments — `:2` 「base(바닥 인프라 — 로깅·PID 판정, 잎 crate, ADR-0175)」 · `:12-13` 「ADR-0175 결정 1: 로깅 + PID 판정만 담는 잎 crate … 셋째 입주자는 ADR 재론을 거친다」 (U1 — the paragraph that reopens the question in base `lib.rs:15` · the same sentence as the CLAUDE.md base item, so it goes stale in the same change. Round 2 review). ★1-3 removes `platform` from base and fixes the 「PID 판정」 part again★ — 1-1 fixes only the tenancy rule · the new-tenant part and hands the rest over to 1-3.
- `CLAUDE.md` 「백엔드 모듈 맵」 base item (tenants · tenancy condition) · 「빌드·검증 명령」 base test line description (U1) · G2 (U4).
- `docs/testing-strategy.md` base section — `:43` unit test list (new modules after `logging` · `platform`) · the base test line descriptions in `:46` · `:138` are copies of that line in CLAUDE.md, so they are fixed together in the same change (U1 · in each unit that adds modules). The flag grounds of those two lines (「`platform` 의 자식 PID 테스트가 실 `cmd.exe` 를 띄운다」) are not changed by 1-1 — that is the job of 1-3, which removes `platform` from base.
- For copies left in place, one 「왜 base 를 안 쓰나」 line — the four in-place `sync` (base `logging` · the `try_lock` of `leftover.rs` · the lookup list helper · shell `output_channel.rs` — §3-5. Shell `settings/mod.rs` gets no comment because the file is not touched) · the two clocks that do not merge (transport `clock.rs` — C5=b · daemon `usage_service/clock.rs` — C3=a · both U5).
- **ADR-0275** (orchestrator · `/adr` — before U1 · ★recorded on 2026-10-04★): the table in §4-2 is canonical. Amendment links were attached to the bodies of ADR-0269 · ADR-0268.

---

## 8. Deferred

| id | What | Reason · trigger |
|---|---|---|
| D1 | `file::set_aside_corrupt` unification (agent `persistence/{mod,presets}.rs` · shell `settings/store.rs` `set_aside`) | storage P3 changed that behavior rule (ADR-0274 — remote branch) · shell `fsutil.rs` · `settings/` add file writes → after P3 lands on master (memo §10 way of proceeding) · ★→ TRD A (`trd-A-data-file-unification.md`) closed it — base `file::copy_aside` (the ADR-0274 rule in four files · ADR-0291 R9 · U2 · U5a)★ |
| D2 | Sweep-up after P3 lands — shell `settings/mod.rs` 3 recovery sites · `settings/store.rs:122` epoch ms (`u128`) · storage's `state/saver.rs` `Clock` (`now` + `wall_ms`) · `SystemClock::wall_ms` (`u64`) · `state/` 1 recovery site | The same trigger. The saver `Clock` must add `Sync` to match the `Send + Sync` of base `Clock` (C1=a) · the 3 shell `settings/mod.rs` sites all move because of S2=(e) (the 「잎」 discipline of `:486` does not apply either) · ★→ TRD A U8 closed it — the 12 shell lock poison recovery sites go to base `sync` · the saver `Clock` becomes a subtrait of base `Clock` (ADR-0291 decision 14) · the `settings/store.rs:122` epoch ms was already gone, because ADR-0274 had removed the timestamp name before that (TRD A §2-3)★ |
| D3 | Several copies of `write_atomic` → platform | ADR-0266 decision 8 · 1-3 · after P3 · ★→ TRD A closed it — its home is not platform but base `file::write_atomic` (retry · folder sync is handed to `OsHooks` by the caller · ADR-0291 decisions 1 · 3 amend ADR-0266 decision 8)★ |
| D4 | transport clock and transport's base dependency (C5=b) — truncation ends with std in U1 | phase 3 (transport attachment) TRD — ADR-0275 #3 |
| D5 | messaging XML escape · command `testing.rs` 3 recovery sites | ADR-0267 — connect when it becomes necessary |
| D6 | `UsageClock` (C3=a) | when an occasion to merge arises (e.g. when looking again at book's time type) |

---

## 9. Seen in passing (memo §11 accrual candidates — the orchestrator moves them)

| What | Where | When to handle |
|---|---|---|
| The exclusion reason 「`sha2` 를 끈다」 of `sha256_hex` is false — it is a hand-rolled std implementation | `daemon/src/experiment/record.rs:189,247-251` · ADR-0269 「거부한 대안」 | ADR-0275 (§4-2 #8) |
| The clock seams are six, not four (+ storage seven) | §2-2 | ADR-0275 (#6) · U6 · D2 |
| `normalize_cwd`'s `\` → `/` runs on every OS — on POSIX `\` is a character allowed in file names (there is no branch, so it is not a platform candidate) | `agent/src/commands.rs:1143` | at the macOS port (ADR-0230) |
| 9 copies of the test temp-folder helper | §5 M1 | after P3 · 1-3 |
| base `logging/mod.rs:155` also reads epoch (log file name) — because of gate ③ it cannot call `time` | base | as is (record of fact) |
| The warning of lock poison recovery is not reached in operational builds (`panic = "abort"`) | `Cargo.toml:34-35` | the basis of S2=(e) (ADR-0275 #1) |
| The head of the first-submit latch, 「래치 뮤텍스를 쥔 채 … 로그도 놓은 뒤에 쓴다」, is true only for the latch's own logs — the commit port emits `warn!` · `info!` · `debug!` while holding the latch, and the registry · store below it also log (the store even writes to disk) | `session_id_latch.rs:23` · `manager.rs:541-569` · `profile.rs:717` · `persistence/mod.rs:101-106` | when fixing the latch doc (outside 1-1 — the S2=(e) helper emits no logs, so it adds nothing to this mismatch) |
| The queued-input registry says 「★이 가드를 쥔 채 emit·IO·다른 락을 잡지 말 것★」, but the branch that rewrites `Queued` after sealing emits `debug!` while holding that guard (and the replay lock) | `queued_input.rs:399` · `output_core.rs:558-570` | when fixing that place (outside 1-1) — an example of the limit of the warning-attaching option leaning on documented discipline (the basis of ADR-0275 #1) |
| The codex head allows 「락 순서 = 상태 → 대기표」, but no operational site takes the locks that way (the destructor deliberately releases first and takes afterward) | `backend/codex/transport.rs:21` · `:2966` | irrelevant to 1-1 (S2=e) — if the recovery warning is attached again it becomes a latent edge (§2-4) |

---

## 10. Basis · verification status

**Measuring commands (all `0ef6292`, 2026-10-03):**

- UTF-8 truncation: `rg -n "is_char_boundary|floor_char_boundary|ceil_char_boundary" crates src-tauri/src --glob '*.rs'`.
- std boundary functions: scratch `fcb.rs` (`"가나다".floor_char_boundary(4)` · `ceil_char_boundary(4)`) → `rustc --edition 2021` compile · run output `3 6` (rustc 1.95.0).
- hex: `rg -n '02x\}|:02x' crates src-tauri/src --glob '*.rs'` · `grep -n sha2 crates/*/Cargo.toml src-tauri/Cargo.toml` (0 lines).
- epoch ms: `rg -n "fn (now_epoch_ms|now_ms|…|now_millis)\b"` · `rg -n "UNIX_EPOCH" crates src-tauri/src` · `rg -n "now_millis\(\)"`.
- clock: `rg -n "trait (Clock|UsageClock|\w*Clock)\b" crates src-tauri/src` · `rg -n "impl\b.*\b(Clock|UsageClock|\w+Clock) for\b"` · `rg -n "mono_now\(" crates/engram-dashboard-agent/src | wc -l` (22) · `rg -n "\.mono\(\)"` (11) · `rg -n "\.wall\(\)"` (13).
- lock poison: ADR-0269 counting method `git grep -h -o -E '\.lock\(\)\s*\.unwrap_or_else\(\|[a-z_]+\|\s*[a-z_]+\.into_inner\(\)\)' HEAD -- crates src-tauri/src | wc -l` (67) · `rg -c 'PoisonError::into_inner' crates src-tauri/src` (total 27). The 117 was counted with a scratch Python script (multi-line regex — closure and function-pointer forms of `unwrap_or_else` after `.lock|read|write|wait_timeout(...)` 109 + `match` branch 6 + block closure 1 + `try_lock` 1). The same 109 can also be reproduced with `rg -U -o`.
- poison assertions: `rg -n "is_poisoned|clear_poison" crates src-tauri/src`.
- mixed locks (after the round 1 review): a scratch Python script — extract the receiving field names from the recovery sites in the operational section, then in the same crate's operational code look for `.lock()/.read()/.write()` + `.expect(`/`.unwrap(` after that name. Other people's structs that only share the field name (codex `child` · `stdout` versus stdio · `self.inner` versus `connection_core` etc.) were filtered out by hand. The `try_lock` neighbors were examined separately with `rg -n 'try_lock\(\)' crates/engram-dashboard-agent/src`. Because it is a name-equality comparison, it cannot catch sites that hold the same lock under a different name (a cloned `Arc` received in a variable of another name) — the `watcher_child` of the PTY watcher thread was found by hand.
- Locks where logging while held is forbidden · nesting (up to edition 4 — dropped from the move rule by S2=(e) · §2-4 historical record): we read the widened wording `rg -n '찍지 않는다|놓은 뒤 (찍|부른)|락 밖에서|잠금 밖에서|쥔 채.{0,40}(로그|IO|emit|찍|보내)|로그도 놓은|잎이다' crates/engram-dashboard-agent/src crates/engram-dashboard-daemon/src src-tauri/src` (74 lines) one by one and, for each remaining operational site, read the lock definition · the doc of the acquiring helper (bold text · English `leaf` slip past the regex) · from the documented lock order `rg -n '락 순서|lock order|Lock order' crates/engram-dashboard-agent/src crates/engram-dashboard-daemon/src src-tauri/src --glob '*.rs'` (33 lines) we extracted the edges where an excluded lock is outer and read the guard span and the callbacks · ports · clocks inside it · codex state lock sites = the lines before `:4284` (start of the test module) among `rg -n -U '(\.lock\(\)|wait_timeout\([^;]*?\))\s*\.unwrap_or_else\(' crates/engram-dashboard-agent/src/backend/codex/transport.rs` · we confirmed in the 25 lines before each site that its `lock` comes from `SharedState` · operational subscriber `rg -n 'set_global_default|\.try_init\(\)|\.init\(\)|registry\(\)' crates src-tauri/src`. Operational/test reclassification = a scratch script (for each recovery form, `#[cfg(test)]` · `#[cfg(any(test` above the wrapping `fn`) — we checked codex `:1348` · `input_queue.rs:262` by eye. All of it is reading.
- Source-string tests: `rg -n 'include_str!\(' crates src-tauri/src --glob '*.rs'` (21 places, excluding generated output · docs) · we found the tests that read source files with `rg -n 'CARGO_MANIFEST_DIR' crates src-tauri` and read each test's needles.
- waiting in tests: `rg -n "fn (wait_until|poll_until|wait_for|eventually|spin_until|wait_while)\b" crates src-tauri --glob '*.rs'`.
- operational graph: `cargo tree --locked --offline -p engram-dashboard-daemon -e normal,features -i engram-dashboard-base --target all` · `… -p engram-dashboard-agent -e normal,dev,features -i engram-dashboard-command`.
- storage overlap: `git diff --stat $(git merge-base HEAD origin/v0.3.3/feat/storage) origin/v0.3.3/feat/storage -- crates src-tauri/src` after `git fetch` · a grep of the `+` lines of the same diff.
- transport gates: `crates/engram-dashboard-transport/src/lib.rs:31-85` · reading of `.github/workflows/ci.yml:686-760`.

**Edition 5 re-cross-check (2026-10-04 · code tree still `0ef6292`):**

- ADR number: `git ls-tree --name-only <브랜치> docs/decisions/ | grep -E '027[4-9]'` on each remote branch → only 0274 of `origin/v0.3.3/feat/storage` · the last one on this branch is 0273 → new number 0275.
- Source-string tests (the part widened by S2=(e)): of the 21 places above, we re-extracted the `split/find/contains/matches` needles of codex `transport.rs` (`:5029 · 5631 · 6641 · 6702 · 7076 · 7360`) · `manager.rs` (`:3960 · 4289 · 4319 · 4348 · 4439 · 4473 · 6250 · 6374 · 6512`) · daemon `command_delivery.rs:4779` · `connection_core.rs:2532 · 3166` · `profile.rs:1167` and checked them against the lock recovery form — the only one that measures lock recovery is `:7360` (stdin count).
- The 6 sites of the `match` form: `rg -n -U --pcre2 'Err\((\w+)\)\s*=>\s*\1\.into_inner\(\)' crates src-tauri/src --glob '*.rs'` · `wait_timeout` = operational 3 (`input_queue.rs:178 · 254` · codex `:2192`) + test 1 (codex `:10176`) — `rg -n -U 'wait_timeout\([^;]*?\)\s*\.unwrap_or_else'`.
- MSRV of the std boundary functions: `grep -rn rust-version Cargo.toml crates/*/Cargo.toml src-tauri/Cargo.toml` → 0 lines (no crate declares a low compiler floor — the only pin is `rust-toolchain.toml` 1.95.0).
- base dependency: `grep -n engram-dashboard-base crates/*/Cargo.toml src-tauri/Cargo.toml` → agent · daemon · discovery · net (optional) · shell (`src-tauri/Cargo.toml:61`). transport has none.

**Where memo line numbers shifted (memo → now):** agent `output_core.rs:716` → `:1059-1062` · agent `commands.rs:763` → `:1143` · discovery `lib.rs:425` → `:431` · daemon `tests/control_send.rs:70` → `:76` · daemon `lib.rs:65` → `:73-82` · daemon `control/mod.rs:141` → `:142-151`.

**Unverified (claims in this document that were not confirmed by running):**

- The starting figures of the workspace regression (`0ef6292`) — not measured (when U1 starts — §6-1).
- Whether `use` is needed when calling a supertrait method through a `dyn` subtrait — not compiled.
- We did not run S2=(e) — that all 83 sites compile with that helper, and that leaving `output_channel.rs` in place keeps `:145-148` green, are both from reading. (e) does not add logs, so there is nothing to overlap with the log-capture tests.
- T1's std replacement — the `fcb.rs` measurement is the function's behavior. The behavior after changing the three sites is measured by U1's tests.
- The completeness of the audit of locks where logging while held is forbidden (up to edition 4) — it was reading, and the wording may have missed other disciplines. With S2=(e) this audit dropped out of the move rule — it becomes a problem again only when the recovery warning is attached again (§2-4 · ADR-0275 #1).
- The mixed-lock table (§2-4) was found by field-name equality — the same lock received under a different name was found only by hand, so some may be missing.
- The operational/test split (§2-4) is a script estimate + hand corrections.
- The storage branch facts are as of `5a2cf7f` — measured again when P3 lands.
