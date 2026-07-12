// selectOpenTarget — "열기"(트리 행 → 슬롯 배정) 대상 슬롯 선택 로직(PURE, 외부 의존 0).
//
// ★배경(제어 슬롯 포커스 제외)★: 트리(agent_list)·팔레트(preset_palette)는 "작업 슬롯"이 아니라
//   포커스 대상이 아니다(ViewLayoutRenderer 의 click-to-focus 게이트가 앞으로는 이들을 포커스하지
//   않는다). 그러나 기존/엣지 백엔드 상태(이미 제어 슬롯이 포커스됐거나 focus=null)에서도 "열기"가
//   트리/팔레트를 에이전트 터미널로 덮어쓰지 않도록 이 함수가 방어한다.
//
// 규칙(우선순위):
//   1. 포커스 슬롯의 content 가 empty/agent(콘텐츠 슬롯) → 그 슬롯(기존 동작 = 포커스 슬롯 배정).
//   2. 아니면(포커스가 제어 슬롯이거나 focus=null) → 레이아웃 트리를 순회한 첫 empty 슬롯.
//   3. 그것도 없으면 → null(배정 안 함 → 호출부가 실패 토스트). 제어·타 에이전트 슬롯을 임의로
//      덮어쓰지 않는다(클로버 금지).

import type { LayoutNode, SlotContent } from '../../api/layoutTypes'

/**
 * content 가 콘텐츠 슬롯(empty/agent)인가 — 제어 슬롯(agent_list/preset_palette)이면 false.
 *
 * ★단일 분류기(allowlist)★: 이 판별기가 "콘텐츠 슬롯" 정의의 단일 출처다. selectOpenTarget(열기 대상
 * 선택)과 ViewLayoutRenderer 의 click-to-focus 게이트가 **함께** 이걸 쓴다 — 기준을 한 곳에 모아
 * denylist 이원화(미래 제어 variant 가 조용히 포커스/열기 대상이 되는 것)를 막는다. 새 제어 variant
 * (ADR-0060 FileTree/ControlPanel 등)를 추가하면 여기 allowlist 에만 안 걸리면 자동으로 비포커스.
 */
export function isContentSlot(content: SlotContent): boolean {
  return content.type === 'empty' || content.type === 'agent'
}

/** 레이아웃 트리에서 id 가 일치하는 slot 노드를 찾는다(재귀 DFS). 없으면 null. */
function findSlotById(node: LayoutNode, slotId: string): Extract<LayoutNode, { type: 'slot' }> | null {
  if (node.type === 'slot') return node.id === slotId ? node : null
  // split: a 먼저, 없으면 b(트리 순서 = 읽기 순서 a→b).
  return findSlotById(node.a, slotId) ?? findSlotById(node.b, slotId)
}

/** 레이아웃 트리를 a→b 순서로 순회해 첫 empty 슬롯 id 를 찾는다. 없으면 null. */
function firstEmptySlotId(node: LayoutNode): string | null {
  if (node.type === 'slot') return node.content.type === 'empty' ? node.id : null
  return firstEmptySlotId(node.a) ?? firstEmptySlotId(node.b)
}

/**
 * "열기" 대상 슬롯 id 를 고른다(PURE). 위 규칙 우선순위대로:
 *   포커스가 콘텐츠 슬롯이면 그 슬롯 → 아니면 첫 empty 슬롯 → 그것도 없으면 null.
 *
 * @param layout 뷰 레이아웃 트리(백엔드 권위 미러 — selectView 로 조회한 것).
 * @param focusedSlotId 현재 포커스 슬롯 id(없으면 null).
 * @returns 배정 대상 slot id, 또는 null(적합한 슬롯 없음 → 호출부 조기 실패).
 */
export function selectOpenTarget(layout: LayoutNode, focusedSlotId: string | null): string | null {
  // 규칙 1: 포커스 슬롯이 콘텐츠 슬롯(empty/agent)이면 그 슬롯을 그대로 쓴다(기존 동작 보존).
  if (focusedSlotId != null) {
    const focused = findSlotById(layout, focusedSlotId)
    if (focused != null && isContentSlot(focused.content)) return focused.id
  }
  // 규칙 2: 포커스가 제어 슬롯이거나 focus=null → 첫 empty 슬롯으로 폴백.
  // 규칙 3: 첫 empty 도 없으면 null(호출부가 실패 토스트 — 임의 클로버 금지).
  return firstEmptySlotId(layout)
}
