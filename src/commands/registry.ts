// ADR-0055: 프론트 command 레지스트리 — 모든 동작을 id 로 등록된 command 로 발견/라우팅/메타만 담는
//   얇은 층이다. ★상태 권위가 아니다★ — 실행은 handler 가 기존 진입점(store 액션 / 기존 invoke)으로
//   라우팅한다(새 싱글톤·새 arg 파싱 0, ADR-0035 레이아웃 권위 유지). 소비자(사람 클릭·전역 keydown·
//   window.__engramCmd)는 전부 run(id, args) 하나로 통일한다(§5).
//
// ★DOM-free 유지★: 이 파일은 순수 Map 이라 headless(vitest) 로 단위테스트된다. DOM/Tauri 의존은
//   어댑터(*Commands.ts)·키바인딩 리스너로 밀어낸다.

/** command 인자 = 객체 하나(가방). 각 handler 가 필요한 키만 destructure(가변인자 금지, ADR-0055). */
export type CommandArgs = Record<string, unknown>

export interface Command {
  /** 전역 고유 id(예: 'theme.set'). 소비자는 이 id 로만 실행을 지시한다. */
  id: string
  /** 사람이 읽는 이름(팔레트·메뉴 표기용, 후속 소비자). */
  title: string
  /** 그룹핑용 분류(옵션 — 팔레트 섹션 등). */
  category?: string
  /** 기본 키 조합 힌트(예: 'Ctrl+Shift+T'). 키바인딩 소비자가 참조(옵션). */
  keybinding?: string
  /** 노출/실행 가능 조건(옵션 — 후속 when-context 는 골격 밖, ADR-0055). */
  when?: () => boolean
  /**
   * 실행 handler. 기존 store 액션/invoke 로 라우팅한다. 반환은 그대로 흘려보낸다 —
   * 일부는 Promise(예: layout createView → Promise<id>)라 cdp/호출부가 await 할 수 있어야 한다.
   */
  run: (args?: CommandArgs) => unknown
}

// 모듈 레벨 단일 Map. 어댑터가 import 부수효과로 register 한다.
const registry = new Map<string, Command>()

/**
 * command 등록. 중복 id 는 조용히 덮지 않고 warn 한 뒤 등록한다(dev 안전 — 어댑터 중복 로드·오타를
 * 드러낸다). 마지막 등록이 이긴다(HMR 재평가에서 최신 정의로 갱신되도록).
 */
export function register(cmd: Command): void {
  if (registry.has(cmd.id)) {
    console.warn(`[commands] 중복 id 재등록 — 기존 command 를 덮어씀: '${cmd.id}'`)
  }
  registry.set(cmd.id, cmd)
}

/**
 * command 실행. 모르는 id 는 명확히 throw 한다(조용한 no-op 은 LLM/cdp 디버깅을 어렵게 함).
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
 * 등록된 command 객체 조회(없으면 undefined). 소비자(키바인딩)가 when/keybinding 등 메타를 읽을 때 쓴다.
 * ★run() 과 다르다★: run 은 실행(모르는 id throw), get 은 순수 조회(발견/메타). 상태 권위 아님.
 * ★얕은 복사본을 돌려준다★(FIX-C): 저장된 원본을 그대로 넘기면 호출자가 getCommand(id)!.run = evil 로
 *   레지스트리 Map 엔트리를 갈아끼워 register()/중복경고 불변식을 우회할 수 있다. list() 도 이미 사본을
 *   주므로 일관되게 사본을 반환한다(소비자는 when 만 읽어 얕은 복사로 충분). 핫패스 없음 → 복사 비용 무시.
 */
export function getCommand(id: string): Command | undefined {
  const cmd = registry.get(id)
  return cmd ? { ...cmd } : undefined
}

/** 등록된 command 의 메타 스냅샷(발견용 — 팔레트·LLM introspection). run 함수는 제외한다. */
export function list(): Array<Pick<Command, 'id' | 'title' | 'category' | 'keybinding'>> {
  return Array.from(registry.values()).map(({ id, title, category, keybinding }) => ({
    id,
    title,
    category,
    keybinding,
  }))
}

/** 테스트 전용 — 레지스트리 초기화(테스트 간 격리). 프로덕션 코드에서 호출 금지. */
export function __resetRegistryForTest(): void {
  registry.clear()
}
