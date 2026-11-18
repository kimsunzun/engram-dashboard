// RichSlot backend notation — title · marker · color tone come out of **one decision** (same grain as statusGlyph.ts: only the pure mapping; JSX · style values belong to the drawing side).
// ★Why★: rich mode drew claude and codex identically, so the screen did not say who you talk to. This tells **who you talk to (notation)**, not **what to draw (renderer)** — renderer selection stays the single field `capabilities.output.structured` (`api/types.ts` AgentBackendKind).
// ★No control surface added★: title · marker · tone are display values derived from the profile (`command.kind`) — no setting, no screen state, no command; the way to change it is the profile itself (CLAUDE.md 「LLM-우선 제어」 — put on the existing surface; hangul-ok). ADR-0265 replaced ADR-0167 decision 1 (display state = "data + re-read"); the `brand` token below goes out to the DOM for automation to read.

import type { ComponentType } from 'react'

import type { AgentCommand } from '../../api/types'
import { ClaudeMascot } from './ClaudeMascot'
import { CodexMascot } from './CodexMascot'

/**
 * Stable token on the DOM observation surface (tests · CDP · LLM selectors). The marker is an svg (no text) and title and marker both fold once a conversation starts (ADR-0145 empty state), so this token is the only basis for observation.
 * ★Never delete or rename an existing token★ (same discipline as `shape` of statusGlyph) — selectors hang on it; **only growing is safe.**
 * ★Spelled like wire `AgentBackendKind` but a different thing★ — that is the backend the spawn packet picks, this is a screen notation token; do not assign between them (decided by `command.kind` below).
 */
export type RichBrand = 'claude' | 'codex'

export interface RichBranding {
  brand: RichBrand
  /**
   * Empty-state title. ★Not a translation target★ — a product name used as is (ADR-0145 kept "Claude Code" outside i18n).
   */
  title: string
  /** Empty-state marker component. Shape and color are owned by each marker file. */
  Mascot: ComponentType
  /**
   * Panel tone class (`richBranding.css`); `''` = no tone. ★Applied to the root only★ — editing `:root` would tint all three themes · every slot · even the terminal.
   */
  tintClass: string
}

const CLAUDE: RichBranding = {
  brand: 'claude',
  title: 'Claude Code',
  Mascot: ClaudeMascot,
  // No tone = the look as before; adding a claude tone here changes every existing screen.
  tintClass: '',
}

const CODEX: RichBranding = {
  brand: 'codex',
  // The product name is "Codex", not the surface name "Codex CLI": the same backend comes up as an interactive TUI and a resident JSON server (codex app-server) (`agentClient.createCodexProfile`).
  title: 'Codex',
  Mascot: CodexMascot,
  tintClass: 'rich-tint-codex',
}

/**
 * Run command kind of the profile → notation.
 * ★Unknown → claude (regression-free default)★: paths with no profile really exist (ad-hoc sessions · unit mocks — see the `profiles?.` comment in `RichSlot`); a neutral notation would **change every panel that looks like claude now**, a screen changing with nothing learned. Only the single **confirmed** codex case splits.
 */
export function richBranding(kind: AgentCommand['kind'] | null | undefined): RichBranding {
  return kind === 'Codex' ? CODEX : CLAUDE
}
