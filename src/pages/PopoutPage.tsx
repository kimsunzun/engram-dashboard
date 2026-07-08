// PopoutPage — 슬롯 팝업 분리(pop-out)로 런타임 생성된 OS 창이 그리는 페이지.
//
// ★역할★: URL 해시 쿼리 `#/popup?view=<viewId>` 로 지정된 **단일 View** 하나만 렌더한다. 메인 창의
//   AppLayout 이 activeViewId 캐시를 렌더하는 것과 달리, 팝업은 자기 view 를 고정 렌더한다(탭 전환 없음 —
//   agent-tree 창이 자기 뷰만 그리는 것과 동형). 창→View 바인딩은 백엔드 pop_out_slot 이 이미 window_bindings
//   에 넣어뒀으므로(ADR-0035/0046), 이 페이지는 그 view 의 레이아웃을 pull + listen 해서 그리기만 한다.
//
// ★출력 구독은 자동★: 이 창은 자기 TauriTransport 싱글톤(별 WebView2 프로세스라 모듈 그래프가 신선)이
//   connected 전이에서 subscribe_output 을 자기 window_label 로 등록한다 — 그래서 window_bindings 에 묶인
//   agent 출력이 이 창 Channel 로 라우팅된다(백엔드 OutputRouter 일반 메커니즘). 슬롯(TerminalSlot/RichSlot)은
//   메인과 동일하게 agentClient 로 구독하고 request_replay(gen 펜스, ADR-0046)로 replay 를 받는다 — 팝업
//   전용 배선이 따로 없다(같은 컴포넌트·같은 client seam).
//
// ★§5★: 이 창의 생성/바인딩 자체가 window.__engramLayout.popOutSlot(slotId)(= invoke pop_out_slot)으로
//   LLM 제어 가능하다. 이 페이지는 순수 I/O 표시 표면(손발) — 제어는 백엔드측(두뇌)이 쥔다.

import { useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'

import ViewLayoutRenderer from '../components/layout/ViewLayoutRenderer'
import type { LayoutNode, ViewSnapshot } from '../api/layoutTypes'
// ★단일 출처(Fix 3)★: 팝업 컨텍스트 view id 파싱은 viewStore 의 공유 헬퍼를 쓴다(PopoutPage 와
//   window.__engramLayout 이 같은 판정을 공유 — §5 제어 표면 일관).
import { readViewIdFromHash } from '../store/viewStore'

/** view:closed 페이로드(백엔드 ViewClosedPayload 미러 — 닫힌 view id). 자기 view 소멸 감지 전용. */
interface ViewClosedPayload {
  id: string
}

export default function PopoutPage() {
  // 이 팝업이 고정 렌더할 view id(URL 에서 1회 확정 — 창 수명 동안 불변).
  const [viewId] = useState<string | null>(readViewIdFromHash)
  // 그 view 의 레이아웃 트리 + focus. 백엔드 권위 pull(get_view) + listen(layout:updated) 로만 갱신.
  const [layout, setLayout] = useState<LayoutNode | null>(null)
  const [focusedSlotId, setFocusedSlotId] = useState<string | null>(null)
  // 같은 view 안 stale emit 가드용 version(전역 단조 — viewStore 캐시 모델과 동일 규율). 렌더에 안 쓰고
  // 비교 전용이라 ref(리렌더 불필요 + 콜백에서 최신값 동기 참조).
  const versionRef = useRef<number>(-1)

  useEffect(() => {
    if (!viewId) return
    let disposed = false
    let unlistenLayout: (() => void) | null = null
    let unlistenClosed: (() => void) | null = null

    // 이 view 의 layout:updated 만 채택(다른 view emit 은 무시). version 단조 가드로 stale 폐기.
    const apply = (snap: ViewSnapshot): void => {
      if (snap.view_id !== viewId) return
      if (snap.version <= versionRef.current) return // stale — 폐기(같은 view 내 전역 단조 비교)
      versionRef.current = snap.version
      setLayout(snap.layout)
      setFocusedSlotId(snap.focused_slot_id)
    }

    // ★자기 view 가 *실제로 닫혔을 때만* 이 팝업 창을 자가종료한다(Finding 1 재작업).★
    //   ★왜 view:closed 인가★: 옛 구현은 view:list-updated 목록에서 자기 viewId 가 *빠졌으면* 닫았는데,
    //   그 목록(view_metas)은 "창에 바인딩된 View 를 제외한 *탭 바용* 필터"라 팝업 자기 view 는 (바인딩+
    //   비활성이라) 항상 목록에 없다 → 첫 emit 에서 모든 팝업이 자가종료·연쇄 붕괴했다. "탭 목록에 없음"
    //   ≠ "닫힘"이므로, 특정 view 가 닫혔다는 *양성 신호*(view:closed{id})를 close_view command 경로가
    //   emit 하고, 그 id 가 자기 viewId 와 정확히 일치할 때만 닫는다(필터 목록을 신호로 쓰지 않는다).
    //   반대 방향(창 close→백엔드 정리)은 Destroyed arm 의 cleanup_popup_window 담당 — view:closed 는
    //   거기서 안 쏜다(창이 이미 소멸한 뒤라 자가종료가 무의미 + 재진입 위험).
    const onClosed = (payload: ViewClosedPayload): void => {
      if (payload.id !== viewId) return // 다른 view 가 닫힘 — 이 팝업과 무관
      // 이 팝업의 백킹 view 가 (LLM/사람 close_view 로) 닫힘 → 창을 자가종료(Destroyed → 백엔드 정리 연쇄).
      getCurrentWindow()
        .close()
        .catch(err => console.warn('[PopoutPage] 자기 view 닫힘 — 창 close 실패:', err))
    }

    void (async () => {
      // ★구독 먼저★(F-listen 동형): listen 등록 완료 전 도착한 emit 을 놓치지 않도록 구독을 먼저 걸고
      //   그 뒤 초기 pull 한다. 더 최신 emit 이 pull 을 덮으면 version 가드가 역전을 막는다.
      const [offLayout, offClosed] = await Promise.all([
        listen<ViewSnapshot>('layout:updated', e => apply(e.payload)),
        listen<ViewClosedPayload>('view:closed', e => onClosed(e.payload)),
      ])
      if (disposed) {
        offLayout() // 등록 완료 전 unmount 됐으면 즉시 해제(누수 가드)
        offClosed()
        return
      }
      unlistenLayout = offLayout
      unlistenClosed = offClosed
      // 초기 pull — 이 view 의 현재 스냅샷(version 포함). 창 mount 시점의 레이아웃을 즉시 그린다.
      try {
        const snap = await invoke<ViewSnapshot>('get_view', { viewId })
        if (!disposed) apply(snap)
      } catch (err) {
        console.warn('[PopoutPage] get_view 실패:', err)
      }
    })()

    return () => {
      disposed = true
      if (unlistenLayout) unlistenLayout()
      if (unlistenClosed) unlistenClosed()
    }
  }, [viewId])

  if (!viewId) {
    return (
      <div style={centerStyle}>
        <span>팝업 view id 없음 — URL 확인(#/popup?view=&lt;id&gt;)</span>
      </div>
    )
  }
  if (!layout) {
    return (
      <div style={centerStyle}>
        <span>View 로딩 중…</span>
      </div>
    )
  }
  return (
    <div style={{ width: '100vw', height: '100vh', background: 'var(--bg)' }}>
      {/* ★Fix 3: viewIdOverride 로 이 팝업의 고정 view 를 내려꽂는다★ — 안쪽 SlotContextMenu 의 분할/닫기/
          pop-out 액션이 전역 activeViewId(=main)가 아니라 이 팝업 view 를 좌표로 쓰게 한다(엉뚱한 View 오변형
          방지). 메인 창 AppLayout 경로는 이 prop 을 안 넘겨 종전대로 activeViewId 폴백. */}
      <ViewLayoutRenderer node={layout} focusedSlotId={focusedSlotId} viewIdOverride={viewId} />
    </div>
  )
}

const centerStyle: React.CSSProperties = {
  width: '100vw',
  height: '100vh',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  background: 'var(--bg)',
  color: 'var(--text-muted)',
  fontFamily: 'var(--font-ui)',
  fontSize: '12px',
}
