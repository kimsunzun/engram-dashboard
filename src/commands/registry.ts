// ADR-0055 ★상태 권위가 아니다★: 발견/라우팅/메타만 담는다. 실행은 handler 가 기존 진입점(store 액션 /
//   기존 invoke)으로 라우팅한다(새 싱글톤·새 arg 파싱 0, ADR-0035 레이아웃 권위 유지). 소비자(사람 클릭·
//   전역 keydown·window.__engramCmd)는 전부 run(id, args) 하나로 통일한다(§5).
//
// ★DOM-free 유지★: 순수 Map 이라 headless(vitest) 로 단위테스트된다. DOM/Tauri 의존은
//   어댑터(*Commands.ts)·키바인딩 리스너로 밀어낸다.

/** 각 handler 가 필요한 키만 destructure(가변인자 금지, ADR-0055). */
export type CommandArgs = Record<string, unknown>

/** 인자 한 칸의 모양 — JSON Schema 조각으로 그대로 펴진다(셸 `view_commands.rs`). */
export interface CommandArgSchema {
  type?: 'string' | 'number' | 'integer' | 'boolean' | 'object' | 'array'
  enum?: string[]
  description?: string
}

/**
 * 밖에서 이 command 를 부를 수 있게 하는 설명 — ★있으면 버스에 오르고 없으면 이 창 안에만 남는다★.
 *
 * 이름만으로는 인자를 채울 수 없어서 붙인다(ADR-0156 — 소유자에게 되묻는 왕복을 없앤 그 값). Rust 쪽은
 * 선언 매크로가 자동으로 채우지만 TypeScript 엔 매크로가 없어 **여기가 손으로 적는 유일한 자리**다.
 * 셸이 이 값을 데몬 카탈로그 항목 하나로 펴서 등록한다(`src-tauri/src/view_commands.rs`).
 */
export interface CommandHelp {
  /** 이 command 가 무엇을 하는가 — 한 줄. */
  summary: string
  /**
   * 읽기인가 쓰기인가 — ★생략할 수 없다★.
   *
   * Rust 선언은 `#[effect(..)]` 로 컴파일 옆에서 정하지만 TypeScript 엔 그 자리가 없다. 셸이 기본값을
   * 고르면 첫 조회 command 가 붙는 날 명부가 거짓 표식을 광고하고, 그 값은 데몬의 쓰기 보존 회계에
   * 그대로 먹인다 — 그래서 `help` 를 다는 사람이 매번 답하게 한다(셸은 이 칸이 없는 항목을 등록에서 뺀다).
   */
  effect: 'read' | 'write'
  /** 인자 이름 → 모양. 인자가 없으면 생략한다. */
  args?: Record<string, CommandArgSchema>
  /** 반드시 있어야 하는 인자 이름. */
  required?: string[]
}

export interface Command {
  /** 전역 고유 id(예: 'tab.next'). 소비자는 이 id 로만 실행을 지시한다. */
  id: string
  /** 사람이 읽는 이름(팔레트·메뉴 표기용, 후속 소비자). */
  title: string
  /** 그룹핑용 분류(팔레트 섹션 등). */
  category?: string
  /** 기본 키 조합 힌트(예: 'Ctrl+Shift+T'). 키바인딩 소비자가 참조. */
  keybinding?: string
  /**
   * 밖(LLM·CLI)에서 부를 수 있게 하는 설명. ★생략 = 이 창 안에서만 도는 command★ — 셸이 데몬 명부에
   * 얹는 것은 이 칸을 가진 것뿐이다(`commands/viewCommandBridge.ts` 의 `offeredCommands`).
   */
  help?: CommandHelp
  /**
   * ★있으면 이 command 는 **사람 경로에서만** 돈다 — 값은 그 사유(거절 문구에 그대로 실린다)★.
   *
   * 이 레지스트리의 항목 하나가 **두 소비자를 겸한다**: 사람 클릭·키바인딩(`dispatch.fireAndForget` →
   * [`runAsHuman`])과 LLM 표면(`window.__engramCmd` · 버스 다리 → [`run`]). 대부분은 그것이 장점이지만
   * (§5 「같은 핸들을 흔든다」), **둘 중 한쪽에만 열려야 하는 동작**이 생기면 그 겸직이 곧 구멍이 된다.
   * 그때 닫는 축은 **인자 값이 아니라 호출자**라 `when` 으로는 못 가른다 — `when` 은 양쪽에 똑같이 적용된다.
   *
   * ★쓰기 전에 확인할 것 — 이 칸은 §5 의 예외이고, 예외에는 사유가 필요하다★: 「LLM 이 못 지나는 UI 가
   * 뒤에 있다」는 사유가 **아니다**(그건 고칠 결함이지 닫을 근거가 아니다). 정당한 사유는 「사람이 그
   * 자리에 있어야 한다는 **정책 결정**이 있고, 그 결정의 정본이 딴 데 있다」다. 오늘 유일한 사용처
   * (`agentlist.createCodex`)의 정본 = `engram-dashboard-agent` 의 `commands::LLM_BACKEND_POLICY`.
   *
   * ★이 칸은 `help` 와 다른 축이다★ — `help` 없음 = 데몬 명부에 **광고 안 함**이고, 그것은 실행을 막지
   * 않는다(버스 다리의 `settle` 은 이름만 있으면 `run` 을 부른다). 광고를 빼는 것으로 닫았다고 읽지 말 것.
   */
  humanOnly?: string
  /** 노출/실행 가능 조건(후속 when-context 는 골격 밖, ADR-0055). */
  when?: () => boolean
  /**
   * 기존 store 액션/invoke 로 라우팅한다. 반환은 그대로 흘려보낸다 —
   * 일부는 Promise(예: layout createTab → Promise<id>)라 cdp/호출부가 await 할 수 있어야 한다.
   */
  run: (args?: CommandArgs) => unknown
}

const registry = new Map<string, Command>()

/**
 * 중복 id 는 조용히 덮지 않고 warn 한 뒤 등록한다(dev 안전 — 어댑터 중복 로드·오타를
 * 드러낸다). 마지막 등록이 이긴다(HMR 재평가에서 최신 정의로 갱신되도록).
 */
export function register(cmd: Command): void {
  if (registry.has(cmd.id)) {
    console.warn(`[commands] 중복 id 재등록 — 기존 command 를 덮어씀: '${cmd.id}'`)
  }
  registry.set(cmd.id, cmd)
}

/** 두 진입점의 공통부 — id 해소만 한다(게이트는 호출자별로 다르다). */
function resolve(id: string): Command {
  const cmd = registry.get(id)
  if (!cmd) {
    throw new Error(`[commands] 알 수 없는 command id: '${id}'`)
  }
  return cmd
}

/**
 * ★LLM 표면의 진입점★ — `window.__engramCmd.run` · 버스 다리(`viewCommandBridge`) · cdp eval 이 여기로
 * 온다. 사람 클릭·키바인딩은 [`runAsHuman`] 으로 간다(`dispatch.fireAndForget` 경유).
 *
 * 모르는 id 는 명확히 throw 한다(조용한 no-op 은 LLM/cdp 디버깅을 어렵게 함).
 * handler 반환을 그대로 반환 → 호출부가 Promise 를 await 할 수 있다.
 *
 * ★`humanOnly` 는 여기서만 막힌다★ — 그 칸의 doc 이 언제 쓰는지를 진다. 거절은 조용하지 않다(사유를
 * 실어 throw): LLM 이 「눌렸는데 아무 일도 안 일어났다」로 읽고 같은 자리를 맴도는 것을 막는다.
 */
export function run(id: string, args?: CommandArgs): unknown {
  const cmd = resolve(id)
  if (cmd.humanOnly) {
    throw new Error(
      `[commands] '${id}' 는 이 표면(LLM·버스)으로는 실행하지 않는다 — ${cmd.humanOnly}`,
    )
  }
  return cmd.run(args)
}

/**
 * ★사람 경로의 진입점★ — 클릭·키바인딩만 부른다(`dispatch.fireAndForget` 이 유일한 호출자이고, 그
 * 파일이 복사 템플릿이다). `humanOnly` 게이트를 지나지 않는 것이 이 함수의 존재 이유다.
 *
 * ★이것을 `__engramCmd`(`store/eventBus.ts`)나 버스 다리(`commands/viewCommandBridge.ts`)에 노출하지
 * 말 것★ — 노출하는 순간 [`run`] 의 게이트가 우회로를 얻어 아무것도 안 지킨 셈이 된다. ★그 금지를 재는
 * 게이트는 없다(호출부 규율이다)★ — `commands/registry.test.ts` 가 재는 것은 두 진입점이 실제로 다르게
 * 행동한다는 것까지다.
 */
export function runAsHuman(id: string, args?: CommandArgs): unknown {
  return resolve(id).run(args)
}

/**
 * ★`help` 는 값 사본으로 준다 — 얕은 복사로는 부족하다★.
 *
 * 스냅샷을 돌려준다는 계약(아래 두 함수)이 `help` 가 객체가 된 순간 그 칸에서만 깨졌다:
 * `list().find(...)!.help.summary = 'x'` 가 등록된 command 를 직접 바꾼다(레지스트리 Map 엔트리 우회).
 * 중첩 두 겹(`args` 의 각 칸, `enum` 배열)까지 갈라야 별칭이 안 남는다.
 */
function copyHelp(help: CommandHelp | undefined): CommandHelp | undefined {
  if (!help) return undefined
  const args = help.args
    ? Object.fromEntries(
        Object.entries(help.args).map(([field, schema]) => [
          field,
          { ...schema, ...(schema.enum ? { enum: [...schema.enum] } : {}) },
        ]),
      )
    : undefined
  return {
    ...help,
    ...(args ? { args } : {}),
    ...(help.required ? { required: [...help.required] } : {}),
  }
}

/**
 * 없으면 undefined.
 * ★run() 과 다르다★: run 은 실행(모르는 id throw), get 은 순수 조회(발견/메타). 상태 권위 아님.
 * ★사본을 돌려준다★(FIX-C): 저장된 원본을 그대로 넘기면 호출자가 getCommand(id)!.run = evil 로
 *   레지스트리 Map 엔트리를 갈아끼워 register()/중복경고 불변식을 우회할 수 있다. list() 도 같은 사본을
 *   주므로 일관된다. `help` 만 얕은 복사로 부족해 따로 가른다([`copyHelp`]). 핫패스 없음 → 복사 비용 무시.
 */
export function getCommand(id: string): Command | undefined {
  const cmd = registry.get(id)
  return cmd ? { ...cmd, help: copyHelp(cmd.help) } : undefined
}

/**
 * 등록된 command 의 메타 스냅샷(발견용 — 팔레트·LLM introspection).
 *
 * ★`humanOnly` 도 싣는다★ — 이 목록을 읽는 쪽이 곧 [`run`] 이 막을 쪽이라, 안 실으면 LLM 이 목록에서
 * 본 것을 부르고 나서야 닫힌 것을 안다(그 왕복이 §5 가 없애려던 바로 그 되묻기다).
 */
export function list(): Array<
  Pick<Command, 'id' | 'title' | 'category' | 'keybinding' | 'help' | 'humanOnly'>
> {
  return Array.from(registry.values()).map(
    ({ id, title, category, keybinding, help, humanOnly }) => ({
      id,
      title,
      category,
      keybinding,
      help: copyHelp(help),
      humanOnly,
    }),
  )
}

/** 테스트 전용 — 레지스트리 초기화(테스트 간 격리). 프로덕션 코드에서 호출 금지. */
export function __resetRegistryForTest(): void {
  registry.clear()
}
