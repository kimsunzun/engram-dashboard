// WindowLayout — 한 창(label)의 탭바 + 활성 탭 슬롯 캔버스를 그리는 단일 컴포넌트(ADR-0057, §7-1).
//
// ★main·팝업 통일 경로(D-2 "동일 코드경로")★: main 창(AppLayout 이 크롬으로 감쌈)과 팝업 창(PopoutPage
// 이 얇게 감쌈)이 둘 다 이걸 마운트한다(각자 자기 label). 옛 "AppLayout=전역 active 렌더 vs
// PopoutPage=고정 뷰 렌더" 갈라짐(D-2 위반)을 제거한다. agent-tree 는 이 경로 밖(TreePage 를 그대로 그림).
//
// ★keep-alive(ADR-0056)★: windows[label].tabs 를 *전부* 마운트하고 활성 탭만 표시한다(숨은 탭
// display:none — xterm 인스턴스·버퍼 유지, 전환 즉시·무손실). WebglAddon 좌석은 보이는 슬롯만
// (숨은 탭은 화면에서 빠져 CSS 로 안 보이지만 인스턴스는 살아 출력 계속 누적 — 백엔드가 모든 탭 라우팅).
//
// ★자기 활성 탭 학습(G3)★: mount 시 ① list_tabs(label) 초기 pull 로 활성 탭 확정 + ② window:tabs-updated
// {label,...} listen(자기 label 만)으로 전환/추가/닫기 시 스왑·재렌더. 창 닫힘의 주 경로는 백엔드
// close_window→destroy_window(단일 소스, §5-2/G2) — 프론트 0탭 자가닫힘은 그 신호가 어쩌다 닿았을 때의
// 방어적 idempotent fallback 이다(정상 흐름에선 dead — S4-F3, 아래 effect 주석 상술).

import { useEffect, useRef, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'

import type { ViewSnapshot } from '../../api/layoutTypes'
import {
  getCurrentWindow,
  selectView,
  useViewStore,
  type WindowTabsPayload,
} from '../../store/viewStore'
import { retryAsync, RetryCancelledError } from '../../util/retryInvoke'
import ViewLayoutRenderer from './ViewLayoutRenderer'
import TabBar from './TabBar'
import AgentMonitoringPicker from '../slot/AgentMonitoringPicker'
import { useMonitoringPickerStore } from '../../store/monitoringPickerStore'
import { t } from '../../i18n'

interface WindowLayoutProps {
  /** 이 창의 label(main·slot-popup-N). 모든 탭 command·이벤트 필터가 이 label 을 쓴다. */
  label: string
}

export default function WindowLayout({ label }: WindowLayoutProps) {
  const win = useViewStore(s => s.windows[label])
  // ADR-0067: fresh remount on each open — key 변경이 stale query/activeIndex 플래시를 방지한다.
  const openId = useMonitoringPickerStore(s => s.openId)
  const applyWindowTabsUpdated = useViewStore(s => s.applyWindowTabsUpdated)
  const applyLayoutUpdated = useViewStore(s => s.applyLayoutUpdated)
  const switchTab = useViewStore(s => s.switchTab)
  const createTab = useViewStore(s => s.createTab)
  const closeTab = useViewStore(s => s.closeTab)
  const renameTab = useViewStore(s => s.renameTab)

  // 자가닫힘 재진입 가드 — 0탭 신호가 두 번 와도 close 를 한 번만 시도(idempotent, §5-2/G2).
  const closingRef = useRef(false)

  // ADR-0102: 부팅 pull 최종 실패 표면화 — 재시도 소진 후에도 이 창 상태(win)가 안 채워지면 로딩
  //   플레이스홀더에 영구 고착되므로(main 은 이벤트 복구 경로 없음), 조용한 console.warn 대신 이 플래그로
  //   가시적 에러 상태를 렌더한다. 그 사이 emit 으로 win 이 채워지면(경합) 아래 렌더가 정상 경로를 택한다.
  const [bootFailed, setBootFailed] = useState(false)

  // ── mount: 자기 창 탭 상태 초기 pull + window:tabs-updated listen(자기 label 만) ────────────────
  // ★구독 먼저, pull 나중★(F-listen 동형): listen 등록 완료 전 도착한 emit 을 놓치지 않게 구독을 먼저
  // 걸고 그 뒤 pull 한다. 더 최신 emit 이 pull 을 덮으면 applyWindowTabsUpdated 의 version 가드가 역전을 막는다.
  useEffect(() => {
    let disposed = false
    let unlisten: (() => void) | null = null

    // ADR-0102(FIX-2): ★부팅 경로 전체★를 하나의 try 로 감싼다 — 옛 구조는 listen() await 가 이 try
    //   밖이라, 부팅 중 리스너 등록이 reject 하면 async IIFE 가 unhandled 로 죽고 list_tabs pull 자체가
    //   시작조차 안 돼 bootFailed 가 영영 안 걸린다(로딩 플레이스홀더 영구 고착 — 무신호). listen() 실패도
    //   pull 실패와 동일하게 bootFailed 로 표면화해 "조용한 영구 고착" 불변식(ADR-0102)을 지킨다.
    //   ★구독 먼저, pull 나중★(F-listen) 순서와 disposed unlisten 정리는 그대로 유지한다.
    void (async () => {
      try {
        const off = await listen<WindowTabsPayload>('window:tabs-updated', e => {
          if (e.payload.label !== label) return // 자기 창만 반응(§7-1)
          applyWindowTabsUpdated(e.payload)
        })
        if (disposed) {
          off() // 등록 완료 전 unmount 됐으면 즉시 해제(누수 가드)
          return
        }
        unlisten = off
        // ADR-0102: 초기 pull 을 유계 재시도로 감싼다 — main 은 이벤트 복구 경로가 없어(window:tabs-updated 는
        //   탭 변형 시에만 발화) 이 pull 이 one-shot 이면 조기 실패 = 영구 고착이다. 성공하면 채우고, 재시도
        //   소진 시엔 bootFailed 로 가시화한다(조용히 삼키지 않음). disposed(unmount)면 재시도를 즉시 끊는다.
        const payload = await retryAsync(
          () => invoke<WindowTabsPayload>('list_tabs', { window: label }),
          {
            isCancelled: () => disposed,
            onRetry: (err, attempt) =>
              console.warn(`[WindowLayout] list_tabs(${label}) 재시도 ${attempt}:`, err),
          },
        )
        if (!disposed) {
          applyWindowTabsUpdated(payload)
          setBootFailed(false)
        }
      } catch (err) {
        // unmount 로 취소된 건 정상 종료(RetryCancelledError) — 에러 표면화 대상 아님.
        if (err instanceof RetryCancelledError) return
        // listen 등록 실패 or 재시도 소진 최종 실패 — 로딩 고착 대신 가시적 에러 상태로 전환(main 복구
        //   경로 부재 방어). 둘 다 동일 처리: 부팅 경로 어디서 깨져도 무신호 고착은 없다.
        console.error(`[WindowLayout] 부팅 실패(listen 등록 or list_tabs 재시도 소진, ${label}):`, err)
        if (!disposed) setBootFailed(true)
      }
    })()

    return () => {
      disposed = true
      if (unlisten) unlisten()
    }
  }, [label, applyWindowTabsUpdated])

  // ── keep-alive: 이 창의 모든 탭 layout 을 캐시에 채운다(숨은 탭도 렌더 유지, ADR-0056) ───────────
  // 각 탭 view_id 의 레이아웃 캐시가 없으면 get_view 로 pull 한다. layout:updated 는 eventBus 전역 구독이
  // 이미 캐시에 반영하므로(뷰별 version 가드), 여기선 "아직 한 번도 안 받은 탭"만 초기 pull 한다.
  const tabIdsKey = win ? win.tabs.map(t => t.id).join(',') : ''
  // ★S4-F2: deps 는 탭 집합(tabIdsKey)만★ — deps 에 win(객체 통째)을 넣으면 active 만 바뀌는 switch 마다
  //   win 참조가 갈려 effect 가 재실행돼 in-flight get_view 를 취소·재발행한다(탭 전환마다 pull 낭비).
  //   탭 목록이 실제로 바뀔 때(추가/닫기)만 재실행하도록 tabIdsKey 하나로 좁힌다. 탭 id 배열은 tabIdsKey
  //   에서 되짚어(store 직접 조회로 win 참조 회피) 순회한다.
  useEffect(() => {
    if (tabIdsKey === '') return
    let cancelled = false
    for (const tabId of tabIdsKey.split(',')) {
      if (useViewStore.getState().layouts[tabId]) continue // 이미 캐시 있음 → skip
      // ADR-0102(FIX-3): 이 pull 도 유계 재시도로 감싼다 — 옛 one-shot 은 transient 실패 시 console.warn
      //   뿐이고, tabIdsKey 가 그대로라 재실행 트리거가 없어 그 탭 캔버스가 무관한 layout:updated 가 올
      //   때까지 "View 로딩 중"에 갇힌다. 재시도로 순간적 미준비를 스스로 회복한다. ★창 전체 bootFailed
      //   와는 분리★: 단일 (대개 숨은) 탭의 최종 실패는 앱 전체 고착이 아니므로 console.error 로만 남긴다.
      void retryAsync(() => invoke<ViewSnapshot>('get_view', { viewId: tabId }), {
        isCancelled: () => cancelled,
        onRetry: (err, attempt) =>
          console.warn(`[WindowLayout] get_view(${tabId}) 재시도 ${attempt}:`, err),
      })
        .then(snap => {
          if (!cancelled) applyLayoutUpdated(snap)
        })
        .catch(err => {
          // unmount/탭변경 취소는 정상 — 조용히 무시. 그 외는 재시도 소진 최종 실패로 로깅만(탭 단위).
          if (err instanceof RetryCancelledError) return
          console.error(`[WindowLayout] get_view(${tabId}) 최종 실패(재시도 소진):`, err)
        })
    }
    return () => {
      cancelled = true
    }
    // tabIdsKey — 탭 집합이 바뀔 때(추가/닫기)만 재실행. applyLayoutUpdated 는 안정 참조. win 은 deps 밖(S4-F2).
  }, [tabIdsKey, applyLayoutUpdated])

  // ── 0탭 자가닫힘 = ★방어적 idempotent fallback★(S4-F3, §5-2/G2) ─────────────────────────────────
  // ★이 경로는 정상 흐름에선 dead 다★: 팝업 마지막 탭을 닫으면 백엔드 close_tab 이 WindowClosed 로 분류해
  //   close_window→destroy_window 로 OS 창을 *직접 파괴*한다(commands/layout.rs — WindowClosed 분기는
  //   emit 을 (None,None) 으로 두어 window:tabs-updated{tabs:[]} 를 프론트에 안 쏜다). 즉 정상 닫힘의 주
  //   경로는 백엔드 destroy 이고, 프론트는 tabs:[] 신호를 받지 못한다.
  // 그래도 이 분기를 남기는 이유(제거 X): 어떤 비정상 경로로든 tabs 가 0인 window:tabs-updated 가 프론트에
  //   닿으면(방어), 여기서 idempotent 하게 자기 창을 한 번만 닫아 stranded 창을 정리한다(main 은 백엔드가
  //   빈 탭을 강제 유지하므로 0이 안 됨). ★재진입 가드(closingRef)★: 이미 닫히는 중이면 no-op.
  useEffect(() => {
    if (!win) return
    if (win.tabs.length === 0 && !closingRef.current) {
      closingRef.current = true
      getCurrentWindow()
        .close()
        .catch(err => console.warn('[WindowLayout] 0탭 자가닫힘 실패:', err))
    }
  }, [win])

  if (!win) {
    // ADR-0102: 부팅 pull 재시도 소진 후에도 win 미도착이면 로딩 대신 가시적 에러(main 은 이벤트 복구
    //   경로가 없어 여기서 표면화하지 않으면 영구 고착). 그 외엔 로딩 플레이스홀더(부팅 직후 pull 전 또는
    //   유효 label 못 찾음 — dev 리로드 stale 팝업, §3-3/G3).
    if (bootFailed) {
      return (
        <div style={centerStyle} data-testid="window-boot-error">
          <span>{t('window.loadFailed', { label })}</span>
        </div>
      )
    }
    return (
      <div style={centerStyle}>
        <span>{t('window.loading', { label })}</span>
      </div>
    )
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', width: '100%', height: '100%' }}>
      <TabBar
        label={label}
        tabs={win.tabs}
        active={win.active}
        onSwitch={viewId => void switchTab(label, viewId).catch(e => console.error('[switchTab]', e))}
        onCreate={() => void createTab(label).catch(e => console.error('[createTab]', e))}
        onClose={viewId => void closeTab(label, viewId).catch(e => console.error('[closeTab]', e))}
        onRename={(viewId, name) =>
          void renameTab(viewId, name).catch(e => console.error('[renameTab]', e))
        }
      />
      {/* keep-alive 슬롯 캔버스 영역 — 모든 탭 마운트, 활성만 표시(display). */}
      <div style={{ flex: 1, position: 'relative', minHeight: 0 }}>
        {win.tabs.map(tab => (
          <div
            key={tab.id}
            data-testid="tab-canvas"
            data-view-id={tab.id}
            // ★keep-alive★: 활성 탭만 보이고 숨은 탭은 display:none(인스턴스·xterm 버퍼 유지). 절대배치로
            // 겹쳐 두어 전환이 리마운트가 아니라 표시 토글이 되게 한다(ADR-0056).
            style={{
              position: 'absolute',
              inset: 0,
              display: tab.id === win.active ? 'block' : 'none',
            }}
          >
            <TabCanvas viewId={tab.id} />
          </div>
        ))}
      </div>
      {/* ADR-0067: 에이전트 모니터링 검색 팝업 — 창당 하나 마운트. slot 우클릭 "에이전트 모니터링"
          (slot.assignRunningAgent)이 monitoringPickerStore 로 열고, on-select 는 assign_agent 로 흘린다(§5).
          닫힘 상태(target=null)면 아무것도 렌더하지 않는다.
          key={openId}: open() 마다 key 가 바뀌어 fresh remount → query/activeIndex 가 useState 초기값으로
          재설정된다(stale 플래시 없음, monitoringPickerStore.openId ADR-0067). */}
      <AgentMonitoringPicker key={openId} />
    </div>
  )
}

/**
 * 한 탭(view_id)의 슬롯 캔버스. 그 view 의 캐시 레이아웃을 ViewLayoutRenderer 로 그린다. viewIdOverride 로
 * 자기 view 를 내려꽂아 안쪽 SlotContextMenu 액션이 이 탭 좌표를 쓰게 한다(창별 active 무지 — §3-4).
 */
function TabCanvas({ viewId }: { viewId: string }) {
  const cached = useViewStore(s => selectView(s, viewId))
  if (!cached) {
    return (
      <div style={centerStyle}>
        <span>{t('common.viewLoading')}</span>
      </div>
    )
  }
  return (
    <ViewLayoutRenderer
      node={cached.layout}
      focusedSlotId={cached.focusedSlotId}
      viewIdOverride={viewId}
    />
  )
}

const centerStyle: React.CSSProperties = {
  width: '100%',
  height: '100%',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  background: 'var(--bg)',
  color: 'var(--text-muted)',
  fontFamily: 'var(--font-ui)',
  fontSize: '12px',
}
