# ADR-0304: The snap layout overlay is not turned on; it is kept behind the default-off cargo feature snap-overlay

- 상태: 확정 (2026-10-26, basis: user decision 2026-10-23 (Q1 — a short spike · give up if it does not work cheaply · the user does not use that feature) · 2026-10-24 (Q1 — do not turn it on · keep the code behind a default-off cargo feature) + main-agent decision M19 (a feature, not a comment · CI compiles the build with it on) + spike measured by the user 2026-10-24 (S6 — the menu appeared) + implementation U9 `8836c537` · `b6b3c780` · review fix `c7295bdb`)
- 관련: Amends ADR-0275 (decision 13's 「platform 의 cargo feature 는 test-support 하나다」) · PRD `docs/process/S21-window-chrome/prd.md` (version 0.5 — R15 · §2 · W8 · no AC19) · TRD `docs/process/S26-window-chrome/trd.md` (version 0.3 — §8-3 · §8-4 · §10-1 · §10-4 · §14) · research `docs/research/custom-titlebar-tauri-2026-10-18.md` §3 · §4 · ADR-0266 (OS code = platform) · ADR-0278 (OS boundary gate) · ADR-0303 (stripping the OS frame loses the snap menu) · reference repository `Hyph-M/tauri-plugin-snap-layout` (MIT) · `crates/engram-dashboard-platform/src/window/snap.rs` · `src-tauri/src/commands/snap_overlay.rs` · `src/components/layout/titleRow/snapOverlay.ts` · `.github/workflows/ci.yml` (`snap-overlay gate 1` · `2`) · `/qa` binding standard 「스냅 오버레이 기능 게이트」 · step-log S26

## 맥락

Stripping the OS frame (ADR-0303) loses the Windows 11 snap layout menu (the layout picker that appears when hovering over the maximize button) — both drag-region techniques lose it (PRD §5-2 table). The way to get it back is to cover the maximize button with a Win32 child window (an overlay) so that the OS shows the menu (research title §3 · §4 ② — build it directly in `platform` · keep the reference plugin's MIT attribution). In the spike (`t66_snap_spike.rs`) the menu appeared (S6 — observed by the user 2026-10-24).

ADR-0275 decision 13 does not split platform's `windows` bindings into per-feature cargo features, and it states 「platform 의 cargo feature 는 `test-support` 하나다」.

## 결정

1. **The snap layout menu is not turned on — having no snap menu is accepted as a regression** (user decision 2026-10-24 — Q1). Drag-to-screen-edge snap · Win+arrow remain (the OS move loop). There is no AC19.
2. **The code is kept — not as comments, but behind the default-off cargo feature `snap-overlay`** (user decision 2026-10-24 · main-agent decision M19):
   - platform `[features] snap-overlay = []` · the whole module `window::snap` (`place_overlay(parent, rect, on_hover)` · `remove_overlay(parent)` — the spike was moved over and rebuilt · the reference repository's attribution is kept at the module head) is behind that feature.
   - The shell's `src-tauri/Cargo.toml` `[features] snap-overlay = ["engram-dashboard-platform/snap-overlay"]` (the shell's first `[features]` table · off by default). The invoke `report_max_button_rect` · the hover event `window:snap-hover` · their registration are behind the same feature.
   - The web's `titleRow/snapOverlay.ts` is always compiled and tested, and reports the maximize button rectangle only when the window fact `caption.snap_overlay` (= the shell's `cfg!(feature = "snap-overlay")` — the default build is always false) is true. Because the shell-side type is behind the feature, there is no ts-rs binding — the web writes the same shape by hand.
3. **ADR-0275 decision 13 is amended — platform's cargo features are two: `test-support` · `snap-overlay`.** The rest of decision 13 (do not split the `windows` bindings into per-feature features · a union of features on one dependency) is unchanged — the bindings the overlay uses (`Win32_Foundation` · `Win32_System_Threading` (`GetCurrentThreadId`) · `Win32_UI_WindowsAndMessaging` · `Win32_UI_Input_KeyboardAndMouse`) were already on, so none were added (TRD §8-3 — block 4g PASS).
4. **CI measures so that the build with it on does not rot** (U9) — `snap-overlay gate 1` (backend job) = the shell's `cargo check --locked -p engram-dashboard --features snap-overlay` + platform's `cargo test --locked -p engram-dashboard-platform --features snap-overlay -- window::snap` (seven real-window overlay tests — compilation included · the exit code is 0 even when the name filter matches 0 tests, so count at least 1 line of `test window::snap::… ok`) · `snap-overlay gate 2` (gate job) = 0 lines of that feature in the shell's production graph + the counterpart (when on, at least 1 line of `engram-dashboard-platform feature "snap-overlay"`). The same four were put in the `/qa` binding standard.
5. **U9 review fixes** (2026-10-26 · main agent) — the leave (`WM_NCMOUSELEAVE`) is passed on to the previous window procedure after we do our share (the hover field · the callback) (the same shape as the spike) · `place_overlay` · `remove_overlay` do nothing unless called from the thread that created the parent window, and return `Err`.

## 거부한 대안

- **Turn it on** (PRD Q1 ② — get the menu back with the overlay · it appeared in the spike) — the user decided not to turn it on (2026-10-24). The circumstance that user decision 2026-10-23 cited = the user does not use that feature. The cost of turning it on = a Win32 child window · a path where the web measures and passes the button spot · relaying hover/press · DPI · one per popout (PRD Q1 ② · W8). Rejected = user decision.
- **Block it with comments** — code blocked by comments is not compiled, so it rots, and fixes pile up for the day it is turned on (main-agent decision M19 · TRD §8-4). Rejection strength = descriptive.
- **Delete the code** — the user decided 「코드는 남긴다」 (keep the code) (2026-10-24). The reason for dropping the deletion branch is not recorded. Rejected = user decision (no reason recorded).
- **Depend on a plugin** (the ③ path of research title §3 · §4 — PRD R15) — the spike and U9 went with ② build it directly. The reason for not choosing ③ is not recorded (the three plugins were not built or run — PRD W8). No rejection reason recorded.
- **Only `cargo check` for platform's share in CI** — the overlay's real-window tests are behind the feature and run nowhere else (TRD §8-4). Rejection strength = code.
- **Swallow `WM_NCMOUSELEAVE`** (U9 first implementation) — what showed the menu appearing is the spike, which passes that message on (S6). It was run in the same shape as the spike (U9 review · TRD §8-4). Rejection strength = measured (a shape different from the spike observation).

## 근거

- User decision PRD §5-1 (Q1 — 2026-10-23 · 2026-10-24).
- Spike S6 (observed by the user 2026-10-24 — the menu appeared).
- TRD §8-3 (measured 2026-10-26): the OS `cfg` matcher of 4f does not catch `#[cfg(feature = "snap-overlay")]` · `cfg!(feature = "snap-overlay")` (the four shell files that contain it and the sample are not flagged, and only the `#[cfg(windows)]` sample put in alongside is flagged · block 4f PASS) · block 4g PASS.
- Seven real-window overlay tests (U9 six + one from the review fix — TRD §10-1 platform `window::snap` row).

## 영향 / 불변식

- The default build (production · release) has no overlay code — `snap-overlay gate 2` turns red if that feature is on in the shell's production graph. If the counterpart (at least 1 line) is removed, the 0 expectation passes blind when the feature name changes.
- The OS boundary gate (4f · 4g · 4h) does not change — a `feature` predicate is not an OS predicate.
- How to turn it on = `--features snap-overlay` (shell) — put one line in CLAUDE.md 「빌드·검증 명령」 (U10).
- ★**What to look at again when turning it on (not re-measured — TRD §8-4)**★: it is not re-measured when only the scale changes and the CSS size stays the same · a tab drop on top of the overlay (W14) · the `STATIC` overlay paints the background opaque (U9 review codex probe — test pixel `ff00ff` → `f0f0f0` · it looked different in the spike) · a z-order race with the resize bands · the possibility of `RefCell` re-entrancy in the hover emit · reordering of rectangle reports that carry no sequence number · the attribution form (the module head states only the reference repository's name and license) — whether that satisfies the license requirement is to be confirmed with legal · whoever is in charge of licensing before turning it on.
- Anchors — put `// ADR-0304` (`# ADR-0304` in TOML) on the head of platform `window/snap.rs` · the `[features]` of the platform and shell `Cargo.toml` · `commands/snap_overlay.rs` · `snapOverlay.ts`.
