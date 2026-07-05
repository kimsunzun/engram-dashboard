import { useSearchParams } from 'react-router-dom'
import TerminalSlot from '../components/slot/TerminalSlot'
import { useSlotStore, findSlot } from '../store/slotStore'

export default function PopupPage() {
  const [params] = useSearchParams()
  const slotId = parseInt(params.get('slotId') ?? '1', 10)
  const layout = useSlotStore(s => s.layout)
  const slot = findSlot(layout, slotId)
  // 팝업은 터미널만 표시. 슬롯이 터미널일 때만 agentId가 있다(tree면 null).
  const agentId = slot?.content.kind === 'terminal' ? slot.content.agentId : null

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
        <TerminalSlot viewId={`popup-slot-${slotId}`} agentId={agentId} />
      </div>
    </div>
  )
}
