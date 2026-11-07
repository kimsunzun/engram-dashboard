// RichSlot backend notation — title · marker · color tone come out of **one decision** (the same grain as statusGlyph.ts: only the pure mapping is
//   kept, and JSX · style values are owned by the drawing side).
//
// ★Why it is needed★: rich (structured) mode drew claude and codex identically, so from the screen alone you could not tell
//   who you were talking to. What is told apart here is not **what to draw (renderer)** but **who you talk to (notation)** —
//   renderer selection is still decided by the single field `capabilities.output.structured` (`api/types.ts` AgentBackendKind comment).
//
// ★No control surface is added★: title · marker · tone are display values derived from the profile (`command.kind`), so there is no setting for a person to choose,
//   no screen state, no separate command — the way to change it is the profile itself and that way already exists
//   (CLAUDE.md 「LLM-우선 제어」 — put on the existing surface). The old basis ADR-0167 decision 1 (display state = 「데이터 + 다시
//   읽기」) (data + re-read) was discarded by ADR-0265. Instead the `brand` token below goes out to the DOM for automation to read.

import type { ComponentType } from 'react'

import type { AgentCommand } from '../../api/types'
import { ClaudeMascot } from './ClaudeMascot'
import { CodexMascot } from './CodexMascot'

/**
 * A stable token that goes out to the DOM observation surface (tests · CDP · LLM selectors). The marker is an svg so it leaves no text, and when a conversation
 * starts the title and marker both fold (ADR-0145 empty state), so this token is the only basis for observation.
 *
 * ★Do not delete or rename an existing token★ (the same discipline as `shape` of statusGlyph) — selectors hang on it so
 *   renaming is silent destruction. **Only growing is safe.**
 * ★It is spelled the same as wire `AgentBackendKind` but is a different thing★ — that one is the backend the spawn packet picks, and this one is
 *   a screen notation token. Do not assign values between them (the decision is made by `command.kind` below).
 */
export type RichBrand = 'claude' | 'codex'

export interface RichBranding {
  brand: RichBrand
  /**
   * Empty-state title. ★Not a translation target★ — it is a product name so it is used as is (the same discipline by which ADR-0145 put "Claude Code" outside
   * i18n).
   */
  title: string
  /** Empty-state marker component. Shape and color are owned by each marker file. */
  Mascot: ComponentType
  /**
   * Panel tone class (defined in `richBranding.css`). `''` = no tone.
   * ★Applied to the root only★ — editing `:root` would tint all three themes · every slot · even the terminal.
   */
  tintClass: string
}

const CLAUDE: RichBranding = {
  brand: 'claude',
  title: 'Claude Code',
  Mascot: ClaudeMascot,
  // No tone = the look as it is until now. Adding a claude tone here changes every existing screen.
  tintClass: '',
}

const CODEX: RichBranding = {
  brand: 'codex',
  // The product name is "Codex" — the reason the surface name ("Codex CLI") is not used is that the same backend comes up in two shapes, an interactive TUI and a resident
  //   JSON server (codex app-server) (`agentClient.createCodexProfile`).
  title: 'Codex',
  Mascot: CodexMascot,
  tintClass: 'rich-tint-codex',
}

/**
 * Kind of the profile's run command → notation.
 *
 * ★If unknown, claude (a regression-free default)★: paths with no profile really exist (ad-hoc sessions · unit test mocks that do not fill the
 * profile — see the `profiles?.` comment in `RichSlot`). Changing that place to a neutral notation like 「알 수 없음」 (unknown) would
 * **change the notation of every panel that looks like claude now** — a screen that changes with nothing learned is
 * worse. So the only thing that splits is the single case **confirmed** as codex.
 */
export function richBranding(kind: AgentCommand['kind'] | null | undefined): RichBranding {
  return kind === 'Codex' ? CODEX : CLAUDE
}
