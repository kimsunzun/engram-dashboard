# ADR-0246: The usage limit is daemon-owned per-(company, account) state, and vendor policy comes from the backend registry

- 상태: 확정 (2026-09-27, basis: user decision + TRD §7 #1)
- 관련: `docs/process/S21-usage-limit-slot/trd.md` §7 #1 · CLAUDE.md 「백엔드 확장」(ADR-0004) · step-log S21 usage-limit slot

## 맥락
We had to decide which layer owns the usage limit (the remaining usable amount per vendor, such as Claude/Codex), and where to put vendor-specific knowledge (query method, cooldown, etc.).

## 결정
The usage limit is **daemon-owned state per `(회사, 계정)`**. Vendor policy (query method, cooldown, etc.) comes from the backend registry — following the 「백엔드 확장」 principle (ADR-0004) as is.

## 거부한 대안
- **A per-agent `OutputEvent` variant** — it is per-agent and subject to replay, so its lifetime differs from usage (the usage limit lives on even when the agent dies).
- **Shell ownership** — multiple windows would each count one account's usage separately (duplicate queries · mismatch).
- **Daemon-side vendor constants · enum** — vendor-specific knowledge leaks into the daemon core, violating the 「백엔드 확장」 principle.

## 근거
TRD §7 #1.

## 영향 / 불변식
Even when new vendors are added, extending only the backend registry absorbs them — do not hardcode vendor names in the daemon core. Implementation commits: `7aad8f7` (wire + assembly + connection layer), `4fd426a` (review follow-up).
