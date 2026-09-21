// RichSlot 백엔드 표기 — 제목·표식·색조를 **한 판정**에서 낸다(statusGlyph.ts 와 같은 결: 순수 매핑만
//   두고 JSX·스타일값은 그리는 쪽이 소유한다).
//
// ★왜 필요한가★: 리치(구조화) 모드는 claude 와 codex 를 똑같이 그려서, 화면만 보고는 누구와 말하는지 알
//   수 없었다. 여기서 가르는 것은 **무엇을 그리느냐(렌더러)** 가 아니라 **누구와 말하느냐(표기)** 다 —
//   렌더러 선택은 여전히 `capabilities.output.structured` 한 칸이 가른다(`api/types.ts` AgentBackendKind 주석).
//
// ★제어 표면을 늘리지 않는다★: 제목·표식·색조는 프로필에서 파생되는 표시값이라 사람이 고를 설정도,
//   개별 command 도 두지 않는다(ADR-0167 결정 1 — 표시 상태는 「데이터 + 다시 읽기」로 다룬다).
//   대신 아래 `brand` 토큰이 DOM 에 나가 자동화가 읽는다.

import type { ComponentType } from 'react'

import type { AgentCommand } from '../../api/types'
import { ClaudeMascot } from './ClaudeMascot'
import { CodexMascot } from './CodexMascot'

/**
 * DOM 관측 표면(테스트·CDP·LLM 셀렉터)에 나가는 안정 토큰. 표식은 svg 라 글자를 남기지 않고, 대화가
 * 시작되면 제목·표식이 모두 접히므로(ADR-0145 빈 상태) 이 토큰이 유일한 관측 근거다.
 *
 * ★기존 토큰을 지우거나 이름을 바꾸지 말 것★(statusGlyph 의 `shape` 와 같은 규율) — 셀렉터가 걸려 있어
 *   개명은 조용한 파괴다. **늘리는 것만 안전하다.**
 * ★wire `AgentBackendKind` 와 글자가 같지만 다른 것이다★ — 저쪽은 스폰 패킷이 고르는 백엔드고, 이쪽은
 *   화면 표기 토큰이다. 값을 서로 대입하지 말 것(판정은 아래 `command.kind` 가 한다).
 */
export type RichBrand = 'claude' | 'codex'

export interface RichBranding {
  brand: RichBrand
  /**
   * 빈 상태 제목. ★번역 대상이 아니다★ — 제품명이라 그대로 쓴다(ADR-0145 가 "Claude Code" 를 i18n 밖에
   * 둔 그 규율).
   */
  title: string
  /** 빈 상태 표식 컴포넌트. 형태·색은 각 표식 파일이 소유한다. */
  Mascot: ComponentType
  /**
   * 패널 색조 클래스(정의처 = `richBranding.css`). `''` = 색조 없음.
   * ★루트에만 건다★ — `:root` 를 고치면 세 테마·모든 슬롯·터미널까지 함께 물든다.
   */
  tintClass: string
}

const CLAUDE: RichBranding = {
  brand: 'claude',
  title: 'Claude Code',
  Mascot: ClaudeMascot,
  // 색조 없음 = 지금까지의 모습 그대로. 여기에 claude 색조를 더하면 기존 화면이 전부 바뀐다.
  tintClass: '',
}

const CODEX: RichBranding = {
  brand: 'codex',
  // 제품명은 "Codex" 다 — 표면 이름("Codex CLI")을 쓰지 않는 이유는 같은 백엔드가 대화형 TUI 와 상주
  //   JSON 서버(codex app-server) 두 모양으로 뜨기 때문이다(`agentClient.createCodexProfile`).
  title: 'Codex',
  Mascot: CodexMascot,
  tintClass: 'rich-tint-codex',
}

/**
 * 프로필의 실행 명령 종류 → 표기.
 *
 * ★모르면 claude 다(회귀 없는 기본값)★: 프로필이 없는 경로는 실재한다(ad-hoc 세션 · 프로필을 안 채우는
 * 단위테스트 mock — `RichSlot` 의 `profiles?.` 주석). 그 자리를 「알 수 없음」 같은 중립 표기로 바꾸면
 * **지금 claude 로 보이던 모든 패널의 표기가 함께 갈린다** — 알아낸 것이 없는데 화면만 바뀌는 쪽이 더
 * 나쁘다. 그래서 갈라지는 것은 codex 로 **확인된** 경우 하나뿐이다.
 */
export function richBranding(kind: AgentCommand['kind'] | null | undefined): RichBranding {
  return kind === 'Codex' ? CODEX : CLAUDE
}
