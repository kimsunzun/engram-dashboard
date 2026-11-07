import { useEffect, useSyncExternalStore } from 'react'

import ConnectionNotice from './ConnectionNotice'
import NoticeOverlay from './NoticeOverlay'
import RestoreModal from './RestoreModal'
import StateFileNotice from './StateFileNotice'
import WindowLayout from './WindowLayout'
import { restoreClient, type RestoreClient } from '../../api/restoreClient'
import { MAIN_WINDOW_LABEL } from '../../store/viewStore'

interface Props {
  /** The place where a test plugs in a fake (ADR-0012) — production uses the default. */
  restore?: RestoreClient
}

// ★No fixed chrome (ADR-0063)★: we do not put the left fixed Sidebar (AgentList mount) · the bottom DiffPanel · StatusBar.
//   The agent tree exists only as slot content and is absent at boot — it is placed through the empty slot menu or `layout.setSlotContent`
//   (ADR-0222). Being a slot, it can be moved · split · closed · rearranged (§5 LLM control surface). StatusBar/DiffPanel were
//   S0 view-stage remnants (0 real function) — reimplement if a real diff/status bar is needed.
//
// ★Single layout authority (ADR-0035 · 0057)★: the main canvas · the slot right-click menu (SlotContextMenu) · tree assignment are all
//   unified on viewStore (= a mirror of the backend ViewManager). main and popups mount the same WindowLayout, the same code path (D-2).
//
// ★The restore modal · state file notice are attached only here★ — this route is main. `App` runs per window, so attaching there
//   would block even popouts (TRD S21-storage §6-7 · the tree window was removed in ADR-0225).
//   The two read the same single state (`restoreClient` — one subscription · one pull).
export default function AppLayout({ restore = restoreClient }: Props) {
  useEffect(() => restore.install(), [restore])
  const status = useSyncExternalStore(restore.subscribe, restore.status)
  const awaiting = status?.crash_copy === 'awaiting'

  return (
    <>
      {/* The notice is also inside the layer that gets locked — while asking, the notice's ✕ is also blocked. The title bar (window buttons · draggable area) is also inside this layer so it is blocked together —
            keeping the title bar alive while asking is not a small change, so we did not do it (TRD S26-window-chrome §12-2 「모달 · 제목 줄」). */}
      <div inert={awaiting} style={{ height: '100%', display: 'flex', flexDirection: 'column' }}>
        {/* The wrapping area = `WindowLayout`'s place. The notice layer attaches not to this area but to the area below the title bar inside `WindowLayout` (the same place on every screen). */}
        <div style={{ flex: 1, minHeight: 0, position: 'relative' }}>
          <WindowLayout
            label={MAIN_WINDOW_LABEL}
            notices={
              <NoticeOverlay
              // ADR-0276: the state file notice covers the layout without pushing it (the rejected alternative = a pushing flow block).
              // ADR-0277: the connection band is also covered in this layer (ADR-0180 amendment).
              // It covers below the title bar — the title bar is not covered (user decision 2026-10-26 · T-66).
              >
                {/* The connection band is at the very top — if it cannot attach to the daemon nothing else works, so it must be read first (session judgment). */}
                <ConnectionNotice />
                <StateFileNotice stateFile={status?.state_file} saves={status?.saves} />
              </NoticeOverlay>
            }
          />
        </div>
      </div>
      {/* Draw the covering layer only while asking — if always laid down, even transparent it eats clicks beneath it. */}
      {awaiting && <RestoreModal status={status} onAnswer={restore.answer} />}
    </>
  )
}
