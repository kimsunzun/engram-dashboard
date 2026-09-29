// 사용량 한도 슬롯(TRD S21 usage-limit-slot §1-8) — 켠 회사의 남은 양을 작은 표시로 그리고, 누르면 상세 팝업을 연다.
//
// ★조작은 command 로만 간다(§5)★ — 작은 표시의 ⟳ 는 `usageSlot.refresh`, 팝업의 표시 토글은 `usageSlot.toggle*` 를
//   부른다. 슬롯 우클릭 메뉴엔 사용량 항목이 없다(사용자 결정 2026-09-29).
// ★켠 회사만 그린다★ — 끈 회사는 스토어에 값이 있어도 작은 표시·배지·팝업 상세 어디에도 없고, 이름은 팝업의 표시
//   토글 줄(다시 켤 자리)에만 남는다. 조회 수요(관심)는 셸이 레이아웃에서 계산하므로 이 컴포넌트는 관심을 보고하지 않는다.
// ★폭 단계는 그려진 크기로 고른다(R3)★ — 단계마다 숨은 렌더의 자연 크기를 재어 실제 크기와 견준다. px 문턱을 두지
//   않는다: 글꼴·문구·값이 바뀌면 문턱도 따라 움직여야 해서다.
// ★시각은 분 단위 tick 하나가 굴린다★ — 나이·남은 시간·다음 시도가 요청 없이 로컬로 흐른다(R32).
// 값은 DOM 텍스트 + `data-usage-vendor`/`data-usage-window` 로 둔다(R27 — LLM·cdp 가 읽는다).

import { Fragment, useCallback, useEffect, useId, useLayoutEffect, useRef, useState } from 'react'
import type { CSSProperties, RefObject } from 'react'
import { RefreshCw } from 'lucide-react'
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

// 요약 버튼의 안쪽 여백. 두 값 다 단계 판정에 든다 — 숨은 사본은 여백 없이 재어지고, 내용이 쓸 폭은 슬롯 폭에서 빼서 얻는다.
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
  /** 슬롯 루트의 크기(`width`·`height:100%` — 내용·단계에 따라 변하지 않는다). */
  rootW: number | null
  rootH: number | null
  /**
   * ⟳ 자리의 폭(⟳ 의 오른쪽 여백 포함). ⟳ 가 없으면(안내 문구) 0 — 관찰자는 0×0 요소를 관찰 시작 때 알리지 않아서
   * null 이 아니라 0 에서 시작한다.
   */
  refreshW: number
  s1: number | null
  s2: number | null
  s2h: number | null
}

/**
 * 내용이 쓸 수 있는 폭 = 슬롯 폭 − ⟳ 자리 − 요약 버튼의 좌우 여백. 측정 사본(`s1`·`s2`)은 ⟳ 없이 잰다 — ⟳ 몫은 여기서
 * 한 번만 뺀다(사본에 넣으면 두 번 뺀다).
 */
function contentAvail({ rootW, refreshW }: Measures): number | null {
  return rootW === null ? null : rootW - refreshW - 2 * SUMMARY_PAD_X_PX
}

/**
 * 들어가는 가장 넓은 단계. 아직 못 쟀으면 1단(가장 자세한 것). 2단은 높이도 봐서, 슬롯보다 키가 크면 3단으로 간다 —
 * 잘린 줄의 배지가 사라지면 안 된다(R31 — 모든 단계에서 배지). 3단보다 좁으면 3단이 말줄임으로 잘린다.
 */
function pickStage(m: Measures): Stage {
  const { rootH, s1, s2, s2h } = m
  const avail = contentAvail(m)
  if (avail === null || s1 === null) return 1
  if (s1 <= avail) return 1
  const fitsHeight = s2h === null || rootH === null || s2h + 2 * SUMMARY_PAD_Y_PX <= rootH
  if (s2 !== null && s2 <= avail && fitsHeight) return 2
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
  const [measures, setMeasures] = useState<Measures>({
    rootW: null,
    rootH: null,
    refreshW: 0,
    s1: null,
    s2: null,
    s2h: null,
  })
  const [open, setOpen] = useState(false)

  // 슬롯 크기 · ⟳ 자리 · 두 단계의 자연 크기를 한 관찰자로 잰다 — 숨은 렌더는 값·문구가 바뀌면 크기가 바뀌어 다시 불린다.
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
          const { width, height } = entry.contentRect
          if (entry.target === rootRef.current) {
            put('rootW', width)
            put('rootH', height)
          } else if (entry.target === refreshAreaRef.current) put('refreshW', width)
          else if (entry.target === measure1Ref.current) put('s1', width)
          else if (entry.target === measure2Ref.current) {
            put('s2', width)
            put('s2h', height)
          }
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
          ★⟳ 는 모든 단계에서 내용 바로 오른쪽이다★(위치 = ADR-0257 결정 1 · 사용자 2026-09-30 「알아서 잘 붙이도록」 ·
          방법 = TRD §3 #104) — 1·2단은 격자의 막대 칸(`1fr`)이 요약 버튼을 줄 끝까지 채우고, 3단은 채울 칸이 없어 줄이 제
          내용 폭으로 줄어든다. 내용이 슬롯보다 넓으면 줄은 슬롯 폭에서 멈추고(`maxWidth` — 없으면 줄바꿈 없는 줄의 최소
          폭이 ⟳ 를 슬롯 밖으로 민다) 요약 버튼이 줄어 내용을 자른다 — ⟳ 자리는 줄지 않는다. 그래서 3단에선 내용 폭이
          바뀌면 ⟳ 가 따라 움직인다: 갱신 중 표식은 폭을 바꾸지 않게 막고(`RefreshingMarkSpace`), 자릿수·배지처럼 값이
          바뀐 것은 움직여도 둔다. */}
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
  const label = t('usage.refreshAll')
  const targets = views.filter(v => !v.rejected)
  // ADR-0257: 누름을 막는 것은 아래 둘뿐이다 — 시간 간격은 두지 않는다(연타로 부를 429 는 받아들인 위험).
  // 보이는 거절 중엔 막는다 — 기한 안엔 ⟳ 도 조회하지 않는다(R24 · D11).
  const blocked = targets.length === 0
  // 대상의 조회가 도는 동안의 누름은 버린다 — 답이 올 때까지 「갱신 중」 표식이 그 동안을 보인다.
  const busy = targets.some(v => v.refreshing)
  return (
    <button
      type="button"
      data-usage-refresh=""
      aria-label={label}
      title={label}
      // ★`disabled` 가 아니라 `aria-disabled` 다★ — 포커스된 ⟳ 가 거절로 바뀌는 순간 `disabled` 는 포커스를 body 로
      //   떨군다(HTML focus fixup). 포커스를 쥔 채 누름만 막는다.
      aria-disabled={blocked || undefined}
      aria-busy={busy || undefined}
      onClick={() => {
        if (!blocked && !busy) onRefresh()
      }}
      style={{
        display: 'inline-flex',
        // 여백은 ⟳ 쪽에 둔다 — ⟳ 자리(`data-usage-refresh-area`)의 내용 폭이 이 여백까지 감싸 단계 판정의 ⟳ 몫에 든다.
        marginRight: `${SUMMARY_PAD_X_PX}px`,
        padding: '2px',
        border: '1px solid var(--border)',
        borderRadius: '3px',
        background: 'transparent',
        color: 'inherit',
        cursor: blocked ? 'default' : busy ? 'progress' : 'pointer',
        opacity: blocked ? 0.4 : undefined,
      }}
    >
      <RefreshCw aria-hidden="true" className="size-3.5" />
    </button>
  )
}

/**
 * `measure` = 크기만 재는 숨은 사본 — 같은 글자·같은 모양을 그리되 역할·이름·`data-usage-*`·애니메이션은 싣지 않는다
 * (cdp·보조기술이 값을 두 번 읽지 않게). 갱신 중 표식만은 조회 여부와 상관없이 늘 그린다(`NameUnit`). 줄도 `span` 인
 * 것은 요약 버튼 안에 들어가서다(버튼 내용 = phrasing).
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
          <span style={{ flex: '1 1 auto', minWidth: 0, display: 'flex', flexWrap: 'wrap', alignItems: 'center' }}>
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
            {!measure && !view.refreshing && <RefreshingMarkSpace view={view} />}
          </span>
        </span>
      ))}
    </>
  )
}

/**
 * 3단 줄 끝의 빈자리 — 갱신 중 표식이 없는 동안 그 표식(+ 이름 묶음 안의 틈)과 같은 폭을 비워 둔다. 3단은 줄이 내용
 * 폭으로 줄어들어 내용 폭이 곧 ⟳ 자리라서, 표식이 켜지고 꺼질 때 ⟳ 가 움직이지 않게 한다(조회 동안 옛 자리를 다시
 * 누르면 ⟳ 대신 요약 버튼이 눌려 팝업이 열린다).
 * ★숫자와 함께 줄바꿈 묶음에 들고 높이가 0 이다★ — 슬롯이 좁으면 이 자리가 먼저 다음 줄(높이 0)로 넘어가 사라지고, 그다음
 * 에야 숫자가 말줄임으로 줄어든다. 숫자 쪽에 넣으면 이 자리 때문에 말줄임이 먼저 나온다. 그래서 슬롯이 3단 내용보다 좁은
 * 동안엔 쉬는 숫자가 폭을 다 쓰고, 조회 중엔 표식 폭(약 14px)만큼 줄어 말줄임이 더 일찍 온다 — ⟳ 는 그때도 움직이지
 * 않는다(줄이 슬롯 폭에 묶여 있다).
 * 측정 사본엔 두지 않는다 — 사본의 이름 묶음이 표식을 늘 세므로(`NameUnit`) 여기까지 두면 두 번 센다.
 */
function RefreshingMarkSpace({ view }: { view: VendorView }) {
  return (
    <span
      aria-hidden="true"
      style={{
        flex: 'none',
        display: 'inline-flex',
        height: 0,
        overflow: 'hidden',
        visibility: 'hidden',
        marginLeft: NAME_UNIT_GAP,
      }}
    >
      <RefreshingMark view={view} measure />
    </span>
  )
}

function windowAttrs(view: VendorView, w: WindowView, measure: boolean): Record<string, string> {
  return measure ? {} : { 'data-usage-vendor': view.vendor, 'data-usage-window': w.key }
}

const NAME_UNIT_GAP = '0.15em'

/** 회사 아이콘 + 배지 + 갱신 중 표식 — 한 덩어리로 줄지 않는다(배지가 말줄임에 잘리지 않게 — R31). */
function NameUnit({ view, measure, cell }: { view: VendorView; measure: boolean; cell?: CSSProperties }) {
  return (
    <span style={{ ...cell, flex: 'none', display: 'inline-flex', alignItems: 'center', gap: NAME_UNIT_GAP }}>
      <VendorIcon
        vendor={view.vendor}
        label={measure ? null : view.name}
        {...(measure ? {} : { 'data-usage-icon': view.vendor })}
      />
      {view.line !== null && <Badge view={view} measure={measure} />}
      {/* 측정 사본은 갱신 중 표식을 늘 센다 — 조회가 켜고 꺼질 때 자연 폭이 바뀌면 경계에서 조회 동안만 단계가
          뒤집힌다(2↔3 이면 ⟳ 까지 크게 움직여 누르던 자리를 잃는다). 대가 = 1·2단이 표식 폭만큼 일찍 넘어간다. */}
      {(view.refreshing || measure) && <RefreshingMark view={view} measure={measure} />}
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

// ★움직임 줄이기·e-ink 에서도 돈다★ — 이 저장소의 회전 기호 규칙(`agent/agentGlyph.css` · `ui/loading-panel.css` —
//   사용자 결정 2026-08-24·2026-09-24: 멈춘 스피너는 「멈춤」과 구분되지 않는다). 숨은 측정 사본만 돌리지 않는다.
function RefreshingMark({ view, measure }: { view: VendorView; measure: boolean }) {
  const label = t('usage.refreshing')
  return (
    <span
      {...(measure ? {} : { 'data-usage-refreshing': view.vendor, role: 'img', 'aria-label': label, title: label })}
      style={{ display: 'inline-flex', color: 'var(--text-muted)' }}
    >
      <RefreshCw aria-hidden="true" className={measure ? 'size-3' : 'size-3 animate-spin'} />
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
            <span
              {...(measure ? {} : { 'data-usage-reset-clock': '' })}
              style={{ ...at(3), marginLeft: PCT_TO_RESET, color: 'var(--text-muted)' }}
            >
              {formatResetClock(r.resetsAt, nowWall)}
            </span>
          )}
        </>
      )}
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

function UsagePopup({
  summaryRef,
  views,
  nowWall,
  ownerRef,
  shown,
  onToggle,
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
    //   팝업을 연 채 새로고침한다(상세의 「갱신 중」을 보며 누르게).
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
      {views.map(view => (
        <VendorDetail key={view.vendor} view={view} nowWall={nowWall} />
      ))}
      <ShowToggles shown={shown} onToggle={onToggle} divided={views.length > 0} />
    </div>
  )
}

// ADR-0257: 켜고 끄기의 사람 UI 는 여기 하나다 — 슬롯 우클릭 메뉴엔 사용량 항목이 없다.
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
 * 한 회사의 상세 — 창별 절대 리셋 시각 · 값마다 나이 · plan(있을 때만) · 모델별 창(값이 있는 것만 — D8) · 링크.
 * ★회사별 ⟳ 는 두지 않는다(사용자 결정 2026-09-29)★ — 작은 표시의 ⟳ 하나가 켠 회사를 한 번에 새로고침한다.
 */
function VendorDetail({ view, nowWall }: { view: VendorView; nowWall: number }) {
  const snapshot = view.entry?.snapshot
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
    <section data-usage-popup-vendor={view.vendor} style={{ display: 'flex', flexDirection: 'column', gap: '3px' }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: '0.6em' }}>
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
        {view.refreshing && (
          <span data-usage-refreshing-text="" style={{ color: 'var(--text-muted)' }}>
            {t('usage.refreshing')}
          </span>
        )}
      </div>
      {rows.map(row => (
        <PopupWindowRow key={row.key} view={view} row={row} nowWall={nowWall} />
      ))}
      <div>
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

function PopupWindowRow({ view, row, nowWall }: { view: VendorView; row: WindowView; nowWall: number }) {
  const r = row.reading
  const resetsAt = r.kind === 'expired' ? null : r.resetsAt
  const resetInSecs = r.kind === 'expired' ? null : r.resetInSecs
  return (
    <div
      data-usage-vendor={view.vendor}
      data-usage-window={row.key}
      style={{ display: 'flex', alignItems: 'center', gap: '0.5em', whiteSpace: 'nowrap', paddingLeft: '0.75em' }}
    >
      <span style={{ color: 'var(--text-muted)', minWidth: '3.5em' }}>{row.label}</span>
      {r.kind === 'value' && <Bar view={view} label={row.label} r={r} meter measure={false} />}
      {/* 팝업은 흐리게 하지 않는다 — 대신 값마다 나이를 늘 보인다(R32). */}
      <ValueText w={row} bare={false} dimStale={false} measure={false} />
      {resetsAt !== null && resetInSecs !== null && (
        <span data-usage-reset-at="" style={{ color: 'var(--text-muted)' }}>
          {t('usage.resetsAtIn', { time: formatResetAt(resetsAt, nowWall), duration: formatDuration(resetInSecs) })}
        </span>
      )}
      {r.kind === 'value' && (
        <span data-usage-age="" style={{ color: 'var(--text-muted)' }}>
          {formatAge(r.ageSecs)}
        </span>
      )}
    </div>
  )
}
