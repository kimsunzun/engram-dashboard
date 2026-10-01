// 사용량 한도 슬롯(TRD S21 usage-limit-slot §1-8) — 켠 회사의 남은 양을 작은 표시로 그리고, 누르면 상세 팝업을 연다.
//
// ★조작은 command 로만 간다(§5)★ — 작은 표시의 ⟳(켠 회사 전부)와 팝업의 회사별 ⟳(`vendor` 인자)는 `usageSlot.refresh`,
//   팝업의 표시 토글은 `usageSlot.toggle*` 를 부른다. 슬롯 우클릭 메뉴엔 사용량 항목이 없다(사용자 결정 2026-09-29).
// ★켠 회사만 그린다★ — 끈 회사는 스토어에 값이 있어도 작은 표시·배지·팝업 상세 어디에도 없고, 이름은 팝업의 표시
//   토글 줄(다시 켤 자리)에만 남는다. 조회 수요(관심)는 셸이 레이아웃에서 계산하므로 이 컴포넌트는 관심을 보고하지 않는다.
// ★폭 단계는 그려진 폭으로 고른다(R3)★ — 단계마다 숨은 렌더의 자연 폭을 재어 실제 폭과 견준다. px 문턱을 두지
//   않는다: 글꼴·문구·값이 바뀌면 문턱도 따라 움직여야 해서다.
// ★시각은 분 단위 tick 하나가 굴린다★ — 나이·남은 시간·다음 시도가 요청 없이 로컬로 흐른다(R32).
// 값은 DOM 텍스트 + `data-usage-vendor`/`data-usage-window` 로 둔다(R27 — LLM·cdp 가 읽는다).

import { Fragment, useCallback, useEffect, useId, useLayoutEffect, useRef, useState } from 'react'
import type { CSSProperties, RefObject } from 'react'
import { Hourglass, RefreshCw } from 'lucide-react'
import { openUrl } from '@tauri-apps/plugin-opener'

import type { SlotContent } from '../../api/layoutTypes'
import type { AgentBackendKind } from '../../api/types'
import { fireAndForget } from '../../commands/dispatch'
import { t } from '../../i18n'
import {
  blocksRefresh,
  useUsagePending,
  useUsageStore,
  useUsageVendor,
  type UsageVendorEntry,
} from '../../store/usageStore'
import { ANCHOR_GAP, clampMenuPosition } from './SlotContextMenu'
import VendorIcon from './VendorIcon'
import {
  STALE_AFTER_SECS,
  elapsedSecs,
  formatAge,
  formatDuration,
  formatResetAt,
  formatResetClock,
  isStale,
  oldestAgeSecs,
  readWindow,
  stateHead,
  statusLine,
  statusSentence,
  type WindowReading,
} from './usageFormat'

type UsageContent = Extract<SlotContent, { type: 'usage' }>
type Stage = 1 | 2 | 3
type ValueReading = Extract<WindowReading, { kind: 'value' }>

/**
 * 회사마다 슬롯 내용의 표시 칸과 그 칸을 뒤집는 command. 팝업의 표시 토글 줄은 이 표의 차례로 그린다 — 회사가 늘면
 * (`AgentBackendKind`) 타입이 빠진 줄을 잡는다.
 */
const VENDOR_SLOT: Record<AgentBackendKind, { show: Exclude<keyof UsageContent, 'type'>; toggle: string }> = {
  claude: { show: 'show_claude', toggle: 'usageSlot.toggleClaude' },
  codex: { show: 'show_codex', toggle: 'usageSlot.toggleCodex' },
}
const USAGE_VENDORS = Object.keys(VENDOR_SLOT) as AgentBackendKind[]

/**
 * 「사용량 페이지 ↗」 목적지. ★셸의 opener 허용 목록(`src-tauri/capabilities/usage-links.json`)과 글자까지 같아야
 * 한다★ — 쿼리·끝 슬래시 하나라도 다르면 셸이 여는 것을 거절한다. 대조는 `UsageSlot.test.tsx` 가 한다.
 */
export const USAGE_PAGE_URL: Record<AgentBackendKind, string> = {
  claude: 'https://claude.ai/settings/usage',
  codex: 'https://chatgpt.com/codex/settings/usage',
}

const TICK_MS = 60_000

// 요약 버튼의 안쪽 여백. 가로 값은 단계 판정에 든다 — 숨은 사본은 여백 없이 재어지고, 내용이 쓸 폭은 슬롯 폭에서 빼서 얻는다.
const SUMMARY_PAD_Y_PX = 4
const SUMMARY_PAD_X_PX = 6

interface WindowView {
  /** `five_hour`·`weekly`, 팝업의 모델별 창은 `model:<label>`. */
  key: string
  label: string
  reading: WindowReading
}

interface VendorView {
  vendor: AgentBackendKind
  name: string
  entry: UsageVendorEntry | undefined
  /** 받은 뒤 흐른 초. */
  elapsed: number
  /** 회사 이름을 단 비정상 상태의 한 줄 — `Ready`·값 없음이면 null. 배지와 팝업 맨 위 줄이 같은 이 문자열을 쓴다. */
  line: string | null
  /** 비정상 상태의 짧은 이름(요약 버튼의 접근성 이름용) — `Ready`·값 없음이면 null. */
  head: string | null
  refreshing: boolean
  rejected: boolean
  windows: WindowView[]
}

function vendorName(vendor: AgentBackendKind): string {
  switch (vendor) {
    case 'claude':
      return t('usage.vendorClaude')
    case 'codex':
      return t('usage.vendorCodex')
  }
}

function buildVendorView(
  vendor: AgentBackendKind,
  entry: UsageVendorEntry | undefined,
  pending: boolean,
  nowPerf: number,
  nowWall: number,
): VendorView {
  const name = vendorName(vendor)
  const snapshot = entry?.snapshot
  const elapsed = entry ? elapsedSecs(entry.receivedAt, nowPerf) : 0
  // `Unavailable` 은 값을 싣지 않는다 — 실린 것이 있어도 그리지 않는다(회색 "—").
  const valueless = snapshot === undefined || snapshot.state.kind === 'Unavailable'
  const windowOf = (key: 'five_hour' | 'weekly') => (valueless ? null : snapshot[key])
  const sentence = snapshot ? statusSentence(snapshot.state, elapsed, nowWall) : null
  return {
    vendor,
    name,
    entry,
    elapsed,
    line: sentence === null ? null : statusLine(name, sentence),
    head: snapshot ? stateHead(snapshot.state, elapsed) : null,
    refreshing: pending || (snapshot?.in_flight ?? false),
    rejected: blocksRefresh(snapshot?.state),
    windows: [
      {
        key: 'five_hour',
        label: t('usage.windowFiveHour'),
        reading: readWindow(windowOf('five_hour'), elapsed, nowWall),
      },
      {
        key: 'weekly',
        label: t('usage.windowWeekly'),
        reading: readWindow(windowOf('weekly'), elapsed, nowWall),
      },
    ],
  }
}

function a11yValue(r: WindowReading): string {
  if (r.kind === 'value') return t('usage.meterValueText', { pct: String(r.visible) })
  if (r.kind === 'expired') return t('usage.resetWaitingShort')
  return t('usage.a11yNoValue')
}

/** 요약 버튼의 접근성 이름 — 회사 · 창마다 보이는 수 · 상태. */
function summaryLabel(views: VendorView[]): string {
  const parts = views.map(view => {
    const [first, second] = view.windows.map(w => t('usage.a11yWindow', { window: w.label, value: a11yValue(w.reading) }))
    const refreshing = view.refreshing ? t('usage.refreshing') : null
    const status =
      view.head !== null && refreshing !== null
        ? t('usage.a11yStatusRefreshing', { status: view.head, refreshing })
        : (view.head ?? refreshing)
    return status === null
      ? t('usage.a11yVendor', { vendor: view.name, first, second })
      : t('usage.a11yVendorStatus', { vendor: view.name, first, second, status })
  })
  return parts.length === 2 ? t('usage.a11yTwoVendors', { first: parts[0], second: parts[1] }) : parts[0]
}

function useMinuteTick(): void {
  const [, setTick] = useState(0)
  useEffect(() => {
    const id = setInterval(() => setTick(n => n + 1), TICK_MS)
    return () => clearInterval(id)
  }, [])
}

export default function UsageSlot({
  content,
  viewId,
  slotId,
}: {
  content: UsageContent
  /** 이 슬롯의 좌표 — ⟳·표시 토글이 command 에 그대로 싣는다. `null` = 활성 탭을 아직 모른다(토글 command 가 거절한다). */
  viewId: string | null
  slotId: string
}) {
  // TRD §3 #82: 방송 잇기 뒤 부팅 pull 과 별개로, 슬롯이 늦게 놓여도 셸 캐시로 곧바로 그린다.
  useEffect(() => {
    void useUsageStore.getState().pull()
  }, [])
  useMinuteTick()
  const claude = useUsageVendor('claude')
  const codex = useUsageVendor('codex')
  const claudePending = useUsagePending('claude')
  const codexPending = useUsagePending('codex')

  const nowPerf = performance.now()
  const nowWall = Date.now() / 1000
  const views: VendorView[] = []
  if (content.show_claude) views.push(buildVendorView('claude', claude, claudePending, nowPerf, nowWall))
  if (content.show_codex) views.push(buildVendorView('codex', codex, codexPending, nowPerf, nowWall))

  return <UsageSummary views={views} nowWall={nowWall} content={content} viewId={viewId} slotId={slotId} />
}

// ── 작은 표시 ──────────────────────────────────────────────────────────────────────────────

interface Measures {
  /** 슬롯 루트의 폭(`width:100%` — 내용·단계에 따라 변하지 않는다). */
  rootW: number | null
  /**
   * ⟳ 자리의 폭(⟳ 의 오른쪽 여백 포함). ⟳ 가 없으면(안내 문구) 0 — 관찰자는 0×0 요소를 관찰 시작 때 알리지 않아서
   * null 이 아니라 0 에서 시작한다.
   */
  refreshW: number
  s1: number | null
  s2: number | null
}

/**
 * 내용이 쓸 수 있는 폭 = 슬롯 폭 − ⟳ 자리 − 요약 버튼의 좌우 여백. 측정 사본(`s1`·`s2`)은 ⟳ 없이 잰다 — ⟳ 몫은 여기서
 * 한 번만 뺀다(사본에 넣으면 두 번 뺀다).
 */
function contentAvail({ rootW, refreshW }: Measures): number | null {
  return rootW === null ? null : rootW - refreshW - 2 * SUMMARY_PAD_X_PX
}

// ADR-0259: 단계는 폭으로만 고른다 — 높이 폴백(2단이 슬롯보다 키가 크면 3단)을 되살리지 말 것. 되살리면 낮은 슬롯에서
//   1단이 2단을 건너뛰고 곧장 3단으로 간다. 높이가 모자라면 루트의 `overflow: hidden` 이 아래 줄을 자르고, 그 줄의
//   배지도 함께 가려진다(받아들인 대가 — PRD R31).
/** 들어가는 가장 넓은 단계. 아직 못 쟀으면 1단(가장 자세한 것). 3단보다 좁으면 3단이 말줄임으로 잘린다. */
function pickStage(m: Measures): Stage {
  const { s1, s2 } = m
  const avail = contentAvail(m)
  if (avail === null || s1 === null) return 1
  if (s1 <= avail) return 1
  if (s2 !== null && s2 <= avail) return 2
  return 3
}

const ROOT_STYLE: CSSProperties = {
  position: 'relative',
  width: '100%',
  height: '100%',
  overflow: 'hidden',
  fontFamily: 'var(--font-ui)',
  fontSize: '12px',
  color: 'var(--text)',
}

const LINE_STYLE: CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: '0.6em',
  whiteSpace: 'nowrap',
  overflow: 'hidden',
  minWidth: 0,
}

// 1·2단은 격자 하나에 모든 줄을 얹는다 — 칸을 함께 써야 두 회사·두 창의 막대·%·리셋 시각이 세로로 맞는다(사용자 결정
//   2026-09-29). 창 하나 = 이름 | 막대 | % | 리셋 시각.
const WINDOW_TRACKS = 'auto minmax(4em, 1fr) auto auto'
const STAGE1_COLUMNS = `auto ${WINDOW_TRACKS} ${WINDOW_TRACKS}`
const STAGE2_COLUMNS = `auto ${WINDOW_TRACKS}`
const WINDOW_TRACK_COUNT = 4

// 칸 사이 틈은 막대↔% 가 가장 좁다(사용자 결정 2026-09-29 — 막대와 % 를 붙여 읽게). 다른 칸 사이는 여백을 더한다.
const GRID_STYLE: CSSProperties = {
  display: 'grid',
  alignItems: 'center',
  columnGap: '4px',
  rowGap: '2px',
  whiteSpace: 'nowrap',
  fontVariantNumeric: 'tabular-nums',
}
const NAME_TO_LABEL = '0.3em'
const GROUP_TO_GROUP = '0.9em'
const LABEL_TO_BAR = '0.25em'
const PCT_TO_RESET = '0.45em'

/** 요약 버튼의 화면 사각형 — 팝업은 아래로 펴고, 뒤집을 땐 요약의 위쪽 변에서 편다(요약을 덮지 않게). */
interface Anchor {
  left: number
  top: number
  bottom: number
}

function UsageSummary({
  views,
  nowWall,
  content,
  viewId,
  slotId,
}: {
  views: VendorView[]
  nowWall: number
  content: UsageContent
  viewId: string | null
  slotId: string
}) {
  const rootRef = useRef<HTMLDivElement>(null)
  const rowRef = useRef<HTMLDivElement>(null)
  const refreshAreaRef = useRef<HTMLSpanElement>(null)
  const measure1Ref = useRef<HTMLDivElement>(null)
  const measure2Ref = useRef<HTMLDivElement>(null)
  const buttonRef = useRef<HTMLButtonElement>(null)
  const [measures, setMeasures] = useState<Measures>({ rootW: null, refreshW: 0, s1: null, s2: null })
  const [open, setOpen] = useState(false)

  // 슬롯 폭 · ⟳ 자리 · 두 단계의 자연 폭을 한 관찰자로 잰다 — 숨은 렌더는 값·문구가 바뀌면 크기가 바뀌어 다시 불린다.
  // ★고른 단계에 따라 크기가 바뀌는 요소(요약 버튼·내용·줄)는 재지 않는다★ — 단계가 제 입력을 바꾸면 한 번 3단으로
  //   줄어든 폭이 다시 넓어지지 않거나 단계가 오간다.
  useLayoutEffect(() => {
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(entries => {
      setMeasures(prev => {
        let next = prev
        const put = (key: keyof Measures, value: number) => {
          if (next[key] !== value) next = { ...next, [key]: value }
        }
        for (const entry of entries) {
          const { width } = entry.contentRect
          if (entry.target === rootRef.current) put('rootW', width)
          else if (entry.target === refreshAreaRef.current) put('refreshW', width)
          else if (entry.target === measure1Ref.current) put('s1', width)
          else if (entry.target === measure2Ref.current) put('s2', width)
        }
        return next
      })
    })
    for (const el of [rootRef.current, refreshAreaRef.current, measure1Ref.current, measure2Ref.current]) {
      if (el) observer.observe(el)
    }
    return () => observer.disconnect()
  }, [])

  const stage = pickStage(measures)

  const closePopup = useCallback((restoreFocus: boolean) => {
    setOpen(false)
    if (restoreFocus) buttonRef.current?.focus()
  }, [])

  const runSlotCommand = (id: string) => fireAndForget(id, { viewId, slotId, content })
  const refreshVendor = (vendor: AgentBackendKind) =>
    fireAndForget('usageSlot.refresh', { viewId, slotId, content, vendor })
  // 누름이 방송 전에 연달아도 옛 `content` 그대로 보낸다 — 토글 command 가 제 칸 하나만 쓰므로 다른 회사의 앞 누름을
  //   되돌리지 않는다(`usageCommands`). 체크 표시도 `content` 다(낙관 갱신 없음 — ADR-0035).
  const toggleShown = (vendor: AgentBackendKind) => runSlotCommand(VENDOR_SLOT[vendor].toggle)
  const shown = Object.fromEntries(USAGE_VENDORS.map(v => [v, content[VENDOR_SLOT[v].show]])) as Record<
    AgentBackendKind,
    boolean
  >

  return (
    <div ref={rootRef} data-usage-slot="" style={ROOT_STYLE}>
      {/* ★⟳ 는 요약 버튼 밖의 형제다★ — 버튼 안에 버튼을 두지 않는다(누름이 팝업 열기와 겹친다).
          ★⟳ 는 모든 단계에서 내용 바로 오른쪽이다★(위치 = ADR-0258 결정 1 · 사용자 2026-09-30 「알아서 잘 붙이도록」 ·
          방법 = TRD §3 #104) — 1·2단은 격자의 막대 칸(`1fr`)이 요약 버튼을 줄 끝까지 채우고, 3단은 채울 칸이 없어 줄이 제
          내용 폭으로 줄어든다. 내용이 슬롯보다 넓으면 줄은 슬롯 폭에서 멈추고(`maxWidth` — 없으면 줄바꿈 없는 줄의 최소
          폭이 ⟳ 를 슬롯 밖으로 민다) 요약 버튼이 줄어 내용을 자른다 — ⟳ 자리는 줄지 않는다. 그래서 3단에선 자릿수·배지처럼
          값이 바뀌어 내용 폭이 바뀌면 ⟳ 가 따라 움직인다. 조회 중 표시는 ⟳ 자신의 회전이라 내용 폭을 바꾸지 않는다(ADR-0261). */}
      <div
        ref={rowRef}
        data-usage-row=""
        style={{ display: 'flex', alignItems: 'center', maxWidth: '100%', width: stage === 3 ? 'fit-content' : undefined }}
      >
        <button
          ref={buttonRef}
          type="button"
          data-usage-summary=""
          aria-haspopup="dialog"
          aria-expanded={open}
          // 버튼의 자식은 보조기술에 평평해지므로 안의 막대·배지 대신 짧은 이름을 준다(D14). 안내 문구만 있으면 그 글자가 이름이다.
          aria-label={views.length > 0 ? summaryLabel(views) : undefined}
          // 키보드(Enter·Space)는 네이티브 버튼 활성화가 같은 click 으로 부른다(D14).
          // ADR-0259: 열고 닫기만 한다 — 팝업을 열 때 조회를 붙이지 말 것. 주기 조회가 있으니 열 때 받지 않는 것이
          //   관용이고, 붙이면 여는 동작마다 Claude 429 위험이 커진다(429 가 거절 중 차단에 안 걸린다 — TRD §6 #21).
          onClick={() => setOpen(o => !o)}
          style={{
            display: 'block',
            flexGrow: 1,
            flexShrink: 1,
            flexBasis: 'auto',
            minWidth: 0,
            padding: `${SUMMARY_PAD_Y_PX}px ${SUMMARY_PAD_X_PX}px`,
            margin: 0,
            border: 0,
            background: 'transparent',
            color: 'inherit',
            font: 'inherit',
            textAlign: 'left',
            cursor: 'pointer',
          }}
        >
          <span data-usage-content="" data-usage-stage={stage} style={{ display: 'block', overflow: 'hidden' }}>
            {views.length === 0 ? (
              <span data-usage-hint="" style={{ color: 'var(--text-muted)' }}>
                {t('usage.hint')}
              </span>
            ) : (
              <StageRender stage={stage} views={views} nowWall={nowWall} measure={false} />
            )}
          </span>
        </button>
        {/* ⟳ 가 없을 때도 둔다 — 관찰자가 한 번 잡은 요소로 ⟳ 자리의 폭(없으면 0)을 받는다. */}
        <span ref={refreshAreaRef} data-usage-refresh-area="" style={{ flex: 'none', display: 'flex' }}>
          {views.length > 0 && <RefreshButton views={views} onRefresh={() => runSlotCommand('usageSlot.refresh')} />}
        </span>
      </div>
      <div
        aria-hidden="true"
        style={{ position: 'absolute', top: 0, left: 0, visibility: 'hidden', pointerEvents: 'none' }}
      >
        <div ref={measure1Ref} data-usage-measure="1" style={{ width: 'max-content' }}>
          <StageRender stage={1} views={views} nowWall={nowWall} measure />
        </div>
        <div ref={measure2Ref} data-usage-measure="2" style={{ width: 'max-content' }}>
          <StageRender stage={2} views={views} nowWall={nowWall} measure />
        </div>
      </div>
      {open && (
        <UsagePopup
          summaryRef={buttonRef}
          views={views}
          nowWall={nowWall}
          ownerRef={rowRef}
          shown={shown}
          onToggle={toggleShown}
          onRefresh={refreshVendor}
          onClose={closePopup}
        />
      )}
    </div>
  )
}

/**
 * 작은 표시의 ⟳ — 켠 회사를 한 번에 새로고침한다. 대상(켠 회사 중 보이는 거절이 아닌 것)은 command 가 누를 때
 * 스토어로 다시 고른다.
 */
function RefreshButton({ views, onRefresh }: { views: VendorView[]; onRefresh: () => void }) {
  const targets = views.filter(v => !v.rejected)
  // 회사별 조회 여부는 이 표식이 나른다(R27 — cdp·LLM) — 회전 하나로는 어느 회사가 도는지 안 보인다. 거절 중인 회사의
  //   조회도 싣는다(회전은 대상만 본다).
  const inFlight = views.filter(v => v.refreshing).map(v => v.vendor)
  return (
    <RefreshIconButton
      label={t('usage.refreshAll')}
      blocked={targets.length === 0}
      busy={targets.some(v => v.refreshing)}
      onRefresh={onRefresh}
      attrs={{
        'data-usage-refresh': '',
        ...(inFlight.length > 0 ? { 'data-usage-refreshing': inFlight.join(' ') } : {}),
      }}
      // 여백은 ⟳ 쪽에 둔다 — ⟳ 자리(`data-usage-refresh-area`)의 내용 폭이 이 여백까지 감싸 단계 판정의 ⟳ 몫에 든다.
      marginRight={SUMMARY_PAD_X_PX}
    />
  )
}

/**
 * ⟳ 모양 버튼 — 작은 표시의 ⟳ 와 팝업의 회사별 ⟳ 가 같은 누름 규칙을 쓴다(ADR-0259 결정 2 — 전역 ⟳ 규칙 준용).
 * `blocked` = 보이는 거절이라 누름을 막는다 · `busy` = 대상의 조회가 돌고 있어 누름을 버린다. 조회 중엔 아이콘이 돈다.
 */
function RefreshIconButton({
  label,
  blocked,
  busy,
  onRefresh,
  attrs,
  marginRight,
}: {
  label: string
  blocked: boolean
  busy: boolean
  onRefresh: () => void
  attrs: Record<string, string>
  marginRight?: number
}) {
  // ADR-0261: 회전 = 조회가 켜지면(자동 조회 포함) 돌기 시작해, 조회가 끝난 뒤 돌던 바퀴의 경계(`animationiteration`)
  //   에서 멈춘다 — 최소 한 바퀴이고, 끝에서 0° 로 튀지 않는다. 누름마다 아이콘을 새로 붙여 회전을 0° 부터 다시 시작한다
  //   (누름의 확인 — 조회를 보내지 않은 누름도). 바퀴 길이는 `animate-spin`(1초) 그대로다.
  // ★움직임 줄이기·e-ink 에서도 돈다★ — 이 저장소의 회전 기호 규칙(`agent/agentGlyph.css` · `ui/loading-panel.css` —
  //   사용자 결정 2026-08-24·2026-09-24: 멈춘 스피너는 「멈춤」과 구분되지 않는다).
  const [spinning, setSpinning] = useState(false)
  const [spinRun, setSpinRun] = useState(0)
  if (busy && !spinning) setSpinning(true)
  // ADR-0258: 누름을 막는 것은 아래 둘뿐이다 — 시간 간격은 두지 않는다(연타로 부를 429 는 받아들인 위험).
  // 보이는 거절 중엔 막는다 — 기한 안엔 ⟳ 도 조회하지 않고 돌지도 않는다(R24 · D11). 조회가 도는 동안의 누름은 버린다 —
  //   회전만 처음부터 다시 돈다(ADR-0261).
  return (
    <button
      type="button"
      {...attrs}
      aria-label={label}
      title={label}
      // ★`disabled` 가 아니라 `aria-disabled` 다★ — 포커스된 ⟳ 가 거절로 바뀌는 순간 `disabled` 는 포커스를 body 로
      //   떨군다(HTML focus fixup). 포커스를 쥔 채 누름만 막는다.
      aria-disabled={blocked || undefined}
      aria-busy={busy || undefined}
      onClick={() => {
        if (blocked) return
        if (!busy) onRefresh()
        setSpinning(true)
        setSpinRun(n => n + 1)
      }}
      style={{
        display: 'inline-flex',
        marginRight,
        padding: '2px',
        border: '1px solid var(--border)',
        borderRadius: '3px',
        background: 'transparent',
        color: 'inherit',
        cursor: blocked ? 'default' : busy ? 'progress' : 'pointer',
        opacity: blocked ? 0.4 : undefined,
      }}
    >
      <RefreshCw
        key={spinRun}
        aria-hidden="true"
        className={spinning ? 'size-3.5 animate-spin' : 'size-3.5'}
        onAnimationIteration={() => {
          if (!busy) setSpinning(false)
        }}
      />
    </button>
  )
}

/**
 * `measure` = 크기만 재는 숨은 사본 — 같은 글자·같은 모양을 그리되 역할·이름·`data-usage-*`·애니메이션은 싣지 않는다
 * (cdp·보조기술이 값을 두 번 읽지 않게). 줄도 `span` 인 것은 요약 버튼 안에 들어가서다(버튼 내용 = phrasing).
 */
function StageRender({
  stage,
  views,
  nowWall,
  measure,
}: {
  stage: Stage
  views: VendorView[]
  nowWall: number
  measure: boolean
}) {
  if (stage === 1) {
    return (
      <span
        {...(measure ? {} : { 'data-usage-grid': '' })}
        style={{ ...GRID_STYLE, gridTemplateColumns: STAGE1_COLUMNS }}
      >
        {views.map((view, v) => (
          <Fragment key={view.vendor}>
            <NameUnit view={view} measure={measure} cell={{ gridRow: v + 1, gridColumn: 1 }} />
            {view.windows.map((w, i) => (
              <WindowCells
                key={w.key}
                view={view}
                w={w}
                nowWall={nowWall}
                measure={measure}
                row={v + 1}
                column={2 + i * WINDOW_TRACK_COUNT}
                lead={i === 0 ? NAME_TO_LABEL : GROUP_TO_GROUP}
              />
            ))}
          </Fragment>
        ))}
      </span>
    )
  }
  if (stage === 2) {
    return (
      <span
        {...(measure ? {} : { 'data-usage-grid': '' })}
        style={{ ...GRID_STYLE, gridTemplateColumns: STAGE2_COLUMNS }}
      >
        {views.map((view, v) => {
          const first = v * view.windows.length + 1
          return (
            <Fragment key={view.vendor}>
              <NameUnit
                view={view}
                measure={measure}
                cell={{ gridRow: `${first} / span ${view.windows.length}`, gridColumn: 1 }}
              />
              {view.windows.map((w, i) => (
                <WindowCells
                  key={w.key}
                  view={view}
                  w={w}
                  nowWall={nowWall}
                  measure={measure}
                  row={first + i}
                  column={2}
                  lead={NAME_TO_LABEL}
                />
              ))}
            </Fragment>
          )
        })}
      </span>
    )
  }
  return (
    <>
      {views.map(view => (
        <span key={view.vendor} style={LINE_STYLE}>
          <NameUnit view={view} measure={measure} />
          {/* 말줄임은 숫자 쪽에만 — 아이콘·배지는 줄지 않아 잘리지 않는다(R31). */}
          <span
            data-usage-numbers=""
            style={{
              flex: '1 1 auto',
              minWidth: 0,
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}
          >
            {view.windows.map((w, i) => (
              <Fragment key={w.key}>
                {i > 0 && '·'}
                <span {...windowAttrs(view, w, measure)}>
                  <ValueText w={w} bare dimStale measure={measure} />
                </span>
              </Fragment>
            ))}
          </span>
        </span>
      ))}
    </>
  )
}

function windowAttrs(view: VendorView, w: WindowView, measure: boolean): Record<string, string> {
  return measure ? {} : { 'data-usage-vendor': view.vendor, 'data-usage-window': w.key }
}

const NAME_UNIT_GAP = '0.15em'

/** 회사 아이콘 + 배지 — 한 덩어리로 줄지 않는다(배지가 말줄임에 잘리지 않게 — R31). */
function NameUnit({ view, measure, cell }: { view: VendorView; measure: boolean; cell?: CSSProperties }) {
  return (
    <span style={{ ...cell, flex: 'none', display: 'inline-flex', alignItems: 'center', gap: NAME_UNIT_GAP }}>
      <VendorIcon
        vendor={view.vendor}
        label={measure ? null : view.name}
        {...(measure ? {} : { 'data-usage-icon': view.vendor })}
      />
      {view.line !== null && <Badge view={view} measure={measure} />}
    </span>
  )
}

/** 비정상 상태 표식 — 이름·문장은 팝업 맨 위 줄과 같다. */
function Badge({ view, measure }: { view: VendorView; measure: boolean }) {
  const label = view.line ?? ''
  return (
    <span
      {...(measure ? {} : { 'data-usage-badge': view.vendor, role: 'img', 'aria-label': label, title: label })}
      style={{ color: 'var(--usage-badge)', fontWeight: 700 }}
    >
      !
    </span>
  )
}

/**
 * 1·2단 격자의 창 하나 — 이름 | 막대(값이 있을 때만) | 남은 % | 리셋 시각, `column` 부터 네 칸. 만료면 막대·%·리셋 시각
 * 세 칸을 「리셋됨 — 갱신 대기」 한 칸이 덮는다. 감싸는 `span` 은 `display: contents` 라 칸들이 곧장 격자 항목이 되고,
 * 값 표식(`data-usage-vendor`·`data-usage-window`)만 나른다.
 */
function WindowCells({
  view,
  w,
  nowWall,
  measure,
  row,
  column,
  lead,
}: {
  view: VendorView
  w: WindowView
  nowWall: number
  measure: boolean
  row: number
  column: number
  lead: string
}) {
  const r = w.reading
  const at = (offset: number, span = 1): CSSProperties => ({
    gridRow: row,
    gridColumn: span === 1 ? column + offset : `${column + offset} / span ${span}`,
  })
  return (
    <span {...windowAttrs(view, w, measure)} style={{ display: 'contents' }}>
      <span style={{ ...at(0), marginLeft: lead, color: 'var(--text-muted)' }}>{w.label}</span>
      {r.kind === 'expired' ? (
        <span style={{ ...at(1, 3), marginLeft: LABEL_TO_BAR }}>
          <ValueText w={w} bare={false} dimStale measure={measure} />
        </span>
      ) : (
        <>
          {r.kind === 'value' && (
            <Bar
              view={view}
              label={w.label}
              r={r}
              meter={false}
              measure={measure}
              cell={{ ...at(1), width: 'auto', marginLeft: LABEL_TO_BAR }}
            />
          )}
          <span style={{ ...at(2), textAlign: 'right' }}>
            <ValueText w={w} bare={false} dimStale measure={measure} />
          </span>
          {r.resetsAt !== null && (
            <ResetMark
              resetsAt={r.resetsAt}
              nowWall={nowWall}
              measure={measure}
              attrs={{ 'data-usage-reset-clock': '' }}
              style={{ ...at(3), marginLeft: PCT_TO_RESET }}
            />
          )}
        </>
      )}
    </span>
  )
}

const RESET_ICON_GAP = '0.15em'

// ADR-0259: 리셋 = 모래시계 + 시각 — 낱말 「리셋」 은 툴팁(`title`)에만 남는다. 화살표 아이콘(`History`·`RotateCcw`·
//   `RotateCw`·`Repeat` 등)을 쓰지 말 것 — ⟳ 와 섞인다.
/**
 * 리셋 시각 — 작은 표시와 팝업이 같이 쓴다. 툴팁 = 「리셋 11:29」(다른 날이면 날짜 + 시각). `measure` = 숨은 사본(이름·
 * 역할·`attrs` 없음).
 */
function ResetMark({
  resetsAt,
  nowWall,
  measure,
  attrs,
  style,
}: {
  resetsAt: number
  nowWall: number
  measure: boolean
  attrs?: Record<string, string>
  style?: CSSProperties
}) {
  const name = formatResetClock(resetsAt, nowWall)
  return (
    <span
      // `aria-label` 을 따로 달지 않는다 — `role="img"` 는 이름을 `title` 에서 얻고, 같은 글을 `aria-label` 로도 주면
      //   이름에 안 쓰인 `title` 이 설명으로 붙어 같은 말이 두 번 읽힌다(accname).
      {...(measure ? {} : { ...attrs, role: 'img', title: name })}
      style={{ display: 'inline-flex', alignItems: 'center', gap: RESET_ICON_GAP, color: 'var(--text-muted)', ...style }}
    >
      <Hourglass aria-hidden="true" className="size-3" />
      {formatResetAt(resetsAt, nowWall)}
    </span>
  )
}

/**
 * 막대 — 값이 있을 때만 그린다(값이 없으면 막대 자체가 없다 — 0% 로 그리지 않는다, R21). `meter` = 보조기술에 막대로
 * 드러낸다(D14 — 팝업). 요약 버튼 안에서는 버튼 이름이 대신하므로 숨긴다.
 * ★색은 토큰만★ — e-ink 은 같은 토큰이 빗금·검정 채움·굵은 테두리로 풀린다(R11). `cell` = 격자 칸 자리·폭(1·2단은
 * 칸 폭을 채운다).
 */
function Bar({
  view,
  label,
  r,
  meter,
  measure,
  cell,
}: {
  view: VendorView
  label: string
  r: ValueReading
  meter: boolean
  measure: boolean
  cell?: CSSProperties
}) {
  const a11y = meter
    ? {
        role: 'meter',
        'aria-valuemin': 0,
        'aria-valuemax': 100,
        'aria-valuenow': r.visible,
        'aria-valuetext': t('usage.meterValueText', { pct: String(r.visible) }),
        'aria-label': t('usage.meterLabel', { vendor: view.name, window: label }),
      }
    : { 'aria-hidden': true }
  return (
    <span
      {...a11y}
      {...(measure ? {} : { 'data-usage-bar': '' })}
      style={{
        flex: 'none',
        display: 'inline-block',
        width: '4em',
        height: '0.6em',
        boxSizing: 'border-box',
        overflow: 'hidden',
        borderRadius: '2px',
        background: 'var(--usage-track)',
        border: r.level === 'danger' ? 'var(--usage-danger-border)' : 'var(--usage-bar-border)',
        ...cell,
      }}
    >
      <span
        data-usage-fill=""
        style={{ display: 'block', height: '100%', width: `${r.visible}%`, background: `var(--usage-${r.level}-fill)` }}
      />
    </span>
  )
}

/**
 * 남은 % 글자. `bare` = 숫자만 남는 3단(「%」 없이, 만료는 짧은 문구). 오래된 값은 숫자만 흐리게 — 나이 문구는
 * 팝업에만 둔다(R32 · 사용자 결정 2026-09-27). ★20% 미만에도 기호를 붙이지 않는다★ — 색 밖의 단서는 e-ink 막대
 * 모양이 진다(사용자 결정 2026-09-29 — R10 개정).
 */
function ValueText({
  w,
  bare,
  dimStale,
  measure,
}: {
  w: WindowView
  bare: boolean
  dimStale: boolean
  measure: boolean
}) {
  const r = w.reading
  const dataValue = measure ? {} : { 'data-usage-value': '' }
  if (r.kind === 'expired') {
    return (
      <span {...dataValue} {...(measure ? {} : { 'data-usage-expired': '' })} style={{ color: 'var(--text-muted)' }}>
        {bare ? t('usage.resetWaitingShort') : t('usage.resetWaiting')}
      </span>
    )
  }
  if (r.kind === 'none') {
    return (
      <span {...dataValue} style={{ color: 'var(--usage-none)' }}>
        —
      </span>
    )
  }
  const dim = dimStale && r.stale
  return (
    <span
      {...dataValue}
      {...(dim && !measure
        ? { 'data-usage-stale': '', title: t('usage.stale', { minutes: String(STALE_AFTER_SECS / 60) }) }
        : {})}
      data-usage-level={measure ? undefined : r.level}
      style={{ color: `var(--usage-${r.level})`, opacity: dim ? 0.5 : undefined }}
    >
      {`${r.visible}${bare ? '' : '%'}`}
    </span>
  )
}

// ── 팝업 ──────────────────────────────────────────────────────────────────────────────────

/**
 * 아래로 펴는 것이 먼저, 안 들어가면 요약의 위쪽 변에서 위로 뒤집는다 — 요약의 아래쪽 변에서 뒤집으면 팝업이 요약을
 * 덮는다. 아래·위 어느 쪽에도 안 들어가면 슬롯 우클릭 메뉴와 같은 밀기로 떨어지고, ★그때는 팝업이 요약을 일부 덮을 수
 * 있다★(화면 안에 두는 것이 먼저다). 가로는 그 메뉴 규칙 그대로(R7).
 */
function placePopup(anchor: Anchor, w: number, h: number, vw: number, vh: number): { top: number; left: number } {
  const pushed = clampMenuPosition(anchor.left, anchor.bottom, w, h, vw, vh)
  const below = anchor.bottom + ANCHOR_GAP
  if (below + h <= vh) return { top: below, left: pushed.left }
  const above = anchor.top - ANCHOR_GAP - h
  if (above >= 0) return { top: above, left: pushed.left }
  return pushed
}

// ADR-0261: 팝업의 두 회사 칸은 한 표다 — 격자 하나에 열을 두고 회사 칸·창 줄은 그 열을 물려받는다(`subgrid`). 줄마다
//   따로 놓으면 긴 이름(`gpt-reserve`)이 그 줄의 막대만 밀고, % 자릿수가 그 줄의 모래시계만 민다.
//   열 = 이름 | 막대 | 남은 % | 리셋 시각 | 남은 시간 「(… 뒤)」. 회사 머리 · 링크 줄은 열 전체에 걸친다.
const POPUP_TABLE_STYLE: CSSProperties = {
  display: 'grid',
  gridTemplateColumns: 'repeat(5, auto)',
  // 팝업이 표보다 넓을 때(긴 상태 줄) `auto` 열이 남는 폭을 나눠 늘어나지 않게 — 늘면 막대와 % 사이가 벌어진다.
  justifyContent: 'start',
  columnGap: '0.5em',
  // 회사 칸 사이 — 팝업의 다른 덩어리 사이 틈과 같다.
  rowGap: '8px',
  whiteSpace: 'nowrap',
  fontVariantNumeric: 'tabular-nums',
}
const POPUP_SUBGRID: CSSProperties = { display: 'grid', gridTemplateColumns: 'subgrid', gridColumn: '1 / -1' }
const POPUP_FULL_ROW: CSSProperties = { gridColumn: '1 / -1' }
// 창 줄의 높이를 박는다 — 글꼴이 섞인 줄(한글 대체 글꼴 · 라틴 이름)은 내용 높이가 줄마다 달라 줄 간격이 들쭉날쭉하다.
const POPUP_WINDOW_ROW_HEIGHT = '1.5em'

// ADR-0259: 팝업은 조회를 부르지 않는다 — 팝업 안에서 새로 받는 길은 회사별 ⟳ 뿐이다(열 때 조회 없음의 사유 = 요약
//   버튼 onClick).
function UsagePopup({
  summaryRef,
  views,
  nowWall,
  ownerRef,
  shown,
  onToggle,
  onRefresh,
  onClose,
}: {
  /** 팝업을 붙이는 요약 버튼. */
  summaryRef: RefObject<HTMLElement | null>
  views: VendorView[]
  nowWall: number
  /** 팝업을 연 요약 줄(요약 버튼 · ⟳) — 그 안의 누름·포커스는 팝업 밖으로 치지 않는다. */
  ownerRef: RefObject<HTMLDivElement | null>
  shown: Record<AgentBackendKind, boolean>
  onToggle: (vendor: AgentBackendKind) => void
  /** 회사 하나 새로고침 — 팝업은 열린 채다. */
  onRefresh: (vendor: AgentBackendKind) => void
  onClose: (restoreFocus: boolean) => void
}) {
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null)

  // 팝업과 요약을 그릴 때마다 재되, 자리가 같으면 멈춘다 — 팝업 높이는 값에 따라 달라지고, 열린 채 회사를 켜면 요약
  //   격자에 줄이 늘어 연 순간의 요약 사각형으로 두면 팝업이 그 줄을 덮는다. 요약의 크기는 이 팝업의 부모가 다시
  //   그려야 바뀐다(내용 · 단계 · 슬롯 크기는 부모의 측정 관찰자가 상태로 받는다) — 그래서 따로 관찰하지 않는다. 크기
  //   없이 자리만 옮는 것(같은 크기 슬롯 맞바꿈 등)은 다시 재지 않는다.
  useLayoutEffect(() => {
    const popup = ref.current
    const summary = summaryRef.current
    if (!popup || !summary) return
    const { width, height } = popup.getBoundingClientRect()
    const next = placePopup(summary.getBoundingClientRect(), width, height, window.innerWidth, window.innerHeight)
    setPos(prev => (prev !== null && prev.top === next.top && prev.left === next.left ? prev : next))
  })

  // ★자리를 잡은 뒤에만 포커스를 옮긴다★ — 첫 커밋은 재기 전이라 `visibility:hidden` 이고, Chromium 은 보이지 않는
  //   요소의 focus() 를 거절한다(그러면 Esc·Tab 이 팝업에 닿지 않는다).
  const positioned = pos !== null
  useEffect(() => {
    if (positioned) ref.current?.focus()
  }, [positioned])

  useEffect(() => {
    const inPopup = (node: Node | null) => node !== null && (ref.current?.contains(node) ?? false)
    const inOwner = (node: Node | null) => node !== null && (ownerRef.current?.contains(node) ?? false)
    // Esc 는 포커스가 팝업·요약 줄에 있을 때만 — 다른 곳의 Esc 를 가로채지 않는다. ★포커스가 아무 데도 없을 때(body·null)도
    //   닫는다★ — 팝업 안의 포커스된 요소가 빠지면(다시 그려져 사라짐 등) 포커스가 body 로 떨어지고, 그때 막으면 닫을
    //   길이 마우스뿐이다. 다른 요소로 간 포커스는 아래 focusin 이 이미 닫았다. 요약 줄(⟳ 등)에 있던 포커스는 그 자리에 둔다.
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      const active = document.activeElement
      if (active === null || active === document.body || inPopup(active)) onClose(true)
      else if (inOwner(active)) onClose(false)
    }
    // 요약 줄 위 누름은 닫지 않는다 — 요약 버튼은 자기 click 이 닫고(여기서도 닫으면 click 이 곧바로 다시 연다), ⟳ 는
    //   팝업을 연 채 새로고침한다(상세를 보며 누르게).
    const onDown = (e: MouseEvent) => {
      const target = e.target as Node
      if (!inPopup(target) && !inOwner(target)) onClose(false)
    }
    // 포커스가 밖으로 나가면 닫되, 포커스는 간 자리에 둔다(되찾아 오지 않는다).
    const onFocusIn = (e: FocusEvent) => {
      const target = e.target as Node
      if (!inPopup(target) && !inOwner(target)) onClose(false)
    }
    document.addEventListener('keydown', onKey)
    document.addEventListener('mousedown', onDown)
    document.addEventListener('focusin', onFocusIn)
    return () => {
      document.removeEventListener('keydown', onKey)
      document.removeEventListener('mousedown', onDown)
      document.removeEventListener('focusin', onFocusIn)
    }
  }, [onClose, ownerRef])

  const troubled = views.filter(v => v.line !== null)
  return (
    <div
      ref={ref}
      role="dialog"
      aria-label={t('usage.popupLabel')}
      data-usage-popup=""
      tabIndex={-1}
      style={{
        position: 'fixed',
        top: pos?.top ?? 0,
        left: pos?.left ?? 0,
        visibility: pos ? 'visible' : 'hidden',
        zIndex: 1000,
        display: 'flex',
        flexDirection: 'column',
        gap: '8px',
        minWidth: '220px',
        maxWidth: 'min(90vw, 32rem)',
        padding: '8px 10px',
        background: 'var(--bg-secondary)',
        border: '1px solid var(--border)',
        borderRadius: '4px',
        boxShadow: '0 2px 8px rgba(0,0,0,0.3)',
        color: 'var(--text)',
        fontFamily: 'var(--font-ui)',
        fontSize: '12px',
        outline: 'none',
        cursor: 'default',
      }}
    >
      {troubled.length > 0 && (
        // 상태 문장의 정본 자리(R31 — 터치엔 hover 가 없다). 배지와 같은 문자열이다.
        <div data-usage-status-lines="" style={{ display: 'flex', flexDirection: 'column', gap: '2px' }}>
          {troubled.map(view => (
            <div key={view.vendor} data-usage-status-line={view.vendor} style={{ color: 'var(--usage-badge)' }}>
              <span data-usage-status-text="">{view.line}</span>
            </div>
          ))}
        </div>
      )}
      {views.length > 0 && (
        <div data-usage-popup-table="" style={POPUP_TABLE_STYLE}>
          {views.map(view => (
            <VendorDetail key={view.vendor} view={view} nowWall={nowWall} onRefresh={onRefresh} />
          ))}
        </div>
      )}
      <ShowToggles shown={shown} onToggle={onToggle} divided={views.length > 0} />
    </div>
  )
}

// ADR-0258: 켜고 끄기의 사람 UI 는 여기 하나다 — 슬롯 우클릭 메뉴엔 사용량 항목이 없다.
/**
 * 팝업 맨 아래 표시 토글 줄 — 켠 회사와 끈 회사를 다 싣는다(끈 회사를 다시 켤 자리가 여기다). 체크 상태는 슬롯 내용을
 * 그대로 그린다 — 누름은 command 로 가고, 레이아웃이 바뀐 내용을 돌려줄 때 체크가 따라 바뀐다.
 */
function ShowToggles({
  shown,
  onToggle,
  divided,
}: {
  shown: Record<AgentBackendKind, boolean>
  onToggle: (vendor: AgentBackendKind) => void
  divided: boolean
}) {
  const headId = useId()
  return (
    <div
      role="group"
      aria-labelledby={headId}
      data-usage-show-toggles=""
      style={{
        display: 'flex',
        flexWrap: 'wrap',
        alignItems: 'center',
        columnGap: '0.8em',
        rowGap: '2px',
        ...(divided ? { borderTop: '1px solid var(--border)', paddingTop: '6px' } : {}),
      }}
    >
      <span id={headId} style={{ color: 'var(--text-muted)' }}>
        {t('usage.showOnSlot')}
      </span>
      {USAGE_VENDORS.map(vendor => (
        <label
          key={vendor}
          style={{ display: 'inline-flex', alignItems: 'center', gap: '0.3em', whiteSpace: 'nowrap', cursor: 'pointer' }}
        >
          <input
            type="checkbox"
            data-usage-show={vendor}
            checked={shown[vendor]}
            onChange={() => onToggle(vendor)}
            style={{ margin: 0, cursor: 'pointer' }}
          />
          {vendorName(vendor)}
        </label>
      ))}
    </div>
  )
}

/**
 * 한 회사의 상세 — 머리(이름 · plan(있을 때만) · 나이 · 그 회사만의 ⟳) · 창별 절대 리셋 시각 · 모델별 창(값이 있는 것만
 * — D8) · 링크. 나이는 머리에 한 번이고 창 줄마다 되풀지 않는다(ADR-0259 결정 1). 창 줄의 열은 팝업 표의 열이다(ADR-0261).
 */
function VendorDetail({
  view,
  nowWall,
  onRefresh,
}: {
  view: VendorView
  nowWall: number
  onRefresh: (vendor: AgentBackendKind) => void
}) {
  const snapshot = view.entry?.snapshot
  // ADR-0259: 머리의 나이 = 5시간·주간 두 창(`view.windows`) 가운데 가장 오래된 나이 — 아래에서 덧붙이는 모델별 창을
  //   넣지 말 것(사용자 결정). 줍기는 모델별 창을 싣지 않아 대화가 이어지는 동안 그 창만 몇 시간씩 늙고, 넣으면 두 창이
  //   새것인데도 머리가 오래됨(호박색)으로 보인다. 두 창 다 값이 없으면 나이를 쓰지 않는다. 호박색도 이 값을 본다.
  const age = oldestAgeSecs(view.windows.map(w => w.reading))
  const stale = age !== null && isStale(age)
  const rows: WindowView[] = [...view.windows]
  if (snapshot && snapshot.state.kind !== 'Unavailable') {
    for (const scoped of snapshot.model_scoped) {
      if (scoped.window.used_pct === null) continue
      rows.push({
        key: `model:${scoped.label}`,
        label: scoped.label,
        reading: readWindow(scoped.window, view.elapsed, nowWall),
      })
    }
  }
  const url = USAGE_PAGE_URL[view.vendor]
  return (
    <section data-usage-popup-vendor={view.vendor} style={{ ...POPUP_SUBGRID, rowGap: '3px' }}>
      <div style={{ ...POPUP_FULL_ROW, display: 'flex', alignItems: 'center', gap: '0.6em' }}>
        <span style={{ display: 'inline-flex', alignItems: 'center', gap: '0.35em', fontWeight: 600 }}>
          {/* 이름 글자가 바로 옆에 있어 아이콘은 장식이다 — 보조기술이 이름을 두 번 읽지 않게. */}
          <VendorIcon vendor={view.vendor} label={null} data-usage-icon={view.vendor} />
          <span>{view.name}</span>
        </span>
        {snapshot?.plan != null && (
          <span data-usage-plan="" style={{ color: 'var(--text-muted)' }}>
            {t('usage.plan', { plan: snapshot.plan })}
          </span>
        )}
        {snapshot?.plan != null && age !== null && (
          <span aria-hidden="true" style={{ color: 'var(--text-muted)' }}>
            {t('usage.headSeparator')}
          </span>
        )}
        {/* ADR-0259: 나이와 ⟳ 는 붙여 둔다 — 시간 바로 옆의 ⟳ 라야 새로고침으로 읽힌다(사용자 결정). 나이에 시계 아이콘을
            붙이지 말 것(「아이콘 2개 나와서 난잡」). */}
        <span style={{ display: 'inline-flex', alignItems: 'center', gap: '0.35em' }}>
          {age !== null && (
            <span
              data-usage-age=""
              {...(stale
                ? { 'data-usage-stale': '', title: t('usage.stale', { minutes: String(STALE_AFTER_SECS / 60) }) }
                : {})}
              style={{ color: stale ? 'var(--usage-stale)' : 'var(--text-muted)' }}
            >
              {formatAge(age)}
            </span>
          )}
          <RefreshIconButton
            label={t('usage.refreshVendor', { vendor: view.name })}
            blocked={view.rejected}
            busy={view.refreshing}
            onRefresh={() => onRefresh(view.vendor)}
            attrs={{
              'data-usage-vendor-refresh': view.vendor,
              ...(view.refreshing ? { 'data-usage-refreshing': view.vendor } : {}),
            }}
          />
        </span>
      </div>
      {rows.map(row => (
        <PopupWindowRow key={row.key} view={view} row={row} nowWall={nowWall} />
      ))}
      <div style={POPUP_FULL_ROW}>
        <button
          type="button"
          data-usage-link={view.vendor}
          title={t('usage.linkTitle', { vendor: view.name, url })}
          onClick={() => {
            openUrl(url).catch((err: unknown) => console.warn(`[UsageSlot] openUrl(${url}) 실패:`, err))
          }}
          style={{
            padding: 0,
            border: 0,
            background: 'transparent',
            color: 'var(--accent)',
            font: 'inherit',
            cursor: 'pointer',
            textDecoration: 'underline',
          }}
        >
          {t('usage.link')}
        </button>
      </div>
    </section>
  )
}

/**
 * 팝업의 창 한 줄 — 표의 다섯 칸에 자리를 박아 놓는다(값이 없어 막대가 없거나 리셋 시각이 없어도 뒤 칸이 당겨지지
 * 않는다). 만료면 「리셋됨 — 갱신 대기」 한 칸이 막대부터 끝까지 덮는다.
 */
function PopupWindowRow({ view, row, nowWall }: { view: VendorView; row: WindowView; nowWall: number }) {
  const r = row.reading
  const resetsAt = r.kind === 'expired' ? null : r.resetsAt
  const resetInSecs = r.kind === 'expired' ? null : r.resetInSecs
  return (
    <div
      data-usage-vendor={view.vendor}
      data-usage-window={row.key}
      style={{ ...POPUP_SUBGRID, alignItems: 'center', height: POPUP_WINDOW_ROW_HEIGHT }}
    >
      {/* 왼쪽 들여쓰기는 이름 칸 안에 둔다 — 줄 쪽 여백은 subgrid 의 첫 열 폭에 섞인다. */}
      <span style={{ gridColumn: 1, paddingLeft: '0.75em', color: 'var(--text-muted)' }}>{row.label}</span>
      {r.kind === 'expired' ? (
        <span style={{ gridColumn: '2 / -1' }}>
          <ValueText w={row} bare={false} dimStale={false} measure={false} />
        </span>
      ) : (
        <>
          {r.kind === 'value' && (
            <Bar view={view} label={row.label} r={r} meter measure={false} cell={{ gridColumn: 2 }} />
          )}
          {/* 팝업은 흐리게 하지 않는다 — 오래됨은 회사 머리의 나이가 호박색으로 보인다(R32 · ADR-0259). */}
          <span style={{ gridColumn: 3, textAlign: 'right' }}>
            <ValueText w={row} bare={false} dimStale={false} measure={false} />
          </span>
        </>
      )}
      {resetsAt !== null && resetInSecs !== null && (
        // 남은 시간은 이름 밖에 둔다 — 이름(「리셋 11:29」)을 진 표식은 안의 글자를 보조기술에 감추므로, 안에 넣으면
        //   남은 시간이 읽히지 않는다.
        // 둘 사이의 공백 글자는 DOM 글자를 읽는 쪽(R27 — LLM·cdp)을 위한 것이다. 보이는 틈은 칸 사이 틈이 그린다 — 격자
        //   안에서 공백만 든 글자는 그려지지 않는다. 감싸는 `span` 은 `display: contents` 라 두 칸이 곧장 줄의 칸이 된다.
        <span data-usage-reset-at="" style={{ display: 'contents' }}>
          <ResetMark resetsAt={resetsAt} nowWall={nowWall} measure={false} style={{ gridColumn: 4 }} />{' '}
          <span data-usage-reset-in="" style={{ gridColumn: 5, color: 'var(--text-muted)' }}>
            {t('usage.resetIn', { duration: formatDuration(resetInSecs) })}
          </span>
        </span>
      )}
    </div>
  )
}
