// 레이아웃 wire 타입 = src-tauri 권위(ADR-0035). ts-rs 가 src-tauri/bindings/*.ts 로 자동생성한다.
// 프론트는 그 타입을 직접 재정의하지 않고(드리프트 원천) 이 한 곳에서 re-export 해 src 안에서 참조한다
// — bindings 는 tsconfig include("src") 밖이라, 컴포넌트/스토어가 src-tauri 상대경로를 흩뿌리지 않도록
// 단일 진입점으로 모은다(경로 변경 시 여기만). 재정의가 아니라 재노출이므로 "직접 재정의 금지"와 무충돌.
export type { LayoutNode } from '../../src-tauri/bindings/LayoutNode'
export type { SlotContent } from '../../src-tauri/bindings/SlotContent'
export type { SplitDir } from '../../src-tauri/bindings/SplitDir'
export type { View } from '../../src-tauri/bindings/View'
export type { ViewMeta } from '../../src-tauri/bindings/ViewMeta'
export type { ViewSnapshot } from '../../src-tauri/bindings/ViewSnapshot'
// ADR-0068: 슬롯 공간 타깃 파생(방향 이웃 + 읽기 순서) — ViewSnapshot.slot_spatial 에 실린다.
export type { SlotSpatial } from '../../src-tauri/bindings/SlotSpatial'
export type { Neighbors } from '../../src-tauri/bindings/Neighbors'
