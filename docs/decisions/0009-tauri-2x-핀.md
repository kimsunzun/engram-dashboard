# ADR-0009: Pin tauri to the latest 2.x (Channel losslessness measured)

- 상태: 확정 (S4 channel spike)
- 관련: CLAUDE.md 의존성 · `src-tauri/Cargo.toml`

## 맥락
The initial LLD said "no tauri 2.5", but the dependency is caret, so it resolved to 2.11.2. There was doubt about whether Channel is lossless (GitHub #11421), so a version pin decision was needed.

## 결정
**Stay on the latest 2.x** (2.11.2 at the time of the spike). Do not pin to an older version.

## 거부한 대안
- **Pin to an older version** — a conservative pin because of the #11421 concern. The measurement showed it was unnecessary, and it would give up the latest security fixes and features.

## 근거
S4 spike: a temporary command sent 1000 Channel messages in a row → the frontend received **1000/1000 with no loss** (Windows WebView2). #11421 is a Linux-specific issue and does not reproduce on Windows.

## 영향 / 불변식
- The dependency is `tauri = "2"` (latest 2.x). Report any change.
- Channel is the first-class transport for the output stream (the Tauri implementation of the ADR-0003 sink).
