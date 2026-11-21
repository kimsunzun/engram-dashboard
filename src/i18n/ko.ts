// ADR-0372: Korean UI string table — a translation of `en.ts` (source of keys and placeholders).
// The `satisfies` line below checks the same keys and per-key `{word}` placeholder set at compile time. A label meant to stay English in Korean mode keeps its English text here.
// `slot.foreignContent` is read as source text by the shell test `foreign_word_is_the_one_the_frontend_shows` (`src-tauri/src/layout/tab_name.rs`): keep the file path and the one-line form `foreignContent: '…'`.

import { deepFreeze } from './deepFreeze'
import type { Translation } from './index'

/** `as const` keeps every value a literal type — the placeholder-set check against `en` reads the value literals. */
export const ko = {
  tab: {
    create: '새 탭',
    switch: '탭 전환',
    close: '탭 닫기: {name}', // Interpolation seed (ADR-0069).
    closeCmd: '탭 닫기', // Palette/menu label without interpolation — a key separate from the close seed.
    next: '다음 탭(순환)',
    rename: '탭 이름 변경',
  },
  slot: {
    setContent: '슬롯 콘텐츠 배치',
    // Key names follow the resulting layout — do not revert to axis abbreviations (splitH/splitV); the label↔layout wiring is in the registerSplit comment in slotCommands.ts.
    splitTopBottom: '가로 분할',
    splitLeftRight: '세로 분할',
    focus: '포커스',
    popout: '팝업으로 분리',
    empty: '비우기',
    close: '닫기',
    resolveSpatial: '공간 타깃 해소',
    fillAgentList: '에이전트 트리 열기',
    fillPresetPalette: '프리셋 팔레트 열기',
    fillUsage: '사용량 한도 열기',
    newContent: '새 콘텐츠', // ADR-0065 "New content ▶" submenu container label.
    renderModeSet: '렌더 모드 지정',
    renderModeClear: '렌더 모드 해제',
    domModeEnable: 'DOM 모드 켜기',
    domModeDisable: 'DOM 모드 끄기',
    domModeToggle: 'DOM 모드 전환',
    renderFailed: '이 슬롯을 표시하지 못했습니다 — 우클릭 메뉴로 비우거나 닫을 수 있습니다',
    scrollToBottom: '맨 아래로',
    // TRD S21-storage §6-2: a slot kind this version does not know — drawn as an icon plus this text (name and tooltip).
    foreignContent: '알 수 없는 내용',
  },
  window: {
    create: '새 창',
    close: '창 닫기',
    minimize: '창 최소화',
    maximize: '창 최대화',
    unmaximize: '창 원래 크기로',
    raise: '창 앞으로 내기',
    hideAll: '모든 창 숨기기',
    showAll: '모든 창 보이기',
    // Accessible names · tooltips of the title-row window buttons (TRD S26-window-chrome §7-4), separate from the command titles above.
    captionMinimize: '최소화',
    captionMaximize: '최대화',
    captionRestore: '이전 크기로 복원',
    captionHideAll: '모든 창 숨기기',
    captionClose: '창 닫기',
    // ADR-0102: surfaces the final failure after the bounded boot pull (list_tabs) retries are exhausted (never stuck on loading silently).
    loadFailed: '창을 불러오지 못했습니다 (label: {label}). 백엔드 연결을 확인하세요.',
    renderFailed: '화면을 그리는 중 오류가 났습니다',
    redraw: '다시 그리기',
    reload: '새로고침',
  },
  agent: {
    spawn: '에이전트 생성(spawn)',
    create: '에이전트 생성', // Reuses the value of the ADR-0078 create submenu container label.
    createTerminal: '클로드코드 터미널', // Always created in render mode Terminal (xterm PTY) (ADR-0078).
    createJson: '클로드코드 JSON', // Always created in render mode StreamJson (headless NDJSON→RichSlot) (ADR-0078).
    createCodex: '코덱스 터미널', // Always created in output mode Terminal (interactive TUI).
    createCodexJson: '코덱스 JSON', // Always created in output mode StreamJson (`codex app-server` resident JSON server).
    spawnInto: '스폰 + 배치',
    kill: '에이전트 종료',
    monitor: '에이전트 모니터링',
    connecting: '에이전트 연결 중…', // Only for a slot that has not received the roster and profile list yet — used only in this case (TRD S21-storage §6-8).
    // TRD S21-storage §6-8: the list arrived but has no profile with that id (deleted in the tree). Unlike connecting ("coming soon"), "nothing will come".
    noTarget: '대상 없음',
    monitoringLabel: '에이전트 모니터링 — 이 슬롯에 실행중 에이전트 배정',
    monitoringSearch: '에이전트 검색 (이름·경로)',
    noCandidates: '검색 결과 없음', // Running agents exist but the search misses.
    noRunning: '실행중 에이전트 없음 — 트리에서 에이전트를 생성/활성화하세요.',
    terminatedPlaceholder: '종료된 에이전트', // Chat input only — does not use the slot absence curtain text (SlotUnavailableVeil).
    inputPlaceholder: '메시지 입력 (Enter 전송 · Shift+Enter 줄바꿈)',
    // Only for the centered input of the empty state (ADR-0145); the bottom placement keeps inputPlaceholder, whose Enter/Shift+Enter hint must not disappear — so no merging into one key.
    emptyInputPlaceholder: '메시지를 입력하세요',
    treeLabel: '에이전트 트리',
    emptyList: '에이전트 없음 — 우클릭으로 생성',
    rowActivate: '활성화(spawn)', // reserved row.
    rowCancelReserved: '삭제', // reserved row — same vocabulary as preset.deleteBtn.
    rowOpen: '열기', // running row.
    rowKill: '종료', // running row.
    rowRename: '이름 변경', // running/reserved row (ADR-0061 enrichment).
    rowRestart: '재시작 (준비 중)', // Disabled because there is no backend command.
    doubleClickToActivate: '더블클릭으로 활성화(spawn)', // reserved row title hint.
    rowFailedBadge: '실패',
    // Row glyph hover text / spoken name while waiting for the answer: reading the optimistic `Running` would restore by sound the lie removed from the screen.
    glyphPending: '진행 중…',
    // ── Last failure (ADR-0172 decision 5 table) ──
    // ★Do not hard-code texts in components (ADR-0173 §Impact)★: only `src/components/agent/failureKinds.ts` reads these keys.
    failureNoConversation: '이어받을 대화가 없습니다',
    failureNoConversationAction: '새 대화로 시작해야 합니다',
    failureSpawn: '에이전트를 띄우지 못했습니다',
    failureSpawnAction: '다시 시도해 보세요',
    failureEarlyExit: '이어받은 직후 종료됐습니다',
    failureEarlyExitAction: '다시 시도해 보세요',
    failureOther: '활성화에 실패했습니다',
    failureOtherAction: '다시 시도해 보세요',
    // One-line form (text + recommended action) shared by the tree hover and the slot exit screen.
    failureLine: '{reason} — {action}',
    // AgentList action failure inline messages — a distinct key per action. Do not collapse.
    activateFailed: '활성화 실패: {err}',
    openFailed: '열기 실패: {err}',
    openFailedNoSlot: '열기 실패: 활성 뷰/포커스 슬롯 없음', // Text separate from openFailed.
    openFailedNoEmptySlot: '열기 실패: 빈 슬롯이 없습니다 — 슬롯을 비우거나 새로 분할하세요', // No arbitrary clobbering of control or other agents' slots.
    killFailed: '종료 실패: {err}',
    cancelReservedFailed: '삭제 실패: {err}', // Same vocabulary as rowCancelReserved ('삭제'). // hangul-ok
    renameFailed: '이름 변경 실패: {err}', // ADR-0061 enrichment.
    reparentFailed: '이동 실패: {err}', // ADR-0072 tree hierarchy — drag reparenting.
    rename: '에이전트 이름 변경', // §5 LLM control — RenameProfile.
    cancelQueuedInput: '대기 입력을 목록에서 빼기', // ADR-0231 — the same command as the list ✕.
    interrupt: '응답 중단',
  },
  preset: {
    create: '프리셋 생성',
    list: '프리셋 목록 조회',
    delete: '프리셋 삭제', // Reuses the value of the PresetPalette row delete aria-label.
    add: '추가',
    rename: '이름 변경', // Reuses the value of the PresetPalette row right-click menu.
    label: '프리셋',
    empty: '프리셋 없음 — 우클릭 "추가"로 폴더를 선택하세요.',
    deleteBtn: '삭제',
  },
  /**
   * Usage limit slot (TRD S21 usage-limit-slot §1-8). The daemon and wire carry only status codes and numbers; the text is made here (PRD R28·R31).
   * ★`{upstream}`·`{kind}` take the upstream original text and classification word untranslated (R31)★ — interpolation does not rescan the value, so a `{…}` inside stays literal.
   */
  usage: {
    vendorClaude: 'Claude',
    vendorCodex: 'Codex',
    windowFiveHour: '5시간',
    windowWeekly: '주간',
    stateNotInstalled: '설치 안 됨',
    stateNeedsLogin: '로그인 필요',
    // ★Do not assert the cause (TRD §6 #20)★: the cause set includes a login token without profile scope, so "logged out" or "account without limits" would mislead that user.
    stateUnavailable: '이 계정의 한도 정보를 받을 수 없음',
    stateFailed: '조회 실패',
    stateRejected: '거절됨 — {duration} 뒤',
    // Status sentence frame; `AfterWait` is only for the refusal whose head already carries the wait time with "—".
    sourceResponse: '응답',
    sourceError: '오류', // Not installed — the original text may be an execution error, not the vendor's response.
    withUpstream: '{status} — {source}: {upstream}',
    withUpstreamCode: '{status} — {source} ({code}): {upstream}',
    withUpstreamAfterWait: '{status} · {source}: {upstream}',
    withUpstreamCodeAfterWait: '{status} · {source} ({code}): {upstream}',
    withKind: '{status} ({kind})',
    withKindCode: '{status} ({kind} · {code})',
    withNextAttempt: '{sentence} · 다음 시도 {time}',
    statusLine: '{vendor} {sentence}',
    resetWaiting: '리셋됨 — 갱신 대기',
    resetWaitingShort: '리셋됨', // Only for the width step where only numbers remain.
    resetClock: '리셋 {time}', // The name of the reset time (tooltip · assistive technology) — the visible spot shows an hourglass instead of a word.
    resetDateTime: '{month}/{day} {time}', // A reset time that is not today — date only, no weekday.
    resetIn: '({duration} 뒤)', // The remaining time attached after the reset time in the popup.
    durationMinutes: '{minutes}분',
    durationHours: '{hours}시간',
    durationHoursMinutes: '{hours}시간 {minutes}분',
    durationDays: '{days}일',
    durationDaysHours: '{days}일 {hours}시간',
    ageJustNow: '방금',
    ageMinutes: '{minutes}분 전',
    ageHours: '{hours}시간 전',
    ageDays: '{days}일 전',
    stale: '{minutes}분 넘게 새로 들어오지 않은 값',
    refreshing: '갱신 중',
    refreshAll: '사용량 새로고침', // Name of the small display's ⟳ · the `usageSlot.refresh` title.
    refreshVendor: '{vendor} 사용량 새로고침', // The ⟳ of the popup's vendor head.
    plan: '플랜: {plan}',
    headSeparator: '·', // Between the plan and the age in the popup's vendor head.
    link: '사용량 페이지 ↗',
    linkTitle: '{vendor} 사용량 설정 페이지를 브라우저로 엽니다 — {url}',
    meterLabel: '{vendor} {window} 남은 양',
    meterValueText: '{pct}% 남음',
    // Accessible name of the summary button — read briefly instead of the bars and badges inside (a button's children are flattened for assistive technology).
    a11yWindow: '{window} {value}',
    a11yNoValue: '값 없음',
    a11yVendor: '{vendor} {first}, {second}',
    a11yVendorStatus: '{vendor} {first}, {second} ({status})',
    a11yStatusRefreshing: '{status}, {refreshing}',
    a11yTwoVendors: '{first} · {second}',
    popupLabel: '사용량 상세',
    hint: '클릭해서 표시할 항목 고르기',
    showOnSlot: '슬롯에 표시:', // Head of the display toggle row at the bottom of the popup.
    showVendor: '{vendor} 표시', // `usageSlot.toggle*` titles.
  },
  /**
   * Structured (chat) slot strings and that slot's command titles. ★No protocol words★ (event names, backend status strings, JSON) — see `docs/process/S21-codex-backend/trd-phase2a.md` §6-2; values state only facts the user reads.
   */
  chat: {
    // The three turn endings — ★distinct facts, never merge keys★: an interruption is not a failure, and unknown means "could not read the ending", not "failed".
    turnFailed: '응답이 실패로 끝났습니다',
    turnInterrupted: '응답이 중단됐습니다',
    turnUnknown: '응답이 끝났습니다 — 끝난 이유는 알 수 없습니다',
    // ADR-0244: the interrupt was sent and the turn end is awaited — it does not mean it stopped (turnInterrupted above says stopping).
    interrupting: '중단하는 중…',
    // Unknown event from the daemon: the direction is one-way, so the shell is always the side that lags and the cause can be asserted.
    unsupportedEvent: '표시할 수 없는 신호 {count}건 — 앱이 데몬보다 오래된 버전일 수 있습니다',
    // Queued input list (ADR-0231): the ✕ text is the same for both backends and every item — whether the text is withdrawn is the backend's call, so no warning (a handed-over codex item is not "discarded").
    queuedRemove: '목록에서 빼기',
    queuedMore: '외 {count}개',
    queuedListLabel: '보내기 대기 중인 입력', // Off-screen name (aria-label) — the list has no header line.
    toolGroupSetExpanded: '도구 묶음 펼치기·접기',
    toolGroupSearch: '검색 {count}',
    toolGroupRead: '읽기 {count}',
    toolGroupList: '목록 {count}',
    toolGroupEdit: '편집 {count}',
    toolGroupCommand: '명령 {count}',
    toolGroupWeb: '웹 {count}',
    toolGroupAgent: '에이전트 {count}',
    toolGroupMcp: 'MCP {count}',
    toolGroupOther: '기타 {count}',
    toolGroupErrors: '오류 {count}',
    // ADR-0241: declines are kept apart from the error keys — a call that was not run and a call that failed are different facts.
    toolGroupDeclined: '거부 {count}',
    // ADR-0263: progressive form of a running group's head; when it ends this drops out and the per-kind counts remain as the past form.
    toolGroupRunning: '진행 중…',
    toolDeclined: '거부됨', // Badge — shared by both endings (Refused · Declined).
    // ADR-0241: Refused = attributed to the dashboard's refusal; Declined = a refusal not attributed to us (backend's own, or a file change restored/superseded). No backend name in the Declined text — frontend strings are backend-neutral (ADR-0004). When T-43 makes claude results end this way they use this text too.
    toolRefusedReason: '대시보드가 승인 요청을 처리하지 않아 실행되지 않음',
    toolDeclinedReason: '실행되지 않음',
    errorTitle: '오류',
    toolErrorBadge: '오류',
    toolInput: '입력',
    toolOutput: '출력',
    toolNoArgs: '인수',
    wait: 'Wait',
    droppedOutput: '이 출력 사건을 싣지 못했습니다',
  },
  /**
   * "Restore?" modal after an abnormal exit and the state file notices (TRD S21-storage §6-7). The sentences rest on the shell's contract (docs of `src-tauri/bindings/CrashCopyStatus.ts` · `RestoreStatusView.ts` · `StateFileStatus.ts`) — change both together.
   */
  restore: {
    title: '이전 화면을 복원할까요?',
    intro: '정상적으로 종료되지 않은 실행의 화면이 저장되어 있습니다.',
    savedAt: '저장 시각',
    windows: '창',
    tabs: '탭',
    count: '{count}개',
    accept: '이전 화면 복원',
    reject: '새로 시작',
    rejectWarning: '「새로 시작」을 고르면 저장된 이전 화면을 지웁니다. 되돌릴 수 없습니다.',
    // `durable:false` — this run saves nothing, so a refusal cannot delete the copy. Several causes (unreadable `state.json` · one saved by a newer version · copy not made), so no file name.
    rejectWarningNotSaving:
      '이번 실행은 화면 상태를 저장하지 않습니다. 「새로 시작」을 골라도 저장된 이전 화면은 그대로 남고, 다음 실행 때 다시 묻습니다.',
    working: '처리 중…',
    // Error kinds are not told apart — the shell's error string is not a contract that carries the kind.
    answerFailed: '답을 처리하지 못했습니다. 잠시 뒤 다시 시도해 주세요.',
    stateFileUnreadable:
      '화면 상태 파일(state.json)을 읽지 못해 기본 화면으로 시작했습니다. 이번 실행에서는 화면 상태를 저장하지 않으며, 다음 실행 때 다시 읽습니다.',
    // ADR-0291: a file written by a newer version is neither copied aside nor overwritten — it is a guard, so this run does not save (`saves:false`).
    stateFileNewer:
      '화면 상태 파일(state.json)이 이 앱보다 새 버전에서 저장한 것이라 기본 화면으로 시작했습니다. 이번 실행에서는 화면 상태를 저장하지 않으며, 파일은 그대로 둡니다.',
    // An "unusable file" is not only damage (unknown format, over the size cap also count) — do not assert one cause. A newer-version file goes to `stateFileNewer`.
    stateFileCorruptCopiedAside:
      '화면 상태 파일(state.json)이 사용할 수 없는 상태(손상 · 알 수 없는 형식 등)라 기본 화면으로 시작했습니다. 원래 파일 내용은 같은 폴더의 state.json.corrupt 에 남겨 두었습니다.',
    // ★Do not call the `.corrupt` of this case this run's backup★ — a file with that name was copied aside by an earlier run.
    stateFileCorruptNotCopied:
      '화면 상태 파일(state.json)이 사용할 수 없는 상태(손상 · 알 수 없는 형식 등)라 기본 화면으로 시작했습니다. 원래 파일은 따로 보관하지 못했습니다. 같은 폴더에 state.json.corrupt 가 있다면 이전 실행에서 남긴 것이며, 이번 파일의 백업이 아닙니다.',
    // Guard ⅱ (`saves:false` · `state_file:'ok'`). Several causes (write failure · unreadable copy · a newer version's copy in place) — do not assert one. "Asked again" = this run does not touch `state.json`, so it keeps no clean-exit mark and the next boot judges again.
    stateFileNotSaving:
      '정상적으로 종료되지 않은 이전 실행의 화면을 따로 보관하지 못해 이번 실행에서는 화면 상태를 저장하지 않습니다. 다음 실행 때 다시 묻습니다.',
    dismissNotice: '화면 상태 알림 닫기',
  },
  // The connection bands, their 「Reconnect」 and the LLM command titles (TRD S36 §3-3 · ADR-0356 · ADR-0395 · ADR-0399).
  connection: {
    head: '데몬에 연결하지 못했습니다.',
    refusedCause: '데몬이 이 앱의 인증을 거절했습니다',
    versionCause: '실행 중인 데몬이 이 앱과 버전이 다릅니다. 데몬을 끈 뒤 다시 연결하세요',
    // ADR-0395: the daemon's listener runs under another Windows account (the app sent nothing), or its account could not be read.
    otherAccountCause: '이 데몬은 다른 Windows 계정의 것입니다. 계정마다 데이터 폴더를 따로 쓰세요',
    accountUnverifiedCause:
      '데몬의 계정을 확인하지 못했습니다. 앱과 Engram을 같은 방식으로(둘 다 관리자 권한 또는 둘 다 일반 권한) 실행하세요',
    // ADR-0399: the last start was refused before a spawn — the data folder's drive cannot keep owner-only files, or its properties
    //   could not be read. Both name the fix.
    unprotectedVolumeCause:
      'Engram 폴더가 파일 권한을 저장하지 못하는 드라이브(FAT 또는 exFAT)에 있습니다. Engram을 종료하고 Engram 폴더(포터블 사본이면 exe가 든 폴더 전체)를 NTFS 드라이브로 옮긴 뒤 다시 시작하세요',
    volumeUnreadableCause:
      'Engram 폴더가 있는 드라이브의 속성을 읽지 못했습니다. Engram을 종료하고 Engram 폴더(포터블 사본이면 exe가 든 폴더 전체)를 NTFS 드라이브로 옮긴 뒤 다시 시작하세요',
    reconnect: '다시 연결',
    reconnecting: '연결 중…',
    dismiss: '알림 닫기',
    statusCmd: '연결 상태',
    reconnectCmd: '다시 연결',
    stopCmd: '데몬 끄기',
  },
  /** Native OS dialog titles — user-visible text outside the webview. */
  dialog: {
    pickAgentCwd: '에이전트 작업 디렉토리 선택',
    pickPresetPath: '프리셋 경로 선택',
  },
  common: {
    emptySlot: '- 비어있음 -',
    defaultTabName: 'View {index}', // Interpolation seed.
    // Repeated placeholder seed — the same token twice. For verifying global replace regressions (index.test.ts). ADR-0069.
    duplicatePreview: '{name} / {name}',
    viewLoading: 'View 로딩 중…',
    // Off-screen name (aria-label) of LoadingPanel — not visible text.
    loading: '로딩 중…',
    emptyResult: '(빈 결과)',
    copied: '복사됨',
    copy: '복사',
    codeCopy: '코드 복사',
    contentPrivate: '내용 비공개',
    noPath: '(경로 없음)',
  },
} as const

ko satisfies Translation<typeof ko>

deepFreeze(ko)
