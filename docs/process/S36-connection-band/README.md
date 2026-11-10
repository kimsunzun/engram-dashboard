# S36 — connection band (frontend)

Frontend for the two connection bands the shell already decides (S21 stage 3, ADR-0350): band text, one 「다시 연결」 button on both bands, and the LLM commands `connection.status` / `connection.reconnect` / `daemon.stop`. The version band carries no stop button (user decision 2026-11-10 — version handling moves to a later update system).

Material: parked branch `v0.4.1/feat/connection-ui-parked` (`4d25dfa7`) — rebuilt from master, not rebased.
