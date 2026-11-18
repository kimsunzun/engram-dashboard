// ★FIX-1 regression safety net★: the height cap of the expanded reasoning box (max-h-200px) must sit on the Radix Viewport (= the real scroll node)
//   (not the Root). If it sits only on the Root, the Viewport (height:100%) resolves to auto against an indefinite parent, grows to the content
//   height, and the Root overflow-hidden cuts it so scrolling cannot reach it (the original bug). Real scroll behavior (reaching the
//   last line by wheel/drag) depends on the layout engine so jsdom cannot — cdp measurement (manual check). Here we assert only which DOM
//   node (Viewport vs Root) carries max-h.

import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'

import { ThoughtRow } from './ThoughtRow'

// jsdom does not provide ResizeObserver — the Radix ScrollArea internals refer to it, so a no-op stub.
globalThis.ResizeObserver ||= class {
  observe() {}
  unobserve() {}
  disconnect() {}
} as unknown as typeof ResizeObserver

afterEach(() => cleanup())

describe('ThoughtRow', () => {
  it('with content it is interactive — clicking to expand shows the reasoning text', () => {
    render(<ThoughtRow content="line-a\nline-b" />)
    expect(screen.queryByText(/line-a/)).toBeNull()
    fireEvent.click(screen.getByRole('button'))
    expect(screen.getByText(/line-a/)).toBeTruthy()
  })

  it('empty content (encrypted thinking) → non-interactive label only (no content to expand)', () => {
    render(<ThoughtRow content="" />)
    const btn = screen.getByRole('button')
    fireEvent.click(btn)
    expect(document.querySelector('[data-radix-scroll-area-viewport]')).toBeNull()
  })

  it('★FIX-1★ the max-h cap of the expanded box attaches to the Viewport (scroll node) — not the Root', () => {
    render(<ThoughtRow content="x" />)
    fireEvent.click(screen.getByRole('button'))
    const viewport = document.querySelector('[data-radix-scroll-area-viewport]') as HTMLElement
    expect(viewport).toBeTruthy()
    expect(viewport.className).toContain('max-h-[200px]')
    const root = viewport.parentElement as HTMLElement
    expect(root.className).not.toContain('max-h-[200px]')
  })
})
