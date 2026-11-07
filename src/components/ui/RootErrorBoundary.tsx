// RootErrorBoundary — catches render errors outside slots (tab bar · layout branch · split component · page) so they do not erase the whole window.
//
// Without a boundary, React 19 takes down the whole root on a single throw during render and the window stays blank until restart. Errors inside a slot
//   are contained first by SlotErrorBoundary, so only errors outside that reach here.
// ★The fallback display does not lean on the router · store · theme store★ — we do not know which of them was the cause. For color it uses only the CSS variables main.tsx
//   plants on :root before the first paint.
// ★The two buttons have no LLM path★ — a 「LLM-우선 제어」 gap (user decision 2026-09-24: record only). In a release build the LLM has
//   no means to notice this state either, so adding `window.reload` to the existing command list alone does not close it.

import { Component, type ErrorInfo, type ReactNode } from 'react'

import { t } from '../../i18n'

interface RootErrorBoundaryProps {
  children: ReactNode
}

interface RootErrorBoundaryState {
  failed: boolean
}

export class RootErrorBoundary extends Component<RootErrorBoundaryProps, RootErrorBoundaryState> {
  state: RootErrorBoundaryState = { failed: false }

  static getDerivedStateFromError(): Partial<RootErrorBoundaryState> {
    return { failed: true }
  }

  componentDidCatch(error: unknown, info: ErrorInfo): void {
    console.error(
      `[RootErrorBoundary] render failed (route ${window.location.hash || '#/'})`,
      error,
      info.componentStack,
    )
  }

  private redraw = (): void => {
    this.setState({ failed: false })
  }

  private reload = (): void => {
    window.location.reload()
  }

  render(): ReactNode {
    if (!this.state.failed) return this.props.children
    return (
      <div
        data-root-error="1"
        role="alert"
        className="flex h-screen w-screen flex-col items-center justify-center gap-3 bg-background p-4 text-center text-sm text-foreground"
        style={{ fontFamily: 'var(--font-ui)' }}
      >
        <div>{t('window.renderFailed')}</div>
        <div className="flex gap-2">
          <button
            type="button"
            onClick={this.redraw}
            className="rounded border border-border bg-surface px-3 py-1 hover:bg-elevated"
          >
            {t('window.redraw')}
          </button>
          <button
            type="button"
            onClick={this.reload}
            className="rounded border border-border bg-surface px-3 py-1 hover:bg-elevated"
          >
            {t('window.reload')}
          </button>
        </div>
      </div>
    )
  }
}
