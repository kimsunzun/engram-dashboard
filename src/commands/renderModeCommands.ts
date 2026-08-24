// ADR-0055: 레지스트리는 상태 권위가 아니다 — 이 다섯은 viewStore 의 렌더 모드 액션으로 라우팅만 한다
//   (새 상태 경로 0). 사람 클릭·팔레트·LLM(__engramCmd)이 같은 command 를 실행한다(§5 단일 제어 표면).
//
// ★형제 slot.* 과 달리 백엔드 권위 루프를 안 탄다★: renderModeOverride 는 프론트 전용 override 라
//   invoke→emit(ADR-0035)을 거치지 않는다(그 예외의 계약 = viewStore 의 renderModeOverride 필드 JSDoc).
//   그래서 이 다섯의 반환은 항상 undefined 다 — 호출부가 await 할 완료 시점이 없다.
//
// ★★버스에 올리지 말 것 — `help` 를 더하면 슬롯을 소유하지 않은 창에 쓸 수 있다★★ (ADR-0167)
//   위 항의 실무적 귀결이다. renderModeOverride 는 **창(웹뷰)마다 따로 있는** 프론트 상태다. 형제 slot.*
//   의 변경은 invoke 를 타고 **백엔드 권위 한 곳**에 쓰이지만, 이 다섯은 **부른 창의** store 에 쓴다.
//   그런데 봉투가 갈 창(host)은 셸이 배달 시점에 고르고 앱이 도는 중에도 옮겨 다니므로
//   (`src-tauri/src/view_commands.rs` 의 `Registered::repick`), 그 슬롯을 소유하지 않은 창의 store 에
//   기록하고 화면은 아무것도 안 바뀐 채 `ok` 로 답하는 일이 생긴다 — 호출자는 성공으로 읽는다.
//   ★창 좌표를 인자로 붙이는 것은 탈출구가 아니다★ — 디스패처가 언제나 host 로만 보내서 그 인자를
//   무시한다. 지목한 창으로 배달하려면 라우팅 구현이 바뀌어야 하고, 그 ADR 이 거부한 대안 1 이 그것이다
//   (거기 적힌 「구조상 불가능」은 코드보다 센 표현이다 — 리스너·보고는 창마다 이미 있고 미구현인 것은
//   선정뿐이다. 실측 2026-08-23). 지금 이걸 막는 유일한 것이 **`help` 부재**다
//   (`commands/viewCommandBridge.ts` 의 `offeredCommands` 가 그 칸 하나로 승선을 가른다).

import { t } from '../i18n'
import { RENDER_MODES, isRenderMode, type RenderMode } from '../components/slot/renderMode'
import { useViewStore } from '../store/viewStore'
import { register } from './registry'

/** 렌더 모드 command 의 실행 컨텍스트 인자(단일 가방, ADR-0055). */
interface RenderModeCtx {
  slotId?: unknown
  mode?: unknown
}

/**
 * ★viewId 를 받지 않는다★: 오버라이드 키는 slot node.id 이고(`ViewLayoutRenderer` 의
 * `renderModeOverride[slotId]`), slotId 는 창 간 전역 고유 UUID 라(ADR-0035 불변식) viewId 로 좁힐 것이
 * 없다. 형제 slot.* 이 요구하는 viewId 를 여기서도 요구하면 대상 액션이 쓰지도 않는 값을 호출자(LLM)가
 * 찾아 채워야 한다.
 *
 * ★모르는 칸은 거절하지 않는다(여분 키 무시)★ — ADR-0055 의 「인자 = 객체 하나(가방), 각 handler 가
 * 필요한 키만 destructure」 그대로다. 덕분에 **나중에** 이 id 들을 슬롯 메뉴에 기여시키면(지금 기여는
 * 없다 — 이 파일에 registerSlotMenu 호출이 없다) 메뉴가 넘기는 ctx 가방 `{viewId, slotId, agentId}`
 * (`SlotContextMenu`)이 그대로 들어맞는다. ADR-0157 의 「입구에서 선언에 없는 칸을 거절한다」를 여기에
 * 옮겨 심지 말 것 — 그 거절 목록은 **인자 선언에서 파생**해야 하는데(같은 ADR 불변식: 손으로 유지하는
 * 사본 금지) 이 다섯은 `help` 를 안 달아 파생할 선언 자체가 없다.
 */
function requireSlotId(args: RenderModeCtx | undefined, cmd: string): string {
  const slotId = args?.slotId
  if (typeof slotId !== 'string' || slotId.length === 0) throw new Error(`[${cmd}] slotId 필요`)
  return slotId
}

/**
 * ★무효 mode 는 throw — no-op 으로 삼키지 않는다★. store 의 `setRenderMode` 는 무효 mode 를 warn 후
 * 무시하므로, 걸러내지 않고 그대로 흘리면 아무 일도 안 일어난 호출이 `undefined` 반환 = 성공으로 보인다
 * (버스 다리도 error 없는 결말로 회신한다 — `viewCommandBridge.settle`). 같은 판단이 이 레지스트리의
 * 진입점에 이미 박혀 있다: `registry.run` 이 모르는 id 를 throw 하는 근거가 「조용한 no-op 은 LLM/cdp
 * 디버깅을 어렵게 함」이고, 형제 slot.* 의 좌표 가드도 같은 자리에서 throw 한다.
 *
 * ★이건 「모르는 칸」 규칙(ADR-0157)이 아니다★ — 그 규칙이 가리는 것은 **이름을 모르는 칸**이고,
 * 여기서 걸리는 것은 아는 칸의 **값이 유효 집합 밖**인 경우다.
 */
function requireMode(args: RenderModeCtx | undefined, cmd: string): RenderMode {
  const mode = args?.mode
  if (!isRenderMode(mode)) {
    throw new Error(
      `[${cmd}] mode 는 ${RENDER_MODES.join('|')} 중 하나여야 한다 — 받은 값: ${JSON.stringify(mode)}`,
    )
  }
  return mode
}

register({
  id: 'slot.renderMode.set',
  title: t('slot.renderModeSet'),
  category: 'slot',
  run: args => {
    const cmd = 'slot.renderMode.set'
    // 두 인자를 모두 검사한 *뒤*에 store 를 부른다 — 부분 적용(mode 만 무효인데 이미 쓴 상태)이 없도록.
    const slotId = requireSlotId(args, cmd)
    const mode = requireMode(args, cmd)
    useViewStore.getState().setRenderMode(slotId, mode)
  },
})

register({
  id: 'slot.renderMode.clear',
  title: t('slot.renderModeClear'),
  category: 'slot',
  run: args => {
    useViewStore.getState().clearRenderMode(requireSlotId(args, 'slot.renderMode.clear'))
  },
})

// ── DOM 모드 별칭(= renderMode 'dom' 위 얇은 래퍼 — 도메인 상태는 renderModeOverride 하나뿐) ──────────

register({
  id: 'slot.domMode.enable',
  title: t('slot.domModeEnable'),
  category: 'slot',
  run: args => {
    useViewStore.getState().enableDomMode(requireSlotId(args, 'slot.domMode.enable'))
  },
})

register({
  id: 'slot.domMode.disable',
  title: t('slot.domModeDisable'),
  category: 'slot',
  run: args => {
    useViewStore.getState().disableDomMode(requireSlotId(args, 'slot.domMode.disable'))
  },
})

register({
  id: 'slot.domMode.toggle',
  title: t('slot.domModeToggle'),
  category: 'slot',
  // ★토글 판정은 store 가 한다★ — 현재 값을 여기서 읽어 분기하면 레지스트리가 상태를 해석하는 두 번째
  //   자리가 된다(ADR-0055 「상태 권위 아님」). 읽기-판정-쓰기가 store 액션 한 번 안에서 끝나야 한다.
  run: args => {
    useViewStore.getState().toggleDomMode(requireSlotId(args, 'slot.domMode.toggle'))
  },
})
