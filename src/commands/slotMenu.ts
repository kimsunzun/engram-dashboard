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

/** 기여 항목(고정 스키마) — command id 참조 + 정렬 좌표만. 실제 title/run 은 빌드 시 registry 에서 resolve. */
export interface SlotMenuItem {
  /** registry 에 등록된 command id(예: 'slot.close'). fail-loud: 미등록이면 buildSlotMenu 가 skip+error 로그. */
  commandId: string
  /** 정렬 1축(그룹) — 'content'(콘텐츠 전용) 먼저, 'slot-ops'(공통) 나중. GROUP_ORDER 로 순서 고정. */
  group: string
  /** 정렬 2축(그룹 내 순서). 같은 group 안에서 오름차순. */
  order: number
}

/** buildSlotMenu 산출 = 렌더러가 그대로 쓰는 resolve 된 항목(+ 그룹 경계 표시). */
export interface ResolvedSlotMenuItem {
  id: string
  title: string
  run: (args?: Record<string, unknown>) => unknown
  group: string
  /** 이 항목이 자기 그룹의 첫 항목이면서 앞에 다른 그룹이 있었으면 true(렌더러가 구분선을 그린다). */
  separatorBefore: boolean
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

/**
 * 슬롯 메뉴 기여 등록. 같은 (target, commandId) 재기여는 조용히 덮지 않고 warn 후 교체한다
 * (registry.register 와 동형 — HMR 재평가·중복 로드·오타를 드러낸다). idempotent: 마지막 등록이 이긴다.
 */
export function registerSlotMenu(target: SlotMenuTarget, items: SlotMenuItem[]): void {
  const list = contributions.get(target) ?? []
  for (const item of items) {
    const idx = list.findIndex(existing => existing.commandId === item.commandId)
    if (idx >= 0) {
      // HMR 재평가나 실수 중복 — 마지막이 이기되 조용히 넘어가지 않는다.
      console.warn(`[slotMenu] 중복 기여 재등록 — 교체: target='${target}' commandId='${item.commandId}'`)
      list[idx] = item
    } else {
      list.push(item)
    }
  }
  contributions.set(target, list)
}

/**
 * contentType 기여 ∪ '*'(공통) 기여를 모아 결정적 정렬 → registry resolve → 렌더 가능한 항목 배열.
 *
 * ★정렬은 등록 순서 무관 — (groupRank, order) 로만★(ADR-0064): import 순서·HMR 순서에 렌더가 흔들리지
 *   않게 한다. 안정 정렬을 위해 group 은 GROUP_ORDER 랭크, 그 안은 order, 동률이면 commandId 로 타이브레이크.
 * ★dedupe(FIX-2)★: '*'(공통) 와 contentType(전용) 가 같은 commandId 를 기여하면 중복 항목·중복 React key 가
 *   생긴다. 정렬 후 commandId 로 dedupe 해 최종 순서의 *첫 등장*만 남긴다(content 그룹이 slot-ops 보다 먼저
 *   정렬되므로, 공통 op 를 콘텐츠가 재선언하면 content 쪽이 이긴다 — 다만 공통 op 재선언은 원래 없어야 하고
 *   방어적으로 dedupe 한다).
 * ★fail-loud but crash-free(FIX-1)★: 기여한 commandId 가 registry 에 없으면 이 함수는 렌더 시점(우클릭)에
 *   불리므로 throw 하면 error boundary 부재 시 앱 전체가 blank 로 죽는다. 대신 console.error 로 시끄럽게
 *   알리고 그 항목만 skip 한다(조용한 누락은 아니되, 렌더는 살린다). 부팅 시 전수 검증은
 *   validateSlotMenuContributions() 가 담당(즉시 발각).
 * separatorBefore: 그룹이 바뀌는 첫 항목(맨 앞 제외)에 true → 렌더러가 그 위에 구분선을 그린다.
 */
export function buildSlotMenu(contentType: SlotContent['type']): ResolvedSlotMenuItem[] {
  // '*'(공통) + contentType(전용) 기여를 합친다. contentType==='*' 는 타입상 불가(SlotContent['type']).
  const merged: SlotMenuItem[] = [...(contributions.get('*') ?? []), ...(contributions.get(contentType) ?? [])]

  const sorted = merged.slice().sort((a, b) => {
    const gr = groupRank(a.group) - groupRank(b.group)
    if (gr !== 0) return gr
    // 같은 랭크의 서로 다른 group 명(둘 다 미등록 등) — 이름순으로 안정화.
    if (a.group !== b.group) return a.group < b.group ? -1 : 1
    if (a.order !== b.order) return a.order - b.order
    // 최종 타이브레이크 — commandId 로 결정적.
    return a.commandId < b.commandId ? -1 : a.commandId > b.commandId ? 1 : 0
  })

  const resolved: ResolvedSlotMenuItem[] = []
  const seen = new Set<string>() // FIX-2: commandId dedupe — 첫 등장만.
  let prevGroup: string | null = null
  for (const item of sorted) {
    // FIX-2: '*' 와 콘텐츠가 같은 commandId 를 기여하면 정렬 후 첫 등장만 남긴다(중복 key 방지).
    //   prevGroup 은 skip 항목으로 갱신하지 않아 구분선 계산이 실제 렌더 순서를 따른다.
    if (seen.has(item.commandId)) continue
    const cmd = getCommand(item.commandId)
    if (!cmd) {
      // FIX-1: fail-loud 이되 crash-free — 미등록 commandId 는 시끄럽게 로그하고 그 항목만 skip 한다
      //   (렌더 시점 호출이라 throw 하면 error boundary 부재 시 앱 blank). 부팅 전수 검증은
      //   validateSlotMenuContributions() 가 맡는다.
      console.error(
        `[slotMenu] unregistered commandId "${item.commandId}" (target=${contentType}) — skipped`,
      )
      continue
    }
    seen.add(item.commandId)
    resolved.push({
      id: cmd.id,
      title: cmd.title,
      run: cmd.run,
      group: item.group,
      // 첫 항목이 아니고 그룹이 직전과 다르면 구분선(그룹 경계).
      separatorBefore: prevGroup !== null && prevGroup !== item.group,
    })
    prevGroup = item.group
  }
  return resolved
}

/**
 * 부팅 전수 검증(FIX-1) — 등록된 모든 기여의 commandId 가 registry 에 있는지 확인하고, 없는 것마다
 * console.error 한다. contributions.ts 끝(모든 command·기여 모듈 import 후)에서 1회 호출해 오타·미등록을
 * 우클릭 없이 부팅 즉시 발각한다("즉시 발각"을 crash 없이 달성 — buildSlotMenu 의 skip+log 와 짝).
 * ★buildSlotMenu 와 달리 정렬/resolve 하지 않는다★ — 순수 검증만(부팅 부수효과 최소).
 */
export function validateSlotMenuContributions(): void {
  for (const [target, items] of contributions) {
    for (const item of items) {
      if (!getCommand(item.commandId)) {
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
