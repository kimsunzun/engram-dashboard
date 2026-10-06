import AgentList from '../components/agent/AgentList'
import ConnectionNotice from '../components/layout/ConnectionNotice'
import NoticeOverlay from '../components/layout/NoticeOverlay'

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
        padding: '10px 12px 6px',
        display: 'flex',
        alignItems: 'center',
        fontFamily: 'var(--font-ui)',
        fontSize: '11px',
        color: 'var(--text-muted)',
        flexShrink: 0,
      }}>
        Agent Tree
      </div>
      {/* 덮는 층의 자리 기준 칸 — 머리줄 아래 나머지를 다 쓰고, 안쪽도 세로 flex 라 `AgentList`(`flex: 1`)가 칸을
            그대로 채운다. */}
      <div style={{ flex: 1, minHeight: 0, position: 'relative', display: 'flex', flexDirection: 'column' }}>
        {/* 알림은 모든 창에 나온다 — 부팅은 창마다 돌고 실패 이유도 창마다 도착한다(ADR-0134). 목록을 밀지 않고
              덮는다(ADR-0277 — ADR-0180 개정). */}
        <NoticeOverlay>
          <ConnectionNotice />
        </NoticeOverlay>
        <AgentList />
      </div>
    </div>
  )
}
