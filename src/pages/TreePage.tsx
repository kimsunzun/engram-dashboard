import AgentList from '../components/agent/AgentList'
import ConnectionNotice from '../components/layout/ConnectionNotice'

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
      {/* 알림은 모든 창에 나온다 — 부팅은 창마다 돌고 실패 이유도 창마다 도착한다(ADR-0134). */}
      <ConnectionNotice />
      <AgentList />
    </div>
  )
}
