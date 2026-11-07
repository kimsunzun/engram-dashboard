# Feedback and rapid repeated presses after pressing interrupt — peer precedents (light)

- 상태: Research complete (light · no adversarial review) · decision = ADR-0244 (2026-09-28 · A adopted) — the choice this research supported was the re-examination of TRD `docs/process/S21-chat-ux/trd.md` §3-2 「낙관 상태를 바꾸지 않는다」 (§11 ⑬).
- Method: 1 collector (reading local clone sources + public docs) · main spot-checked the key three (paseo · vibe-kanban · t3code) against the sources.
- Date: 2026-09-28
- Confidence legend: certain · likely · uncertain

## Questions

1. Right after interrupt is pressed, until the stop is confirmed, is an immediate indicator such as 「중단 중…」 shown?
2. What happens if it is pressed again in between?

## Conclusion (first)

- **4 of the 5 GUI peers change the screen the moment it is pressed.** Three of them (paseo · vibe-kanban · cline) disable the button or ignore a repeated press in the meantime (likely).
- **Terminal UIs (codex TUI · Claude Code) show no immediate indicator.** The codex TUI sends interrupt again on a repeated press (likely).
- **Some peers tie interrupt to a turn number** — t3code attaches the id of the running turn to every interrupt, and the codex app-server protocol also carries `turn_id`. Our claude interrupt line has no turn-id field, so this method is unavailable (TRD §3-4 remaining race).

## Per peer

| Peer | Immediate indicator | On a repeated press | Evidence | Confidence |
|---|---|---|---|---|
| paseo | Yes — the button gets a spinner + a 「canceling」 label. It clears when the interrupt request finishes or another turn opens | Button disabled · the shortcut is ignored too | `packages/app/src/composer/index.tsx:1012-1025` (main cross-check — `disabled={!isConnected \|\| isCancellingAgent}`) · `timeline/turn-liveness.ts:52-115` | Certain |
| vibe-kanban | Yes — a disabled 「Stopping」 button + spinner. But it clears when the request response arrives (not at the turn end) | Ignored (`if (isStopping) return`) | `packages/web-core/src/shared/hooks/useWorkspaceExecution.ts:77-86` (main cross-check) · `packages/ui/src/components/SessionChatBox.tsx:557-563` | Certain |
| cline | Partial — blocks send and the button. No 「취소 중」 text was found | Ignored (in-progress indicator ref) | `webview-ui/src/components/chat/chat-view/hooks/useMessageHandlers.ts:384-404` | Likely |
| orca (chat over a PTY) | Yes — hides 「작업 중」 immediately | Does not block — the second press writes the ESC byte straight to the PTY (the same risk as ours) | `src/renderer/src/components/native-chat/NativeChatView.tsx:343-352` · `native-chat-working-suppression.ts` | Q1 certain · Q2 likely |
| t3code | No stop button in the main chat (main cross-check — no disabled prop). Stopping background work shows 「Stopping...」 | Does not block, but each interrupt carries the id of the running turn, so a late second request does not target the next turn | `apps/web/src/components/chat/ComposerPrimaryActions.tsx:85-104` · `ChatView.logic.ts:546-555` | Certain (how the server handles a stale turn id is unknown) |
| codex TUI | Not found (an absence search, so uncertain) | Sends interrupt again · interrupts 「도는 것」 (whatever is running) with no target. **Ctrl-C**: the first press interrupts and arms a time-limited quit hint, and a second press inside that window **quits** | `codex-rs/tui/src/bottom_pane/mod.rs:1700-1715` · `core/src/session/handlers.rs:436-439` · Ctrl-C = `codex-rs/tui/src/chatwidget/interaction.rs:99-114` · `:204-217` · `:503-510` (openai/codex master · at the time of research · commit not recorded) | Likely |
| Claude Code CLI | Not in the docs — 「Esc 로 멈추고, 대기 메시지가 있으면 이어 보낸다」 (back-translated: Esc stops, and if there are queued messages it sends them on) | Not in the docs (pressing Esc twice is a different feature — rewind) | https://code.claude.com/docs/en/interactive-mode | The docs' description is certain · the actual screen is unknown |

## Gaps

- The Zed agent panel was not looked at (light budget).
- Claude Code · the codex TUI were not run.
- How the paseo · cline servers handle a cancel that arrives after a new turn has opened was not looked at.
- Our key open question — whether claude ignores a second interrupt received while 「멈추는 중」 (stopping), or applies it to the next queued message — was not tested (outside the B2 spike's scope).
