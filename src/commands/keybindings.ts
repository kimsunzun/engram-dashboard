// ADR-0055: 전역 키바인딩 — key-combo → command id 매핑 후 run(id). 사람 클릭·__engramCmd 와 함께
//   소비자 계층(레지스트리는 발견/라우팅/메타만). 골격 최소 — 커스텀 키맵 저장(localStorage)은 후속.

import { fireAndForget } from './dispatch'
import { getCommand } from './registry'

// ─────────────────────────────────────────────────────────────────────────────
// ★LOAD-BEARING 포커스 가드 (ADR-0055 · CLAUDE.md §5)★
//   전역 keydown 은 사용자가 입력 중일 때 단축키를 가로채면 안 된다. <input>/<textarea>/
//   contenteditable/터미널(.xterm — xterm.js) 안에서 타이핑 중이면 즉시 bail-out 해 키를
//   그대로 흘려보낸다. 안 그러면 터미널·입력창 타이핑을 단축키가 삼키는 회귀(예: 't' 를 치면
//   테마가 바뀌는 식). ★이 가드에 구멍이 나면 단축키가 터미널/입력 타이핑을 가로챈다★(load-bearing).
//   이 술어를 순수 함수로 뽑아 headless 로 단위테스트한다.
// ─────────────────────────────────────────────────────────────────────────────
export function isEditableTarget(target: EventTarget | null): boolean {
  // Element 가 아니면(document/window 등) 편집 대상 아님.
  if (!target || typeof (target as Element).closest !== 'function') return false
  const el = target as Element

  const tag = el.tagName
  if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return true

  // ★contenteditable 은 isContentEditable 만으로 판정★(FIX-A, load-bearing): HTMLElement.isContentEditable 은
  //   HTML 스펙상 *실효(effective)* 편집 가능성을 돌려준다 — contenteditable="" / "true" / "plaintext-only" 는
  //   물론 조상에서 상속된 편집 가능성까지 true 로 보고, 반대로 contenteditable="false" 섬(island)은 편집 가능한
  //   조상 밑에 있어도 정확히 false 로 본다. 예전엔 여기에 closest('[contenteditable]:not([="false"])') 폴백을
  //   더 얹었는데, closest 는 "false" 경계를 지나쳐 그 위 편집 조상에 매칭돼 → 비편집 섬(예: 편집 div 안의
  //   contenteditable="false" 버튼) 위에서도 단축키를 삼키는 버그를 만들었다. isContentEditable 이 상속까지
  //   이미 처리하므로 폴백은 중복이자 버그원 → 제거한다. HTMLElement 가 아닌 Element(SVG 등)는 이 프로퍼티가
  //   없어 undefined→falsy 인데, SVG 는 HTML-contenteditable 대상이 아니므로 그게 정답이다.
  if ((el as HTMLElement).isContentEditable) return true

  // xterm.js 터미널 내부 — 터미널 키 입력을 단축키가 삼키면 안 된다(load-bearing).
  if (el.closest('.xterm')) return true

  return false
}

/** KeyboardEvent → 정규화된 combo 문자열(예: 'ctrl+shift+t'). 매핑 테이블 키와 대조. */
export function comboOf(e: KeyboardEvent): string {
  const parts: string[] = []
  if (e.ctrlKey) parts.push('ctrl')
  if (e.altKey) parts.push('alt')
  if (e.shiftKey) parts.push('shift')
  if (e.metaKey) parts.push('meta')
  // 수식키 자체는 제외. key 는 소문자로 정규화(shift 조합 시 대문자 방지).
  const key = e.key.toLowerCase()
  if (key !== 'control' && key !== 'alt' && key !== 'shift' && key !== 'meta') {
    parts.push(key)
  }
  return parts.join('+')
}

// combo → command id 매핑(골격 최소 — 커스텀 키맵은 후속). 첫 어댑터: 테마 순환.
const BINDINGS: Record<string, string> = {
  'ctrl+shift+t': 'theme.toggle',
}

/**
 * 전역 키바인딩 설치. import 시점이 아니라 명시 호출로 리스너를 건다(배선·정리 제어 가능).
 * 반환 = disposer(리스너 제거). HMR/언마운트에서 호출해 중복 누적을 막는다.
 */
export function installKeybindings(): () => void {
  const onKeyDown = (e: KeyboardEvent): void => {
    // ★포커스 가드 먼저★ — 입력/터미널 타이핑 중이면 단축키 무시(위 불변식).
    if (isEditableTarget(e.target)) return

    const id = BINDINGS[comboOf(e)]
    if (!id) return

    // ★when 게이트는 여기(키바인딩 소비자)에서만★(FIX-5, VS Code 시맨틱): when 은 "UI 컨텍스트 게이트"이지
    //   command 자체의 실행 가능 조건이 아니다. 키/팔레트 같은 *컨텍스트 발동* 소비자는 when 이 false 면
    //   해당 command 를 못 본 것처럼 넘겨 키를 그대로 흘려보낸다. 반면 명시적 run('id')·__engramCmd·cdp
    //   호출은 무조건 실행한다(호출자가 이미 판단함) → registry.run() 은 when 을 읽지 않는다.
    const cmd = getCommand(id)
    if (cmd?.when) {
      // ★when 은 방어적으로 평가★(FIX-B): 사용자 제공 when 이 throw 하면 전역 keydown 리스너 밖으로
      //   uncaught 예외가 새어나간다(load-bearing 글로벌 리스너라 치명적). VS Code when-절 시맨틱대로
      //   throw = "컨텍스트 미충족 = false" 로 취급 → command 를 건너뛰고 키를 그대로 통과시킨다.
      let ok = false
      try {
        ok = cmd.when()
      } catch {
        ok = false
      }
      if (!ok) return // when=false(또는 throw) → 이 키를 삼키지 않고 통과(preventDefault 안 함)
    }

    e.preventDefault()
    // ★fire-and-forget 로 실행★(FIX-3/4): 결과를 기다리지 않으므로 sync throw·async reject 를 모두
    //   삼키는 공유 helper 를 쓴다(리스너가 죽지 않게). run() 직접 호출 금지 — 안전망 재구현 방지.
    fireAndForget(id)
  }

  // ★리스너는 document 에 건다★(FIX-2, ADR-0055 계약): 전역 keydown 은 document 레벨에서 버블을 받는다.
  //   add/remove 는 반드시 같은 핸들러 참조를 넘겨야 정리가 성립한다(disposer 가 정확히 이 리스너만 뗀다).
  document.addEventListener('keydown', onKeyDown)
  return () => document.removeEventListener('keydown', onKeyDown)
}
