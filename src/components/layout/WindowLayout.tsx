// ★main·팝업 통일 경로(D-2 "동일 코드경로")★: main 창(AppLayout 이 크롬으로 감쌈)과 팝업 창(PopoutPage
// 이 얇게 감쌈)이 둘 다 이걸 마운트한다(각자 자기 label). 옛 "AppLayout=전역 active 렌더 vs
// PopoutPage=고정 뷰 렌더" 갈라짐(D-2 위반)을 제거한다.
//
// ★keep-alive(ADR-0056)★: windows[label].tabs 를 *전부* 마운트하고 활성 탭만 표시한다(숨은 탭
// display:none — xterm 인스턴스·버퍼 유지, 전환 즉시·무손실). WebglAddon 좌석은 보이는 슬롯만
// (숨은 탭은 화면에서 빠져 CSS 로 안 보이지만 인스턴스는 살아 출력 계속 누적 — 백엔드가 모든 탭 라우팅).
//
// ★탭 끌기(TRD S21-slot-docking §4-3 · §4-4)★: 이 창 뿌리에서 우리 탭 끌기를 받아(`dock/useWindowDrop` — 판정 · 안내 · 놓기
// 명령) 그 안내를 캔버스 위 층(`DockOverlayView`)에 그린다. 탭바의 끌기 모습은 `TabBar` 몫이고, 이 창에서 끄는 동안 보일
// 캔버스(R15 남은 탭)는 출처 쪽 `useShownTab` 이 정한다 — keep-alive 의 `display` 가 그것을 따른다. 파일 · URL 놓기는
// `dock/fileDropGuard.ts` 가 막는다.
//
// ★대기 모드(예열 창 — ADR-0299 · TRD S21-slot-docking §3-9)★: 예비 창 풀의 창(숨은 예비 창)은 탭 모델 밖이다. 그 창은
// 제목 줄(빈 탭바 — 번호 표식 · 창 단추 없음) + 빈 캔버스(빈 틀)만 그리고, 탭을 끌어오지 않고, 0탭 자가닫힘을 끈다. 재기
// 보고(캔버스 · 지표)는 그대로 보낸다. 모드는 `spareMode.ts` 가 정한다.

import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'

import type { ViewMeta, ViewSnapshot } from '../../api/layoutTypes'
import { fireAndForget } from '../../commands/dispatch'
import { selectView, useViewStore, type WindowTabsPayload } from '../../store/viewStore'
import { REPORT_DEBOUNCE_MS } from '../../util/latestValueReporter'
import { retryAsync, RetryCancelledError } from '../../util/retryInvoke'
import {
  getCanvasReport,
  observeCanvasSize,
  releaseCanvasReport,
  submitCanvasSize,
  type CanvasPx,
} from './windowCanvasReport'
import ViewLayoutRenderer from './ViewLayoutRenderer'
import TabBar from './TabBar'
import TitleRow from './titleRow/TitleRow'
import { dragProps } from './titleRow/dragRegion'
import { minWindowSizeSettled } from './titleRow/minSizeReport'
import { watchSpareMode, type WindowMode } from './spareMode'
import { installFileDropGuard } from './dock/fileDropGuard'
import { DockOverlayView } from './dock/DockOverlay'
import { windowFrame } from './dock/windowFrame'
import { useWindowDrop } from './dock/useWindowDrop'
import { useShownTab } from './tabStrip/useTabDragSource'
import { reportUiMetrics, SLOT_FRAME_BORDER } from './uiMetricsReport'
import AgentMonitoringPicker from '../slot/AgentMonitoringPicker'
import { useMonitoringPickerStore } from '../../store/monitoringPickerStore'
import { reportFirstPaint } from '../../theme/firstPaint'
import { t } from '../../i18n'

interface WindowLayoutProps {
  /** 이 창의 label(main·slot-popup-N). */
  label: string
  /**
   * 알림 층(`NoticeOverlay`) — 제목 줄 바로 아래 칸의 맨 위를 덮는다. ★제목 줄은 가리지 않는다★ — 창 단추 · 끌 자리가
   * 알림에 덮이면 창을 옮기지도 닫지도 못한다(사용자 결정 2026-10-26 · T-66 · TRD S26-window-chrome §12-2). 로딩 · 부팅
   * 실패 화면도 같은 자리다 — ★어느 화면이든 트리의 같은 자리에 둔다★: 자리가 바뀌면 React 가 층을 다시 마운트해 ✕ 로
   * 닫은 기억(`ConnectionNotice` · `StateFileNotice` — ADR-0277 이 걷지 말라고 한 것)이 사라진다.
   */
  // ADR-0306 — 알림 띠 자리
  notices?: ReactNode
}

// 대기 모드의 탭 목록 — 참조가 고정돼야 탭 목록에 걸린 effect(창 틀 보고 · keep-alive pull)가 렌더마다 다시 돌지 않는다.
const NO_TABS: ViewMeta[] = []

export default function WindowLayout({ label, notices }: WindowLayoutProps) {
  const win = useViewStore(s => s.windows[label])
  const openId = useMonitoringPickerStore(s => s.openId)
  const applyWindowTabsUpdated = useViewStore(s => s.applyWindowTabsUpdated)
  const applyLayoutUpdated = useViewStore(s => s.applyLayoutUpdated)
  const switchTab = useViewStore(s => s.switchTab)
  const createTab = useViewStore(s => s.createTab)
  const closeTab = useViewStore(s => s.closeTab)
  const renameTab = useViewStore(s => s.renameTab)

  const closingRef = useRef(false)
  const windowIdRef = useRef<string | null>(null)
  const canvasElRef = useRef<HTMLDivElement | null>(null)
  const reportCanvas = useWindowCanvasReport()
  const canvasRef = useCallback(
    (el: HTMLDivElement | null) => {
      canvasElRef.current = el
      reportCanvas(el)
    },
    [reportCanvas],
  )
  const canvasOf = useCallback(() => canvasElRef.current, [])
  const guide = useWindowDrop({ label, canvas: canvasOf })
  const titleRowRef = useRef<HTMLDivElement>(null)

  // ADR-0102: 부팅 pull 최종 실패 표면화 — 재시도 소진 후에도 이 창 상태(win)가 안 채워지면 로딩
  //   플레이스홀더에 영구 고착되므로(main 은 이벤트 복구 경로 없음), 조용한 console.warn 대신 이 플래그로
  //   가시적 에러 상태를 렌더한다. 그 사이 emit 으로 win 이 채워지면(경합) 아래 렌더가 정상 경로를 택한다.
  const [bootFailed, setBootFailed] = useState(false)

  // `null` = 아직 모름(`window_role` 답 전) — 탭 끌어오기 · 자가닫힘은 `normal` 일 때만 돈다.
  const [mode, setMode] = useState<WindowMode | null>(null)
  const [wasSpare, setWasSpare] = useState(false)
  // 이 페이지가 받은 채택 번호(`spare:adopted` · `window_role` 답 — TRD S21-slot-docking §3-9 I26). `0` 도 번호다.
  const [adoption, setAdoption] = useState<number | null>(null)
  const adoptionRef = useRef<number | null>(null)
  // 빈 틀을 거두는 때는 셸이 들인 창의 탭을 대기를 풀기 전에 알리는 순서에 기댄다 — 예열 창 채택(ADR-0299 · TRD
  //   S21-slot-docking §3-9 「`committed`」)이 그 순서로 알린다는 가정이다.
  // 새 채택 번호는 부팅 실패를 같은 렌더에서 내린다 — `window_role` 이 실패해 보통 모드로 모델이 모르는 자기 label 을
  //   당긴 예열 페이지는 이미 `bootFailed` 이고, 모드가 그대로 `normal` 이라 번호가 바뀌어야 당기기가 다시 돈다(아래
  //   deps). 따로 렌더하면 그 사이 실패 문구 화면을 새 번호로 알려 셸이 그 화면을 보인다. 같은 번호가 거듭 오면(답과
  //   사건이 둘 다 실음) 내리지 않는다 — 당기기가 다시 돌지 않아 로딩 바탕에 남는다.
  // ADR-0299
  useEffect(
    () =>
      watchSpareMode(next => {
        setMode(next.mode)
        if (next.mode === 'spare') setWasSpare(true)
        if (next.generation !== null && next.generation !== adoptionRef.current) {
          adoptionRef.current = next.generation
          setAdoption(next.generation)
          setBootFailed(false)
        }
      }),
    [],
  )
  const spare = mode === 'spare'
  const normal = mode === 'normal'
  // 들인 예열 창(`spare:adopted`)은 제 탭을 받기 전까지 빈 틀 그대로다 — 「로딩」 화면(바탕만)으로 바꾸면 탭바가
  //   사라졌다 다시 마운트된다. 예열 창은 새 label 로 대기에 든 채 떠 마운트 때 스토어에 그 창 항목이 없다 — 그래서
  //   `win` 이 있다 = 채택 때 받은 탭이다.
  const framed = spare || (wasSpare && win === undefined && !bootFailed)

  useEffect(() => installFileDropGuard(), [])

  // ── 보통 모드: 자기 창 탭 상태 초기 pull + window:tabs-updated listen(자기 label 만) ────────────────
  // ★구독 먼저, pull 나중★(F-listen 동형): listen 등록 완료 전 도착한 emit 을 놓치지 않게 구독을 먼저
  // 걸고 그 뒤 pull 한다. 더 최신 emit 이 pull 을 덮으면 applyWindowTabsUpdated 의 version 가드가 역전을 막는다.
  // 대기 모드에서 들여지면(`spare:adopted`) 여기로 다시 들어와 들인 탭을 당긴다 — 이미 보통 모드면 새 채택 번호가 다시 돌린다.
  // ADR-0295
  // ADR-0299
  useEffect(() => {
    if (!normal) return
    let disposed = false
    let unlisten: (() => void) | null = null

    // ADR-0102(FIX-2): ★부팅 경로 전체★를 하나의 try 로 감싼다 — 옛 구조는 listen() await 가 이 try
    //   밖이라, 부팅 중 리스너 등록이 reject 하면 async IIFE 가 unhandled 로 죽고 list_tabs pull 자체가
    //   시작조차 안 돼 bootFailed 가 영영 안 걸린다(로딩 플레이스홀더 영구 고착 — 무신호). listen() 실패도
    //   pull 실패와 동일하게 bootFailed 로 표면화해 "조용한 영구 고착" 불변식(ADR-0102)을 지킨다.
    void (async () => {
      try {
        const off = await listen<WindowTabsPayload>('window:tabs-updated', e => {
          if (e.payload.label !== label) return
          applyWindowTabsUpdated(e.payload)
        })
        if (disposed) {
          off() // 등록 완료 전 unmount 됐으면 즉시 해제(누수 가드)
          return
        }
        unlisten = off
        // ADR-0102: 초기 pull 을 유계 재시도로 감싼다 — main 은 이벤트 복구 경로가 없어(window:tabs-updated 는
        //   탭 변형 시에만 발화) 이 pull 이 one-shot 이면 조기 실패 = 영구 고착이다.
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
        if (err instanceof RetryCancelledError) return
        console.error(`[WindowLayout] 부팅 실패(listen 등록 or list_tabs 재시도 소진, ${label}):`, err)
        if (!disposed) setBootFailed(true)
      }
    })()

    return () => {
      disposed = true
      if (unlisten) unlisten()
    }
  }, [label, normal, adoption, applyWindowTabsUpdated])

  // 대기 중에 이 창 항목이 다시 차도(셸은 들일 탭을 알린 뒤 대기를 푼다 — 전역 미러가 받는다) 대기가 풀리기 전에는 그리지 않는다.
  const tabs = framed ? NO_TABS : win?.tabs
  const active = framed ? null : (win?.active ?? null)
  const tabIds = useMemo(() => tabs?.map(tab => tab.id) ?? null, [tabs])
  // 보이는 캔버스 — 이 창에서 끄는 동안 탭바 구역 밖이면 남은 탭(R15 · 모델은 그대로 — TRD S21-slot-docking §4-2).
  const shown = useShownTab(label, tabIds, active)

  // 셸은 이 창을 숨긴 채 만들고 첫 화면 알림에 창을 보인다(`theme/firstPaint.ts`) — 그 알림을 첫 화면(제목 줄 + 활성 탭의
  //   레이아웃 · 부팅 실패 문구)이 커밋된 뒤에 보낸다. 그 전 화면(바탕만 · 활성 탭의 「View 로딩」)이 보이는 것은 알림이
  //   오지 않아 셸 시한이 보일 때뿐이다.
  // 빈 틀은 첫 화면이 아니다 — 대기 창은 숨은 채 맡겨질 때를 기다리고, 채택한 창은 제 탭을 그린 뒤 채택 번호로 알린다
  //   (TRD S21-slot-docking §3-9 I26). 번호가 deps 에 드는 것은 채택 전후로 `firstScreen` 이 참으로 이어져도(부팅 실패를
  //   내림과 탭 반영이 한 렌더) 다시 알리게 하려는 것이다 — 빠지면 채택한 창이 늘 셸 시한에만 보인다.
  // 알림은 제목 줄의 최소 창 크기 보고(`titleRow/minSizeReport.ts`)가 끝난 뒤다 — 셸이 보이기 전에 그 창을 넓히게(TRD
  //   S26-window-chrome §7-7). 기다림에는 상한이 있다. 기다리는 사이 deps 가 바뀌면 그 알림은 버린다 — 옛 채택 번호가 새
  //   번호 뒤에 나가면 한 번 더 보낸다.
  // ADR-0299
  const activeLaidOut = useViewStore(s => active === null || selectView(s, active) !== null)
  const firstScreen = bootFailed || (!framed && tabs !== undefined && activeLaidOut)
  useEffect(() => {
    if (!firstScreen) return
    let current = true
    void minWindowSizeSettled().then(() => {
      if (current) reportFirstPaint(adoption)
    })
    return () => {
      current = false
    }
  }, [firstScreen, adoption])

  // ── keep-alive: 이 창의 모든 탭 layout 을 캐시에 채운다(숨은 탭도 렌더 유지, ADR-0056) ───────────
  // layout:updated 는 eventBus 전역 구독이 이미 캐시에 반영하므로(뷰별 version 가드), 여기선 "아직 한 번도
  // 안 받은 탭"만 초기 pull 한다.
  const tabIdsKey = tabs ? tabs.map(t => t.id).join(',') : ''
  // ★S4-F2: deps 는 탭 집합(tabIdsKey)만★ — deps 에 win(객체 통째)을 넣으면 active 만 바뀌는 switch 마다
  //   win 참조가 갈려 effect 가 재실행돼 in-flight get_view 를 취소·재발행한다(탭 전환마다 pull 낭비).
  //   탭 목록이 실제로 바뀔 때(추가/닫기)만 재실행하도록 tabIdsKey 하나로 좁힌다. 탭 id 배열은 tabIdsKey
  //   에서 되짚어(store 직접 조회로 win 참조 회피) 순회한다.
  useEffect(() => {
    if (tabIdsKey === '') return
    let cancelled = false
    for (const tabId of tabIdsKey.split(',')) {
      if (useViewStore.getState().layouts[tabId]) continue
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
          if (err instanceof RetryCancelledError) return
          console.error(`[WindowLayout] get_view(${tabId}) 최종 실패(재시도 소진):`, err)
        })
    }
    return () => {
      cancelled = true
    }
  }, [tabIdsKey, applyLayoutUpdated])

  // ── 0탭 자가닫힘 = ★방어적 idempotent fallback★(S4-F3, §5-2/G2) ─────────────────────────────────
  // ★이 경로는 정상 흐름에선 dead 다★: 팝업 마지막 탭을 닫으면 셸 close_tab 이 WindowClosed 로 분류해
  //   OS 창을 *직접 파괴*한다(`layout/apply.rs` `close_tab` — WindowClosed 분기는 emit 을 (None,None) 으로 두어
  //   window:tabs-updated{tabs:[]} 를 프론트에 안 쏜다). 즉 정상 닫힘의 주 경로는 셸 destroy 이고, 프론트는
  //   tabs:[] 신호를 받지 못한다.
  // 그래도 이 분기를 남기는 이유(제거 X): 어떤 비정상 경로로든 tabs 가 0인 window:tabs-updated 가 프론트에
  //   닿으면(방어), 자기 창을 한 번만 닫아 달라고 셸에 청해 stranded 창을 정리한다(main 은 백엔드가 빈 탭을 강제
  //   유지하므로 0이 안 됨). ★재진입 가드(closingRef)★: 청한 뒤 탭이 다시 올 때까지는 no-op.
  // ★닫기는 셸이 판정한다 — 웹이 창 API 로 닫지 않는다★(TRD S26-window-chrome §5-5 · AC10 · 팝아웃 권한에 창 닫기가 없다):
  //   `window.close { if_empty }` 는 그 창이 모델에 다시 있으면(그사이 탭이 들어왔다) 닫지 않는다. 그 판정이 영속 id 로
  //   찾으므로 탭이 있던 때 받아 둔 id 만 넘긴다(§5-5) — 탭 0 개 창은 이미 모델에서 빠져 있다. 받아 둔 id 가 없으면(이
  //   화면이 그 창의 탭을 본 적이 없다) 청하지 않는다.
  // ★보통 모드에서만★ — 대기 창은 탭 0 개가 정상이다(TRD S21-slot-docking §3-9). 모드를 모르는 동안 온 0탭은 모드가
  //   정해진 뒤 다시 본다.
  // ADR-0295
  useEffect(() => {
    if (!normal || !win) return
    if (win.tabs.length > 0) {
      windowIdRef.current = win.windowId
      closingRef.current = false
      return
    }
    if (closingRef.current) return
    const windowId = windowIdRef.current
    if (!windowId) {
      console.warn(`[WindowLayout] 0탭 자가닫힘 — ${label} 이 탭을 가진 때의 window_id 가 없어 청하지 않는다`)
      return
    }
    closingRef.current = true
    fireAndForget('window.close', { window: windowId, if_empty: true })
  }, [win, normal, label])

  // ★틀은 화면마다 같다 — 제목 줄 → 칸(맨 앞 = 알림 층) → 화면 내용★. OS 틀을 걷었으니 로딩 · 부팅 실패 화면에도 제목 줄(창
  //   단추 · 끌 자리)이 있어야 창을 옮기고 닫는다. 알림 층이 화면마다 같은 자리여야 ✕ 로 닫은 기억이 남는다(`notices` doc).
  // 탭을 받기 전 화면: 부팅 직후 pull 전, 또는 유효 label 못 찾음(dev 리로드 stale 팝업, §3-3/G3) · 부팅 실패(ADR-0102).
  //   로딩은 바탕만 그린다 — 문구 · 표시를 그리면 셸이 시한으로 창을 보였을 때 탭 화면 앞에 한 번 깜빡인다(사용자 요구
  //   2026-10-19: 창의 첫 화면은 테마 바탕 다음 곧바로 실제 내용). 이 화면의 칸은 캔버스가 아니다 — 크기를 보고하지 않는다.
  let content: ReactNode
  if (!tabs) {
    content = bootFailed ? (
      <div style={{ ...centerStyle, ...FILL }} data-testid="window-boot-error">
        <span>{t('window.loadFailed', { label })}</span>
      </div>
    ) : (
      <div data-testid="window-loading" style={{ ...blankStyle, ...FILL }} />
    )
  } else {
    content = (
      <>
        {framed && <SpareCanvas />}
        {tabs.map(tab => (
          <div
            key={tab.id}
            data-testid="tab-canvas"
            data-view-id={tab.id}
            // ★keep-alive★: 절대배치로 겹쳐 두어 전환이 리마운트가 아니라 표시 토글이 되게 한다(ADR-0056).
            style={{
              position: 'absolute',
              inset: 0,
              display: tab.id === shown ? 'block' : 'none',
            }}
          >
            <TabCanvas viewId={tab.id} />
          </div>
        ))}
        {/* 끌기 안내 — 늘 마운트하고 `null` 로 치운다(내리면 사라짐 페이드가 없다). 이 층이 캔버스와 같은 상자라 안내 좌표 =
            캔버스 CSS px(판정과 같은 원점). */}
        <div data-testid="dock-overlay-layer" style={OVERLAY_LAYER}>
          <DockOverlayView guide={guide} />
        </div>
      </>
    )
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', width: '100%', height: '100%' }}>
      <TitleRow ref={titleRowRef} label={label} number={framed ? null : (win?.number ?? null)} standby={framed}>
        {tabs ? (
          <TabBar
            label={label}
            titleRow={titleRowRef}
            tabs={tabs}
            active={active}
            onSwitch={viewId => void switchTab(label, viewId).catch(e => console.error('[switchTab]', e))}
            onCreate={() => void createTab(label).catch(e => console.error('[createTab]', e))}
            onClose={viewId => void closeTab(label, viewId).catch(e => console.error('[closeTab]', e))}
            onRename={(viewId, name) =>
              void renameTab(viewId, name).catch(e => console.error('[renameTab]', e))
            }
            canvas={canvasOf}
            // 모드를 모르는 동안도 당기지 않는다 — 대기 창이면 셸이 그 창을 몰라 실패한다.
            pullFrame={normal}
          />
        ) : (
          <TabBarStandIn pullFrame={normal} />
        )}
      </TitleRow>
      {/* 캔버스 크기 보고(callback ref)는 탭을 그리는 동안만 붙는다 — 탭 내용 영역의 크기다(ADR-0227). */}
      <div ref={tabs ? canvasRef : undefined} style={{ flex: 1, position: 'relative', minHeight: 0 }}>
        {/* 알림 층은 이 칸(positioned)의 맨 위 = 제목 줄 바로 아래에 붙는다 — 흐름 밖이라 캔버스 크기 보고를 바꾸지 않는다(ADR-0306). */}
        {notices}
        {content}
      </div>
      {/* ADR-0067: 에이전트 모니터링 검색 팝업 — 창당 하나 마운트. slot 우클릭 "에이전트 모니터링"
          (slot.assignRunningAgent)이 monitoringPickerStore 로 열고, on-select 는 assign_agent 로 흘린다(§5).
          닫힘 상태(target=null)면 아무것도 렌더하지 않는다.
          key={openId}: open() 마다 key 가 바뀌어 fresh remount → query/activeIndex 가 useState 초기값으로
          재설정된다(stale 플래시 없음, monitoringPickerStore.openId ADR-0067). */}
      {tabs && <AgentMonitoringPicker key={openId} />}
    </div>
  )
}

// 탭을 받기 전 화면의 탭바 자리 — 탭바처럼 남는 폭을 차지해 창 단추를 오른쪽 끝에 두고, 그 자리를 끌 자리로 둔다. 창
//   사실(최대화 · 앞 창)도 탭바처럼 받는다 — 그래야 이 화면의 줄 높이 · 최대화 단추가 창 상태를 따른다.
function TabBarStandIn({ pullFrame }: { pullFrame: boolean }) {
  useEffect(() => (pullFrame ? windowFrame.install() : undefined), [pullFrame])
  return <div data-testid="tab-bar-standin" aria-hidden {...dragProps()} style={TAB_BAR_STAND_IN} />
}

// 탭바 뿌리와 같은 몫 — 줄어드는 쪽 · 아래 선(`TabBar` 뿌리). 기준 폭은 `0px` 로 단위를 붙인다(jsdom 은 단위 없는 0 을 버린다).
const TAB_BAR_STAND_IN: React.CSSProperties = {
  flex: '1 1 0px',
  minWidth: 0,
  alignSelf: 'stretch',
  boxSizing: 'border-box',
  borderBottom: '1px solid var(--tab-bar-edge)',
}

const FILL: React.CSSProperties = { position: 'absolute', inset: 0 }

// 끌기 안내 층의 z = 칸 안의 분할 막대 · 포커스 링(z 20) 위 · 연결 안내(z 40) · 메뉴(1000 이상) 아래. 캔버스 상자와 탭 캔버스
//   층은 쌓임 맥락을 만들지 않아 그 안의 z 와 이 층이 같은 맥락에서 견준다.
const OVERLAY_LAYER: React.CSSProperties = { position: 'absolute', inset: 0, pointerEvents: 'none', zIndex: 30 }

/** 훅 인스턴스가 소유하는 관측 상태. */
interface CanvasObserverState {
  ro: ResizeObserver | null
  timer: ReturnType<typeof setTimeout> | null
  /** 디바운스 대기 중인 최신 측정(반올림 후). */
  measured: CanvasPx | null
}

// ADR-0227: 탭 내용 영역(창 캔버스) 크기를 재어 바뀔 때만 셸에 보고한다. 탭들이 이 영역을 공유하므로 창당
//   하나다. 부팅·팝아웃 생성·웹뷰 재로드는 모두 새 마운트라 첫 측정이 곧 보고다. 무엇을 언제 보낼지(한 번에
//   하나·합치기·재시도)는 `windowCanvasReport.ts` 가 정한다.
// callback ref 인 이유: 재는 div 는 win 이 온 뒤에야 렌더되므로(조기 반환) 마운트 때 effect 에선 아직 없다.
//   참조를 고정하는 이유: 렌더마다 새 함수면 React 가 떼고 다시 붙여 디바운스·재시도가 매번 날아간다.
function useWindowCanvasReport(): (el: HTMLDivElement | null) => void {
  const obsRef = useRef<CanvasObserverState>({ ro: null, timer: null, measured: null })

  return useCallback((el: HTMLDivElement | null) => {
    const obs = obsRef.current
    const st = getCanvasReport()
    detachCanvasObserver(obs)
    releaseCanvasReport(st)
    if (!el) return
    // jsdom(vitest 에 setupFiles 없음)처럼 RO 가 없는 환경에선 관측 없이 지나간다.
    if (typeof ResizeObserver === 'undefined') return
    const ro = new ResizeObserver(entries => {
      const rect = entries[entries.length - 1]?.contentRect
      if (!rect) return
      const size: CanvasPx = { w: Math.round(rect.width), h: Math.round(rect.height) }
      observeCanvasSize(st, size)
      obs.measured = size
      if (obs.timer !== null) clearTimeout(obs.timer)
      obs.timer = setTimeout(() => {
        obs.timer = null
        const measured = obs.measured
        obs.measured = null
        if (measured) submitCanvasSize(st, measured)
      }, REPORT_DEBOUNCE_MS)
    })
    ro.observe(el)
    obs.ro = ro
  }, [])
}

function detachCanvasObserver(obs: CanvasObserverState): void {
  obs.ro?.disconnect()
  obs.ro = null
  if (obs.timer !== null) {
    clearTimeout(obs.timer)
    obs.timer = null
  }
  obs.measured = null
}

// 대기 모드의 빈 캔버스 — 칸 틀 하나만 그린다. 탭이 없어 잎이 마운트되지 않으므로 칸 틀 지표(`report_ui_metrics`)를
//   여기서 보낸다 — 예비 창을 채택할 때 셸이 이 지표를 모델에 옮겨 적는다(TRD S21-slot-docking §3-9 `committed`).
// ADR-0295
function SpareCanvas() {
  const borderRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (borderRef.current !== null) void reportUiMetrics(borderRef.current)
  }, [])
  return (
    <div
      ref={borderRef}
      data-testid="spare-canvas"
      style={{ position: 'absolute', inset: 0, background: 'var(--bg)', boxSizing: 'border-box', ...SLOT_FRAME_BORDER }}
    />
  )
}

/**
 * viewIdOverride 로 자기 view 를 내려꽂아 안쪽 SlotContextMenu 액션이 이 탭 좌표를 쓰게 한다
 * (창별 active 무지 — §3-4).
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
      slotRects={cached.slotRects}
      splitRects={cached.splitRects}
      ratioBounds={{ min: cached.ratioMin, max: cached.ratioMax }}
      version={cached.version}
      foreignSlots={cached.foreignSlots}
    />
  )
}

const blankStyle: React.CSSProperties = {
  width: '100%',
  height: '100%',
  background: 'var(--bg)',
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
