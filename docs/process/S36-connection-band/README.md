# S36 — connection band (frontend)

Frontend for the two connection bands the shell already decides (S21 stage 3): band text, one 「다시 연결」 button on both bands, and the LLM commands `connection.status` / `connection.reconnect` / `daemon.stop`. The version band carries no stop button. Material: parked branch `v0.4.1/feat/connection-ui-parked` (`4d25dfa7`) — rebuilt on master, not rebased.

- PRD: `prd.md` · TRD: `trd.md`
- ADR: ADR-0350, ADR-0356 (this topic), ADR-0345, ADR-0287
- Tracking: T-84 (update system — out of scope)
- Timeline: `docs/process/step-log.md` → `## S36`
