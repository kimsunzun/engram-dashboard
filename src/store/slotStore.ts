import { create } from 'zustand'

export type SplitDir = 'horizontal' | 'vertical'

/**
 * 슬롯에 담기는 콘텐츠. 확장 가능한 discriminated union(kind로 분기).
 * 지금은 terminal(에이전트 터미널) / tree(에이전트 목록 트리). 추후 diff·markdown 등 추가.
 * agentId는 terminal일 때만 의미가 있으므로 terminal payload 안에 둔다
 * (최상위에 두면 tree일 때 쓸모없는 유령 필드가 된다 — consult 지적).
 */
export type SlotContent =
  | { kind: 'terminal'; agentId: string | null }
  | { kind: 'tree' }

export interface SlotNode {
  type: 'slot'
  id: number
  content: SlotContent
}

export interface SplitNode {
  type: 'split'
  dir: SplitDir
  children: LayoutNode[]
}

export type LayoutNode = SlotNode | SplitNode

let nextId = 3

const emptyTerminal = (): SlotContent => ({ kind: 'terminal', agentId: null })

// ── tree helpers ──────────────────────────────────────────────────────────────

export function findSlot(node: LayoutNode, slotId: number): SlotNode | null {
  if (node.type === 'slot') return node.id === slotId ? node : null
  for (const child of node.children) {
    const found = findSlot(child, slotId)
    if (found) return found
  }
  return null
}

export function getAllSlotIds(node: LayoutNode): number[] {
  if (node.type === 'slot') return [node.id]
  return node.children.flatMap(getAllSlotIds)
}

function splitInTree(node: LayoutNode, slotId: number, dir: SplitDir): LayoutNode {
  if (node.type === 'slot') {
    if (node.id !== slotId) return node
    return {
      type: 'split',
      dir,
      children: [node, { type: 'slot', id: nextId++, content: emptyTerminal() }],
    }
  }
  return { ...node, children: node.children.map(c => splitInTree(c, slotId, dir)) }
}

function closeInTree(node: LayoutNode, slotId: number): LayoutNode | null {
  if (node.type === 'slot') return node.id === slotId ? null : node
  const kept = node.children
    .map(c => closeInTree(c, slotId))
    .filter((c): c is LayoutNode => c !== null)
  if (kept.length === 0) return null
  if (kept.length === 1) return kept[0]
  return { ...node, children: kept }
}

function setContentInTree(node: LayoutNode, slotId: number, content: SlotContent): LayoutNode {
  if (node.type === 'slot') return node.id === slotId ? { ...node, content } : node
  return { ...node, children: node.children.map(c => setContentInTree(c, slotId, content)) }
}

// ── initial layout ────────────────────────────────────────────────────────────

const initialLayout: LayoutNode = {
  type: 'split',
  dir: 'horizontal',
  children: [
    { type: 'slot', id: 1, content: emptyTerminal() },
    { type: 'slot', id: 2, content: emptyTerminal() },
  ],
}

// ── LayoutCommand: 레이아웃 제어의 단일 진입점(§5) ───────────────────────────────
// 사람 UI도 LLM도 이 한 곳을 거친다. UI 컴포넌트는 store 액션을 직접 부르지 말고 dispatch만 호출.
// (지금은 프론트 내부 처리. 백엔드 invoke 이관은 데몬화 때 — 갈아끼울 facade가 이 dispatch 한 곳.)
export type LayoutCommand =
  | { kind: 'focusSlot'; slotId: number }
  | { kind: 'splitSlot'; slotId: number; dir: SplitDir }
  | { kind: 'setSlotContent'; slotId: number; content: SlotContent }
  | { kind: 'assignAgent'; slotId: number; agentId: string }
  | { kind: 'closeSlot'; slotId: number }

interface SlotState {
  layout: LayoutNode
  focusedSlotId: number
  dispatch: (cmd: LayoutCommand) => void
}

export const useSlotStore = create<SlotState>((set) => ({
  layout: initialLayout,
  focusedSlotId: 1,
  dispatch: (cmd) =>
    set(s => {
      switch (cmd.kind) {
        case 'focusSlot':
          return { focusedSlotId: cmd.slotId }
        case 'splitSlot':
          return { layout: splitInTree(s.layout, cmd.slotId, cmd.dir) }
        case 'setSlotContent':
          return { layout: setContentInTree(s.layout, cmd.slotId, cmd.content) }
        case 'assignAgent':
          // 에이전트 배치 = 그 슬롯을 해당 에이전트의 터미널로 만든다.
          return {
            layout: setContentInTree(s.layout, cmd.slotId, {
              kind: 'terminal',
              agentId: cmd.agentId,
            }),
          }
        case 'closeSlot': {
          const newLayout = closeInTree(s.layout, cmd.slotId)
          const layout: LayoutNode = newLayout ?? { type: 'slot', id: nextId++, content: emptyTerminal() }
          const ids = getAllSlotIds(layout)
          const focusedSlotId = ids.includes(s.focusedSlotId) ? s.focusedSlotId : (ids[0] ?? 1)
          return { layout, focusedSlotId }
        }
        default:
          return s
      }
    }),
}))
