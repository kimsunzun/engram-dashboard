import { useEffect, useRef } from 'react'
import { useSlotStore, findSlot } from '../../store/slotStore'
import { agentClient } from '../../api/clientFactory'

interface SlotContextMenuProps {
  x: number
  y: number
  slotId: number
  onClose: () => void
}

export default function SlotContextMenu({ x, y, slotId, onClose }: SlotContextMenuProps) {
  const ref = useRef<HTMLDivElement>(null)
  // UI는 store 액션을 직접 부르지 않고 dispatch(단일 제어 표면, §5)만 호출한다.
  const dispatch = useSlotStore(s => s.dispatch)
  const layout = useSlotStore(s => s.layout)

  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose()
    }
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [onClose])

  const slot = findSlot(layout, slotId)
  const slotAgentId = slot?.content.kind === 'terminal' ? slot.content.agentId : null

  const items = [
    {
      label: '에이전트 생성',
      action: () => {
        const cwd = window.prompt('작업 디렉토리', 'C:/') ?? ''
        if (!cwd.trim()) return
        agentClient
          .spawnAgent(cwd.trim())
          .then(agent => dispatch({ kind: 'assignAgent', slotId, agentId: agent.id }))
          .catch(e => console.error('[spawn]', e))
      },
    },
    {
      label: '에이전트 종료',
      action: () => {
        if (!slotAgentId) return
        agentClient.killAgent(slotAgentId).catch(e => console.error('[kill]', e))
      },
    },
    {
      label: '에이전트 트리 보기',
      action: () => dispatch({ kind: 'setSlotContent', slotId, content: { kind: 'tree' } }),
    },
    {
      label: '터미널 보기',
      action: () =>
        dispatch({ kind: 'setSlotContent', slotId, content: { kind: 'terminal', agentId: null } }),
    },
    { label: '가로 분할', action: () => dispatch({ kind: 'splitSlot', slotId, dir: 'horizontal' }) },
    { label: '세로 분할', action: () => dispatch({ kind: 'splitSlot', slotId, dir: 'vertical' }) },
    { label: '팝업으로 분리', action: () => window.open(`index.html#/popup?slotId=${slotId}`, '_blank') },
    { label: '닫기', action: () => dispatch({ kind: 'closeSlot', slotId }) },
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
