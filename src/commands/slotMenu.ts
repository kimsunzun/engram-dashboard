// ADR-0064: 슬롯 컨텍스트 메뉴 = 단일 기여 API + 결정적 빌더.
//
// ★역할★: 슬롯 우클릭 메뉴 항목을 "command id 참조"로만 조립한다(메뉴에서 직접 store 호출 금지 —
//   ADR-0064 불변식). 공통 슬롯 ops 는 target='*' 로, 콘텐츠 전용 항목은 target=<SlotContentType> 로
//   같은 API(registerSlotMenu)를 통해 기여된다(VS Code contributes.menus 결). buildSlotMenu 가
//   (contentType 기여 ∪ '*' 기여)를 group·order 로 결정적 정렬하고 registry 에서 title/run 을 resolve 한다.
//
// ★DOM-free 유지★: registry.ts 와 동일하게 순수 Map/함수라 headless(vitest)로 단위테스트된다. DOM/Tauri
//   의존은 command handler(*Commands.ts / slotCommands.ts)로 밀어낸다.

import type { SlotContent } from '../api/layoutTypes'
import { getCommand } from './registry'

/** 기여 대상 = 특정 SlotContent 타입, 또는 '*'(모든 슬롯 = 공통). */
export type SlotMenuTarget = SlotContent['type'] | '*'

/**
 * 기여 항목(ADR-0064 고정 스키마 + ADR-0065 additive 확장). 두 형태 중 하나다:
 *   - **실행 항목**: `commandId` 있음(+ `children` 없음). title/run 은 빌드 시 registry 에서 resolve.
 *   - **컨테이너**(1단 서브메뉴): `children` 있음 + 자체 `title` 필수(+ `commandId` 없음).
 * 둘 다 없거나 둘 다 있으면 dev 에러(fail-loud, ADR-0065 §영향/불변식). hideOn·children·title 은 모두
 * 옵셔널이라 기존 `{commandId, group, order}` 기여는 무변경으로 그대로 유효(하위호환).
 */
export interface SlotMenuItem {
  /** 실행 항목의 registry command id(예: 'slot.close'). 컨테이너면 생략. fail-loud: 미등록이면 buildSlotMenu 가 skip+error 로그. */
  commandId?: string
  /** 컨테이너(children 有) 표기 이름. 실행 항목이면 생략(registry title 을 쓴다). */
  title?: string
  /** 정렬 1축(그룹) — 'content'(콘텐츠 전용) 먼저, 'slot-ops'(공통) 나중. GROUP_ORDER 로 순서 고정. */
  group: string
  /** 정렬 2축(그룹 내 순서). 같은 group 안에서 오름차순. */
  order: number
  /**
   * ADR-0065 제외 조건 — 나열된 콘텐츠 타입 슬롯에서 이 항목을 숨긴다(subtraction 전용, allowlist 아님).
   * 예: slot.empty·slot.popout 에 `['empty']` → '*' 보편 등록은 유지하되 빈 슬롯에서만 뺀다(공통 ops 단일소스 유지).
   */
  hideOn?: SlotContent['type'][]
  /** ADR-0065 1단 서브메뉴 자식들. 있으면 이 항목은 컨테이너(commandId 없이 title 필수). 자식은 flat 실행 항목만(중첩 금지). */
  children?: SlotMenuItem[]
}

/**
 * buildSlotMenu 산출 = 렌더러가 그대로 쓰는 resolve 된 항목(+ 그룹 경계 표시).
 * children 이 있으면 컨테이너(1단 서브메뉴) — 렌더러는 이 경우 run 을 부르지 않고 hover flyout 으로 children 을 편다.
 */
export interface ResolvedSlotMenuItem {
  id: string
  title: string
  run: (args?: Record<string, unknown>) => unknown
  group: string
  /** 이 항목이 자기 그룹의 첫 항목이면서 앞에 다른 그룹이 있었으면 true(렌더러가 구분선을 그린다). */
  separatorBefore: boolean
  /** ADR-0065 1단 서브메뉴 — 컨테이너면 resolve 된 자식들. 실행 항목이면 undefined. */
  children?: ResolvedSlotMenuItem[]
}

// ★그룹 렌더 순서 고정★(ADR-0064 §5): 콘텐츠 전용('content')을 위에, 공통 슬롯 ops('slot-ops')를 아래에
//   두고 그 사이에 구분선. 여기 없는 group 은 뒤로 밀되(Infinity) 이름순 안정 정렬 — 새 group 추가 시
//   여기 명시해 순서를 고정한다(암묵 순서 금지).
const GROUP_ORDER: Record<string, number> = {
  content: 0,
  'slot-ops': 1,
}

function groupRank(group: string): number {
  return GROUP_ORDER[group] ?? Number.POSITIVE_INFINITY
}

// target → 기여 항목 배열. registerSlotMenu 가 import 부수효과로 채운다.
const contributions = new Map<SlotMenuTarget, SlotMenuItem[]>()

/** 기여의 dedupe/교체 키 — 실행 항목은 commandId, 컨테이너(children 有)는 title 로 식별한다(commandId 없음). */
function itemKey(item: SlotMenuItem): string {
  return item.commandId ?? `container:${item.title ?? ''}`
}

/**
 * 슬롯 메뉴 기여 등록. 같은 (target, key) 재기여는 조용히 덮지 않고 warn 후 교체한다
 * (registry.register 와 동형 — HMR 재평가·중복 로드·오타를 드러낸다). idempotent: 마지막 등록이 이긴다.
 * key = commandId(실행 항목) 또는 'container:'+title(컨테이너) — 컨테이너는 commandId 가 없어 title 로 식별한다.
 */
export function registerSlotMenu(target: SlotMenuTarget, items: SlotMenuItem[]): void {
  const list = contributions.get(target) ?? []
  for (const item of items) {
    const key = itemKey(item)
    const idx = list.findIndex(existing => itemKey(existing) === key)
    if (idx >= 0) {
      // HMR 재평가나 실수 중복 — 마지막이 이기되 조용히 넘어가지 않는다.
      console.warn(`[slotMenu] 중복 기여 재등록 — 교체: target='${target}' key='${key}'`)
      list[idx] = item
    } else {
      list.push(item)
    }
  }
  contributions.set(target, list)
}

/** (group, order) 결정적 비교자 — 등록 순서 무관. 동률 타이브레이크는 실행 항목 commandId / 컨테이너 title. */
function compareItems(a: SlotMenuItem, b: SlotMenuItem): number {
  const gr = groupRank(a.group) - groupRank(b.group)
  if (gr !== 0) return gr
  // 같은 랭크의 서로 다른 group 명(둘 다 미등록 등) — 이름순으로 안정화.
  if (a.group !== b.group) return a.group < b.group ? -1 : 1
  if (a.order !== b.order) return a.order - b.order
  // 최종 타이브레이크 — key(commandId 또는 container:title) 로 결정적.
  const ka = itemKey(a)
  const kb = itemKey(b)
  return ka < kb ? -1 : ka > kb ? 1 : 0
}

/**
 * 실행 항목(commandId) 하나를 resolve 한다. 성공 시 ResolvedSlotMenuItem, 실패(미등록 등) 시 null(skip).
 * ★fail-loud but crash-free(FIX-1)★: 렌더 시점(우클릭) 호출이라 throw 하면 error boundary 부재 시 앱 blank.
 *   시끄럽게 console.error 하고 그 항목만 null 로 skip 한다(부팅 전수 검증은 validateSlotMenuContributions).
 */
function resolveRunnable(commandId: string, target: SlotContent['type']): ResolvedSlotMenuItem | null {
  const cmd = getCommand(commandId)
  if (!cmd) {
    console.error(`[slotMenu] unregistered commandId "${commandId}" (target=${target}) — skipped`)
    return null
  }
  return { id: cmd.id, title: cmd.title, run: cmd.run, group: '', separatorBefore: false }
}

/**
 * 항목 형태 검증(ADR-0065 §영향/불변식) — 실행 항목 XOR 컨테이너. 위반이면 시끄럽게 로그하고 false(skip).
 *   - 실행 항목: commandId 有 + children 無.
 *   - 컨테이너: children 有(비어있지 않음) + title 有 + commandId 無.
 * ★1단 한정★: nested=true(= 이 항목이 이미 컨테이너의 자식)면 children 을 또 가지는 것은 2단 중첩이라 금지.
 * ★fail-loud but crash-free★: buildSlotMenu 의 FIX-1 스타일과 동형 — dev 에러(console.error) + skip.
 */
function validateItemShape(item: SlotMenuItem, target: SlotMenuTarget, nested: boolean): boolean {
  const hasCmd = typeof item.commandId === 'string' && item.commandId.length > 0
  const hasChildren = Array.isArray(item.children) && item.children.length > 0
  const label = item.commandId ?? item.title ?? '(untitled)'
  if (hasCmd && hasChildren) {
    console.error(`[slotMenu] item "${label}" (target=${target}) — commandId 와 children 을 동시에 가질 수 없음 (실행 항목 XOR 컨테이너) — skipped`)
    return false
  }
  if (!hasCmd && !hasChildren) {
    console.error(`[slotMenu] item "${label}" (target=${target}) — commandId 도 children 도 없음 (실행 항목 XOR 컨테이너 필요) — skipped`)
    return false
  }
  if (hasChildren) {
    if (nested) {
      console.error(`[slotMenu] item "${label}" (target=${target}) — 2단+ 중첩 금지(children 안의 children) — skipped`)
      return false
    }
    if (typeof item.title !== 'string' || item.title.length === 0) {
      console.error(`[slotMenu] container "${label}" (target=${target}) — 컨테이너는 title 필수 — skipped`)
      return false
    }
  }
  return true
}

/**
 * contentType 기여 ∪ '*'(공통) 기여를 모아 결정적 정렬 → registry resolve → 렌더 가능한 항목 배열.
 *
 * ★정렬은 등록 순서 무관 — (groupRank, order) 로만★(ADR-0064): import 순서·HMR 순서에 렌더가 흔들리지
 *   않게 한다. 안정 정렬을 위해 group 은 GROUP_ORDER 랭크, 그 안은 order, 동률이면 key(commandId/title) 로 타이브레이크.
 * ★hideOn 필터(ADR-0065)★: hideOn 에 contentType 이 포함된 항목은 먼저 뺀다(subtraction — '*' 보편성은 유지,
 *   특정 타입만 제외). 공통 ops 단일소스('*') 불변식을 유지하면서 빈 슬롯에서 slot.empty/slot.popout 을 트림한다.
 * ★1단 서브메뉴(ADR-0065)★: children 있는 컨테이너는 title passthrough + 각 자식 commandId 를 resolve 한다.
 *   ★자식은 선언 순서 보존★(최상위와 달리 재정렬 없음) — 기여가 이미 의도한 상대 순서로 나열한다(ADR-0065
 *   "keeping their relative order"). 자식이 또 children 을 가지면 2단 중첩이라 dev 에러 + skip(nested=true).
 *   ★hideOn 은 자식에도 적용(FIX-2)★: 최상위와 동일 subtraction 을 자식마다 돌린다(가시성 계약 = 자식 포함).
 *   ★빈 컨테이너 omit(FIX-1)★: 자식이 전부 skip(미등록·2단 중첩·hideOn)돼 비면 컨테이너를 leaf 로 오인해
 *   dead 항목이 되므로 컨테이너째 console.error + skip(children:[] 로 emit 하지 않는다).
 * ★dedupe(FIX-2)★: '*'(공통) 와 contentType(전용) 가 같은 key 를 기여하면 중복 항목·중복 React key 가 생긴다.
 *   정렬 후 key 로 dedupe 해 최종 순서의 *첫 등장*만 남긴다(content 그룹이 slot-ops 보다 먼저 정렬되므로,
 *   공통 op 를 콘텐츠가 재선언하면 content 쪽이 이긴다 — 다만 공통 op 재선언은 원래 없어야 하고 방어적 dedupe).
 * ★fail-loud but crash-free(FIX-1)★: 미등록 commandId·형태 위반은 throw 하지 않고 console.error + skip.
 * separatorBefore: 그룹이 바뀌는 첫 항목(맨 앞 제외)에 true → 렌더러가 그 위에 구분선을 그린다.
 */
export function buildSlotMenu(contentType: SlotContent['type']): ResolvedSlotMenuItem[] {
  // '*'(공통) + contentType(전용) 기여를 합친다. contentType==='*' 는 타입상 불가(SlotContent['type']).
  const merged: SlotMenuItem[] = [...(contributions.get('*') ?? []), ...(contributions.get(contentType) ?? [])]

  // ADR-0065: hideOn 제외(subtraction) — 이 콘텐츠 타입에서 숨길 항목을 먼저 뺀다.
  const visible = merged.filter(item => !(item.hideOn?.includes(contentType) ?? false))

  const sorted = visible.slice().sort(compareItems)

  const resolved: ResolvedSlotMenuItem[] = []
  const seen = new Set<string>() // FIX-2: key dedupe — 첫 등장만.
  let prevGroup: string | null = null
  for (const item of sorted) {
    // FIX-2: '*' 와 콘텐츠가 같은 key 를 기여하면 정렬 후 첫 등장만 남긴다(중복 key 방지).
    //   prevGroup 은 skip 항목으로 갱신하지 않아 구분선 계산이 실제 렌더 순서를 따른다.
    const key = itemKey(item)
    if (seen.has(key)) continue
    // ADR-0065: 실행 항목 XOR 컨테이너 형태 검증(위반 시 skip — prevGroup·seen 미갱신).
    if (!validateItemShape(item, contentType, false)) continue

    let entry: ResolvedSlotMenuItem
    if (item.children) {
      // 컨테이너(1단 서브메뉴): title passthrough + 각 자식 resolve. 자식은 실행 항목만(nested 검증).
      const children: ResolvedSlotMenuItem[] = []
      for (const child of item.children) {
        // FIX-2(ADR-0065): hideOn 필터를 자식에도 적용 — 최상위와 동일 subtraction. 이 콘텐츠 타입을 hideOn
        //   에 담은 자식은 서브메뉴에서도 빠진다(가시성 계약 = 자식 포함). 필터 후 자식이 0개면 아래 FIX-1 로 컨테이너째 skip.
        if (child.hideOn?.includes(contentType) ?? false) continue
        if (!validateItemShape(child, contentType, true)) continue
        // 컨테이너 자식은 항상 실행 항목(validateItemShape nested=true 가 children 재보유를 막음).
        const rc = resolveRunnable(child.commandId as string, contentType)
        if (rc) children.push(rc)
      }
      // FIX-1(ADR-0065): 자식이 전부 skip(미등록·2단 중첩·hideOn 등)돼 비면 컨테이너를 leaf 로 오인해
      //   dead 항목(arrow 없음 + 미등록 container:<title> dispatch)이 된다 → 컨테이너째 skip + 시끄럽게 로그.
      //   (fail-loud but crash-free — buildSlotMenu 의 다른 skip 과 동형, seen·prevGroup 미갱신.)
      if (children.length === 0) {
        console.error(
          `[slotMenu] container "${item.title}" (target=${contentType}) — 자식이 모두 skip 되어 빈 컨테이너 — omitted`,
        )
        continue
      }
      entry = {
        // 컨테이너 id 는 title 기반 합성(commandId 없음) — React key·data-attr 용. run 은 no-op(렌더러가 안 부름).
        id: `container:${item.title}`,
        title: item.title as string,
        run: () => undefined,
        group: item.group,
        separatorBefore: prevGroup !== null && prevGroup !== item.group,
        children,
      }
    } else {
      const rr = resolveRunnable(item.commandId as string, contentType)
      if (!rr) continue // 미등록 — seen·prevGroup 미갱신(FIX-1 skip)
      entry = {
        ...rr,
        group: item.group,
        // 첫 항목이 아니고 그룹이 직전과 다르면 구분선(그룹 경계).
        separatorBefore: prevGroup !== null && prevGroup !== item.group,
      }
    }
    seen.add(key)
    resolved.push(entry)
    prevGroup = item.group
  }
  return resolved
}

/**
 * 부팅 전수 검증(FIX-1 + FIX-3) — 등록된 모든 기여를 우클릭 없이 부팅 즉시 검사한다. 두 종류:
 *   1. **형태 검증(FIX-3)** — validateItemShape 를 자식까지 재귀로 돌린다(both-cmd-and-children / 2단 중첩 /
 *      컨테이너 title 누락). 옛 버전은 commandId 존재만 봐서 형태 위반이 우클릭(buildSlotMenu) 때만 잡혔다 —
 *      이제 부팅(dev-time)에 발각된다.
 *   2. **commandId 존재(FIX-1)** — 실행 항목·컨테이너 자식의 commandId 가 registry 에 있는지 확인.
 * 둘 다 crash 없이 console.error(부팅 부수효과 최소 — buildSlotMenu 의 skip+log 와 짝, throw 로 승격 금지).
 * ★buildSlotMenu 와 달리 정렬/resolve/hideOn 은 하지 않는다★ — 순수 검증만(가시성은 렌더 관심사).
 */
export function validateSlotMenuContributions(): void {
  for (const [target, items] of contributions) {
    for (const item of items) {
      // FIX-3: 형태 검증(실행 항목 XOR 컨테이너). 위반이면 아래 commandId 검증은 무의미하므로 skip.
      if (!validateItemShape(item, target, false)) continue
      // 컨테이너(children 有)는 자체 commandId 가 없다 — 자식들을 재귀 검증한다(형태 + commandId, 1단).
      if (item.children) {
        for (const child of item.children) {
          if (!validateItemShape(child, target, true)) continue
          if (child.commandId && !getCommand(child.commandId)) {
            console.error(
              `[slotMenu] unregistered commandId "${child.commandId}" (target=${target}) — contribution will be skipped at render`,
            )
          }
        }
        continue
      }
      if (item.commandId && !getCommand(item.commandId)) {
        console.error(
          `[slotMenu] unregistered commandId "${item.commandId}" (target=${target}) — contribution will be skipped at render`,
        )
      }
    }
  }
}

/** 테스트 전용 — 기여 초기화(테스트 간 격리). 프로덕션 코드에서 호출 금지(registry.__reset… 미러). */
export function __resetSlotMenuForTest(): void {
  contributions.clear()
}
