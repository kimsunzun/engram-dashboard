import { useEffect, useRef } from 'react'

interface SlotContextMenuProps {
  x: number
  y: number
  slotId: number
  onClose: () => void
}

interface MenuItem {
  label: string
  action: () => void
}

export default function SlotContextMenu({ x, y, slotId, onClose }: SlotContextMenuProps) {
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose()
    }
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [onClose])

  const items: MenuItem[] = [
    { label: '분할',        action: () => {} },
    { label: '에이전트 전환', action: () => {} },
    { label: '팝업으로 분리', action: () => window.open(`index.html#/popup?slotId=${slotId}`, '_blank') },
    { label: '닫기',        action: () => {} },
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
