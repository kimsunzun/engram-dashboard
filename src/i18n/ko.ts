// ADR-0069: UI 문자열 중앙화 — 네임스페이스드 `key → 한국어 문자열` 테이블(단일 언어).
//
// ★역할★: 사용자 노출 UI 문자열의 단일 소스(command 제목·우클릭 메뉴 라벨·컴포넌트 텍스트·aria-label·
//   기본 탭명·빈상태 등). drift(aria-label ↔ 표시 텍스트 불일치) 방지 + §5(LLM 이 라벨을 한 곳에서 읽기/수정)
//   + 튜토리얼 문자열 소스. 나중 로컬화 = `t()` 백엔드를 locale-aware 로 교체(이 테이블이 그 base).
//
// ★시드 전용(이 커밋)★: 인프라 형태를 증명하는 대표 엔트리만 담는다. 기존 ~100개 인라인 문자열의
//   실제 마이그레이션은 후속 커밋(command → 컴포넌트 점진). 여기 나열된 것이 전부가 아니다.
//
// ★네임스페이스 = UI 도메인★: 도메인별 1단 그룹(tab/slot/agent/preset/common …). 키 경로는
//   `namespace.key` 로 평탄화돼 `t('tab.close', …)` 형태로 접근된다(index.ts 참조).
//
// ★보간★: 값 안의 `{name}` 같은 `{placeholder}` 토큰은 `t()` 호출 시 params 로 치환된다. placeholder
//   가 있는 값은 index.ts 의 타입이 params 를 필수로 강제한다(오타·누락은 tsc 가 잡음).

/**
 * 네임스페이스드 UI 문자열 테이블. 최상위 키 = UI 도메인(namespace), 그 아래 = 개별 문자열 키.
 * `as const` 로 값을 리터럴 타입으로 고정한다 — index.ts 가 placeholder 유무를 값 리터럴에서 추론해
 * params 타입을 만든다(값을 넓은 `string` 으로 두면 그 추론이 불가능하므로 `as const` 는 필수).
 */
export const ko = {
  /** 탭(View) 관련 — command 제목·라벨. */
  tab: {
    create: '새 탭', // tab.create command 제목.
    switch: '탭 전환', // tab.switch command 제목.
    close: '탭 닫기: {name}', // 보간 시드 — ADR-0069 예시(닫기 확인 등 name 을 붙이는 소비자용).
    closeCmd: '탭 닫기', // tab.close command 제목(보간 없는 팔레트/메뉴 표기 — close 시드와 별개 키).
    next: '다음 탭(순환)', // tab.next command 제목.
    rename: '탭 이름 변경', // tab.rename command 제목.
  },
  /** 슬롯(레이아웃 한 칸) 관련 — command 제목·우클릭 메뉴 라벨. */
  slot: {
    setContent: '슬롯 콘텐츠 배치', // layout.setSlotContent command 제목.
    splitH: '가로 분할', // slot.split.h command 제목.
    splitV: '세로 분할', // slot.split.v command 제목.
    focus: '포커스', // slot.focus command 제목.
    popout: '팝업으로 분리', // slot.popout command 제목.
    empty: '비우기', // slot.empty command 제목.
    close: '닫기', // slot.close command 제목(슬롯 닫기 — 우클릭 메뉴).
    resolveSpatial: '공간 타깃 해소', // slot.resolveSpatial command 제목.
    fillAgentList: '에이전트 트리 열기', // slot.fill.agentList command 제목.
    fillPresetPalette: '프리셋 팔레트 열기', // slot.fill.presetPalette command 제목.
    newContent: '새 콘텐츠', // empty 슬롯 우클릭 "새 콘텐츠 ▶" 서브메뉴 컨테이너 라벨(ADR-0065).
  },
  /** 창(WebView2 윈도우) 관련. */
  window: {
    create: '새 창', // window.create command 제목.
    close: '창 닫기', // window.close command 제목.
    loading: '창 로딩 중… (label: {label})', // WindowLayout: 창 상태 미도착 시 로딩 플레이스홀더(보간 label).
  },
  /** 에이전트(claude 프로세스) 관련 — command 제목·우클릭 메뉴 라벨. */
  agent: {
    spawn: '에이전트 생성(spawn)', // agent.spawn command 제목.
    create: '에이전트 생성', // agentlist.createAgent / slot.createAgentHere command 제목(폴더 다이얼로그 스폰).
    spawnInto: '스폰 + 배치', // agent.spawnInto command 제목.
    kill: '에이전트 종료', // agent.kill command 제목.
    monitor: '에이전트 모니터링', // slot.assignRunningAgent command 제목(실행중 에이전트 배치).
    connecting: '에이전트 연결 중…', // ViewLayoutRenderer: caps 미도착 슬롯의 중립 플레이스홀더.
    monitoringLabel: '에이전트 모니터링 — 이 슬롯에 실행중 에이전트 배정', // AgentMonitoringPicker 팝업 라벨.
    monitoringSearch: '에이전트 검색 (이름·경로)', // AgentMonitoringPicker 검색창 placeholder.
    noCandidates: '검색 결과 없음', // AgentMonitoringPicker: 실행중은 있으나 검색 미스.
    noRunning: '실행중 에이전트 없음 — 트리에서 에이전트를 생성/활성화하세요.', // AgentMonitoringPicker: 실행중 0.
    terminatedPlaceholder: '종료된 에이전트', // RichSlot 입력창 placeholder(종료 상태) — 오버레이 '종료됨'과 별개.
    inputPlaceholder: '메시지 입력 (Enter 전송 · Shift+Enter 줄바꿈)', // RichSlot 입력창 placeholder(활성).
    treeLabel: '에이전트 트리', // AgentList 슬롯 콘텐츠 라벨.
    emptyList: '에이전트 없음 — 우클릭으로 생성', // AgentList 빈 상태 안내.
    terminatedOverlay: '종료됨', // TerminalSlot/DomSlot 종료 오버레이 — placeholder '종료된 에이전트'와 별개.
    // AgentList 행 우클릭 메뉴 라벨(reserved: 활성화/예약취소 · running: 열기/종료/이름변경/재시작).
    rowActivate: '활성화(spawn)', // reserved 행 활성화(spawnProfile) 메뉴 라벨.
    rowCancelReserved: '예약 취소', // reserved 행 예약취소(deleteProfile) 메뉴 라벨.
    rowOpen: '열기', // running 행 "열기"(포커스 슬롯 배정) 메뉴 라벨.
    rowKill: '종료', // running 행 "종료"(kill) 메뉴 라벨.
    rowRename: '이름변경 (준비 중)', // running 행 이름변경 — 백엔드 command 부재로 비활성.
    rowRestart: '재시작 (준비 중)', // running 행 재시작 — 백엔드 command 부재로 비활성.
    doubleClickToActivate: '더블클릭으로 활성화(spawn)', // reserved 행 title 힌트(더블클릭 = spawn).
    rowFailedBadge: '실패', // AgentList 행 옆 인라인 실패 배지 텍스트(err 있을 때).
    // AgentList 액션 실패 인라인 메시지 — 각 액션별 distinct 키({err} = 원인 문자열 보간). collapse 금지.
    activateFailed: '활성화 실패: {err}', // spawnProfile 실패.
    openFailed: '열기 실패: {err}', // assignAgent 실패.
    openFailedNoSlot: '열기 실패: 활성 뷰/포커스 슬롯 없음', // 활성 뷰/포커스 슬롯 부재로 조기 실패(보간 없음, openFailed 와 별개 텍스트).
    killFailed: '종료 실패: {err}', // killAgent 실패.
    cancelReservedFailed: '예약 취소 실패: {err}', // deleteProfile 실패.
  },
  /** 프리셋(cwd 프리셋) 관련 — command 제목·우클릭 메뉴 라벨. */
  preset: {
    create: '프리셋 생성', // preset.create command 제목.
    list: '프리셋 목록 조회', // preset.list command 제목.
    delete: '프리셋 삭제', // preset.delete command 제목 겸 PresetPalette 행 삭제 aria-label(값 동일 — 재사용).
    add: '추가', // preset.add command 제목(preset_palette 슬롯 메뉴 "추가").
    label: '프리셋', // PresetPalette 슬롯 콘텐츠 라벨.
    empty: '프리셋 없음 — 우클릭 "추가"로 폴더를 선택하세요.', // PresetPalette 빈 상태 안내.
    deleteBtn: '삭제', // PresetPalette 행 삭제 버튼 텍스트.
  },
  /** 테마 관련 — command 제목. */
  theme: {
    set: '테마 설정', // theme.set command 제목.
    toggle: '테마 순환', // theme.toggle command 제목.
  },
  /** 네이티브 OS 다이얼로그 제목(폴더 선택 창 — webview 밖 사용자 노출 텍스트). */
  dialog: {
    pickAgentCwd: '에이전트 작업 디렉토리 선택', // 에이전트 스폰용 cwd 폴더 선택.
    pickPresetPath: '프리셋 경로 선택', // 프리셋 등록용 폴더 선택.
  },
  /** 도메인 공통 — 기본명·빈상태 등. */
  common: {
    emptySlot: '- 비어있음 -',
    defaultTabName: 'View {index}', // 보간 시드 — 기본 탭명(예: "View 1").
    // 반복 placeholder 시드 — 같은 토큰 2회. 전역 치환(global replace) 회귀 검증용(index.test.ts). ADR-0069.
    duplicatePreview: '{name} / {name}',
    viewLoading: 'View 로딩 중…', // WindowLayout TabCanvas: 뷰 캐시 미도착 로딩 플레이스홀더.
    viewEmpty: '— empty —', // ViewLayoutRenderer: empty 슬롯 플레이스홀더(em-dash — emptySlot '- 비어있음 -'과 별개 텍스트).
    emptyResult: '(빈 결과)', // StructuredTextView: 도구 결과 본문이 빈 경우 대체 표기.
    copied: '복사됨', // CopyButton: 복사 완료 aria-label.
    copy: '복사', // CopyButton: 기본 복사 aria-label(label prop 기본값).
    codeCopy: '코드 복사', // Markdown PreBlock: 코드블록 CopyButton label(aria-label).
    contentPrivate: '내용 비공개', // ThoughtRow: 암호화 thinking(펼칠 내용 없음) title.
  },
} as const

/** `ko` 테이블의 정적 타입(index.ts 가 키 유니온·params 추론에 쓴다). */
export type KoTable = typeof ko

// ★단일 소스 무결성(FIX E)★: `as const` 는 컴파일 타임 readonly 일 뿐 — JS 소비자(또는 역직렬화 경계)는
//   런타임에 `ko.tab.close = ...` 로 변조해 t() 백엔드를 오염시킬 수 있다. deep-freeze 로 런타임에도 잠근다.
//   (타입 추론은 위 `as const` 가 계속 담당 — freeze 는 값만 동결하고 타입엔 영향 없다.)
function deepFreeze<T>(obj: T): T {
  if (obj && typeof obj === 'object') {
    for (const value of Object.values(obj)) deepFreeze(value)
    Object.freeze(obj)
  }
  return obj
}
deepFreeze(ko)
