import { useSearchParams } from 'react-router-dom'
import TerminalSlot from '../components/slot/TerminalSlot'

export default function PopupPage() {
  const [params] = useSearchParams()
  const slotId = params.get('slotId') ?? '1'

  return (
    <div style={{ width: '100vw', height: '100vh', background: 'var(--bg)', display: 'flex', flexDirection: 'column' }}>
      <div style={{
        padding: '0 8px',
        height: '28px',
        borderBottom: '1px solid var(--border)',
        display: 'flex',
        alignItems: 'center',
        fontFamily: 'var(--font-ui)',
        fontSize: '11px',
        color: 'var(--text-muted)',
        background: 'var(--bg-secondary)',
        flexShrink: 0,
      }}>
        Slot {slotId} — Popup
      </div>
      <div style={{ flex: 1, minHeight: 0 }}>
        <TerminalSlot />
      </div>
    </div>
  )
}
