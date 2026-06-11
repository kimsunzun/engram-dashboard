# Gemini 응답

제공해주신 최종 설계(session-restore-lld.md)와 실측 결과(spike-results.md)는 Spike를 통해 블랙박스 영역이었던 Claude의 내부 메커니즘(~/.claude/sessions/<PID>.json)을 명확히 밝혀내고 예외 케이스를 방어한 훌륭한 엔지니어링 문서입니다.

그러나 Adversarial(반론 및 결함 탐색) 관점에서 현미경을 들이대면, 실제 배포(Production) 단계에서 시스템을 무너뜨릴 수 있는 치명적인 레이스 컨디션, OS 특성 오판, UX 결함이 몇 가지 숨어있습니다. 코어 구현 단계로 진입하기 전, 반드시 해결해야 할 5가지 핵심 결함과 개선 방향을 제시합니다.

1. [치명적] Rust canonicalize와 Windows UNC 접두사(\\?\) 충돌
결함 분석

현상: 문서에서는 spawn 전 cwd를 canonicalize 하겠다고 명시했습니다(Spike #8 반영). 그러나 Windows 환경에서 Rust의 std::fs::canonicalize를 사용하면 경로 앞에 \\?\ (UNC prefix)가 무조건 붙습니다 (예: C:\temp → \\?\C:\temp).

문제점: Claude 세션 디렉토리명은 경로 문자열을 그대로 치환(\ -> -)하여 생성됩니다. 만약 Claude 내부적으로 이 UNC 접두사를 제거하는 로직이 없다면, 세션 폴더명이 ----C--temp 형태로 완전히 왜곡되어 생성됩니다.

결과: 사용자가 터미널에서 일반적인 claude 명령어로 접근할 때의 폴더명(C--temp)과 우리 앱이 PTY로 실행할 때의 폴더명이 불일치하게 되어, 세션 복원 및 공유가 완전히 깨집니다.

대응 전략

Rust 순정 canonicalize 대신, UNC 접두사를 안전하게 제거해 주는 dunce::canonicalize 크레이트를 사용하거나 사후 문자열 처리를 통해 \\?\를 반드시 스트립(Strip)해야 합니다.

2. [레이스 컨디션] ~/.claude/sessions/<PID>.json 파일 Watch 방식의 맹점
결함 분석

현상: 설계에서는 "능동 동기화 = sessions/<child_pid>.json 한 파일만 watch" 하겠다고 결정했습니다.

문제점 (Atomic Write의 함정): Claude가 이 파일을 갱신할 때 기존 파일에 write를 할까요, 아니면 현대적인 App들이 하듯 .tmp로 쓰고 rename(Atomic replace)을 할까요? 후자일 확률이 매우 높습니다.

결과: 파일 경로 자체를 콕 집어 디렉터리 감시자(Inotify/ReadDirectoryChangesW)에 등록하면, 파일이 삭제되고 새로 rename되는 순간 Watcher가 이벤트를 잃어버리거나(Linux Inode 변경) 깨지는 현상이 발생합니다.

초기 기동 레이스: child process를 spawn 하자마자 해당 파일은 존재하지 않습니다. Claude가 구동되어 파일을 생성할 때까지의 미세한 시차가 존재하므로, "존재하지 않는 파일"에 직접 Watch를 걸 수 없어 초기 에러가 발생합니다.

대응 전략

개별 파일 Watch는 불가합니다. ~/.claude/sessions/ 디렉토리 전체를 Watch하되, 이벤트 필터링 조건으로 event.path.filename() == format!("{}.json", child_pid)를 걸어야 안전합니다.

3. [좀비 및 데이터 오염] PID 재사용(Wrap-around) 검증의 실효성
결함 분석

현상: 설계 §11-4에서 PID 재사용 위험을 방지하기 위해 startedAt/updatedAt으로 검증하겠다고 했습니다.

문제점: Claude가 비정상 종료(Crash/Kill)될 때 ~/.claude/sessions/<PID>.json 파일을 지우지 못하고 남겨둘 가능성이 매우 높습니다. 이후 OS가 다른 프로세스나 새 Claude 에러에 동일한 PID를 할당(PID Wrap-around)했을 때, 백엔드가 보유한 child PID의 "실제 프로세스 시작 시간"을 정확히 모른다면 구별이 어렵습니다.

결과: 잘못된 시점에 남겨진 오래된 JSON 값을 현재 살아있는 세션의 ID인 줄 알고 읽어와 오염(Wrong Session Tracking)되거나 Fallback 메커니즘이 오작동할 수 있습니다.

대응 전략
Rust
// 구상 단계가 아닌 확정 코드로 검증해야 합니다.
let child_start_time = get_process_start_time(child.id()); 
// 우리 백엔드가 보관한 이 child_start_time과 
// Claude JSON의 `startedAt`을 비교하여 허용 오차(예: ±2초) 이내일 때만 유효한 파일로 인정해야 합니다.

4. [UX 결함] 자동 재시작 시 Replay Buffer 리셋과 스크롤백 증발
결함 분석

현상: 설계 §6에서 "새 세션 시작이므로 replay buffer 리셋(이전 출력은 끝났음 표시)"을 결정했습니다.

문제점: 사용자가 에이전트와 긴 대화를 나누던 중 에이전트가 크래시(Exited)나서 백엔드가 자동으로 재시작(Restarting -> Running)을 수행했다고 가정해 봅시다. 이때 프론트엔드의 Replay Buffer를 날려버리면 사용자 화면에서 이전까지의 대화 및 터미널 출력 이력이 순간적으로 '텅 빈 화면'으로 초기화(Flash)됩니다.

결과: 사용자는 "이전 대화가 보존되었다"는 안내 배너를 보기도 전에, 화면이 완전히 밀려버려 데이터가 유실되었다고 느끼는 최악의 UX를 경험합니다.

대응 전략

Replay Buffer를 하드 리셋(Clear)하면 안 됩니다. 터미널 스크롤백에 [시스템: 에이전트 비정상 종료로 인해 세션을 자동으로 재시작합니다...] 와 같은 의사 터미널 라인(Pseudo-line Break) 마커를 삽입하고 기존 출력은 뷰포트에 남겨둔 채, 그 아랫줄부터 새 세션의 출력을 이어 붙여야(Append) 합니다.

5. [인증 만료/신뢰 멈춤] Hang 상태 판정 스펙의 미정의
결함 분석

현상: 설계 §5에서 "재시작이 로그인 프롬프트에서 멈추는 것(인증 만료 좀비) 감지 필요"라고 서술했으나 구체적인 스펙이 누락되었습니다.

문제점: Claude 로그인 세션이 만료되었거나, 새로운 네트워크 환경이라 신뢰 프롬프트가 뜨면 프로세스는 Exit Code를 뱉지 않고 표준 입력(Stdin)을 기다리며 무한 대기(Hang)합니다. 이때 ~/.claude/sessions/<PID>.json의 status는 idle 또는 busy 상태로 고착됩니다.

결과: 백엔드는 프로세스가 살아있으므로 "정상 가동 중"으로 오판하고, 프론트엔드는 아무 반응 없는 터미널을 보며 무한히 대기하게 됩니다. 재시도 사다리(Retry Ladder)도 작동하지 않습니다.

대응 전략

구동 타임아웃 도입: 복원(Resume) 또는 생성(Fresh) 후 첫 5초 이내에 status가 정상적으로 순환하지 않거나, 특정 키워드(예: Login required, Trust this directory?)가 stderr/stdout에 포착되는지 스트림 스캐닝을 병행해야 합니다.

타임아웃 초과 시 자식 프로세스를 강제 Kill하고 재시도 카운터를 소모한 뒤 최종 실패 단계(User Notification)로 진입해야 합니다.

요약: 코어 구현 전 보완 체크리스트
설계 영역	발견된 취약점	수정 및 반영 사항
§1. 데이터 모델 & Spawn	Windows canonicalize 적용 시 UNC Prefix(\\?\)로 인한 경로 매핑 실패	dunce 크레이트 도입 또는 \\?\ 수동 제거 로직 추가
§11-2. sid 추적	개정된 파일 단위 Watch 방식이 Atomic Write(Tmp->Rename) 발생 시 유실됨	개별 파일이 아닌 sessions/ 디렉터리 전체 Watch로 아키텍처 수정
§11-4. 방어 원칙	PID 재사용(Wrap-around) 발생 시 오래된 잔재 JSON 오독 위험	OS 수준의 실제 Child Process Start Time과 JSON 내 startedAt 교차 검증 필터 추가
§6. 상태머신 충돌	Replay Buffer 리셋으로 인한 이전 터미널 출력 시각적 증발 (UX 불쾌감)	리셋 대신 '세션 끊김 마커 행' 추가 및 기존 버퍼 보존(Append)으로 선회
§5. 자동 재시작	로그인/신뢰 프롬프트 대기 시 프로세스가 죽지 않아 Hang 감지 불가능	기동 후 초기 상태 진입 타임아웃(5초) 가드 및 스트림 스캔 구현 명시
