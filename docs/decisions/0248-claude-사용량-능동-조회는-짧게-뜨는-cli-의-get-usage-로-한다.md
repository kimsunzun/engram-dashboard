# ADR-0248: Claude usage active query is done with the short-lived CLI's get_usage

- 상태: 확정 (2026-09-27, basis: user decision + TRD §7 #4)
- 관련: TRD §7 #4 · step-log S21

## 맥락
We had to choose how to actively query Claude usage. The candidates were A (`get_usage` on a short-lived CLI) and B (read the token file and call the private OAuth usage URL directly).

## 결정
**A** — query with `get_usage` on a short-lived CLI.

## 거부한 대안
- **B (token file + private OAuth usage URL)** — rejected for now. The app would touch credentials directly, and a first TLS dependency · direct internet sending · per-OS credential branching would be new. **If A breaks, B is brought in behind the same seam** — the replacement path is not closed.

## 근거
User decision 2026-09-27 · TRD §7 #4.

## 영향 / 불변식
The Claude query path sits behind the CLI-subprocess seam — even if it is later swapped for B, the consumers of that seam (the usage bus command · state update) must not change. Implementation commits: `56776d6`, `c68b885`.
