// Layout wire types = src-tauri authority (ADR-0035). ts-rs generates them automatically into src-tauri/bindings/*.ts.
// The frontend does not redefine those types (a source of drift) but re-exports them from this one place to reference within src
// — bindings is outside the tsconfig include ("src"), so we gather them into a
// single entry point so components/stores do not scatter src-tauri relative paths (only this file changes when the path changes).
export type { LayoutNode } from '../../src-tauri/bindings/LayoutNode'
export type { SlotContent } from '../../src-tauri/bindings/SlotContent'
export type { SplitDir } from '../../src-tauri/bindings/SplitDir'
export type { View } from '../../src-tauri/bindings/View'
export type { ViewMeta } from '../../src-tauri/bindings/ViewMeta'
export type { ViewSnapshot } from '../../src-tauri/bindings/ViewSnapshot'
// ADR-0068: slot spatial target derivation (directional neighbors + reading order) — carried in ViewSnapshot.slot_spatial.
export type { SlotSpatial } from '../../src-tauri/bindings/SlotSpatial'
export type { Neighbors } from '../../src-tauri/bindings/Neighbors'
// ADR-0227: shell geometry (rectangles in normalized boundary form) — carried in ViewSnapshot.slot_rects / split_rects.
export type { SlotRect } from '../../src-tauri/bindings/SlotRect'
export type { SplitRect } from '../../src-tauri/bindings/SplitRect'
// ADR-0227: the reply to a split ratio write (`set_split_ratio`).
export type { SplitRatioApplied } from '../../src-tauri/bindings/SplitRatioApplied'
export type { SplitRatioOutcome } from '../../src-tauri/bindings/SplitRatioOutcome'
// ADR-0227: argument of the area frame default metrics report (`report_ui_metrics`).
export type { UiMetrics } from '../../src-tauri/bindings/UiMetrics'
// TRD S26-window-chrome §3-2: new window reply (`create_window`) — label · persistent id · number.
export type { NewWindow } from '../../src-tauri/bindings/NewWindow'
// TRD S26-window-chrome §6-4: argument and reply of the window state action (`window_control`).
export type { WindowControlAction } from '../../src-tauri/bindings/WindowControlAction'
export type { WindowControlOutcome } from '../../src-tauri/bindings/WindowControlOutcome'
export type { WindowCloseResult } from '../../src-tauri/bindings/WindowCloseResult'
