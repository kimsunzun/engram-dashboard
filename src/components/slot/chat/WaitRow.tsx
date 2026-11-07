// ★Temporary/PROVISIONAL★: the user plans a proper redesign later — for now it is a minimal self-contained implementation.
//
// User decision 2026-10-01: the wait indicator stands outside the conversation scroll list, on its own row (`WaitStrip`) right above the input box. ★That row is always
//   mounted and its height is fixed by the single `--chat-wait-strip-h`★ — even when the indicator turns on and off the conversation list height stays the same, so text
//   does not jump. Moving it back to a tail at the end of the list would make the last lines bounce by the indicator height every time a turn ends.
// ★Timer identity (load-bearing)★: the elapsed seconds are held by the `WaitRow` instance — it mounts when `streaming` turns on and counts from 0,
//   and disappears when it turns off. It is the only one of the same type in the same position in the row, so a re-render in between does not remount it.
// ★「중단하는 중」 (interrupting) (ADR-0244) is a prop of the same instance★ — if drawn split into another component or key, then when the interrupt is refused and it goes back to the wait
//   indicator the elapsed seconds would be 0. So the timer keeps running in the meantime (it is just not visible).

import { useEffect, useState, type ReactNode } from 'react'
import { CircleStop } from 'lucide-react'

import { t } from '../../../i18n'

// ADR-0263 — fixed wait row above the input box (amendment of ADR-0244 implementation detail b).
/** @param label right-hand area — wait text takes its place first and this side shrinks when narrow (the passing side attaches `min-w-0 truncate`). */
export function WaitStrip({
  streaming,
  interrupting,
  label,
}: {
  streaming: boolean
  interrupting: boolean
  label?: ReactNode
}) {
  return (
    <div
      data-wait-strip="1"
      className="flex flex-none items-center gap-2 overflow-hidden pr-3"
      // Left = conversation row body indentation (ChatRow px-4 + rail area) — the rail dot · connector are not drawn.
      style={{ height: 'var(--chat-wait-strip-h)', paddingLeft: 'calc(1rem + var(--chat-rail-gutter))' }}
    >
      {streaming && (
        <div className="flex-none whitespace-nowrap">
          <WaitRow interrupting={interrupting} />
        </div>
      )}
      {label !== undefined && <div className="ml-auto flex min-w-0 justify-end">{label}</div>}
    </div>
  )
}

export function WaitRow({ interrupting = false }: { interrupting?: boolean }) {
  const [seconds, setSeconds] = useState(0)

  useEffect(() => {
    // The cleanup clearInterval is invariant — prevents leaks and test act warnings.
    const id = setInterval(() => setSeconds((s) => s + 1), 1000)
    return () => clearInterval(id)
  }, [])

  if (interrupting) {
    // Uses the same icon as the outcome row (interrupt — bold · accent color) but the tone stays that of the wait indicator — it has not stopped yet.
    return (
      <div
        data-wait-interrupting="1"
        className="flex items-center gap-1.5 text-[13px] text-muted select-none"
      >
        <CircleStop className="size-3.5 flex-none" aria-hidden />
        <span className="animate-pulse">{t('chat.interrupting')}</span>
      </div>
    )
  }

  return (
    <div className="flex items-center gap-1 text-[13px] text-muted select-none">
      <span className="animate-pulse">Wait</span>
      <span className="animate-pulse" aria-hidden>
        …
      </span>
      <span className="tabular-nums">{seconds}s</span>
    </div>
  )
}

export default WaitRow
