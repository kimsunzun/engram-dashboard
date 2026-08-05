// ★배경(제어 슬롯 포커스 제외)★: 트리(agent_list)·팔레트(preset_palette)는 "작업 슬롯"이 아니라
//   포커스 대상이 아니다(ViewLayoutRenderer 의 click-to-focus 게이트가 앞으로는 이들을 포커스하지
//   않는다). 그러나 기존/엣지 백엔드 상태(이미 제어 슬롯이 포커스됐거나 focus=null)에서도 "열기"가
//   트리/팔레트를 에이전트 터미널로 덮어쓰지 않도록 이 함수가 방어한다.

import type { LayoutNode, SlotContent } from '../../api/layoutTypes'

/**
 * ★단일 분류기(allowlist)★: 이 판별기가 "콘텐츠 슬롯" 정의의 단일 출처다. selectOpenTarget(열기 대상
 * 선택)과 ViewLayoutRenderer 의 click-to-focus 게이트가 **함께** 이걸 쓴다 — 기준을 한 곳에 모아
 * denylist 이원화(미래 제어 variant 가 조용히 포커스/열기 대상이 되는 것)를 막는다. 새 제어 variant
 * (ADR-0060 FileTree/ControlPanel 등)를 추가하면 여기 allowlist 에만 안 걸리면 자동으로 비포커스.
 */
export function isContentSlot(content: SlotContent): boolean {
  return content.type === 'empty' || content.type === 'agent'
}

function findSlotById(node: LayoutNode, slotId: string): Extract<LayoutNode, { type: 'slot' }> | null {
  if (node.type === 'slot') return node.id === slotId ? node : null
  return findSlotById(node.a, slotId) ?? findSlotById(node.b, slotId)
}

function firstEmptySlotId(node: LayoutNode): string | null {
  if (node.type === 'slot') return node.content.type === 'empty' ? node.id : null
  return firstEmptySlotId(node.a) ?? firstEmptySlotId(node.b)
}

export function selectOpenTarget(layout: LayoutNode, focusedSlotId: string | null): string | null {
  if (focusedSlotId != null) {
    const focused = findSlotById(layout, focusedSlotId)
    if (focused != null && isContentSlot(focused.content)) return focused.id
  }
  return firstEmptySlotId(layout)
}
