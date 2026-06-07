import AgentTree from '../agent/AgentTree'

interface SidebarProps {
  onToggle: () => void
}

const BTN: React.CSSProperties = {
  background: 'none',
  border: 'none',
  color: 'var(--text-muted)',
  cursor: 'pointer',
  fontSize: '11px',
  padding: '0 2px',
}

export default function Sidebar({ onToggle }: SidebarProps) {
  const handleDetach = () => {
    window.open('index.html#/tree', '_blank')
    onToggle()
  }

  return (
    <div style={{
      height: '100%',
      background: 'var(--bg-secondary)',
      borderRight: '1px solid var(--border)',
      display: 'flex',
      flexDirection: 'column',
    }}>
      <div style={{
        padding: '0 8px',
        height: '28px',
        borderBottom: '1px solid var(--border)',
        display: 'flex',
        justifyContent: 'space-between',
        alignItems: 'center',
        fontFamily: 'var(--font-ui)',
        fontSize: '11px',
        color: 'var(--text-muted)',
        flexShrink: 0,
        gap: '4px',
      }}>
        <span>Agent Tree</span>
        <div style={{ display: 'flex', gap: '2px' }}>
          <button onClick={handleDetach} style={BTN} title="트리 분리">↗</button>
          <button onClick={onToggle} style={BTN} title="접기">◀</button>
        </div>
      </div>
      <AgentTree />
    </div>
  )
}
