// 사용량 한도 슬롯(TRD S21 usage-limit-slot §1-8) — 켠 회사의 남은 양을 작은 표시로 그리고, 누르면 상세 팝업을 연다.
//
// ★켠 회사만 그린다★ — 끈 회사는 스토어에 값이 있어도 작은 표시·팝업·배지 어디에도 없다. 조회 수요(관심)는 셸이
//   레이아웃에서 계산하므로 이 컴포넌트는 관심을 보고하지 않는다.
// ★폭 단계는 그려진 크기로 고른다(R3)★ — 단계마다 숨은 렌더의 자연 크기를 재어 실제 크기와 견준다. px 문턱을 두지
//   않는다: 글꼴·문구·값이 바뀌면 문턱도 따라 움직여야 해서다.
// ★시각은 분 단위 tick 하나가 굴린다★ — 나이·남은 시간·다음 시도가 요청 없이 로컬로 흐른다(R32).
// 값은 DOM 텍스트 + `data-usage-vendor`/`data-usage-window` 로 둔다(R27 — LLM·cdp 가 읽는다).

import { Fragment, useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'
import type { CSSProperties, RefObject } from 'react'
import { RefreshCw } from 'lucide-react'
import { openUrl } from '@tauri-apps/plugin-opener'

import type { SlotContent } from '../../api/layoutTypes'
import type { AgentBackendKind } from '../../api/types'
import { t } from '../../i18n'
import {
  blocksRefresh,
  useUsagePending,
  useUsageStore,
  useUsageVendor,
  type UsageVendorEntry,
} from '../../store/usageStore'
import { ANCHOR_GAP, clampMenuPosition } from './SlotContextMenu'
import {
  STALE_AFTER_SECS,
  elapsedSecs,
  formatAge,
  formatDuration,
  formatResetAt,
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
 * 「사용량 페이지 ↗」 목적지. ★셸의 opener 허용 목록(`src-tauri/capabilities/usage-links.json`)과 글자까지 같아야
 * 한다★ — 쿼리·끝 슬래시 하나라도 다르면 셸이 여는 것을 거절한다. 대조는 `UsageSlot.test.tsx` 가 한다.
 */
export const USAGE_PAGE_URL: Record<AgentBackendKind, string> = {
  claude: 'https://claude.ai/settings/usage',
  codex: 'https://chatgpt.com/codex/settings/usage',
}

const TICK_MS = 60_000

// 요약 버튼의 안쪽 여백. 세로 값은 2단의 높이 판정에도 든다 — 숨은 사본은 여백 없이 재어지기 때문이다.
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
  shortName: string
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
  return vendor === 'claude' ? t('usage.vendorClaude') : t('usage.vendorCodex')
}

function vendorShortName(vendor: AgentBackendKind): string {
  return vendor === 'claude' ? t('usage.vendorClaudeShort') : t('usage.vendorCodexShort')
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
    shortName: vendorShortName(vendor),
    entry,
    elapsed,
    line: sentence === null ? null : statusLine(name, sentence),
    head: snapshot ? stateHead(snapshot.state, elapsed) : null,
    refreshing: pending || (snapshot?.in_flight ?? false),
    rejected: blocksRefresh(snapshot?.state),
    windows: [
      { key: 'five_hour', label: t('usage.windowFiveHour'), reading: readWindow(windowOf('five_hour'), elapsed, nowWall) },
      { key: 'weekly', label: t('usage.windowWeekly'), reading: readWindow(windowOf('weekly'), elapsed, nowWall) },
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

export default function UsageSlot({ content }: { content: UsageContent }) {
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

  if (views.length === 0) {
    return (
      <div data-usage-slot="" data-usage-hint="" style={{ ...ROOT_STYLE, padding: '6px 8px', color: 'var(--text-muted)' }}>
        {t('usage.hint')}
      </div>
    )
  }
  return <UsageSummary views={views} nowWall={nowWall} />
}

// ── 작은 표시 ──────────────────────────────────────────────────────────────────────────────

interface Measures {
  /** 작은 표시가 쓸 수 있는 폭. */
  avail: number | null
  /** 슬롯 루트의 높이(`height:100%` — 내용에 따라 늘지 않는다). */
  rootH: number | null
  s1: number | null
  s2: number | null
  s2h: number | null
}

/**
 * 들어가는 가장 넓은 단계. 아직 못 쟀으면 1단(가장 자세한 것). 2단은 높이도 봐서, 슬롯보다 키가 크면 3단으로 간다 —
 * 잘린 줄의 배지가 사라지면 안 된다(R31 — 모든 단계에서 배지). 3단보다 좁으면 3단이 말줄임으로 잘린다.
 */
function pickStage({ avail, rootH, s1, s2, s2h }: Measures): Stage {
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

/** 요약 버튼의 화면 사각형 — 팝업은 아래로 펴고, 뒤집을 땐 요약의 위쪽 변에서 편다(요약을 덮지 않게). */
interface Anchor {
  left: number
  top: number
  bottom: number
}

function UsageSummary({ views, nowWall }: { views: VendorView[]; nowWall: number }) {
  const rootRef = useRef<HTMLDivElement>(null)
  const contentRef = useRef<HTMLSpanElement>(null)
  const measure1Ref = useRef<HTMLDivElement>(null)
  const measure2Ref = useRef<HTMLDivElement>(null)
  const buttonRef = useRef<HTMLButtonElement>(null)
  const [measures, setMeasures] = useState<Measures>({ avail: null, rootH: null, s1: null, s2: null, s2h: null })
  const [anchor, setAnchor] = useState<Anchor | null>(null)

  // 실제 크기와 두 단계의 자연 크기를 한 관찰자로 잰다 — 숨은 렌더는 값·문구가 바뀌면 크기가 바뀌어 다시 불린다.
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
          if (entry.target === contentRef.current) put('avail', width)
          else if (entry.target === rootRef.current) put('rootH', height)
          else if (entry.target === measure1Ref.current) put('s1', width)
          else if (entry.target === measure2Ref.current) {
            put('s2', width)
            put('s2h', height)
          }
        }
        return next
      })
    })
    for (const el of [rootRef.current, contentRef.current, measure1Ref.current, measure2Ref.current]) {
      if (el) observer.observe(el)
    }
    return () => observer.disconnect()
  }, [])

  const stage = pickStage(measures)

  const closePopup = useCallback((restoreFocus: boolean) => {
    setAnchor(null)
    if (restoreFocus) buttonRef.current?.focus()
  }, [])

  return (
    <div ref={rootRef} data-usage-slot="" style={ROOT_STYLE}>
      <button
        ref={buttonRef}
        type="button"
        data-usage-summary=""
        aria-haspopup="dialog"
        aria-expanded={anchor !== null}
        // 버튼의 자식은 보조기술에 평평해지므로 안의 막대·배지 대신 짧은 이름을 준다(D14).
        aria-label={summaryLabel(views)}
        // 키보드(Enter·Space)는 네이티브 버튼 활성화가 같은 click 으로 부른다(D14).
        onClick={() => {
          if (anchor !== null) {
            setAnchor(null)
            return
          }
          const rect = buttonRef.current?.getBoundingClientRect()
          setAnchor({ left: rect?.left ?? 0, top: rect?.top ?? 0, bottom: rect?.bottom ?? 0 })
        }}
        style={{
          display: 'block',
          width: '100%',
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
        <span
          ref={contentRef}
          data-usage-content=""
          data-usage-stage={stage}
          style={{ display: 'block', overflow: 'hidden' }}
        >
          <StageRender stage={stage} views={views} measure={false} />
        </span>
      </button>
      <div
        aria-hidden="true"
        style={{ position: 'absolute', top: 0, left: 0, visibility: 'hidden', pointerEvents: 'none' }}
      >
        <div ref={measure1Ref} data-usage-measure="1" style={{ width: 'max-content' }}>
          <StageRender stage={1} views={views} measure />
        </div>
        <div ref={measure2Ref} data-usage-measure="2" style={{ width: 'max-content' }}>
          <StageRender stage={2} views={views} measure />
        </div>
      </div>
      {anchor !== null && (
        <UsagePopup anchor={anchor} views={views} nowWall={nowWall} buttonRef={buttonRef} onClose={closePopup} />
      )}
    </div>
  )
}

/**
 * `measure` = 크기만 재는 숨은 사본 — 같은 글자·같은 모양을 그리되 역할·이름·`data-usage-*`·애니메이션은 싣지 않는다
 * (cdp·보조기술이 값을 두 번 읽지 않게). 줄도 `span` 인 것은 요약 버튼 안에 들어가서다(버튼 내용 = phrasing).
 */
function StageRender({ stage, views, measure }: { stage: Stage; views: VendorView[]; measure: boolean }) {
  if (stage === 1) {
    return (
      <>
        {views.map(view => (
          <span key={view.vendor} style={LINE_STYLE}>
            <NameUnit view={view} short={false} measure={measure} />
            {view.windows.map(w => (
              <WindowSegment key={w.key} view={view} w={w} measure={measure} />
            ))}
          </span>
        ))}
      </>
    )
  }
  if (stage === 2) {
    return (
      <>
        {views.map(view => (
          <span key={view.vendor} style={{ display: 'block' }}>
            <span style={LINE_STYLE}>
              <NameUnit view={view} short={false} measure={measure} />
            </span>
            {view.windows.map(w => (
              <span key={w.key} style={{ ...LINE_STYLE, paddingLeft: '0.75em' }}>
                <WindowSegment view={view} w={w} measure={measure} />
              </span>
            ))}
          </span>
        ))}
      </>
    )
  }
  return (
    <>
      {views.map(view => (
        <span key={view.vendor} style={LINE_STYLE}>
          <NameUnit view={view} short measure={measure} />
          {/* 말줄임은 숫자 쪽에만 — 이름·배지는 줄지 않아 잘리지 않는다(R31). */}
          <span
            data-usage-numbers=""
            style={{ flex: '1 1 auto', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}
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

function NameUnit({ view, short, measure }: { view: VendorView; short: boolean; measure: boolean }) {
  return (
    <span style={{ flex: 'none', display: 'inline-flex', alignItems: 'center', gap: '0.15em', fontWeight: 600 }}>
      <span title={short && !measure ? view.name : undefined}>{short ? view.shortName : view.name}</span>
      {view.line !== null && <Badge view={view} measure={measure} />}
      {view.refreshing && <RefreshingMark view={view} measure={measure} />}
    </span>
  )
}

/** 비정상 상태 표식 — 글리프는 `⚠`(20% 미만 전용, R10)와 겹치지 않는다. 이름·문장은 팝업 맨 위 줄과 같다. */
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

/** 1·2단의 창 하나 — 이름 · 막대(값이 있을 때만) · 남은 % · 남은 시간(1·2단만 — D16). */
function WindowSegment({ view, w, measure }: { view: VendorView; w: WindowView; measure: boolean }) {
  const r = w.reading
  const resetInSecs = r.kind === 'expired' ? null : r.resetInSecs
  return (
    <span
      {...windowAttrs(view, w, measure)}
      style={{ flex: 'none', display: 'inline-flex', alignItems: 'center', gap: '0.35em' }}
    >
      <span style={{ color: 'var(--text-muted)' }}>{w.label}</span>
      {r.kind === 'value' && <Bar view={view} label={w.label} r={r} meter={false} measure={measure} />}
      <ValueText w={w} bare={false} dimStale measure={measure} />
      {resetInSecs !== null && (
        <span {...(measure ? {} : { 'data-usage-reset-in': '' })} style={{ color: 'var(--text-muted)' }}>
          {t('usage.resetIn', { duration: formatDuration(resetInSecs) })}
        </span>
      )}
    </span>
  )
}

/**
 * 막대 — 값이 있을 때만 그린다(값이 없으면 막대 자체가 없다 — 0% 로 그리지 않는다, R21). `meter` = 보조기술에 막대로
 * 드러낸다(D14 — 팝업). 요약 버튼 안에서는 버튼 이름이 대신하므로 숨긴다.
 * ★색은 토큰만★ — e-ink 은 같은 토큰이 빗금·검정 채움·굵은 테두리로 풀린다(R11).
 */
function Bar({
  view,
  label,
  r,
  meter,
  measure,
}: {
  view: VendorView
  label: string
  r: ValueReading
  meter: boolean
  measure: boolean
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
 * 팝업에만 둔다(R32 · 사용자 결정 2026-09-27).
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
  const danger = r.level === 'danger' ? (bare ? '⚠' : ' ⚠') : ''
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
      {`${r.visible}${bare ? '' : '%'}${danger}`}
    </span>
  )
}

// ── 팝업 ──────────────────────────────────────────────────────────────────────────────────

/**
 * 아래로 펴는 것이 먼저, 안 들어가면 요약의 위쪽 변에서 위로 뒤집는다 — 요약의 아래쪽 변에서 뒤집으면 팝업이 요약을
 * 덮는다. 둘 다 안 들어가면(뷰포트보다 큼) 슬롯 우클릭 메뉴와 같은 밀기로 떨어진다. 가로는 그 메뉴 규칙 그대로(R7).
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
  anchor,
  views,
  nowWall,
  buttonRef,
  onClose,
}: {
  anchor: Anchor
  views: VendorView[]
  nowWall: number
  buttonRef: RefObject<HTMLButtonElement | null>
  onClose: (restoreFocus: boolean) => void
}) {
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null)

  // 내용 높이가 값에 따라 달라 그릴 때마다 재되, 같으면 멈춘다.
  useLayoutEffect(() => {
    if (!ref.current) return
    const rect = ref.current.getBoundingClientRect()
    const next = placePopup(anchor, rect.width, rect.height, window.innerWidth, window.innerHeight)
    setPos(prev => (prev !== null && prev.top === next.top && prev.left === next.left ? prev : next))
  })

  // ★자리를 잡은 뒤에만 포커스를 옮긴다★ — 첫 커밋은 재기 전이라 `visibility:hidden` 이고, Chromium 은 보이지 않는
  //   요소의 focus() 를 거절한다(그러면 Esc·Tab 이 팝업에 닿지 않는다).
  const positioned = pos !== null
  useEffect(() => {
    if (positioned) ref.current?.focus()
  }, [positioned])

  useEffect(() => {
    const ours = (node: Node | null): boolean =>
      node !== null && ((ref.current?.contains(node) ?? false) || (buttonRef.current?.contains(node) ?? false))
    // Esc 는 포커스가 팝업·요약에 있을 때만 — 다른 곳의 Esc 를 가로채지 않는다.
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && ours(document.activeElement)) onClose(true)
    }
    // 요약 버튼 위 누름은 그 버튼의 click 이 닫는다 — 여기서도 닫으면 click 이 곧바로 다시 연다.
    const onDown = (e: MouseEvent) => {
      if (!ours(e.target as Node)) onClose(false)
    }
    // 포커스가 밖으로 나가면 닫되, 포커스는 간 자리에 둔다(되찾아 오지 않는다).
    const onFocusIn = (e: FocusEvent) => {
      if (!ours(e.target as Node)) onClose(false)
    }
    document.addEventListener('keydown', onKey)
    document.addEventListener('mousedown', onDown)
    document.addEventListener('focusin', onFocusIn)
    return () => {
      document.removeEventListener('keydown', onKey)
      document.removeEventListener('mousedown', onDown)
      document.removeEventListener('focusin', onFocusIn)
    }
  }, [onClose, buttonRef])

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
    </div>
  )
}

/** 한 회사의 상세 — 창별 절대 리셋 시각 · 값마다 나이 · plan(있을 때만) · 모델별 창(값이 있는 것만 — D8) · ⟳ · 링크. */
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
  const refreshLabel = t('usage.refresh', { vendor: view.name })
  const url = USAGE_PAGE_URL[view.vendor]
  return (
    <section data-usage-popup-vendor={view.vendor} style={{ display: 'flex', flexDirection: 'column', gap: '3px' }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: '0.6em' }}>
        <span style={{ fontWeight: 600 }}>{view.name}</span>
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
        <span style={{ flex: '1 1 auto' }} />
        {/* 보이는 거절 중엔 막는다 — 기한 안엔 ⟳ 도 조회하지 않는다(R24 · D11). 30초 간격은 데몬이 지킨다. */}
        <button
          type="button"
          data-usage-refresh={view.vendor}
          aria-label={refreshLabel}
          title={refreshLabel}
          disabled={view.rejected}
          onClick={() => void useUsageStore.getState().refresh(view.vendor)}
          style={{
            display: 'inline-flex',
            padding: '2px',
            border: '1px solid var(--border)',
            borderRadius: '3px',
            background: 'transparent',
            color: 'inherit',
            cursor: view.rejected ? 'default' : 'pointer',
            opacity: view.rejected ? 0.4 : undefined,
          }}
        >
          <RefreshCw aria-hidden="true" className="size-3.5" />
        </button>
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
