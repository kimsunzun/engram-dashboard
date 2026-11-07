# ADR-0284: The usage-query rejection deadline is kept in memory only (remove usage_rejects.json)

- 상태: 확정 (2026-10-08, basis: user decision + implementation `9ab1523` (2026-10-08 · `/review code light` PASS (1 re-fix) · `/qa quick` PASS — light by the user's instruction · numbers · items = step-log))
- 관련: Amends ADR-0264 (the rejection record file of decision 1) · PRD `docs/process/S21-usage-limit-slot/prd.md` R24 · R29 · TRD `docs/process/S21-usage-limit-slot/trd.md` §3 #30 · the section 「거절 저장(R29)」 · TRD `docs/process/S21-storage/trd.md` §2-1 · survey `docs/research/usage-reject-persistence-2026-10-08.md` · `crates/engram-dashboard-daemon/src/usage_service/reject_store.rs`

## 맥락

The daemon queries claude · codex usage for the usage slot. When upstream rejects with 429, it sets a rejection deadline (`reject_until`), and within that deadline it issues no automatic or ⟳ query (R24 — a `Retry-After` of about 1 hour was observed on claude). Today, to carry that deadline across a daemon restart, it is saved separately in `daemon\state\usage_rejects.json` (`RejectStore` · `FileRejectStore` — TRD §3 #30).

That persistence was not a user decision. PRD review round 2 pointed out 「재시작하면 거절 기한을 잃는다」, and it was put in as 「리뷰 반영 기본값 — 확인 필요」 (PRD review record). During the storage-management cleanup, the user looked at this file and asked whether it was excessive.

## 결정

1. **The rejection deadline is kept only in daemon memory.** `usage_rejects.json` is neither written nor read — `reject_store.rs` (`RejectStore` · `FileRejectStore`) and the rejection-save wiring in assembly · the completion guard are removed.
2. **After a daemon restart, it starts with no rejection deadline.** The other rules of the rejection deadline (R24 — no automatic or ⟳ query within the deadline · hidden rejection · folding when the rejection ends) stay as they are.
3. **The old file is not deleted.** A `usage_rejects.json` already on disk is only left unread and kept as it is — this is an application of ADR-0264 decision 5, which does not touch old data (main session judgment — no deletion code is added).

## 거부한 대안

- **B. Keep the current state (a dedicated file per feature)** — only the code is complicated (the user). There is also no peer precedent of saving just the deadline in a dedicated file (survey).
- **C. Gather the daemon's small runtime state into one file and make the rejection deadline a field in it** — a shared storage layer would have to be newly built, and there is no other state to go in with it now (code fact — the usage values are not saved either). The user saw no need for saving itself.

## 근거

- User decision (2026-10-08): 「구지 저장까지 할 필요는 없다고 생각. 코드만 복잡함」.
- Survey (`docs/research/usage-reject-persistence-2026-10-08.md` · light): of four peers that call a usage endpoint and handle 429, two use memory (orca · ClaudeBar) and two persist (CodexBar — a general key-value store · Claude-Code-Usage-Monitor — a field of the value cache file). There is no dedicated file that holds only the deadline. orca chose memory while knowing the same 1-hour window (`src/main/rate-limits/service.ts:91`).
- The daemon lives longer than the shell so restarts are rare, and the usage values themselves are not carried across a restart either.

## 영향 / 불변식

- **Cost (accepted):** if the daemon is restarted within the rejection deadline, a query may go out once more and receive 429 again. Whether that re-query increases upstream rejections has never been measured.
- The files to keep in the data folder `daemon\state\` become two, `agents.json` · `presets.json` (an amendment of the rejection record file of ADR-0264 decision 1).
- When the storage method is raised again, look first at alternative C (into one place) — files are not added per feature (the user).
