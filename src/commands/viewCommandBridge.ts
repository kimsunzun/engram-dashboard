// 이 창의 command 를 셸에 알리고, 셸이 내려보낸 봉투를 레지스트리로 흘리는 배선(TRD §6 Step 4).
//
// ★두 번째 제어 표면이 아니다★: 실행은 여전히 `registry.run(id, args)` 하나를 지난다(ADR-0055 §5). 새
// 전역 핸들도 새 레지스트리도 만들지 않는다 — 늘어나는 것은 그 입구로 오는 **바깥 경로**뿐이다.
//
// ★invoke/listen 을 직접 부른다★: 「컴포넌트·스토어는 agentClient 에만 의존한다」의 예외로, 백엔드가
// 권위인 표면은 Tauri 를 직접 쓴다(`theme/uiSettings.ts`·`store/viewStore.ts` 가 쓰는 그 예외 — 명령
// 명부의 주인은 데몬이고 셸이 대신 얹는다).
//
// ★어느 창이 봉투를 받나는 셸이 정한다★ — 창마다 이 App 이 떠서 전부 보고하지만 셸은 그중 하나에만
// 보낸다(`src-tauri/src/view_commands.rs` 「배달 대상이 창 하나인 이유」). 그래서 여기서 창을 고르는
// 코드를 두지 않는다 — 두면 두 곳이 서로 다른 창을 목적지로 믿는다.

import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'

import { list, run, type CommandHelp } from './registry'

/** 셸 → 이 창. 목적지로 뽑힌 창에만 온다. */
const EVT_COMMAND_REQUEST = 'command:request'
/** 부팅 보고 — 창마다 한 번. */
const CMD_REPORT_VIEW_COMMANDS = 'report_view_commands'
/** 결말 회수 — 봉투 하나에 정확히 한 번. */
const CMD_REPORT_OUTCOME = 'report_command_outcome'

interface CommandRequest {
  request_id: string
  name: string
  args: unknown
}

/** 셸에 보내는 항목 — 이름과 모양뿐이다(제목·키바인딩은 화면 것이라 안 나른다). */
export interface OfferedCommand {
  name: string
  help: CommandHelp
}

/**
 * 밖에 내놓을 command — ★`help` 를 가진 것만★.
 *
 * 그 칸이 곧 「밖에서 부를 수 있다」는 표시다(`registry.ts` 의 `Command.help`). 별도 목록을 두지 않는
 * 이유는 목록과 선언이 갈리면 목록만 뒤처지기 때문이다 — 새 command 를 버스에 올리는 것은 그 등록에
 * `help` 한 칸을 더하는 일이다.
 *
 * ★셸·데몬이 답하는 이름을 여기서 거르지 않는다★: 그 판정의 재료(셸 17개 선언 · 데몬 어휘)는 Rust 쪽에
 * 있고, 거기 한 번만 둔다(`src-tauri/src/view_commands.rs` 의 `reserved_names`). 여기에 사본을 두면 두
 * 목록이 갈리고, 갈린 쪽이 옳다고 믿는 순간 등록 패킷 하나가 통째로 반려된다.
 */
export function offeredCommands(): OfferedCommand[] {
  return list()
    .filter((cmd): cmd is typeof cmd & { help: CommandHelp } => Boolean(cmd.help?.summary?.trim()))
    .map(cmd => ({ name: cmd.id, help: cmd.help }))
}

/** 셸이 결말을 상관시킬 수 있게 되돌린다 — ★성공이든 실패든 정확히 한 번★. */
async function answer(requestId: string, ok: unknown, error: string | null): Promise<void> {
  try {
    await invoke(CMD_REPORT_OUTCOME, { requestId, ok: ok ?? null, error })
  } catch (e) {
    // 여기서 실패하면 셸은 마감까지 기다렸다가 TIMEOUT 으로 답한다 — 조용히 넘기면 그 원인이 어디에도
    //   안 남는다(창 쪽 로그가 유일한 단서다).
    console.error(`[viewCommandBridge] 결말 회신 실패 (${requestId}):`, e)
  }
}

/**
 * `run` 의 반환은 Promise 일 수도 값일 수도 있다(`registry.ts` 의 `Command.run` 계약) — 둘 다 기다린다.
 * ★JSON 으로 못 옮기는 값은 버린다★: 반환 모양은 계약된 적이 없어(명령마다 제각각) 셸이 광고하는
 * `ok` 도 「무엇이든」이다. 직렬화가 터지면 그것 때문에 성공한 조작이 실패로 보고되는 쪽이 더 나쁘다.
 */
async function settle(request: CommandRequest): Promise<void> {
  try {
    const args = (request.args ?? {}) as Record<string, unknown> | undefined
    const value = await run(request.name, args)
    await answer(request.request_id, jsonSafe(value), null)
  } catch (e) {
    await answer(request.request_id, null, e instanceof Error ? e.message : String(e))
  }
}

function jsonSafe(value: unknown): unknown {
  try {
    return JSON.parse(JSON.stringify(value ?? null)) as unknown
  } catch {
    return null
  }
}

/**
 * 구독을 걸고 이 창의 목록을 보고한다. 반환값은 disposer(언마운트/HMR 시 리스너 중복 누적 방지).
 *
 * ★구독을 **먼저** 건다★ — 보고가 셸의 명부를 채우는 순간부터 봉투가 올 수 있는데, 그때 리스너가 없으면
 * 그 봉투는 이 창에 **도착조차 안 하고** 셸의 마감까지 매달린다(`theme/uiSettings.ts` 의 같은 순서 조항).
 * ★재시도를 걸지 않는다★ — 구독이 실패하면 이 창은 봉투를 못 받지만, 셸은 목적지를 창 하나로 잡으므로
 * 그 실패가 다른 창을 막지는 않는다. 그리고 보고를 건너뛰므로 셸이 이 창을 목적지로 삼지도 않는다.
 */
export function installViewCommandBridge(): () => void {
  let disposed = false
  let unlisten: UnlistenFn | undefined

  void (async () => {
    try {
      // ★★`target` 을 빼지 말 것 — 그러면 창 수만큼 같은 명령이 실행된다★★
      //   `listen()` 의 기본 타깃은 `Any` 이고, Tauri 는 `Any` 로 등록된 리스너를 **필터와 무관하게 전부**
      //   깨운다(`match_any_or_filter` — `*target == Any || filter(target)`). 즉 셸이 창 하나를 골라
      //   `emit_to(label, …)` 로 보내도 기본 등록으로는 main·트리·팝아웃이 다 받아서 같은 봉투를 각자
      //   실행하고 각자 답한다(한 `request_id` 에 답장 하나 — TRD §4-⑤ 위반). 자기 label 로 등록하면
      //   `AnyLabel` 끼리 label 이 같은 하나만 걸린다.
      //   ★label 은 Tauri 가 준다★ — 해시·설정에서 유추하지 않는다(그 값이 틀리면 남의 창 봉투를 받는다).
      const off = await listen<CommandRequest>(EVT_COMMAND_REQUEST, e => {
        // 정리된 뒤에 온 것은 버린다 — 죽은 인스턴스(StrictMode 첫 회 · HMR 이전 판)가 답하면 같은
        //   봉투에 답이 둘이 된다(셸이 두 번째를 경고로 남긴다).
        if (disposed) return
        void settle(e.payload)
      }, { target: getCurrentWindow().label })
      if (disposed) {
        off()
        return
      }
      unlisten = off
    } catch (e) {
      console.error(
        `[viewCommandBridge] ${EVT_COMMAND_REQUEST} 구독 실패 — 이 창은 버스 명령을 못 받는다:`,
        e,
      )
      return
    }

    try {
      await invoke(CMD_REPORT_VIEW_COMMANDS, { commands: offeredCommands() })
    } catch (e) {
      console.warn(`[viewCommandBridge] ${CMD_REPORT_VIEW_COMMANDS} 실패:`, e)
    }
  })()

  return () => {
    disposed = true
    unlisten?.()
    unlisten = undefined
  }
}
