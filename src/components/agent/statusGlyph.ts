// ★공유 모듈로 분리★: 원래 AgentList.tsx 에 있던 순수 매핑 함수들이 여기 산다. RichSlot 헤더가 같은
//   상태 글리프를 그리는데, 무거운 AgentList 모듈(react-arborist·트리 로직 전부) 전이 의존을 피하려
//   순수 헬퍼만 이 경량 모듈에 둔다. AgentList 는 이 값을 re-export 해 기존 importer/test 를 그대로 유지한다.
//
// 에이전트 상태 → 아이콘·색 매핑. ADR-0062 §38 이 요구하는 것은 **headless 전 분기 고정**이고, 이 파일은
//   그것만 지킨다 — react-arborist 를 끌고 오는 AgentList 없이 매핑을 단독 테스트할 수 있다.
//   ★"외부 의존 0"은 아니다★: lucide 아이콘 컴포넌트를 값으로 돌려주므로 그 import 는 불가피하다(ADR-0168 로
//   문자 글리프에서 아이콘으로 교체되며 생긴 변화). JSX 는 여기 두지 않는다(그리는 쪽 = AgentList).
//
// ★한 행의 기호는 하나뿐이다(ADR-0173)★: 축이 늘어도(상태 · 마지막 실패) 기호를 나란히 두지 않고
//   갈아끼운다. 그 판정은 `isFailureBlocked` 한 곳에 있고 기호·색·hover 가 함께 그것을 따른다.
//   ★아이콘화(ADR-0168)가 이 축을 바꾸지 않는다★: 갈린 것은 반환물의 *형태*뿐이다 — 옛 문자 `⊘` 자리에
//   {아이콘 · 채움 · 모양 토큰} 한 벌이 들어왔고, 판정식·색·hover 문구는 손대지 않았다. 관측 표면도
//   textContent 에서 `shape` 토큰으로 옮겨갔을 뿐 사라지지 않았다(svg 는 글자를 남기지 않는다).

import { Ban, Circle, Loader, Square, X, type LucideIcon } from 'lucide-react'

import type { AgentFailureKind } from '../../api/types'

/** 렌더러가 그대로 svg 에 넘기는 한 벌. */
export type StatusGlyphIcon = {
  Icon: LucideIcon
  /** lucide 기본은 `none`(외곽선) — 채움 여부가 Running(채운 원)과 Reserved(빈 원)를 가르는 *모양* 차이다. */
  fill: 'currentColor' | 'none'
  /**
   * DOM 관측 표면(테스트·CDP·LLM 셀렉터)에 나가는 안정 토큰. 아이콘 컴포넌트는 직렬화되지 않는다.
   *
   * ★기존 토큰을 지우거나 이름을 바꾸지 말 것★: 이 문자열에 셀렉터가 걸려 있어서 개명은 조용한 파괴다.
   *   **늘리는 것만 안전하다** — `blocked` 가 그렇게 늘어난 것이고(ADR-0173 의 마지막 실패 축),
   *   `pending` 이 그 다음이다(아래 시연 층 — 답을 기다리는 동안).
   */
  shape: 'filled-circle' | 'square' | 'x' | 'circle' | 'blocked' | 'pending'
}

/**
 * 마지막 실패가 상태 기호를 갈아끼울 때 쓰는 금지 표식(ADR-0173 — 기호를 하나 더 늘리지 않는다).
 *
 * ★`Ban` 을 고른 이유 = 옛 문자 `⊘` 와 *같은 도형*이기 때문★: lucide 의 원+사선 후보 셋 중 원의 지름을
 *   꽉 채우고 테두리에서 정확히 멈추는 것은 `Ban`(4.929,4.929 → 19.071,19.071) 하나다. `CircleSlash` 는
 *   사선이 가운데 6단위 토막(9,15 → 15,9)이라 12px 에서 사선이 아니라 점처럼 보이고, `CircleSlash2` 는
 *   viewBox 모서리를 잇는 선(22,2 → 2,22)이라 원 밖으로 삐져나온다. 방향(↘ vs ↗)만 다르고 "그어 지운 원"
 *   이라는 읽힘은 같으므로, 금지 기호의 표준형인 `Ban` 이 의미(못 연다)와 도형 양쪽에서 맞는다.
 * ★채움은 `none`★: 채우면 원 안의 사선이 묻혀 Running(채운 원)과 실루엣이 겹친다 — e-ink 에서 색이
 *   중립화되면 그 둘을 가를 것이 없어진다(ADR-0062 「모양이 1차 신호」).
 */
const BLOCKED_GLYPH: StatusGlyphIcon = { Icon: Ban, fill: 'none', shape: 'blocked' }

/**
 * 마지막 실패가 지금 이 행의 기호를 차지하나 — **기호·색·hover 가 함께 따르는 단일 판정**.
 *
 * ★도는 중이 이긴다(ADR-0173)★: 마지막 실패를 들고 있어도 떠 있으면 원래 상태 기호를 그린다. 실패 표시는
 *   "지금 이 항목을 못 연다" 는 신호라, 도는 항목에 얹으면 거짓말이 된다.
 * ★`Exiting` 도 「도는 중」 쪽이다★: 그건 매니저가 내는 **과도기** 전이지 머무는 상태가 아니다(ADR-0005).
 *   쉬는 상태로 세면 정상 종료 몇 초 동안 행이 금지 표식으로 깜빡인다.
 * ★갈아끼워도 손실이 없는 이유★: 표시가 뜨는 경우는 안 떠 있는 항목뿐이고, 그때 원래 기호(사각/빈 원)는
 *   나를 정보가 없다.
 */
export function isFailureBlocked(
  status: string,
  lastFailure?: AgentFailureKind | null,
): boolean {
  return Boolean(lastFailure) && status !== 'Running' && status !== 'Exiting'
}

/**
 * 색이 아닌 **모양**이 상태를 담아 e-ink 에서도 구분된다.
 *
 * `lastFailure` 가 기호를 차지하면(위 `isFailureBlocked`) 상태와 무관하게 금지 표식을 **갈아끼운다** —
 * 두 번째 기호를 나란히 두지 않는다(ADR-0173). 인자를 생략하면 상태 축만 도는 옛 동작 그대로다 —
 * ★두 번째 인자가 optional 인 이유★: 마지막 실패라는 개념이 없는 호출자(RichSlot 헤더처럼 상태만 아는
 * 자리)가 그대로 컴파일돼야 한다. 필수로 만들면 이 축과 무관한 호출자까지 `null` 을 나르게 된다.
 *
 * 매핑(ADR-0062):
 *   - Running                → 채운 원 (작업중)
 *   - Exiting/Exited/Killed  → 사각    (멈춤 — Exiting 은 terminal 직전 전이)
 *   - Failed                 → ✗ 모양  (에러)
 *   - Reserved(프론트 합성)   → 빈 원   (유휴/미spawn 깡통)
 *   - 그 외(미지 status)      → 빈 원   (안전 degrade — 빈 칸 방지)
 *
 * ★모양이 겹치는 분기를 만들지 말 것★: 색만 다르고 모양이 같은 두 상태는 e-ink 에서 구분이 사라진다
 *   (e-ink 블록이 `--status-running` 을 `var(--text-muted)` 로 중립화한다 — `statusGlyphColor` 주석).
 *
 * ★◐(입력대기)에 해당하는 반쯤 채운 모양은 절대 반환하지 않는다★: 백엔드가 "입력 대기" 신호를 내지
 *   않으므로 그 분기가 없다(ADR-0062 — 미점등은 결함이 아니라 의도). 백엔드가 신호를 낼 때 여기 분기를
 *   추가하는 것이 정규 경로.
 *
 * ★텍스트 글리프(`● ◻ ✗ ○ ⊘`)로 되돌리지 말 것★: UI 폰트가 monospace 에서 Segoe UI 로 바뀐 뒤 글자마다
 *   크기·굵기·baseline 이 달라 행 라벨과 세로 정렬이 어긋났다. 아이콘은 폰트와 무관하게 12px 정사각이다.
 */
export function statusGlyphIcon(
  status: string,
  lastFailure?: AgentFailureKind | null,
): StatusGlyphIcon {
  // ADR-0173: 상태 분기보다 먼저 본다 — 갈아끼우는 축이라 상태 매핑을 덮는다.
  if (isFailureBlocked(status, lastFailure)) return BLOCKED_GLYPH
  switch (status) {
    case 'Running':
      return { Icon: Circle, fill: 'currentColor', shape: 'filled-circle' }
    case 'Exiting':
    case 'Exited':
    case 'Killed':
      return { Icon: Square, fill: 'none', shape: 'square' }
    case 'Failed':
      return { Icon: X, fill: 'none', shape: 'x' }
    case 'Reserved':
      return { Icon: Circle, fill: 'none', shape: 'circle' }
    default:
      return { Icon: Circle, fill: 'none', shape: 'circle' }
  }
}

/**
 * ADR-0062 개정: 원래 "상태=모양(색 아님)"이었으나
 * e-ink 를 별도 모드로 분리하기로 하며 다른 테마에선 색 허용. 색은 모양을 대체하지 않고 *부가*한다.
 * ★색 리터럴 금지·변수만★(ADR-0062 §44):
 * green 값은 theme.css 의 --status-running 이 소유하고, e-ink 블록이 이를 var(--text-muted) 로 중립화해
 * e-ink 에선 모양만 남는다(모양이 여전히 1차 신호). 경고색(--status-blocked)도 같은 규칙을 따른다.
 * ★`statusGlyphIcon` 과 **같은 판정**을 부른다★(ADR-0173): 기호가 갈린 행은 색도 함께 갈려야 한다 —
 * 판정을 복제하면 두 사본이 어긋나 금지 표식이 초록으로 뜨는 조합이 생긴다.
 */
export function statusGlyphColor(status: string, lastFailure?: AgentFailureKind | null): string {
  // ADR-0173
  if (isFailureBlocked(status, lastFailure)) return 'var(--status-blocked)'
  return status === 'Running' ? 'var(--status-running)' : 'var(--text-muted)'
}

// ─────────────────────────────────────────────────────────────────────────────
// 시연(presentation) 층 — 「답을 기다리는 중」·「방금 거절당했다」
//
// ★백엔드 상태를 늘리지 않는다★: `status` 는 세션이 생기는 순간 낙관적으로 `Running` 이 되고(코어의
//   OutputCore 초기값) "연결됐다·쓸 수 있다"를 뜻한 적이 없다. 그래서 더블클릭한 행은 답이 오기 전
//   구간에서 초록 채운 원을 그렸다 — 아직 아무것도 모르는데 "돌고 있다"고 말한 셈이고, 활성화가
//   거절된 뒤에도 프로세스가 죽어 관측될 때까지 그 초록이 몇 초 더 남았다(실측 2026-08-24: 거절
//   +2.4s → 초록 4.2s → 그제서야 금지 표식).
//
// ★고친 방향 = 프론트가 이미 들고 있는 답을 쓰는 것★: 활성화 RPC 의 in-flight/거절은 이 화면이
//   먼저 안다(거절이 「실패」 배지를 띄우는 그 순간이다). 그러니 상태 축을 건드리지 않고 그 위에
//   한 겹을 얹는다 — `isFailureBlocked` 의 의미는 한 글자도 바뀌지 않는다.
//
// ★셋(기호·색·hover)이 한 판정을 따르는 성질은 유지된다★(ADR-0173): 아래 `rowPhase` 가 그 한 판정의
//   자리이고, 기호(`rowGlyphIcon`)·색(`rowGlyphColor`)·hover(AgentList 의 title)가 같은 값을 받는다.
// ─────────────────────────────────────────────────────────────────────────────

/**
 * 지금 이 행이 무엇을 말하고 있나 — **결정 순서가 곧 우선순위**다.
 *
 * - `pending`  : 이 행에 건 요청이 아직 돌고 있다. 답을 모르므로 상태 기호를 그리지 않는다.
 * - `rejected` : 방금 건 활성화가 **거절당했다**. 프로세스가 죽어 관측되기 전이라 `status` 는 아직
 *                `Running` 일 수 있고 「마지막 실패」 기록도 한 박자 뒤에 온다 — 둘 다 기다리지 않는다.
 * - `settled`  : 그 밖 — 상태·마지막 실패 축(위 `statusGlyphIcon`) 그대로.
 */
export type RowPhase = 'settled' | 'pending' | 'rejected'

/**
 * ★`pending` 이 `rejected` 를 이긴다★: 거절 표식은 *지난* 시도의 결과라, 같은 행에 새 시도가 걸려
 *   있으면 그 새 시도가 지금의 사실이다(재시도 즉시 스피너로 갈아타야 한다).
 */
export function rowPhase(inFlight: boolean, activationRejected: boolean): RowPhase {
  if (inFlight) return 'pending'
  if (activationRejected) return 'rejected'
  return 'settled'
}

/**
 * 답을 기다리는 동안의 기호.
 *
 * ★`Loader`(바퀴살 8개)를 고른 이유 = e-ink★: 그 테마는 `--status-*` 를 통째로 중립화하므로
 *   (`--status-running` → `var(--text-muted)`) **모양만 남는다.** 후보 중 `LoaderCircle` 은 한 군데
 *   트인 원 하나(`M21 12a9 9 0 1 1-6.219-8.56`)라 12px 에서 Reserved 의 빈 원과 실루엣이 사실상
 *   같아진다 — 색도 움직임도 못 믿는 자리에서 그건 신호가 아니다. `Loader` 는 축 4 + 대각 4 = 여덟
 *   갈래가 가운데를 비우고 뻗은 별 모양이라 기존 넷(채운 원·빈 원·사각·✗)·금지 표식 어느 것과도
 *   겹치지 않는다. 그래서 **회전이 멈춰도**(reduced-motion) 읽힌다.
 * ★채움은 `none`★: 바퀴살은 선이라 채울 면이 없다(채우면 lucide 가 획을 뭉갠다).
 * ★회전은 CSS 가 소유한다★: 키프레임은 `agentGlyph.css`(클래스 `engram-glyph-spin`) — JSX 인라인
 *   키프레임 금지. 이 모듈은 여전히 JSX 도 스타일도 만들지 않는다(ADR-0168).
 */
const PENDING_GLYPH: StatusGlyphIcon = { Icon: Loader, fill: 'none', shape: 'pending' }

/**
 * 행이 실제로 그리는 기호 — 시연 층을 얹은 최종 선택.
 *
 * `settled` 는 `statusGlyphIcon` 을 **그대로 위임**한다(복제 금지 — 두 사본이 갈리면 같은 상태가
 * 자리마다 다른 모양으로 뜬다).
 */
export function rowGlyphIcon(
  phase: RowPhase,
  status: string,
  lastFailure?: AgentFailureKind | null,
): StatusGlyphIcon {
  if (phase === 'pending') return PENDING_GLYPH
  // ★기록을 기다리지 않는다★: 거절은 이 화면이 직접 받은 답이고, 「마지막 실패」는 그 사실이 백엔드를
  //   한 바퀴 돌아 오는 사본일 뿐이다. 기록이 도착하면 `settled` 로 내려앉아도 같은 금지 표식이 나온다.
  if (phase === 'rejected') return BLOCKED_GLYPH
  return statusGlyphIcon(status, lastFailure)
}

/**
 * 행이 실제로 쓰는 색 — 기호와 **같은 phase** 를 받는다(어긋나면 금지 표식이 초록으로 뜬다).
 *
 * ★`pending` 은 muted★: 초록은 "돌고 있다"는 주장이라 답을 모르는 구간에 쓸 수 없다 — 이 결함의
 *   본체가 바로 그 초록이었다.
 */
export function rowGlyphColor(
  phase: RowPhase,
  status: string,
  lastFailure?: AgentFailureKind | null,
): string {
  if (phase === 'pending') return 'var(--text-muted)'
  if (phase === 'rejected') return 'var(--status-blocked)'
  return statusGlyphColor(status, lastFailure)
}
