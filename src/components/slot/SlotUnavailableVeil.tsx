// SlotUnavailableVeil — the curtain the three slots (terminal · DOM · rich) and the leaf (LayoutLeaf — a slot with no memory: one where the leaf does not hold that agent's mount memory, never mounted or dropped on reassignment · TRD S21-storage §6-8) draw **as one set** for "the targeted agent is not there now". The leaf draws only when it does not put up the slot component, so the four never overlap in one area.
// ADR-0280
// ★Content is not erased (ADR-0148 decisions 2·3)★: conversation or terminal screen is left as it was at that moment, covered with a translucent curtain, and only a single symbol is laid at the center — emptying the screen loses "what to read on resuming when it returns". The upper-level decision splitting absence into three branches is ADR-0149.
// ★No text is put in★: the old terminal·DOM slots wrote waiting/exit/`Failed: <message>` as text, so the same fact was told in a different sentence per slot, and in spans where the decision wavers like a daemon restart an assertion like "ended" stays. A single symbol makes no such assertion. If a need for the failure reason shows up, revive it then — no place made in advance.
// ★Why a single component★: if each drawing side drew on its own, "the same look for the same condition" would survive only as a convention. The condition (absent decision) is computed by the drawing side from its own data (the leaf from the registry·profile list and memory presence); this one place draws the result.

import { PowerOff } from 'lucide-react'

import type { ViewPhase } from '../../api/agentClient'

interface SlotUnavailableVeilProps {
  /**
   * The phase the subscription last announced (omitted if none yet). It does not change the look; it is used **only on the observation surface (cdp · tests) to tell the basis of absence apart** — waiting (`detached`) and re-request exhaustion (`error`) are the same picture to the user but different facts to automation.
   */
  phase?: ViewPhase | null
}

export function SlotUnavailableVeil({ phase }: SlotUnavailableVeilProps) {
  return (
    <div
      data-slot-dead="1"
      data-slot-phase={phase ?? undefined}
      // Keeps the old observation surface — only "waiting for an agent to attach" is true (`error` is not).
      data-slot-detached={phase === 'detached' ? '1' : undefined}
      aria-hidden
      // pointer-events-none lets operation through (ADR-0226); input to a dead agent is blocked by each slot's guard, not the curtain (ADR-0165).
      className="pointer-events-none absolute inset-0 z-20 flex items-center justify-center bg-background/70"
    >
      <PowerOff className="size-10 text-muted" />
    </div>
  )
}
