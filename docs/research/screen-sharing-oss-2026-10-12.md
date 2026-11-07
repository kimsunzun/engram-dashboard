# Screen sharing · remote screen — OSS survey (2026-10-12)

- Status: **medium (adversarial review skipped — no cross-family verification)** · the main session's source cross-check also skipped (the user chose to move on to a proposal before the cross-check, 2026-10-12) · no decision
- Method: `/research` · two collection tracks (finished products · Rust components) → main session synthesis. ★The editions · dates · licenses below come from a single collector source — re-check before writing them down★
- Trigger: can the other side's screen (capture · real time) also be seen over a remote connection — user question
- Related: `docs/research/transport-security-build-vs-adopt-2026-10-12.md` (the security axis of the same day)

## Conclusions

1. **Screen transmission is not the transport's job** — the transport only carries one more binary message, and capture · compression · rate control are features on top of it. No effect on this transport design. `가능성 높음`
2. **There are two tracks** — a video codec stream (RustDesk · Sunshine/Moonlight · the WebRTC family — H.264/H.265/AV1, bandwidth efficiency) vs rectangles of changed regions (VNC/RFB · RDP · IronRDP). `가능성 높음`
3. **Putting a finished product into the app is hard because of licenses** — RustDesk = AGPL-3.0 · Sunshine = GPL-3.0 (likely) · Selkies = MPL-2.0 · TigerVNC = GPL-2.0 (likely). The permissive ones that can be put in = IronRDP (Rust · MIT/Apache) · FreeRDP (C · Apache-2.0, likely) · noVNC (JS · MPL-2.0). The final license decision is up to the responsible department.
4. **The realistic path = assembling components.**
   - Screenshot: `xcap` (Apache-2.0 · Win/macOS/Linux) + PNG/WebP.
   - Real time: capture `windows-capture` (MIT · Windows) / `screencapturekit` (MIT/Apache · macOS) + `openh264` (BSD-2 · software H.264) → our transport.
5. **The gap = hardware encoding** — the only confirmed Rust binding is `nvidia-video-codec-sdk` (NVIDIA only). A trait is needed to swap the backend per OS · GPU (QSV · AMF · Media Foundation · VideoToolbox not surveyed).

## Cautions

- OpenH264 patents: a statement that only the binary Cisco distributes is exempt from MPEG LA patent fees — a build of our own may fall outside the exemption (likely · original not cross-checked). Decision = responsible department.
- Avoid GPL/LGPL builds of FFmpeg/x264 · `scrap`, which has stalled since 2018.
- WebRTC (webrtc-rs · str0m) pays off only when the client is a browser.
- The shell can capture its own window with WebView2 `CapturePreview` (uncertain) — not applicable to a headless daemon.
- Not checked: capture traps (UAC secure desktop black screen · DRM · multi-monitor DPI) · each product's bandwidth control · RustDesk encryption details · some recent release dates (conflicting during collection).

## Reference implementations

- `jrf63/desktop-streaming` — a small Rust example of DXGI capture + NVENC + webrtc-rs (license not confirmed).
- IronRDP (Devolutions) — a bundle of Rust RDP client · server crates.
