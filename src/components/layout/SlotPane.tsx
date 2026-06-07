interface SlotPaneProps {
  slotId: number
  children?: React.ReactNode
}

export default function SlotPane({ slotId, children }: SlotPaneProps) {
  return (
    <div style={{
      height: '100%',
      background: 'var(--bg)',
      border: '1px solid var(--border)',
      overflow: 'auto',
      display: 'flex',
      flexDirection: 'column',
      alignItems: children ? 'stretch' : 'center',
      justifyContent: children ? 'flex-start' : 'center',
      color: 'var(--text)',
      fontFamily: 'var(--font-ui)',
      fontSize: '12px',
    }}>
      {children ?? <span style={{ color: 'var(--text-muted)' }}>Slot {slotId}</span>}
    </div>
  )
}
