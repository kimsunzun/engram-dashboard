# ADR-0252: Add checked and enabled(ctx) to slot menu items, and the assembly site carries menu state through ctx

- 상태: 확정 (2026-09-27/29, basis: TRD §7 #8 + review follow-up — see the 「메인 결정」 section below) · 부분 폐기 by ADR-0258 (only the premise that the usage menu items are consumers of the checked and enabled fields)
- 관련: Amends ADR-0064 (adds optional checked and enabled fields to the fixed SlotMenuItem schema, and adds the assembly site's ctx subscription to the two-step new-item invariant) · ADR-0065(hideOn·children additive amendment precedent) · ADR-0055/0022(command registry) · `src/commands/registry.ts` · `src/components/slot/SlotContextMenu.tsx` · step-log S21 · Amended by ADR-0258 (only the premise that the usage menu items are consumers of the checked and enabled fields)

## 맥락
The usage-slot menu needed a "show" toggle (a check mark) and per-item enabled/disabled decisions (e.g. for a vendor that cannot refresh, disable the ⟳ item). ADR-0064 pins `SlotMenuItem` as a fixed schema and forbids menu items from reading the view store directly.

## 결정
Add **`checked`** and **`enabled(ctx)`** to `SlotMenuItem` additively (following the precedent where ADR-0065 added `hideOn` · `children`). The scope is only these two added fields. The state to show in the menu (e.g. usage display on/off, whether each vendor can refresh) is **subscribed by the assembly site (LayoutLeaf etc.) and carried through `ctx`**, not by the item itself.

### Background — main decision (awaiting user confirmation)
The first 7c implementation (`b59a7ac` · `664d011`) went in the direction of widening the existing `Command.when` to be argument-based in order to flow the usage state into the menu. The code review (codex + Claude, two parties) pointed this out, and **reverting the `Command.when` extension and newly adding `enabled(ctx)` was the main orchestrator's decision** (2026-09-29 session), and **user confirmation has not yet been received**. The revert was applied by `626279c`.

## 거부한 대안
- **Putting a status character in the title** — the title is fixed at registration (build) time and cannot reflect a runtime toggle.
- **Re-registering the menu on every toggle** — the registration API would have to be run again every time the state changes.
- **A contribution reading the view store directly** — violates ADR-0064's 「메뉴에서 직접 store 호출 금지」.
- **(Tried during implementation and reverted) Extending `Command.when` to be argument-based** — reverted by the main decision, see 「경위」 above. Awaiting user confirmation.

## 근거
TRD §7 #8 · main-notes (7c review FIX twice — follow-ups `626279c`/`59bb49e`).

## 영향 / 불변식
ADR-0064's invariant "a new item = only the command + registerSlotMenu registration" is partly bent by this decision — **LayoutLeaf subscribing to the SlotMenu state and injecting it through `ctx`** also became a normal path for new items (the first real use of that channel was the general `ctx` field `usageRefreshable` — room for reuse will be looked at again when a second consumer appears). Implementation commits: `b59a7ac`, `664d011`, `626279c`, `d49a93f`, `59bb49e`.

★**`usageRefreshable` no longer exists (`910c214` — ADR-0258)**★ — when all usage menu items were removed, that ctx field and LayoutLeaf's usage-store subscription were removed. The `checked` · `enabled(ctx)` fields and the rule 「메뉴 상태는 조립처가 구독해 ctx 로 싣는다」 are still alive, and **no state is carried now** (zero menu items use the two fields — whether to remove them is left open in ADR-0258's 「영향」).
