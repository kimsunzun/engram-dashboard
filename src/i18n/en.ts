// ADR-0372: the source UI string table — namespaced `key → English text`. Its type is the key and placeholder contract
//   (`StringKey`, `Placeholders` in `index.ts`); `ko.ts` must have exactly these keys and, per key, the same placeholder set.
//
// Namespace = UI domain, one level deep (tab/slot/agent/common …).
// Labels use sentence-style capitalization ("Close tab"). Placeholder grammar: `WORD_PLACEHOLDER` in `index.ts`.
// Punctuation: complete sentences (messages, tooltips, notices, body text) end with a period; labels, buttons, menu items, titles, fragments, values ending in outside text (`{err}`, `{url}`) and values embedded into other values (`{status}`, `{sentence}`) do not.
// `slot.foreignContent` must equal the shell's `Foreign` tab-name word. The shell test `foreign_word_is_the_one_the_frontend_shows`
//   (`src-tauri/src/layout/tab_name.rs`) reads `ko.ts` as text today and is to read this file too (ADR-0373): keep the file
//   path and the one-line form `foreignContent: '…'`, with no `'` in the value.

import { deepFreeze } from './deepFreeze'

/**
 * `as const` keeps every value a literal type — `index.ts` reads the placeholders out of the value literals to type `params`
 * (widening the values to `string` silently drops all param checking).
 */
export const en = {
  tab: {
    create: 'New tab',
    switch: 'Switch tab',
    close: 'Close tab: {name}', // Interpolation seed (ADR-0069).
    closeCmd: 'Close tab', // Palette/menu label without interpolation — a key separate from `close`.
    next: 'Next tab (cycle)',
    rename: 'Rename tab',
  },
  slot: {
    setContent: 'Set slot content',
    // Key names follow the resulting layout — do not revert to axis abbreviations (splitH/splitV). Labels follow the vim
    //   convention (ADR-0140), so h/v would point the other way (wiring = the `registerSplit` comment in slotCommands.ts).
    splitTopBottom: 'Split horizontally',
    splitLeftRight: 'Split vertically',
    focus: 'Focus',
    popout: 'Pop out',
    empty: 'Clear',
    close: 'Close',
    resolveSpatial: 'Resolve spatial target',
    fillAgentList: 'Open agent tree',
    fillUsage: 'Open usage limits',
    newContent: 'New content', // ADR-0065 "New content ▶" submenu container label.
    renderModeSet: 'Set render mode',
    renderModeClear: 'Clear render mode',
    domModeEnable: 'Turn on DOM mode',
    domModeDisable: 'Turn off DOM mode',
    domModeToggle: 'Toggle DOM mode',
    renderFailed: 'Could not display this slot — you can clear or close it from the right-click menu.',
    scrollToBottom: 'Scroll to bottom',
    // A slot kind this build does not know — the screen draws only an icon; this text is its name and tooltip.
    foreignContent: 'Unknown content',
  },
  window: {
    create: 'New window',
    close: 'Close window',
    minimize: 'Minimize window',
    maximize: 'Maximize window',
    unmaximize: 'Restore window size',
    raise: 'Bring window to front',
    hideAll: 'Hide all windows',
    showAll: 'Show all windows',
    // Accessible names and tooltips of the title-bar window buttons — kept apart from the command
    //   titles above (the words of the Windows caption buttons).
    captionMinimize: 'Minimize',
    captionMaximize: 'Maximize',
    captionRestore: 'Restore down',
    captionHideAll: 'Hide all windows',
    captionClose: 'Close window',
    // ADR-0102: surfaces the final failure after the bounded boot pull (list_tabs) retries run out (never stuck on loading silently).
    loadFailed: 'Could not load the window (label: {label}). Check the backend connection.',
    renderFailed: 'An error occurred while drawing the screen.',
    redraw: 'Redraw',
    reload: 'Reload',
  },
  agent: {
    spawn: 'Create agent (spawn)',
    create: 'Create agent', // ADR-0078 create-submenu container label; the value is reused.
    createTerminal: 'Claude Code terminal', // Always created in Terminal render mode (xterm PTY) (ADR-0078).
    createJson: 'Claude Code JSON', // Always created in StreamJson render mode (headless NDJSON → RichSlot) (ADR-0078).
    createCodex: 'Codex terminal', // Always created in Terminal output mode (interactive TUI).
    createCodexJson: 'Codex JSON', // Always created in StreamJson output mode (resident `codex app-server` JSON server).
    spawnInto: 'Spawn + place',
    kill: 'Stop agent',
    monitor: 'Monitor agent',
    connecting: 'Connecting to agent…', // Only for a slot that has not received the roster and profile list yet.
    // The list arrived but has no profile with that id (deleted in the tree). Distinct from `connecting`:
    //   that one means "coming soon", this one "nothing will come".
    noTarget: 'No target',
    monitoringLabel: 'Agent monitoring — assign a running agent to this slot',
    monitoringSearch: 'Search agents (name, path)',
    noCandidates: 'No results', // Running agents exist but the search misses.
    noRunning: 'No running agents — create or activate one in the tree.',
    terminatedPlaceholder: 'Agent stopped', // Chat input only — does not reuse the slot-unavailable veil text (SlotUnavailableVeil).
    inputPlaceholder: 'Type a message (Enter to send · Shift+Enter for a new line)',
    // Only for the centered input of the empty state (ADR-0145). The bottom input keeps `inputPlaceholder` — do not merge the
    //   two keys, the Enter/Shift+Enter hint must not disappear (user-specified wording).
    emptyInputPlaceholder: 'Type a message',
    treeLabel: 'Agent tree',
    emptyList: 'No agents — right-click to create one.',
    rowActivate: 'Activate (spawn)', // Reserved row.
    rowCancelReserved: 'Delete', // Reserved row.
    rowOpen: 'Open', // Running row.
    rowKill: 'Stop', // Running row.
    rowRename: 'Rename', // Running/reserved row (ADR-0061).
    rowRestart: 'Restart (coming soon)', // Disabled — no backend command yet.
    doubleClickToActivate: 'Double-click to activate (spawn).', // Reserved-row title hint.
    rowFailedBadge: 'Failed',
    // Row glyph (spinning) while waiting for the answer — hover text and accessible name. Reading the optimistic `Running`
    //   aloud would bring back by sound the lie removed from the screen, so this span replaces the status word.
    glyphPending: 'In progress…',
    // ── Last failure (ADR-0172 decision 5 table — one text and one suggested action per kind) ────────────
    // Do not hard-code these texts in components (ADR-0173 §impact): the only reader of these keys is
    //   `src/components/agent/failureKinds.ts`, and the screen takes the text from that table.
    failureNoConversation: 'No conversation to resume',
    failureNoConversationAction: 'Start a new conversation',
    failureSpawn: 'Could not start the agent',
    failureSpawnAction: 'Try again',
    failureEarlyExit: 'Exited right after resuming',
    failureEarlyExitAction: 'Try again',
    failureOther: 'Activation failed',
    failureOtherAction: 'Try again',
    // One-line form (text + suggested action) shared by the tree hover and the slot's exit screen.
    failureLine: '{reason}. {action}.',
    // Inline messages for AgentList action failures — one distinct key per action; do not collapse them.
    activateFailed: 'Activation failed: {err}',
    openFailed: 'Open failed: {err}',
    openFailedNoSlot: 'Open failed: no active view or focused slot.', // Text distinct from openFailed.
    openFailedNoEmptySlot: 'Open failed: no empty slot — clear a slot or split a new one.', // Never clobbers a control or another agent's slot.
    killFailed: 'Stop failed: {err}',
    cancelReservedFailed: 'Delete failed: {err}', // Same word as rowCancelReserved.
    renameFailed: 'Rename failed: {err}', // ADR-0061.
    reparentFailed: 'Move failed: {err}', // ADR-0072 tree hierarchy — drag to reparent.
    rename: 'Rename agent', // LLM control — RenameProfile.
    cancelQueuedInput: 'Remove queued input from the list', // ADR-0231 — the same command as the list ✕.
    interrupt: 'Interrupt response',
  },
  /**
   * Usage-limit slot. The daemon and the wire carry only status codes and numbers; the text is
   * made here.
   *
   * `{upstream}` and `{kind}` receive the upstream original text and classification word untranslated (R31) — interpolation does
   *   not rescan substituted values, so a `{…}` inside the original text stays literal.
   */
  usage: {
    vendorClaude: 'Claude',
    vendorCodex: 'Codex',
    windowFiveHour: '5-hour',
    windowWeekly: 'Weekly',
    stateNotInstalled: 'Not installed',
    stateNeedsLogin: 'Sign-in required',
    // Do not assert the cause — the causes include a login token without profile scope, so saying only
    //   "logged out" or "account without limits" would mislead that user.
    stateUnavailable: 'Limit information is unavailable for this account',
    stateFailed: 'Could not fetch',
    stateRejected: 'Rejected — retry in {duration}',
    // Frames of the status sentence — how the detail is attached. `AfterWait` is only for the rejection, whose head already
    //   carries the wait time after "—".
    sourceResponse: 'Response',
    sourceError: 'Error', // Not installed — the original text may be an execution error, not the vendor's response.
    withUpstream: '{status} — {source}: {upstream}',
    withUpstreamCode: '{status} — {source} ({code}): {upstream}',
    withUpstreamAfterWait: '{status} · {source}: {upstream}',
    withUpstreamCodeAfterWait: '{status} · {source} ({code}): {upstream}',
    withKind: '{status} ({kind})',
    withKindCode: '{status} ({kind} · {code})',
    withNextAttempt: '{sentence} · next try at {time}',
    statusLine: '{vendor}: {sentence}',
    resetWaiting: 'Reset — waiting for update',
    resetWaitingShort: 'Reset', // Only for the width step where only numbers remain.
    resetClock: 'Resets at {time}', // Name of the reset time (tooltip, assistive technology) — the visible spot shows an hourglass instead of a word.
    resetDateTime: '{month}/{day} {time}', // A reset time that is not today — date only, no weekday.
    resetIn: '(in {duration})', // Remaining time after the reset time in the popup.
    durationMinutes: '{minutes}m',
    durationHours: '{hours}h',
    durationHoursMinutes: '{hours}h {minutes}m',
    durationDays: '{days}d',
    durationDaysHours: '{days}d {hours}h',
    ageJustNow: 'Just now',
    ageMinutes: '{minutes}m ago',
    ageHours: '{hours}h ago',
    ageDays: '{days}d ago',
    stale: 'No new value for more than {minutes} minutes',
    refreshing: 'Refreshing',
    refreshAll: 'Refresh usage', // Name of the small ⟳ and the `usageSlot.refresh` title.
    refreshVendor: 'Refresh {vendor} usage', // The ⟳ in the popup's vendor head.
    plan: 'Plan: {plan}',
    headSeparator: '·', // Between the plan and the age in the popup's vendor head.
    link: 'Usage page ↗',
    linkTitle: 'Opens the {vendor} usage settings page in the browser — {url}',
    meterLabel: '{vendor} {window} limit remaining',
    meterValueText: '{pct}% left',
    // Accessible name of the summary button — read briefly instead of the bars and badges inside (a button's children are
    //   flattened for assistive technology).
    a11yWindow: '{window} {value}',
    a11yNoValue: 'No value',
    a11yVendor: '{vendor}: {first}, {second}',
    a11yVendorStatus: '{vendor}: {first}, {second} ({status})',
    a11yStatusRefreshing: '{status}, {refreshing}',
    a11yTwoVendors: '{first} · {second}',
    popupLabel: 'Usage details',
    hint: 'Click to choose what to show',
    showOnSlot: 'Show on slot:', // Head of the show-toggle row at the bottom of the popup.
    showVendor: 'Show {vendor}', // `usageSlot.toggle*` titles.
  },
  /**
   * Structured (chat) slot strings — the rendered text and that slot's command titles.
   *
   * Do not put protocol words here — event names, backend status strings and JSON are not shown on screen
   *   (`docs/process/S21-codex-backend/trd-phase2a.md` §6-2). Values state only facts the user reads.
   */
  chat: {
    // The three turn endings are different facts — do not merge them into one key. An interruption is not a failure, and
    //   unknown means "we could not read the ending", not "it failed".
    turnFailed: 'The response failed.',
    turnInterrupted: 'The response was interrupted.',
    turnUnknown: 'The response ended — the reason is unknown.',
    // ADR-0244: the interrupt was sent and the end of the turn is awaited — it does not mean it stopped (turnInterrupted says that).
    interrupting: 'Interrupting…',
    // An event this shell does not know arrived. The direction is one-way, so the cause can be stated — unknown vocabulary
    //   comes from the daemon, so the side that lags is always the shell.
    unsupportedEvent: 'Signals that cannot be displayed: {count} — the app may be older than the daemon.',
    // Queued input list (ADR-0231). The ✕ text is the same for both backends and every item — whether the text is withdrawn
    //   is decided by the backend, so no warning is added ("discarded" would be wrong for a handed-over codex item — user 2026-09-26).
    queuedRemove: 'Remove from list',
    queuedMore: '+{count} more',
    queuedListLabel: 'Inputs waiting to be sent', // Off-screen name (aria-label) — the list has no header line.
    toolGroupSetExpanded: 'Expand or collapse tool group',
    toolGroupSearch: 'Search {count}',
    toolGroupRead: 'Read {count}',
    toolGroupList: 'List {count}',
    toolGroupEdit: 'Edit {count}',
    toolGroupCommand: 'Command {count}',
    toolGroupWeb: 'Web {count}',
    toolGroupAgent: 'Agent {count}',
    toolGroupMcp: 'MCP {count}',
    toolGroupOther: 'Other {count}',
    toolGroupErrors: 'Errors {count}',
    // ADR-0241: declines are kept apart from errors — a call that was not run and a call that failed are different facts.
    toolGroupDeclined: 'Declined {count}',
    // ADR-0263: progressive form of a running group's head — when it ends this drops out and only the per-kind counts remain.
    toolGroupRunning: 'Running…',
    toolDeclined: 'Declined', // Badge — shared by both endings (Refused · Declined).
    // ADR-0241: the reason differs by ending. Refused = a call attributed to the dashboard's refusal · Declined = a refusal not
    //   attributed to us (the backend's own refusal, or a file change rejected because it was restored or superseded). Do not put
    //   a backend name in the Declined text — frontend strings are backend-neutral (ADR-0004). When T-43 makes claude results
    //   end this way, they use this text too.
    toolRefusedReason: 'Not run because the dashboard did not handle the approval request',
    toolDeclinedReason: 'Not run',
    errorTitle: 'Error', // Head of an error row.
    toolErrorBadge: 'Error', // Badge of a tool call that failed.
    toolInput: 'In', // Caption over a tool call's arguments.
    toolOutput: 'Out', // Caption over a tool call's result.
    toolNoArgs: 'arguments', // Stands in for the argument hint when there is none.
    wait: 'Wait', // ADR-0372: stays English in `ko.ts`. The wait indicator above the input.
    // The chat view shows this for the dropped-output placeholder (`PLACEHOLDER_ERROR_MESSAGE` in `src/api/wsFrame.ts`).
    droppedOutput: 'This output could not be delivered.',
  },
  /**
   * The "Restore?" modal after an abnormal exit and the state-file notices. What the sentences claim
   * rests on the shell's contract (the docs of `src-tauri/bindings/CrashCopyStatus.ts` · `RestoreStatusView.ts` ·
   * `StateFileStatus.ts`) — change these together when that contract changes.
   */
  restore: {
    title: 'Restore previous session?',
    intro: 'The previous session did not end normally and was saved.',
    savedAt: 'Saved at',
    windows: 'Windows',
    tabs: 'Tabs',
    count: '{count}',
    accept: 'Restore session',
    reject: 'Start new session',
    rejectWarning: 'Choosing "Start new session" deletes the saved session. This cannot be undone.',
    // `durable:false` — this run saves nothing, so rejecting cannot delete the copy. There are several causes (`state.json`
    //   unreadable · a `state.json` saved by a newer version · the copy could not be made), so no file name is given.
    rejectWarningNotSaving:
      'This session is not being saved. Choosing "Start new session" keeps the saved session, and you will be asked again the next time the app starts.',
    working: 'Working…',
    // Error kinds are not told apart — the shell's error string is not a contract that carries the kind.
    answerFailed: 'Could not apply your choice. Try again in a moment.',
    stateFileUnreadable:
      'Could not read the session file (state.json), so the app started a new session. This session is not being saved; the file is read again the next time the app starts.',
    // ADR-0291: a file written by a newer build is neither copied aside nor overwritten — it is a guard, so this run does not save (`saves:false`).
    stateFileNewer:
      'The session file (state.json) was saved by a newer version of this app, so the app started a new session. This session is not being saved, and the file is left as is.',
    // An "unusable file" is not only damage — an unknown format, exceeding the size limit and so on count too. Do not name one cause.
    //   A file saved by a newer version does not come this way (`stateFileNewer`).
    stateFileCorruptCopiedAside:
      'The session file (state.json) is unusable (damaged, unknown format or similar), so the app started a new session. The original file contents were kept in state.json.corrupt in the same folder.',
    // Do not call the `.corrupt` of this case this run's backup — a file with that name was made by an earlier run.
    stateFileCorruptNotCopied:
      'The session file (state.json) is unusable (damaged, unknown format or similar), so the app started a new session. The original file could not be kept aside. A state.json.corrupt in the same folder was left by an earlier session and is not a backup of this file.',
    // Guard ii (`saves:false` · `state_file:'ok'`). There are several causes (write failure · the copy is unreadable · a newer
    //   build's copy is in place), so do not name one. "Asked again" = this run does not touch `state.json`, so it stays
    //   without a clean-exit mark and the next boot judges it again.
    stateFileNotSaving:
      'The previous session, which did not end normally, could not be kept aside, so this session is not being saved. You will be asked again the next time the app starts.',
    dismissNotice: 'Dismiss session notice',
  },
  // The two connection bands, their "Reconnect" and the LLM command titles (ADR-0356).
  connection: {
    head: 'Could not connect to the daemon.',
    refusedCause: 'The daemon refused authentication from this app.',
    versionCause: 'The running daemon is a different version from this app. Stop the daemon, then reconnect.',
    reconnect: 'Reconnect',
    reconnecting: 'Reconnecting…',
    dismiss: 'Dismiss notice',
    statusCmd: 'Connection status',
    reconnectCmd: 'Reconnect',
    stopCmd: 'Stop daemon',
  },
  /** Native OS dialog titles — user-visible text outside the webview. */
  dialog: {
    pickAgentCwd: 'Select agent working directory',
  },
  common: {
    emptySlot: '- Empty -',
    defaultTabName: 'View {index}', // Interpolation seed.
    // Repeated-placeholder seed — the same token twice. Pins global replacement (index.test.ts). ADR-0069.
    duplicatePreview: '{name} / {name}',
    viewLoading: 'Loading view…',
    // Off-screen name (aria-label) of LoadingPanel — not visible text.
    loading: 'Loading…',
    emptyResult: '(empty result)',
    copied: 'Copied',
    copy: 'Copy',
    codeCopy: 'Copy code',
    contentPrivate: 'Content hidden',
    noPath: '(no path)', // Name of a path-derived label when the path is blank (`basename` in `src/util/basename.ts`).
  },
} as const

export type EnTable = typeof en

deepFreeze(en)
