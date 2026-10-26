// TRD S26-window-chrome §7-1 · §7-2 · §7-5: 창 맨 위 제목 줄 = 탭바 → 끌 빈 자리 → 번호 표식 → 창 단추. 웹은 그리기만 한다 —
//   창 상태는 창 사실(`dock/windowFrame` — 셸이 민다)로 읽고, 바꾸기는 레지스트리 명령으로만 한다(`CaptionButtons`).
// ★탭이 먼저 준다★ — 탭바만 줄어들고(`TabBar` 뿌리 `flex: 1 1 0px`) 나머지는 고정이다. 그래서 줄이 좁아지면 탭바의 잰 폭이 줄어
//   Chrome 규칙대로 탭이 줄고 숨는다(`tabStrip/layout.ts` · R13).
// 끌 자리 = 이 줄 자신(보통 상태 위 여백 · 번호 표식 자리 — 표식은 누름을 받지 않는다) · 탭 줄 빈 곳(`TabBar`) · 끌 빈 자리
//   (`dragRegion.ts`). 흐림 = `data-window-active` 아래의 글자 토큰(`styles/theme.css`).
// ★대기 모드(예열 창 — ADR-0299)에도 이 줄을 그린다 · 번호 표식과 창 단추만 뺀다(`standby` · `number` = `null`)★ — 셸 창 모델
//   밖이라 번호가 없고 창 명령이 그 창을 찾지 못한다. 줄을 통째로 빼면 들일 때 탭바가 다시 마운트되고(탭바 줄 노드가 바뀐다 —
//   `WindowLayout` 의 빈 틀 조항), 셸이 들일 때 모델에 옮겨 적는 대기 중 캔버스 크기(`src-tauri/src/spare/mod.rs`)가 들인 뒤
//   화면과 위 여백만큼 어긋난다. 로딩 · 부팅 실패 화면도 이 줄을 그린다 — 번호는 아직 없어도 창 단추 · 끌 자리는 있다(OS
//   틀이 없어 창을 옮기고 닫는 자리가 이 줄뿐이다 · `WindowLayout`).
// 최소 창 크기(§7-7)는 이 줄이 그린 것으로 센다 — ★창 번호를 받은 뒤에만 보낸다★(대기 · 로딩 · 부팅 실패는 보내지 않는다).
//   셸은 창을 열 때 세운 열기 표시를 그 창의 다음 보고에서 거두는데(`src-tauri/src/chrome/book.rs` 머리), 예열 창은 대기 중
//   보고와 들인 뒤 보고가 같은 페이지라 보고 순번으로 가를 수 없다 — 대기 중 보고가 들일 때의 표시를 먼저 거두면 들인 창이
//   넓혀 당겨지지 않는다. 번호 없는 화면의 폭(표식 없음)으로 넓히는 것도 같은 어긋남이다.

import {
  useLayoutEffect,
  useRef,
  useSyncExternalStore,
  type CSSProperties,
  type ReactNode,
  type Ref,
} from 'react'

import { MAIN_WINDOW_LABEL } from '../../../store/viewStore'
import { windowFrame, type WindowFrameSource } from '../dock/windowFrame'
import CaptionButtons from './CaptionButtons'
import { dragProps } from './dragRegion'
import { GRAB_MIN_W, minWindowSize, titleRowHeight } from './metrics'
import { reportMinWindowSize } from './minSizeReport'
import NumberMarker from './NumberMarker'

interface TitleRowProps {
  label: string
  /** 보이는 창 번호 — `null` = 아직 없다(대기 모드 · 로딩 · 부팅 실패). 표식을 그리지 않고 최소 창 크기를 보내지 않는다. */
  number: number | null
  /** 대기 모드(예열 창의 빈 틀) — 창 단추를 그리지 않는다. 기본 `false`. */
  standby?: boolean
  /** 창 사실 — 받기(`install`)는 탭바의 끌기 훅이 단다(`pullFrame`). 시험이 갈아 끼운다. */
  frame?: WindowFrameSource
  /** 줄 요소 — 탭바의 놓기 판정이 탭바 구역을 이 줄 전체로 잰다(`UseTabDragSourceOptions.titleRow`). */
  ref?: Ref<HTMLDivElement>
  /** 이 창의 탭바(`TabBar`). */
  children: ReactNode
}

export default function TitleRow({ label, number, standby = false, frame = windowFrame, ref, children }: TitleRowProps) {
  const facts = useSyncExternalStore(frame.subscribe, frame.current)
  // 창 사실을 받기 전 = 최대화 아님 · 앞 창 · 단추를 그린다(지금 OS 의 크롬 모양 — 받은 뒤 바뀌면 탭 폭이 한 번 바뀐다).
  const maximized = facts?.maximized ?? false
  const active = facts?.focused ?? true
  const buttons = !standby && (facts?.caption.buttons ?? true)
  const leading = facts?.caption.leading_inset ?? 0

  // ★layout effect 다★ — 같은 커밋의 `WindowLayout` 첫 화면 effect(passive)보다 먼저 보고를 맡겨야 첫 화면 알림이 그 보고를
  //   기다린다(§7-7). 다시 재는 때 = 번호 · 단추 · 왼 여백이 바뀔 때와 표식 폭이 바뀔 때(자리 수 · 글꼴 · 배율 — 관측). 번호가
  //   없으면 보내지 않는다(머리).
  const markerRef = useRef<HTMLSpanElement>(null)
  useLayoutEffect(() => {
    if (number === null) return
    const measure = (): void =>
      reportMinWindowSize(
        minWindowSize({
          leadingInset: leading,
          markerW: markerRef.current === null ? null : markerRef.current.getBoundingClientRect().width,
          buttons,
        }),
      )
    measure()
    const marker = markerRef.current
    // jsdom 처럼 관측기가 없는 환경은 재기 한 번으로 지나간다.
    if (marker === null || typeof ResizeObserver === 'undefined') return
    const ro = new ResizeObserver(measure)
    ro.observe(marker)
    return () => ro.disconnect()
  }, [number, buttons, leading])

  return (
    <div
      ref={ref}
      data-testid="title-row"
      {...dragProps()}
      data-window-active={active ? 'true' : 'false'}
      data-maximized={maximized ? 'true' : 'false'}
      style={{
        display: 'flex',
        alignItems: 'flex-end',
        flexShrink: 0,
        height: titleRowHeight(maximized),
        boxSizing: 'border-box',
        paddingLeft: leading,
        background: 'var(--tab-bar-bg)',
        fontFamily: 'var(--font-ui)',
        userSelect: 'none',
      }}
    >
      {children}
      <div data-testid="title-grab-gap" aria-hidden {...dragProps()} style={GRAB_GAP} />
      {number !== null && <NumberMarker ref={markerRef} n={number} />}
      {buttons && (
        <CaptionButtons label={label} main={label === MAIN_WINDOW_LABEL} maximized={maximized} snapOverlay={facts?.caption.snap_overlay ?? false} />
      )}
    </div>
  )
}

const GRAB_GAP: CSSProperties = {
  flex: `0 0 ${GRAB_MIN_W}px`,
  alignSelf: 'stretch',
  boxSizing: 'border-box',
  borderBottom: '1px solid var(--tab-bar-edge)',
}
