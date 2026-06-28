import { useState } from 'react'
import { Allotment } from 'allotment'
import 'allotment/dist/style.css'
import Sidebar from './Sidebar'
import ViewLayoutRenderer from './ViewLayoutRenderer'
import DiffPanel from '../diff/DiffPanel'
import StatusBar from './StatusBar'
import { selectActiveView, useViewStore } from '../../store/viewStore'

export default function AppLayout() {
  const [sidebarOpen, setSidebarOpen] = useState(true)
  const [diffOpen, setDiffOpen] = useState(false)
  // 메인 캔버스는 ★항상 백엔드 권위 레이아웃(ADR-0035)★ — active view 의 캐시 항목만 그린다(active-only).
  // 옛 slotStore 폴백은 폐기(FIX-3). 부팅 직후 init(list_views/get_view) 전엔 active 캐시가 비어 null →
  // 빈 화면이고, init 이 기본 View 1(빈 슬롯 1개)을 캐시에 넣으면 곧장 그려진다.
  //
  // ★수용된 전환기 불일치(오너 결정 — un-migrate 금지)★: 메인 캔버스는 *의도적으로* viewStore(백엔드 권위)
  // 의 ViewLayoutRenderer 를 쓰지만, PopupPage/TreePage/SlotContextMenu 는 아직 옛 slotStore(프론트 전용)를
  // 구동한다. 두 store 가 공존하는 이 불일치는 알려진/수용된 상태다 — slotStore→viewStore 전체 이주는
  // 범위가 큰 *별도 다음 슬라이스*로 분리한 의도적 결정이다. 여기서 메인뷰만 새 렌더러로 고정하고, 다른
  // 화면을 임의로 slotStore 로 되돌리거나(un-migrate) 같은 PR 에서 한꺼번에 이주하지 않는다.
  const activeView = useViewStore(selectActiveView)

  return (
    <div style={{ height: '100%', position: 'relative' }}>
      {!sidebarOpen && (
        <button
          onClick={() => setSidebarOpen(true)}
          style={{
            position: 'absolute',
            top: '50%',
            left: 0,
            transform: 'translateY(-50%)',
            zIndex: 10,
            background: 'var(--bg-secondary)',
            border: '1px solid var(--border)',
            color: 'var(--text-muted)',
            cursor: 'pointer',
            padding: '4px 2px',
            fontSize: '12px',
            borderRadius: '0 4px 4px 0',
          }}
        >▶</button>
      )}
      <Allotment>
        <Allotment.Pane preferredSize={200} minSize={120} visible={sidebarOpen}>
          <Sidebar onToggle={() => setSidebarOpen(false)} />
        </Allotment.Pane>
        <Allotment.Pane>
          <Allotment vertical>
            <Allotment.Pane>
              {activeView && (
                <ViewLayoutRenderer
                  node={activeView.layout}
                  focusedSlotId={activeView.focusedSlotId}
                />
              )}
            </Allotment.Pane>
            <Allotment.Pane preferredSize={300} minSize={300} maxSize={300} visible={diffOpen}>
              <DiffPanel />
            </Allotment.Pane>
            <Allotment.Pane preferredSize={24} minSize={24} maxSize={24}>
              <StatusBar diffOpen={diffOpen} onDiffToggle={() => setDiffOpen(v => !v)} />
            </Allotment.Pane>
          </Allotment>
        </Allotment.Pane>
      </Allotment>
    </div>
  )
}
