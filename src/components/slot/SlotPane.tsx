import { useState } from 'react'
import { useSlotStore, findSlot } from '../../store/slotStore'
import { useAgentStore } from '../../store/agentStore'
import SlotContextMenu from './SlotContextMenu'

interface SlotPaneProps {
  slotId: number
  children?: React.ReactNode
}

export default function SlotPane({ slotId, children }: SlotPaneProps) {
  const layout = useSlotStore(s => s.layout)
  const focusedSlotId = useSlotStore(s => s.focusedSlotId)
  const dispatch = useSlotStore(s => s.dispatch)
  const agents = useAgentStore(s => s.agents)
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number } | null>(null)

  const slot = findSlot(layout, slotId)
  const isFocused = focusedSlotId === slotId
  // 슬롯이 터미널일 때만 agentId가 있다(tree면 없음). 이름은 AgentInfo.name, 없으면 id 앞 8자.
  const agentId = slot?.content.kind === 'terminal' ? slot.content.agentId : null
  const agentName = agentId ? (agents.find(a => a.id === agentId)?.name ?? agentId.slice(0, 8)) : '—'

  return (
    <div
      style={{
        height: '100%',
        background: 'var(--bg)',
        border: isFocused ? '2px solid var(--accent)' : '1px solid var(--border)',
        overflow: 'auto',
        position: 'relative',
        display: 'flex',
        flexDirection: 'column',
        alignItems: children ? 'stretch' : 'center',
        justifyContent: children ? 'flex-start' : 'center',
        color: 'var(--text)',
        fontFamily: 'var(--font-ui)',
        fontSize: '12px',
        boxSizing: 'border-box',
        cursor: 'default',
      }}
      onClick={() => dispatch({ kind: 'focusSlot', slotId })}
      onContextMenu={e => { e.preventDefault(); setContextMenu({ x: e.clientX, y: e.clientY }) }}
    >
      {children ?? <span style={{ color: 'var(--text-muted)' }}>Slot {slotId}</span>}
      <span style={{
        position: 'absolute',
        bottom: '4px',
        right: '6px',
        fontSize: '11px',
        color: 'var(--text-muted)',
        pointerEvents: 'none',
      }}>{agentName}</span>
      {contextMenu && (
        <SlotContextMenu x={contextMenu.x} y={contextMenu.y} slotId={slotId} onClose={() => setContextMenu(null)} />
      )}
    </div>
  )
}
