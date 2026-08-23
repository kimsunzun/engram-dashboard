// ADR-0069: UI 문자열 중앙화 — 네임스페이스드 `key → 한국어 문자열` 테이블(단일 언어).
//
// ★역할★: 사용자 노출 UI 문자열의 단일 소스(command 제목·우클릭 메뉴 라벨·컴포넌트 텍스트·aria-label·
//   기본 탭명·빈상태 등). drift(aria-label ↔ 표시 텍스트 불일치) 방지 + §5(LLM 이 라벨을 한 곳에서 읽기/수정)
//   + 튜토리얼 문자열 소스. 나중 로컬화 = `t()` 백엔드를 locale-aware 로 교체(이 테이블이 그 base).
//
// ★시드 전용(이 커밋)★: 인프라 형태를 증명하는 대표 엔트리만 담는다. 기존 ~100개 인라인 문자열의
//   실제 마이그레이션은 후속 커밋(command → 컴포넌트 점진). 여기 나열된 것이 전부가 아니다.
//
// ★네임스페이스 = UI 도메인★: 도메인별 1단 그룹(tab/slot/agent/preset/common …).

/**
 * `as const` 로 값을 리터럴 타입으로 고정한다 — index.ts 가 placeholder 유무를 값 리터럴에서 추론해
 * params 타입을 만든다(값을 넓은 `string` 으로 두면 그 추론이 불가능하므로 `as const` 는 필수).
 */
export const ko = {
  tab: {
    create: '새 탭',
    switch: '탭 전환',
    close: '탭 닫기: {name}', // 보간 시드(ADR-0069).
    closeCmd: '탭 닫기', // 보간 없는 팔레트/메뉴 표기 — close 시드와 별개 키.
    next: '다음 탭(순환)',
    rename: '탭 이름 변경',
  },
  slot: {
    setContent: '슬롯 콘텐츠 배치',
    // 키 이름은 결과 배치 기준 — 축 약자(splitH/splitV)로 되돌리지 않는다. 라벨↔배치 결선이 관례로
    //   고정된 뒤엔 h/v 가 반대를 가리킨다(결선 근거 = slotCommands.ts 의 registerSplit 주석).
    splitTopBottom: '가로 분할',
    splitLeftRight: '세로 분할',
    focus: '포커스',
    popout: '팝업으로 분리',
    empty: '비우기',
    close: '닫기',
    resolveSpatial: '공간 타깃 해소',
    fillAgentList: '에이전트 트리 열기',
    fillPresetPalette: '프리셋 팔레트 열기',
    newContent: '새 콘텐츠', // ADR-0065 "새 콘텐츠 ▶" 서브메뉴 컨테이너 라벨.
    renderModeSet: '렌더 모드 지정',
    renderModeClear: '렌더 모드 해제',
    domModeEnable: 'DOM 모드 켜기',
    domModeDisable: 'DOM 모드 끄기',
    domModeToggle: 'DOM 모드 전환',
  },
  window: {
    create: '새 창',
    close: '창 닫기',
    loading: '창 로딩 중… (label: {label})',
    // ADR-0102: 부팅 pull(list_tabs) 유계 재시도 소진 후 최종 실패 표면화(조용히 로딩에 고착 금지).
    loadFailed: '창을 불러오지 못했습니다 (label: {label}). 백엔드 연결을 확인하세요.',
  },
  agent: {
    spawn: '에이전트 생성(spawn)',
    create: '에이전트 생성', // ADR-0078 생성 서브메뉴 컨테이너 라벨과 값 재사용.
    createTerminal: '클로드코드 터미널', // 렌더 모드 Terminal(xterm PTY) 고정 생성(ADR-0078).
    createJson: '클로드코드 JSON', // 렌더 모드 StreamJson(headless NDJSON→RichSlot) 고정 생성(ADR-0078).
    spawnInto: '스폰 + 배치',
    kill: '에이전트 종료',
    monitor: '에이전트 모니터링',
    connecting: '에이전트 연결 중…', // caps 미도착 슬롯의 중립 플레이스홀더.
    // ADR-0148: 명부를 받았는데 그 id 의 프로필도 없는 슬롯(트리에서 삭제됨). 위 connecting 과 구분한다 —
    // 그쪽은 "곧 온다", 이쪽은 "올 것이 없다".
    noneConnected: '연결된 에이전트가 없습니다',
    monitoringLabel: '에이전트 모니터링 — 이 슬롯에 실행중 에이전트 배정',
    monitoringSearch: '에이전트 검색 (이름·경로)',
    noCandidates: '검색 결과 없음', // 실행중은 있으나 검색 미스.
    noRunning: '실행중 에이전트 없음 — 트리에서 에이전트를 생성/활성화하세요.',
    terminatedPlaceholder: '종료된 에이전트', // 챗 입력창 전용 — 슬롯 부재 막은 문구를 쓰지 않는다(SlotUnavailableVeil).
    inputPlaceholder: '메시지 입력 (Enter 전송 · Shift+Enter 줄바꿈)',
    // 빈 상태(ADR-0145) 가운데 입력창 전용. 하단 배치는 위 inputPlaceholder 를 그대로 쓴다 —
    // Enter/Shift+Enter 안내가 사라지면 안 되므로 한 키로 합치지 않는다(사용자 지정 문구).
    emptyInputPlaceholder: '메시지를 입력하세요',
    treeLabel: '에이전트 트리',
    emptyList: '에이전트 없음 — 우클릭으로 생성',
    rowActivate: '활성화(spawn)', // reserved 행.
    rowCancelReserved: '삭제', // reserved 행 — preset.deleteBtn 과 어휘 통일.
    rowOpen: '열기', // running 행.
    rowKill: '종료', // running 행.
    rowRename: '이름 변경', // running/reserved 행(ADR-0061 리치화).
    rowRestart: '재시작 (준비 중)', // 백엔드 command 부재로 비활성.
    doubleClickToActivate: '더블클릭으로 활성화(spawn)', // reserved 행 title 힌트.
    rowFailedBadge: '실패',
    // AgentList 액션 실패 인라인 메시지 — 각 액션별 distinct 키. collapse 금지.
    activateFailed: '활성화 실패: {err}',
    openFailed: '열기 실패: {err}',
    openFailedNoSlot: '열기 실패: 활성 뷰/포커스 슬롯 없음', // openFailed 와 별개 텍스트.
    openFailedNoEmptySlot: '열기 실패: 빈 슬롯이 없습니다 — 슬롯을 비우거나 새로 분할하세요', // 제어·타 에이전트 슬롯 임의 클로버 금지.
    killFailed: '종료 실패: {err}',
    cancelReservedFailed: '삭제 실패: {err}', // rowCancelReserved('삭제')와 어휘 통일.
    renameFailed: '이름 변경 실패: {err}', // ADR-0061 리치화.
    reparentFailed: '이동 실패: {err}', // ADR-0072 트리 계층 — 드래그 재부모화.
    rename: '에이전트 이름 변경', // §5 LLM 제어 — RenameProfile.
  },
  preset: {
    create: '프리셋 생성',
    list: '프리셋 목록 조회',
    delete: '프리셋 삭제', // PresetPalette 행 삭제 aria-label 과 값 재사용.
    add: '추가',
    rename: '이름 변경', // PresetPalette 행 우클릭 메뉴와 값 재사용.
    label: '프리셋',
    empty: '프리셋 없음 — 우클릭 "추가"로 폴더를 선택하세요.',
    deleteBtn: '삭제',
  },
  /** 네이티브 OS 다이얼로그 제목 — webview 밖 사용자 노출 텍스트. */
  dialog: {
    pickAgentCwd: '에이전트 작업 디렉토리 선택',
    pickPresetPath: '프리셋 경로 선택',
  },
  common: {
    emptySlot: '- 비어있음 -',
    defaultTabName: 'View {index}', // 보간 시드.
    // 반복 placeholder 시드 — 같은 토큰 2회. 전역 치환(global replace) 회귀 검증용(index.test.ts). ADR-0069.
    duplicatePreview: '{name} / {name}',
    viewLoading: 'View 로딩 중…',
    emptyResult: '(빈 결과)',
    copied: '복사됨',
    copy: '복사',
    codeCopy: '코드 복사',
    contentPrivate: '내용 비공개',
  },
} as const

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
