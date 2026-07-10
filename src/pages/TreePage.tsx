import AgentList from '../components/agent/AgentList'

export default function TreePage() {
  return (
    <div style={{
      width: '100vw',
      height: '100vh',
      background: 'var(--bg-secondary)',
      display: 'flex',
      flexDirection: 'column',
    }}>
      <div style={{
        padding: '0 8px',
        height: '28px',
        borderBottom: '1px solid var(--border)',
        display: 'flex',
        alignItems: 'center',
        fontFamily: 'var(--font-ui)',
        fontSize: '11px',
        color: 'var(--text-muted)',
        flexShrink: 0,
      }}>
        Agent Tree
      </div>
      <AgentList />
    </div>
  )
}
