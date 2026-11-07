// SlotErrorBoundary — confines a render error of one slot's content to that slot's place.
//
// Without a boundary, React 19 takes down the whole root on one throw during render and the entire window goes blank. This boundary
//   sits *inside* the slot wrapper so the right-click menu and focus stay alive — the user escapes by emptying or closing the slot with that menu.

import { Component, type ErrorInfo, type ReactNode } from 'react'

import { t } from '../../i18n'

interface SlotErrorBoundaryProps {
  slotId: string
  /**
   * When it changes, the fallback is lifted and the children are drawn again (when slot identity · content · render mode changed). ★Do not use key
   * instead★ — with key the slot component remounts on every agent reassignment, whereas today the same instance resubscribes with the new agentId
   * prop. It is solved by value comparison so that behavior with no error is not changed.
   */
  resetKey: string
  children: ReactNode
}

interface SlotErrorBoundaryState {
  failed: boolean
  resetKey: string
}

export class SlotErrorBoundary extends Component<SlotErrorBoundaryProps, SlotErrorBoundaryState> {
  state: SlotErrorBoundaryState = { failed: false, resetKey: this.props.resetKey }

  static getDerivedStateFromProps(
    props: SlotErrorBoundaryProps,
    state: SlotErrorBoundaryState,
  ): Partial<SlotErrorBoundaryState> | null {
    return props.resetKey !== state.resetKey ? { failed: false, resetKey: props.resetKey } : null
  }

  static getDerivedStateFromError(): Partial<SlotErrorBoundaryState> {
    return { failed: true }
  }

  componentDidCatch(error: unknown, info: ErrorInfo): void {
    console.error(`[SlotErrorBoundary] slot ${this.props.slotId} render failed`, error, info.componentStack)
  }

  render(): ReactNode {
    if (!this.state.failed) return this.props.children
    return (
      <div
        // So the observation surface (cdp · tests) can count broken slots by DOM.
        data-slot-error="1"
        role="alert"
        className="flex h-full w-full items-center justify-center p-2 text-center text-xs text-muted"
        style={{ fontFamily: 'var(--font-ui)' }}
      >
        {t('slot.renderFailed')}
      </div>
    )
  }
}
