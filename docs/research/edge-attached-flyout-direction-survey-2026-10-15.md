# Flyout panels attached to an edge — which way they open, and what happens at the window edge (practice summary)

- **Status:** light · decision pending (an idea from before the preset PRD — when a row is clicked in the edit pane, the detail panel attaches to the edge of the library slot and slides out over the neighboring slot. What if the slot is at the window edge?)
- **Date:** 2026-10-15
- **Method:** research light. 1 collector (4~6 searches · much of it from memory · thin primary sources) · main-session grounding (JetBrains primary source — confirmed in the earlier survey `side-detail-panel-push-vs-overlay-survey-2026-10-15.md`) · no adversarial review.

## Conclusions

1. **A flyout panel attached to an edge (dock) opens toward 「붙은 가장자리의 반대쪽 = 내용 쪽」 (the side opposite the attached edge = the content side). The direction is set by which edge the dock is on, and it is not flipped case by case.** Visual Studio auto-hide tool windows (slide out from the tab on the attached edge — primary source) · JetBrains Dock Unpinned (comes out from its own bar side — primary source · whether it covers is unconfirmed) · Photoshop icon panels (to the left when the dock is on the right — from memory). **Uncertain**, short of **likely** (no source states a single rule).
2. **The 「선호 방향 → 자리 없으면 반대쪽 → 창 안으로 끼워 넣기」 (preferred direction → opposite side if there is no room → fit inside the window) flip is the rule of popups anchored to a reference point (flyouts · popovers) — it is not a dock panel rule.** WinUI `FlyoutPlacementMode.Auto` automatically picks another placement when the preferred one lacks room (primary source) · NSPopover's order of opposite side → perpendicular side is **uncertain** (Apple primary source not checked).
3. **We found no peer that falls back to 「붙박이 · 칸 안으로 되돌리기」 (pinned · back inside the pane) at the window edge.** **Gap.**

## If moved to Engram (candidate — not a decision)

- Our detail panel attaches to the slot edge and opens **outward (over the neighboring slot)**, so its direction is opposite to a dock panel (covering the neighboring slot = the JetBrains Undock kind).
- When the slot is at the window edge, **opening over the inside of its own slot** by the dock panel rule (opposite the attached side = inward) does not conflict with practice — but the practice evidence is thin.

## grounding

| Claim | Source | Verdict |
|---|---|---|
| WinUI Auto — another placement automatically if the preferred one lacks room | https://learn.microsoft.com/en-us/UWP/api/windows.ui.xaml.controls.primitives.flyoutplacementmode?view=winrt-22621 (collector) | unconfirmed (not cross-checked by main) |
| VS auto-hide — comes back out from the attached edge's tab | https://learn.microsoft.com/en-us/previous-versions/z4y0hsax(v=vs.110) (collector) | unconfirmed |
| JetBrains Dock Unpinned · Undock wording | https://www.jetbrains.com/help/idea/viewing-modes.html (confirmed by main in the earlier survey) | supported |
| Photoshop icon panel direction · NSPopover order | memory · third party | uncertain |

## Gaps

- Not looked at: Blender N/T panels · Figma · Unreal content drawer.
