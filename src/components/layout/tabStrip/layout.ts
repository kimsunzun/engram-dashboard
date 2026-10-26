// 탭바 머리 폭 · 숨김 = Chromium 규칙 그대로(사용자 결정 Q22 — 「크롬룰이랑 맞춰 이런건. 우리가 따로 생각하지말고」 · TRD
//   S21-slot-docking R24). 출처 = chromium/src main — 폭은 `chrome/browser/ui/views/tabs/tab_strip_layout.cc`
//   (`CalculateSpaceFractionAvailable` · `TabSizer::CalculateTabWidth` · `AllocateExtraSpace` · `TabSizer::TabAcceptsExtraSpace`
//   — 2026-10-17 읽음) · 최소 폭은 `tab_width_constraints.cc` `GetMinimumWidth` · 숨김은
//   `chrome/browser/ui/views/tabs/tab_container_impl.cc` `ShouldTabBeVisible`(2026-10-18 받음 #1677-1733).
// ★우리 몫은 값의 축척뿐이다★ — 고정(pinned) 탭 · 탭끼리 겹치는 「발」(`tab_overlap`)이 없어 겹침 = 0 으로 읽는다. 규칙을
//   고안해 넣지 말 것(이름 길이 폭 · 활성을 보이는 자리로 당기기 · 넘침 메뉴는 사용자 결정 Q22 가 뺐다). 본문의 번호 1–5 =
//   셈의 단계.

/**
 * 넉넉할 때 머리 하나의 폭(`S` · CSS px · `border-box` — 패딩 · 테두리 포함). Chrome `TabStyle::GetStandardWidth()`
 * 256 dp 의 우리 축척 — 수는 옛 이름 칸 상한(160 px)을 따랐다.
 */
export const TAB_STANDARD_W = 160

/**
 * 닫기(×) · 새 탭(+) 단추의 원 지름(CSS px — ADR-0298 결정 5 · 사용자 결정 2026-10-21). 활성 최소 폭이 × 원을 담아 여기
 * 둔다(`TabBar.tsx` 가 그리기에 쓴다).
 */
export const TAB_BUTTON_SIZE = 22

/**
 * `+` 원 앞 틈(CSS px) — 마지막 머리 오른 가에 걸친 구분선(±2.5)에 원이 닿지 않게. `+` 상자 폭 = 이 틈 + `TAB_BUTTON_SIZE`
 * (`TabBar.tsx` `ADD_STYLE` · 최소 창 폭 셈 `titleRow/metrics.ts`).
 */
export const TAB_ADD_GAP = 4

/** 머리 내용 칸의 좌우 패딩(CSS px — `TabBar.tsx` `HEAD_CONTENT`). */
export const TAB_HEAD_PAD = 12

/** 머리 내용 칸에서 이름과 × 사이 간격(CSS px — `TabBar.tsx` `HEAD_CONTENT`). */
export const TAB_HEAD_GAP = 6

/**
 * 활성 머리의 최소 폭(`A`) = 닫기 버튼이 들어가는 폭 — Chrome `GetMinimumActiveWidth()` 와 같은 뜻. 셈 = 왼 패딩 + 이름 0 +
 * 간격 + 닫기 원 + 오른 패딩(ADR-0298 결정 5 · 머리에 테두리는 없다).
 */
export const TAB_MIN_ACTIVE_W = TAB_HEAD_PAD + 0 + TAB_HEAD_GAP + TAB_BUTTON_SIZE + TAB_HEAD_PAD

/**
 * 비활성 머리의 최소 폭(`I`) — Chrome `GetMinimumInactiveWidth()` ≈ 32 dp 그대로. 좌우 패딩(`TAB_HEAD_PAD` × 2)을 빼고 남는 이름 칸만 있고
 * 닫기 버튼은 들어가지 않는다(`A` 보다 좁은 머리는 닫기 버튼을 감춘다).
 */
export const TAB_MIN_INACTIVE_W = 32

export interface TabStripLayout {
  /** 머리마다 계산 폭(CSS px · 정수). 숨긴 머리도 계산 폭을 싣는다 — 보임 판정이 이 폭으로 오른 끝을 잰다(그릴 때는 0). */
  widths: number[]
  /** 머리마다 보이나. 숨김은 늘 꼬리다 — 보이는 것 = 앞쪽 `k` 개. */
  visible: boolean[]
}

/**
 * 머리 `n` 개의 폭 · 보임. `available` = 잰 「머리들이 쓸 수 있는 폭」(CSS px — 탭바 줄 폭 − `+` 폭 · 정수로 내린다).
 * `null` · 유한하지 않은 수 = 재지 못함(재기 전 · `ResizeObserver` 없음) = 제한 없음.
 * `widthCap` = 폭 셈에만 거는 상한 — 닫기 모드의 고정 폭(`closingMode.ts` `layoutAvailable`). ★보임은 상한이 아니라
 * `available` 로 잰다★ — Chrome 은 폭을 `min(고정 폭, 잰 폭)`(tab_container_impl.cc #1189 · #1220-1227)으로, 보임을 잰 폭
 * (#1688-1691 · #1732)으로 셈한다. 둘 다 없으면 모두 `S` · 숨김 없음.
 * `activeIndex` = 그 탭바가 활성으로 그리는 머리의 자리(입력 머리 기준 — 접힌 머리는 부르는 쪽이 뺀다). 범위 밖(`-1`
 * 포함) = 활성 없음(모두 비활성 최소 `I` 기준 · 숨김의 활성 조건이 걸리지 않는다).
 */
export function tabStripLayout(
  available: number | null,
  n: number,
  activeIndex: number,
  widthCap?: number | null,
): TabStripLayout {
  const count = Math.max(0, Math.floor(n))
  const room = floorOrNull(available)
  const cap = floorOrNull(widthCap)
  const avail = room === null ? cap : cap === null ? room : Math.min(room, cap)
  if (avail === null) {
    return { widths: Array<number>(count).fill(TAB_STANDARD_W), visible: Array<boolean>(count).fill(true) }
  }
  const active = activeIndex >= 0 && activeIndex < count ? activeIndex : -1
  const S = TAB_STANDARD_W
  const A = TAB_MIN_ACTIVE_W
  const I = TAB_MIN_INACTIVE_W

  // 1. 최소 · 넘김점(모든 머리 `A`) · 선호(모든 머리 `S`).
  const mins = Array.from({ length: count }, (_, i) => (i === active ? A : I))
  const minimum = mins.reduce((s, m) => s + m, 0)
  const crossover = count * A
  const preferred = count * S

  let widths: number[]
  const phaseB = avail < crossover
  let f: number
  if (!phaseB) {
    // 2. 단계 A — 모두 같은 폭으로 `A` 에서 `S` 사이.
    f = preferred === crossover ? 1 : clamp01((avail - crossover) / (preferred - crossover))
    widths = Array<number>(count).fill(Math.floor(A + f * (S - A)))
  } else {
    // 3. 단계 B — 활성은 `A` 그대로 · 비활성만 `A` 에서 `I` 쪽으로.
    f = crossover === minimum ? 0 : clamp01((avail - minimum) / (crossover - minimum))
    widths = mins.map(m => Math.floor(m + f * (A - m)))
  }

  // 4. 남는 px(`AllocateExtraSpace`) — 받는 머리에만 왼쪽부터 1 px 씩 · 받는 머리 수를 넘는 몫은 쓰지 않는다(넘쳤으면
  //   `extra ≤ 0` 이라 없음). 받는 머리 = `TabAcceptsExtraSpace`: 비율이 0 · 1 이면 아무도 · 단계 B 는 최소 < 넘김점인
  //   머리(= 비활성 — 활성은 최소가 곧 `A` 라 받지 않는다) · 단계 A 는 넘김점 < 선호인 머리(= 모두).
  const accepts = (i: number) => f !== 0 && f !== 1 && (phaseB ? mins[i] < A : A < S)
  let extra = avail - widths.reduce((s, w) => s + w, 0)
  for (let i = 0; i < count && extra > 0; i += 1) {
    if (!accepts(i)) continue
    widths[i] += 1
    extra -= 1
  }

  // 5. 보임(잰 폭 `room` 기준) — 오른 끝이 넘으면 숨김 · 활성이 이 머리 뒤면 「활성이 이 자리나 그 앞으로 오면」의 오른 끝
  //   (= 왼 끝 + 활성 폭)이 넘어도 숨김(활성을 바꿔도 보이는 집합이 흔들리지 않게 — Chrome 의 안정 집합).
  const visible: boolean[] = []
  let x1 = 0
  for (let i = 0; i < count; i += 1) {
    x1 += widths[i]
    if (room === null) visible.push(true)
    else if (x1 > room) visible.push(false)
    else if (active > i) visible.push(x1 + (widths[active] - widths[i]) <= room)
    else visible.push(true)
  }
  return { widths, visible }
}

function floorOrNull(v: number | null | undefined): number | null {
  return v === null || v === undefined || !Number.isFinite(v) ? null : Math.floor(v)
}

function clamp01(f: number): number {
  return Math.min(1, Math.max(0, f))
}
