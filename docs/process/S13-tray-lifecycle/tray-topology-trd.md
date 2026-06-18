# S13 — 트레이/프로세스 토폴로지 TRD (검토용 초안)

작성 2026-06-18. PRD 의견(사용자 대화) + `/consult` 3종 교차검증(job `20260618-151957-consult-tray-daemon-topology`) 종합. **구현 전 사용자 검토 대기 상태.** 굵은 결정은 확정 시 ADR로 박는다(아직 ADR 미작성).

## 0. 결정 요약 (무엇/왜)

**3프로세스 토폴로지 — 단, tray-host는 WebView 없는 순수 Rust 트레이 유틸.**

```
[tray-host]  순수 Rust(tray-icon + tao, WebView 없음). 시작프로그램 등록. 런처+제어 유틸.
   ├─ ensure/launch ─▶ [daemon]  detached, self-owned. 에이전트 보유. WebView 없음.
   └─ launch/show/hide/kill ─▶ [UI 앱]  Tauri(React). 누를 때만 WebView 뜸.
```

- **왜 3프로세스(tray-host 분리)가 정당한가:** tray-host의 목적은 "크래시 생존"이 아니라 **로그인 상주 런처/제어 유틸**(데몬·UI를 띄웠다·감췄다·없앴다 + 상태 관찰). 교차검증(GPT·Gemini)이 명시한 "tray-host 정당 조건 = WebView를 안 쓰는 순수 Rust 트레이 / 로그인 때 WebView 비용 회피"에 정확히 부합. login 때 무거운 WebView2를 안 띄우고, UI는 필요할 때만 spawn.
- **교차검증이 반대한 것은** "Tauri 앱(WebView)을 tray-host로 하나 더 띄우는" 안이었음 → 그건 채택 안 함(순수 Rust tray로).

## 1. 교차검증이 적출한 필수 수정 (모드 무관, judge 검증)

| # | 수정 | 폐기된 오답 |
|---|---|---|
| C1 | **데몬은 누구의 Job 자식도 아님(detached, self-owned).** tray-host/UI 누가 띄우든 lifetime 독립. | "데몬=tray-host 자식 + KILL_ON_JOB_CLOSE" → tray-host 죽으면 데몬도 죽어 생존 위반(Gemini 오답) |
| C2 | **생사 감지 = WS connection liveness(1차) + lockfile{PID, port, generation/nonce}(재발견·stale 판정).** 재발견 시 **PID 생존·port 응답·generation 일치를 검사해 stale lockfile이면 정리 후 fresh spawn**(좀비 lock 오인 방지, consult race 5). | "부모-자식 exit 신호만으로 polling 불필요" → tray-host 재시작 시 핸들 끊김, 재발견 불가(Gemini 과장) |
| C3 | **데몬 desired-state는 owner(tray-host)가 소유. UI는 직접 ensure 금지, 연결만.** 현재 UI bootstrap이 데몬 ensure → 리팩터. | "UI가 데몬 직접 spawn" → 다중 ensure race·고아 판단 주체 소실 |
| C4 | **전체 종료 순서: ensure/spawn 차단 먼저 → 데몬 graceful drain(ack, 타임아웃+Job 강제폴백) → UI 종료 → tray-host 종료.** | "UI 먼저 종료" → 그 사이 재spawn race(자동재시작 폐기 사유 재발) |

크래시 자동 재실행(supervision)은 **보류** — 수동 "UI 열기"가 이미 복구 경로. 빈발하면 그때 ADR.

## 2. 프로세스 책임

### tray-host (신규, 순수 Rust)
- 시작프로그램 진입점. 트레이 아이콘 소유.
- **lifecycle owner**: 데몬 ensure(detached spawn)·정지, UI launch/show/hide/kill, 전체 종료 coordinator.
- 데몬 desired-state(Running/Stopped/Stopping) 보유.
- 상태 관찰: lockfile + port-ping(v1) / WS control(향후). 메뉴에 "데몬: 실행 중/꺼짐" 반영.
- **데몬·UI를 detached로 spawn**(자기 Job에 안 넣음) → tray-host 죽어도 둘은 생존.

### daemon (기존 crate, 변경 최소)
- 에이전트 코어 보유. detached, self-owned.
- 싱글톤 lock(lockfile: PID·port·generation/nonce). 누가 ensure하든 살아있으면 no-op.
- 고아 방지: idle self-shutdown(연결 0 ∧ 에이전트 0이 N초 지속). tmux 모델(ADR-0021).
- graceful stop: 새 명령 거부 → PTY 자식 정리(ADR-0001 인과) → lock 제거 → exit. 상한 타임아웃 후 Job 강제.

### UI 앱 (기존 Tauri, 변경)
- React 창. 순수 I/O.
- 부팅 시 **데몬 직접 ensure 안 함**(C3) — owner가 띄운 데몬에 **연결만**. 연결 실패 시 owner에 ensure 요청(또는 owner가 이미 보장).
- X 버튼 = 최소화(hide, 프로세스 유지). 단일 인스턴스.
- "전체 종료" 의도(reason=full_shutdown) 수신 시 reconnect/ensure 중단.

## 3. 트레이 메뉴 (v1)
- **데몬 시작** — owner가 detached 데몬 ensure(UI 없이 백그라운드).
- **UI 열기** — UI 살아있으면 show/focus, 없으면(첫 실행/UI 종료 후) spawn.
- **UI 종료** — UI 프로세스만 종료(데몬 생존). ※X=hide와 다름.
- **전체 종료** — §1 C4 순서로 데몬+UI+tray-host 모두 down.
- (세분 항목은 나중에 add — 트레이는 `add(command)` 식.)

## 4. 커맨드 표면 (§5 손발/두뇌)
- 실행 로직 = **백엔드측 핸들**. lifecycle 핸들(launch UI·ensure 데몬·전체 종료)은 **tray-host 소유**, 에이전트 핸들(spawn/kill/write)은 **daemon 소유**.
- 호출자: 트레이 메뉴 · UI 버튼 · LLM · (미래)단축키 — 모두 같은 핸들 호출.
- **[열린 사항]** UI/LLM이 tray-host의 lifecycle 핸들을 부르는 IPC 경로(tray-host가 control endpoint를 열까, 아니면 daemon 경유?). TRD 상세에서 확정.

## 5. 라이프사이클 시퀀스

**부팅(로그인):** tray-host 시작 → 트레이만 표시(WebView 0). (옵션) 데몬 자동 ensure 여부 = 설정. v1 기본: 데몬은 "데몬 시작"/"UI 열기" 때 ensure.

**UI 열기:** tray-host → UI 프로세스 살아있나? → 있으면 show/focus, 없으면 spawn → UI가 데몬에 연결(없으면 owner가 먼저 ensure).

**창 X:** UI hide(프로세스 유지). tray-host 무관.

**UI 종료(트레이):** owner가 UI 프로세스 종료. 데몬 생존.

**전체 종료(트레이):** owner state=Stopping → ensure/open/start 차단 → UI에 full_shutdown(reconnect=false) → daemon graceful(ack, 타임아웃) → daemon exit 확인 → UI 종료 → tray-host 종료.

## 5b. 강제 종료/고아 커버리지 매트릭스
trade-host는 OS 부모가 아니라 제3자(명령 전송)다. 따라서 강제 종료 시:

| 죽인 대상 | 결과 | 커버 |
|---|---|---|
| tray-host 강제kill(작업관리자) | 데몬·UI 생존(detached). graceful 명령 안 감 = 의도됨 | 재실행 시 lockfile로 **재입양**(C2). 데몬 idle면 self-shutdown(C1) |
| UI 강제kill | 데몬 생존. tray-host 무관 | 트레이 "UI 열기"로 재spawn → 재연결 |
| 데몬 강제kill | UI 연결 끊김('down'). 자동재시작 없음 | 명시 "데몬 시작"으로만 부활(ADR-0021). lockfile은 stale → 다음 ensure가 정리 |
| 전원 강제kill, lockfile만 잔존 | — | 다음 실행이 PID/port/generation 검사로 stale 판정→정리→fresh(C2) |

**원칙:** 강제 종료로 데몬이 남는 건 버그가 아니라 "작업 생존" 설계. 단 (a) 재입양으로 다시 잡히고 (b) idle면 self-clean (c) stale lock은 검사로 회수 — 이 3개로 Discord식 "관리 불가 좀비"가 안 되게 한다. 살아있는 에이전트를 든 데몬은 의도적으로 영속(재실행으로 재연결, 정 끄려면 "전체 종료"/작업관리자).

## 6. 모듈/파일 윤곽 (예상)
- **신규 crate/bin** `engram-tray-host` (또는 src-tauri 외 별 bin): tray-icon + tao, 런처, lockfile 관찰, lifecycle 핸들.
- **daemon**: lockfile에 generation/nonce 추가(C2), idle self-shutdown(C1 보강), graceful drain 정비(C4).
- **UI(src-tauri/lib.rs)**: CloseRequested → hide+prevent_close(현 `app.exit(0)` 번복, 664d629 조정). bootstrap의 데몬 ensure 제거(C3) → 연결만.
- **프론트(clientFactory/App.tsx)**: `bootstrapDaemonIfNeeded` 제거/이관, full_shutdown 수신 시 reconnect 차단.

## 7. 리스크 / 비용 (교차검증 (c))
진입점을 앱→tray-host로 바꾸는 비용은 실재하고 과소평가 금물:
- 자동시작 등록(경로/인자), 단일 인스턴스 가드(tray-host + UI 두 겹), 딥링크(`engram://`) 수신·포워딩, 업데이터(상시 tray-host 자기교체), 코드사이닝(바이너리 +1).
- → ADR에 마이그레이션 항목으로 명시.

## 8. 미해결 (구현 착수 시 결정)
- **[결정됨] 데몬 부팅 자동 ensure:** 기본 = **(b) on-demand**("데몬 시작"/"UI 열기" 때만 ensure, ADR-0021 정신). **자동시작(로그인 때 데몬 바로 ensure)은 설정 옵션으로 여지만 둠**(기본 off).
- **[절반 해소] lifecycle 핸들 IPC 경로:** **데몬 제어 = 데몬의 기존 WS 서버 + lockfile 토큰**(tray-host가 인증 클라이언트로 StopDaemon 등 전송, force는 taskkill 폴백 = 현 `DaemonControl.stop`과 동일). **UI 제어(launch/hide/kill)** = tray-host가 OS spawn + 소형 신호로 별개 처리(경로 미확정).
- tray-host = 순수 Rust(tray-icon+tao) vs Tauri-tray-only(no window). 전자 권장(가벼움).
- 단일 인스턴스/딥링크/업데이터 상세(§7).

### 데이터 위치 (data_dir) — 결정됨
- **기본 = engram 내부 폴더**(현 `%APPDATA%\com.engram.dashboard` 대신). embedded·daemon 둘 다 같은 폴더를 봐서 프로필(agents.json) 공유 유지. **영속 유지(휘발 X).**
- **위치 swappable:** 현재 "테스트용"으로 표기된 `resolve_data_dir()` override를 **정식 설정 노브로 승격** → 나중에(특히 보안 필요 시) 외부 폴더로 한 줄 이전 가능("꺼내기"). 설치형은 미계획이나 경로 가정은 안 박음.
- **폴더 = `.engram-data/`** (repo 루트, `.gitignore` 등록 완료). daemon.json(토큰)·agents.json 등 런타임 데이터 일체. 토큰 git 커밋 위험 차단.
- **경로 해소:** 데몬은 별 프로세스(`target/...`)라 repo 루트를 스스로 찾기 애매 → **런처(tray-host)가 data_dir 절대경로를 결정해 override 노브로 데몬·UI에 주입**(override 정식 승격과 연결). 경로 단일 출처 + 나중 외부 이전 용이.
- **embedded = 영속(휘발 아님)** — 단 트레이·daemon.json은 데몬 모드에서만 생성(embedded엔 데몬 없음).

### 제어 메커니즘 (데몬 "허락" 모델)
- **협조적 제어(graceful):** 데몬이 시작 시 lockfile에 {port, token} 기록 = 사전 허락. tray-host가 토큰으로 인증 후 명령 전송, 데몬이 스스로 처리(자식 정리→자진 종료). 데몬이 control 표면을 노출했기에 가능.
- **강제 제어(force):** OS taskkill(같은 사용자면 허락 불필요), 단 고아 위험 → 폴백 전용.

## 9. 다음
사용자 검토 → 확정 시 **ADR 2개**: ① 트레이/프로세스 토폴로지(3프로세스, 순수-Rust tray-host, 664d629 조정) ② 데몬 소유·생사·종료 모델(C1~C4). 그 후 상세 TRD → 코더.
