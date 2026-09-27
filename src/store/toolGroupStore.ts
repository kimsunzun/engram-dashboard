// ADR-0239: 채팅 도구 묶음의 펼침 상태 — 슬롯마다 사람 · LLM 이 고른 값만 담는다(고르지 않은 묶음은 자동 규칙).
//   프론트 전용 · 인메모리라 웹뷰 새로고침에 초기화된다(레이아웃과 같은 수준 — CLAUDE.md 「LLM-우선 제어」).
//   ★재구독 · replay 는 지우지 않는다★ — 묶음 키가 replay 뒤에도 같아서(`components/slot/chat/toolRuns.ts`) 고른 값이
//   그대로 다시 붙는다. 비우는 것은 에이전트가 바뀔 때(`bind`)와 새 화신으로 대화가 비워질 때(`clear`)뿐이다.
//   ★묶임 = 그 슬롯의 대화 뷰가 이 창에 지금 마운트돼 있다★ — 대화 뷰(RichSlot)가 마운트 효과에서 `bind` 를 부르고 그
//   정리(cleanup)에서 돌려받은 해제 함수를 부른다. 마운트되지 않은 슬롯에는 `setOpen` 이 `false` 로 답해, 뷰가 없는
//   슬롯에 command 가 성공으로 답하지 않는다(ADR-0167 — 형제 `slot.scrollToBottom` 의 손잡이 명부도 마운트 해제에 뺀다).
//   ★펼침을 바꾸는 길은 `setOpen` 하나다★ — 사람 토글도 LLM command `chat.toolGroup.setExpanded`(`commands/chatCommands.ts`)도
//   이것으로만 바꾼다. 두 번째 상태 경로를 만들지 말 것(CLAUDE.md 「LLM-우선 제어」).

import { create } from 'zustand'

/**
 * 한 슬롯의 칸 — `open` = 묶음 키 → 고른 펼침(키가 없으면 그 묶음은 자동 규칙) · `mount` = 지금 묶임의 표식(`null` =
 * 마운트되지 않음 — 고른 값은 남아 다시 마운트될 때 그대로 붙는다).
 */
export interface ToolGroupSlot {
  agentId: string
  open: Record<string, boolean>
  mount: number | null
}

export interface ToolGroupState {
  bySlot: Record<string, ToolGroupSlot>
  /**
   * 슬롯의 대화 뷰가 마운트됐다 — 칸이 다른 에이전트의 것이면 고른 값을 비우고, 같은 에이전트면 그대로 둔다.
   * @returns 해제 함수 — 마운트 해제(정리)에서 부른다. 고른 값은 남기고 묶임만 푼다. 두 번 불러도 · 그 뒤 새 `bind` 가
   *   있었어도 해가 없다(새 묶임을 풀지 않는다).
   */
  bind(slotId: string, agentId: string): () => void
  /**
   * 그 슬롯의 고른 값을 비운다(묶임은 남는다). 새 화신으로 대화가 비워질 때만 부른다 — 재구독 · replay 에서 부르면
   * 사람이 고른 펼침이 사라진다.
   */
  clear(slotId: string): void
  /**
   * @returns 적용했나 — 그 슬롯이 지금 묶여 있지 않으면(한 번도 안 묶였거나 해제됐다) `false` 이고 아무것도 적지 않는다.
   *   TRD S21-chat-ux §4-4 의 `void` 를 넓힌 것이다 — command 가 이 값으로 「이 창에 그 뷰가 없다」를 가른다.
   */
  setOpen(slotId: string, key: string, open: boolean): boolean
}

// 슬롯 id · 묶음 키는 밖(LLM)에서도 온다 — `in` · 맨 인덱스로 읽으면 `'constructor'` 같은 프로토타입 이름이 칸으로 읽힌다.
function own<T>(record: Record<string, T>, key: string): T | undefined {
  return Object.prototype.hasOwnProperty.call(record, key) ? record[key] : undefined
}

/**
 * 묶음이 지금 펼쳐져 있나 — 고른 값이 있으면 그것, 없으면 `live`(자동 규칙). 고른 값이 자동 펼침 · 자동 접힘을 둘
 * 다 이긴다. `slotId` 가 없으면 자동 규칙만 쓴다.
 */
export function effectiveOpen(
  state: Pick<ToolGroupState, 'bySlot'>,
  slotId: string | undefined,
  key: string,
  live: boolean,
): boolean {
  if (slotId === undefined) return live
  const slot = own(state.bySlot, slotId)
  const chosen = slot === undefined ? undefined : own(slot.open, key)
  return chosen ?? live
}

// 묶임마다 새 표식 — 해제 함수가 자기 묶임일 때만 풀게 한다(StrictMode 의 마운트 · 정리 · 마운트, 늦은 정리).
let lastMount = 0

export const useToolGroupStore = create<ToolGroupState>((set, get) => ({
  bySlot: {},
  bind: (slotId, agentId) => {
    const mount = ++lastMount
    const slot = own(get().bySlot, slotId)
    const open = slot !== undefined && slot.agentId === agentId ? slot.open : {}
    set(s => ({ bySlot: { ...s.bySlot, [slotId]: { agentId, open, mount } } }))
    return () => {
      const current = own(get().bySlot, slotId)
      if (current === undefined || current.mount !== mount) return
      set(s => ({ bySlot: { ...s.bySlot, [slotId]: { ...current, mount: null } } }))
    }
  },
  clear: slotId => {
    const slot = own(get().bySlot, slotId)
    if (slot === undefined || Object.keys(slot.open).length === 0) return
    set(s => ({ bySlot: { ...s.bySlot, [slotId]: { ...slot, open: {} } } }))
  },
  setOpen: (slotId, key, open) => {
    const slot = own(get().bySlot, slotId)
    if (slot === undefined || slot.mount === null) return false
    if (own(slot.open, key) === open) return true
    set(s => ({ bySlot: { ...s.bySlot, [slotId]: { ...slot, open: { ...slot.open, [key]: open } } } }))
    return true
  },
}))
