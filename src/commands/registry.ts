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

/**
 * 모르는 id 는 명확히 throw 한다(조용한 no-op 은 LLM/cdp 디버깅을 어렵게 함).
 * handler 반환을 그대로 반환 → 호출부가 Promise 를 await 할 수 있다.
 */
export function run(id: string, args?: CommandArgs): unknown {
  const cmd = registry.get(id)
  if (!cmd) {
    throw new Error(`[commands] 알 수 없는 command id: '${id}'`)
  }
  return cmd.run(args)
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

/** 등록된 command 의 메타 스냅샷(발견용 — 팔레트·LLM introspection). */
export function list(): Array<Pick<Command, 'id' | 'title' | 'category' | 'keybinding' | 'help'>> {
  return Array.from(registry.values()).map(({ id, title, category, keybinding, help }) => ({
    id,
    title,
    category,
    keybinding,
    help: copyHelp(help),
  }))
}

/** 테스트 전용 — 레지스트리 초기화(테스트 간 격리). 프로덕션 코드에서 호출 금지. */
export function __resetRegistryForTest(): void {
  registry.clear()
}
