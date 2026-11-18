// ADR-0227: the leaf that draws one slot — the flat renderer (ViewLayoutRenderer) draws leaves side by side under a single root keyed by slot id.
//   Split, close and promote do not change the parent or key, so the leaf and its per-slot memory and terminal survive.
//   A popout has a new slot id and a new webview, so it mounts fresh.

import { memo, useCallback, useEffect, useRef, useState, type CSSProperties } from 'react'
import { FileQuestionMark, Plus } from 'lucide-react'

import type { LayoutNode, SlotRect } from '../../api/layoutTypes'
import { useCurrentViewId, useViewStore } from '../../store/viewStore'
import { useAgentStore } from '../../store/agentStore'
import TerminalSlot from '../slot/TerminalSlot'
import RichSlot from '../slot/RichSlot'
import DomSlot from '../slot/DomSlot'
import PresetPalette from '../slot/PresetPalette'
import UsageSlot from '../slot/UsageSlot'
import AgentList from '../agent/AgentList'
import { isContentSlot } from '../agent/selectOpenTarget'
import { agentPresence } from '../agent/mergeTreeNodes'
import SlotContextMenu from '../slot/SlotContextMenu'
import { SlotErrorBoundary } from '../slot/SlotErrorBoundary'
import { SlotUnavailableVeil } from '../slot/SlotUnavailableVeil'
import { buildSlotMenu } from '../../commands/slotMenu'
import { defaultRenderMode, type RenderMode } from '../slot/renderMode'
import { t } from '../../i18n'
import { useLanguage } from '../../i18n/react'
import { reportUiMetrics, SLOT_FRAME_BORDER } from './uiMetricsReport'

type SlotNode = Extract<LayoutNode, { type: 'slot' }>
type MenuAnchor = { x: number; y: number }
/** Display of an agent slot — the verdict is the ADR-0149 block in SlotBody. `view` = the concrete renderer (a view that is live or kept alive by memory). */
type AgentSlotState = 'view' | 'connecting' | 'stopped' | 'noTarget'

// memo: during a splitter preview a leaf whose rectangle did not change does not redraw its frame (the remap `remapRects` returns that rectangle object unchanged); a leaf with a changed rectangle redraws only the frame (SlotBody). The store subscription updates the leaf regardless of memo.
export default memo(LayoutLeaf)

function LayoutLeaf({
  node,
  rect,
  focusedSlotId,
  isForeign,
  viewIdOverride,
}: {
  node: SlotNode
  /** This area's rectangle computed by the shell (normalized to the view) — a preview remap may have been applied. */
  rect: SlotRect
  focusedSlotId: string | null
  /** This slot is in the snapshot's `foreign_slots` — it holds content this version does not know and was loaded as `empty`, but it is not empty. */
  isForeign: boolean
  viewIdOverride?: string | null
}) {
  // ADR-0373: memo boundary over `t()` text — re-render on a language change.
  useLanguage()
  // ★Right-click slot menu state (§5)★: a leaf exists once per slot, so this is that slot's menu coordinates; when open, SlotContextMenu is mounted directly inside this leaf.
  const [contextMenu, setContextMenu] = useState<MenuAnchor | null>(null)
  // The reference must be stable for SlotBody's memo to hold.
  const closeMenu = useCallback(() => setContextMenu(null), [])
  // ★The View coordinates this menu operates on (ADR-0064)★: lifted here (where ctx is assembled) from the old SlotContextMenu internal fallback.
  const currentViewId = useCurrentViewId()
  const targetViewId = viewIdOverride ?? currentViewId

  // ★Split frame and border (ADR-0227)★: outer frame = the shell rectangle as is + overflow:hidden, no border; position/size rules are computed by `.engram-slot-frame` in index.css from these four custom properties (including rounding).
  //   A border on the frame would exceed the allotted width in a 0–1px area and overlap its neighbour — inner elements are clipped by the frame.
  //   ★Do not apply transform or contain★ — SlotContextMenu is position:fixed, so an ancestor's transform breaks its coordinate system.
  const frameStyle: CSSProperties & Record<`--${string}`, number> = {
    '--x0': rect.x0,
    '--y0': rect.y0,
    '--x1': rect.x1,
    '--y1': rect.y1,
    position: 'absolute',
    overflow: 'hidden',
  }
  return (
    <div
      className="engram-slot-frame"
      style={frameStyle}
      // Data attributes to identify the slot (a cdp eval counts slots from the DOM). Placed on the frame with the events: cdp recipes and tests fire events at [data-slot-id], and inner clicks bubble here.
      data-slot-id={node.id}
      // ADR-0066: click-to-focus — clicking the slot pane focuses this slot. viewStore.focusSlot → invoke(focus_slot) → emit(layout:updated), a single control surface (human click = LLM = slot.focus, §5).
      //   ★No optimistic update★: the ring (isFocused) is updated only by the backend emit snapshot (authority = src-tauri, ADR-0035).
      //   ★Bubbling allowed (no stopPropagation/preventDefault)★: do not intercept inner interactions (terminal focus, AgentList buttons, etc.).
      //   ★Control-slot focus excluded — allowlist (ADR-0066 refinement)★: focusSlot is called only for content slots (empty/agent); the tree (agent_list), palette (preset_palette) and later control variants are non-focusable.
      //   The gate is the single classifier isContentSlot (allowlist) shared with selectOpenTarget, so the "open" target choice and this gate cannot diverge.
      //   Without it a left-click on a tree node bubbles to the tree slot pane and focuses the tree slot, and the right-click "open" would overwrite the tree with an agent terminal.
      //   If targetViewId is not yet known (tab state not arrived right after boot) it is a no-op (prevents focus leaking to a wrong view).
      // ADR-0144: a left-click on an empty slot does not open the menu (focus only); the menu is right-click only (onContextMenu below).
      //   It reverted the ADR-0141/0143 left-click opener (a menu on every click was reported as a nuisance) — do not bring it back.
      onClick={() => {
        if (!isContentSlot(node.content)) return
        if (targetViewId) void useViewStore.getState().focusSlot(targetViewId, node.id)
      }}
      // ADR-0064: menu items run only by command id — the menu does not call the store directly, so a human right-click and the LLM share one entry point (§5).
      onContextMenu={e => {
        e.preventDefault()
        setContextMenu({ x: e.clientX, y: e.clientY })
      }}
    >
      <SlotBody
        node={node}
        isFocused={node.id === focusedSlotId}
        isForeign={isForeign}
        targetViewId={targetViewId}
        contextMenu={contextMenu}
        onCloseMenu={closeMenu}
      />
    </div>
  )
}

// ★Does not take the rectangle★: while dragging the splitter only the frame's four custom properties change per frame; taking the rectangle would redraw the slot renderer on every change (terminal, markdown re-parse).
const SlotBody = memo(function SlotBody({
  node,
  isFocused,
  isForeign,
  targetViewId,
  contextMenu,
  onCloseMenu,
}: {
  node: SlotNode
  isFocused: boolean
  isForeign: boolean
  targetViewId: string | null
  contextMenu: MenuAnchor | null
  onCloseMenu: () => void
}) {
  // ADR-0373: memo boundary over `t()` text, the slot menu titles included — re-render on a language change.
  useLanguage()
  const renderModeOverride = useViewStore(s => s.renderModeOverride)
  // ★M2 caps branch (ADR-0044)★: the renderer of an agent-assigned slot is chosen by that agent's output caps (already in AgentInfo in the store — a lookup only).
  const agents = useAgentStore(s => s.agents)
  // ADR-0148: an agent reaped from the roster on exit must be told apart from "roster not received yet" — these are the signals.
  const agentsLoaded = useAgentStore(s => s.agentsLoaded)
  // profiles is always an array in the real store, but some unit-test mocks do not fill it → defensive default (same as RichSlot).
  const profiles = useAgentStore(s => s.profiles) ?? []
  const profilesLoaded = useAgentStore(s => s.profilesLoaded) ?? false
  // ★The render target this slot last mounted★ (ADR-0148): after the agent disappears from the roster the same component must keep rendering so the screen stays, but the mode is derived from caps (AgentInfo) and cannot be recomputed then.
  //   A leaf is keyed by slot id, one per slot, so this ref is that slot's memory.
  //   ★The daemon discards the replay ring together with the session on exit★ — once the view is torn down that conversation cannot be revived even by resubscribing; this memory is data preservation, not convenience.
  //   ★Keyed by agentId and dropped on mismatch★ (effect below): merely ignored and left, re-assigning that agent would revive it — remounting an already-unmounted empty view dimmed and subscribing with a reaped agent
  //   (a stopped slot without memory brings up no slot component and draws only the veil — verdict below).
  //   ★It holds only the render mode (ADR-0149 decision 5)★ — no epoch, neither here nor in slot props: the incarnation was removed from the slot's resubscribe trigger (clearing is the subscription's onReset alone).
  //   ★Known limitation★: in the absence span there is no mode derivation, so setRenderMode/clearRenderMode are silently ineffective (the call looks successful).
  const lastMountRef = useRef<{ agentId: string; mode: RenderMode } | null>(null)

  // The slot frame base metrics (TRD §2d) are measured from this leaf's border element — shell area content = frame − this border width. Sent once per webview.
  const borderRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (borderRef.current !== null) void reportUiMetrics(borderRef.current)
  }, [])

  // ADR-0060: slot occupant = SlotContent tagged union.
  const slotAgentId = node.content.type === 'agent' ? node.content.agent_id : null
  const agent = slotAgentId != null ? (agents.find(a => a.id === slotAgentId) ?? null) : null
  const mode = agent != null ? (renderModeOverride[node.id] ?? defaultRenderMode(agent)) : null

  // ★Write the memory only after commit (effect)★: writing during render lets a **discarded concurrent render** leave an uncommitted mode, and if the agent later disappears the child remounts with that polluted identity, erasing the committed conversation.
  // ★A mismatched memory is dropped here (G2)★: cleared the moment the assignment is no longer this memory's agent — otherwise re-assigning the same agent revives an already-unmounted empty view dimmed.
  // deps are primitives only — the agent object's identity changes on every store update (the written values are the same).
  useEffect(() => {
    if (agent != null && mode != null) {
      lastMountRef.current = { agentId: agent.id, mode }
      return
    }
    if (lastMountRef.current != null && lastMountRef.current.agentId !== slotAgentId) {
      lastMountRef.current = null
    }
  }, [agent?.id, mode, slotAgentId]) // eslint-disable-line react-hooks/exhaustive-deps

  // ★Mount a concrete renderer only after caps arrive (ADR-0041 replay ownership)★: the daemon replay fires exactly once from the slot-assign delta (keyed (window,agent)) and not again on a component swap (TerminalSlot→RichSlot).
  //   Showing TerminalSlot before caps and swapping to RichSlot after would mount RichSlot empty and lose the pre-swap bytes forever. So before caps (=AgentInfo) only a neutral placeholder shows (connecting / absence veil / no target below)
  //   and the first concrete renderer mounts after caps are fixed, receiving the assign-time replay intact. (A terminal agent usually has AgentInfo before assign, so this is a transient edge state.)
  // Branch: structured output (NDJSON) = live RichSlot, otherwise TerminalSlot (xterm) (ADR-0002/0044).
  const capsReady = slotAgentId != null && agent != null
  // ★The memory is valid only for the same agent★ — void when the assignment changes (an old mode must not draw a new agent); the actual discard is the effect above (this verdict alone would let it revive on re-assignment).
  const kept =
    lastMountRef.current != null && lastMountRef.current.agentId === slotAgentId
      ? lastMountRef.current
      : null
  const renderAs = mode ?? kept?.mode
  const presence = slotAgentId != null ? agentPresence(slotAgentId, agents, profiles) : 'unknown'
  // ★ADR-0149 state verdict — the axis for keeping the view is "has memory"; the display is the table in TRD S21-storage §6-8★
  //   agent present                → as before (caps decide the renderer)
  //   memory + absent              → keep the view (= keepDeadView). Dimming, symbol and input block are the slot component's job.
  //   roster/profile list not received → 「connecting」 — ★only in this case★. Once the list arrives it must move to one of the two below.
  //   profile present + no memory  → absence veil over an empty slot (`SlotUnavailableVeil` — dim + symbol, no text or button); once activated in the tree and listed in the roster it turns into the usual screen.
  //                                  ★Only this no-memory case overturns ADR-0149's reason for rejecting 「unify delete into dimming too」 (「an empty-screen dim is misread」)★ — delete (「no target」) keeps its text.
  //                                  The leaf draws the veil only when it does not bring up the slot component, so there is neither covered content nor duplicated per-slot verdicts (ADR-0149's rejection of a layout-wrapper veil still stands).
  //                                  ADR-0149 (A)'s 「spawn pending is also connecting」 is replaced: right after activation the veil shows briefly before the agent reaches the roster (accepted).
  //   no profile                   → 「no target」. The assignment is left and the menu stays the agent slot menu (「clear」 and 「agent monitoring」 put other content).
  // ★Trust "no profile" only after the list has been received★: refreshProfiles is a one-shot pull with no retry, so failure or delay is possible and presence shows 'unknown'; tearing the view down then while memory exists
  //   would permanently lose the conversation we meant to preserve (the daemon ring is already gone).
  const keepDeadView = agent == null && kept != null && (presence === 'reserved' || !profilesLoaded)
  // ADR-0149
  // ADR-0280
  const agentSlot: AgentSlotState | null =
    slotAgentId == null
      ? null
      : capsReady || keepDeadView
        ? 'view'
        : !agentsLoaded || !profilesLoaded
          ? 'connecting'
          : presence === 'reserved'
            ? 'stopped'
            : 'noTarget'
  // The preset_palette, agent_list and usage variants are real renderers that fill the slot 100%, so hasContent=true (the centering placeholder style must not break them, ADR-0060/0061/0062).
  const isPresetPalette = node.content.type === 'preset_palette'
  const isAgentList = node.content.type === 'agent_list'
  const isUsage = node.content.type === 'usage'
  const hasContent = agentSlot === 'view' || isPresetPalette || isAgentList || isUsage
  return (
    <div
      ref={borderRef}
      data-slot-border=""
      style={{
        position: 'absolute',
        inset: 0,
        background: 'var(--bg)',
        // Border width always fixed at 1px to remove layout shift on focus move.
        ...SLOT_FRAME_BORDER,
        // ★The focus ring is drawn by the absolute overlay below, not here★: an inset box-shadow on this element was covered by children filling 100% of an overflow:hidden slot (terminal canvas, RichSlot) — only the empty slot showed it. This element is absolute, so it also anchors the overlay.
        boxSizing: 'border-box',
        // With content (terminal/rich): remove margin and alignment so the slot is filled 100% (center alignment would break it).
        // Empty slot: keep the flex that centers the placeholder.
        ...(hasContent
          ? { overflow: 'hidden' }
          : {
              display: 'flex',
              flexDirection: 'column',
              alignItems: 'center',
              justifyContent: 'center',
              color: 'var(--text-muted)',
              fontFamily: 'var(--font-ui)',
              fontSize: '12px',
              gap: '4px',
            }),
      }}
    >
      <SlotErrorBoundary
        slotId={node.id}
        resetKey={`${node.id}:${node.content.type}:${slotAgentId ?? ''}:${renderAs ?? ''}`}
      >
        {node.content.type === 'agent' ? (
          agentSlot === 'connecting' ? (
            <span>{t('agent.connecting')}</span>
          ) : agentSlot === 'stopped' ? (
            // Do not bring up the slot component — no mount memory of that agent (never mounted, or dropped on re-assignment), so nothing to subscribe to or keep. Restart/restore, popout/slot-id remount and A→B→A re-assignment all come here.
            <SlotUnavailableVeil />
          ) : agentSlot === 'noTarget' ? (
            // No profile either (deleted in the tree) — an invalid assignment. ★The conversation being preserved disappearing here is intended★: profile deletion is the user's explicit cleanup, and ADR-0149 fixed the display for that case as text.
            <span>{t('agent.noTarget')}</span>
          ) : (
            (() => {
              // ★viewId = node.id (slot id, ADR-0046)★: the slot subscribes by its own slot id — two slots with the same agentId progress independently (bug B). The key is the slot id too (an agent_id key made two slots of one agent share a React key and tangled remounts).
              // ★Do not pass the epoch down here★: the incarnation was removed from the slot's resubscribe trigger; a value passed here would itself be a remount trigger, so the moment the roster reaps on exit it would drop and erase the preserved
              //   conversation **before the replay arrives** (the daemon ring is already gone). Incarnation rotation is judged by the subscription against the authoritative roster and emitted as a clear signal (protocolClient.observeRoster).
              switch (renderAs) {
                case 'dom':
                  // ★DOM mode (§5 observation)★: draws the same output stream as a plain <pre> so CDP eval/innerText can read it (xterm is a canvas).
                  return <DomSlot key={node.id} viewId={node.id} agentId={slotAgentId!} />
                case 'rich':
                  return <RichSlot key={node.id} viewId={node.id} agentId={slotAgentId!} />
                case 'terminal':
                default:
                  return <TerminalSlot key={node.id} viewId={node.id} agentId={slotAgentId!} />
              }
            })()
          )
        ) : node.content.type === 'preset_palette' ? (
          // List/add/delete flow inside PresetPalette through agentClient (single control surface).
          <PresetPalette />
        ) : node.content.type === 'agent_list' ? (
          // Actions flow inside AgentList through agentClient/viewStore (single control surface) (§5).
          <AgentList />
        ) : node.content.type === 'usage' ? (
          // The value is read only from the usageStore mirror — the shell computes query demand from this slot's content (show_*) (TRD §1-7).
          <UsageSlot content={node.content} viewId={targetViewId} slotId={node.id} />
        ) : isForeign ? (
          // TRD S21-storage §6-2: only the shell holds the original — the menu is the empty-slot menu as is, and placing content there erases the original.
          // ADR-0280
          // ★One icon with no text, no veil (ADR-0280 decision 7)★: does not distinguish 「saved by a newer version」 and shows no notice line — like the absence veil's symbol, but this is not a dead agent so no dimming.
          //   The meaning is kept in the name (`aria-label`), the hover tooltip (`title`) and the automation marker (`data-slot-foreign`).
          // Does not cut pointer-events (unlike the `+` below) — the `title` tooltip must receive hover. Click and right-click bubble to the frame; a `role="img"` with no handler or tabIndex is not an interactive role ADR-0143 blocks.
          <span
            data-slot-foreign=""
            role="img"
            aria-label={t('slot.foreignContent')}
            title={t('slot.foreignContent')}
            className="flex"
          >
            <FileQuestionMark className="size-10 text-muted" />
          </span>
        ) : (
          // ★Pure picture (ADR-0143)★: the target is the slot container. A handler, tabIndex or role on the icon would overlap the container's left-click so the menu opens twice, and revive a menu path that cannot be left by keyboard.
          //   pointer-events is cut so a click on the icon also reaches the slot — inline rather than a utility class because this cut is a behavior contract (a class rule cannot override it, and it is verified as a computed value even without Tailwind).
          <Plus className="size-11 text-muted" style={{ pointerEvents: 'none', opacity: 0.5 }} />
        )}
      </SlotErrorBoundary>
      {contextMenu && (
        // ADR-0064: unified slot menu — buildSlotMenu(content.type) deterministically sorts and resolves command references (content-specific ∪ common '*') into items and passes ctx (viewId/slotId/agentId/content);
        //   command.run flows through those to the backend-authoritative path (viewStore/agentClient) (§5). The content kind is the visibility gate.
        <SlotMenu
          anchor={contextMenu}
          node={node}
          targetViewId={targetViewId}
          slotAgentId={slotAgentId}
          onClose={onCloseMenu}
        />
      )}
      {isFocused && (
        // ★Strength 40%★: lowered from 65% because the frame was judged noisy (compared by eye on the actual screen). It equals the 40% ADR-0066 rejected as "too weak to see", but that rejection was made without comparing the real thing, so this measured decision takes precedence. To revert, raise only this line.
        //   All three themes adapt via color-mix. Control slots (tree/preset) are excluded from focusSlot (isContentSlot gate), so isFocused=false → no ring.
        // ADR-0280
        // ★The ring color must not vary with slot state★ — so the ring stands above the absence veil (z-20, whether the leaf or the slot component draws it) and the veil does not dim it. Two premises: no ancestor of the veil under this element has a z-index above 20,
        //   and the ring is this element's last child — then z is equal and the ring, later in tree order, is on top. Below the connection notice (z 40) and menus (1000 and up).
        // ★Remaining difference (accepted — reopen if it looks awkward)★: the ring is translucent (accent 40% mix) so the visible color still follows the background — a live terminal has a fixed dark background, so in light and e-ink themes its ring and a veiled slot's ring look different.
        // pointer-events is cut so clicks and menus still reach the slot.
        <div
          data-slot-focus-ring=""
          style={{
            position: 'absolute',
            inset: 0,
            pointerEvents: 'none',
            boxShadow: 'inset 0 0 0 1px color-mix(in srgb, var(--accent) 40%, transparent)',
            zIndex: 20,
          }}
        />
      )}
    </div>
  )
})

/**
 * The open slot menu — assembles the menu ctx. ★If the menu gains store state to show, subscribe here and carry it in ctx (ADR-0252)★ — menu contributions read only ctx (ADR-0064), and subscribing here makes the open menu redraw with that state, the subscription living only while the menu is open. No store state is carried now.
 */
function SlotMenu({
  anchor,
  node,
  targetViewId,
  slotAgentId,
  onClose,
}: {
  anchor: MenuAnchor
  node: SlotNode
  targetViewId: string | null
  slotAgentId: string | null
  onClose: () => void
}) {
  const content = node.content
  return (
    <SlotContextMenu
      x={anchor.x}
      y={anchor.y}
      items={buildSlotMenu(content.type)}
      ctx={{ viewId: targetViewId, slotId: node.id, agentId: slotAgentId, content }}
      onClose={onClose}
    />
  )
}
