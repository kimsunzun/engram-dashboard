import { useEffect, useRef } from 'react'
import { useSlotStore, findSlot } from '../../store/slotStore'
import { ptyApi } from '../../api/ptyApi'

interface SlotContextMenuProps {
  x: number
  y: number
  slotId: number
  onClose: () => void
}

export default function SlotContextMenu({ x, y, slotId, onClose }: SlotContextMenuProps) {
  const ref = useRef<HTMLDivElement>(null)
  const splitSlot = useSlotStore(s => s.splitSlot)
  const closeSlot = useSlotStore(s => s.closeSlot)
  const layout = useSlotStore(s => s.layout)
  const assignAgent = useSlotStore(s => s.assignAgent)

  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose()
    }
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [onClose])

  const items = [
    {
      label: '에이전트 생성',
      action: () => {
        const cwd = window.prompt('작업 디렉토리', 'C:/') ?? ''
        if (!cwd.trim()) return
        ptyApi
          .spawnAgent(cwd.trim())
          .then(agent => assignAgent(slotId, agent.id))
          .catch(e => console.error('[spawn]', e))
      },
    },
    {
      label: '에이전트 종료',
      action: () => {
        const slot = findSlot(layout, slotId)
        if (!slot?.agentId) return
        ptyApi.killAgent(slot.agentId).catch(e => console.error('[kill]', e))
      },
    },
    { label: '가로 분할', action: () => splitSlot(slotId, 'horizontal') },
    { label: '세로 분할', action: () => splitSlot(slotId, 'vertical') },
    { label: '에이전트 전환', action: () => {} },
    { label: '팝업으로 분리', action: () => window.open(`index.html#/popup?slotId=${slotId}`, '_blank') },
    { label: '닫기', action: () => closeSlot(slotId) },
  ]

  return (
    <div ref={ref} style={{
      position: 'fixed',
      top: y,
      left: x,
      background: 'var(--bg-secondary)',
      border: '1px solid var(--border)',
      borderRadius: '4px',
      zIndex: 1000,
      minWidth: '130px',
      boxShadow: '0 2px 8px rgba(0,0,0,0.3)',
      fontFamily: 'var(--font-ui)',
      fontSize: '12px',
    }}>
      {items.map(item => (
        <div
          key={item.label}
          style={{ padding: '6px 12px', cursor: 'pointer', color: 'var(--text)' }}
          onMouseEnter={e => (e.currentTarget.style.background = 'color-mix(in srgb, var(--accent) 20%, transparent)')}
          onMouseLeave={e => (e.currentTarget.style.background = 'transparent')}
          onClick={e => { e.stopPropagation(); item.action(); onClose() }}
        >{item.label}</div>
      ))}
    </div>
  )
}
