# 구조 선택지 — "에이전트 죽음"의 백엔드 무관 추상 (Slot ≠ Run ≠ Transport)

> **결론(2026-06-16): 사용자가 (A) 채택 → ADR-0017로 확정.** 슬롯=한 모드의 한 세션, 끝나면 슬롯도 끝(슬롯 내 백엔드 교체·zombie 슬롯 없음). 터미널(셸)은 저장/복원 안 함(이어받을 세션 없음). consult가 민 (나) 슬롯-persist 컨테이너 모델은 **도메인 혼란(in-place 모드교체·dead pane)으로 기각.** 단 consult 합의 중 "백엔드별 죽음 정의 + exit-reason 분류 + KilledByUser 재시작제외 + API stream≠death"는 채택. 아래 (가)/(나) 비교·OSS 조사는 이력으로 보존. 상세: ADR-0017.

- 상태: **해소됨 → ADR-0017 (A 채택)**
- 근거: `/consult` job `20260616-011502-consult-death-abstraction` — GPT·Gemini·Claude(opus) 블라인드 + judge. 유명 OSS 조사 포함. 원자료 `I:\Engram\agents\web-runner\shared\<job>\`.
- 연결: ADR-0016(에이전트 수명 모델)의 "sid 인스턴스"가 여기서 말하는 **Run**에 해당 — 그 위에 **Slot** 계층을 추가하는 구조 정정.

## 배경 — 왜 "죽음"이 애매했나
현재 `cmd.exe /c claude`라 claude가 빠지면 cmd도 끝나 PTY EOF → **세션(슬롯)이 통째로 사라짐.** 그래서 "claude 빠짐"과 "슬롯 소멸"이 한 사건이 됨. 사용자가 "claude 죽어도 터미널/슬롯은 남아야 하고, 그 자리에 codex로 바꿔 띄울 수도 있어야" 한다고 지적 → "죽음"을 백엔드 무관 추상으로 다시 정의해야 함.

## ★ 교차검증 합의 (3종 + judge 일치 — 거의 확정) ★
1. **데이터 모델 = Slot ≠ Run ≠ Transport 3계층 분리.**
   - **Slot**: UI/도메인 슬롯. 데몬이 보유, persist, 오래 산다. 안정적 `slot_id`.
   - **Run(=ADR-0016의 sid 인스턴스)**: 한 번의 실행(claude/codex/shell/api). 시작~종료(코드/시그널 포함). 짧게 산다.
   - **Transport**: PTY / HTTP stream / None. Run과 묶임.
2. **"죽음"을 Slot이 아니라 Run의 "종료 사건"으로 정의.** 나쁜 추상="Session is dead" / 좋은 추상="Slot은 살아있고, Run이 reason과 함께 끝났고, Transport는 Open/Eof/Closed."
3. **exit-reason 분류 필수**(EOF 하나로 죽음 정의 부족): Completed(0) / Failed(≠0) / Signaled / KilledByUser / InterruptedByUser / TransportLost / StartupFailed / RestoreFailed. (systemd·supervisord 선례)
4. **API는 stream-end ≠ death** — 정상 스트림 종료=Completed이지 Dead 아님. (백엔드 무관성 핵심)
5. **KilledByUser / Interrupted는 respawn·자동재시작 제외.** interrupt(Ctrl-C)와 kill(프로세스레벨)은 계속 분리(ADR-0001 유지).
6. **자동재시작은 미래 옵션** — UI=뷰어·자동복원 금지 제약상 지금은 death 관측만, respawn은 명시 트리거. (켜면 backoff/FATAL/rate-limit 필요 — supervisord/systemd 선례)
7. **추상(타입/이벤트)은 지금 (나) 3계층으로 깐다**(저위험·장기 인터페이스, CLAUDE.md §0). **실동작은 (가)에 머물 수 있다**(점진 이행).
8. **★Windows 구현 = cmd /k 아님, "Rust Slot 수명 격상"★** — 셸을 살려두는 `cmd /k`는 위험(TUI raw-mode 오염·PTY EOF 안 옴→child watcher 필요·JobObject가 셸까지 잡음). 대신 **`cmd /c` 그대로 두고**, inner 종료로 PTY가 EOF 나면 **Rust의 `AgentSlot` struct를 드롭하지 않고 `state: Dead`로 유지**(PTY는 닫힘). 이러면 EOF 신호원·kill 인과(ADR-0001)·finalize 1회(ADR-0005) 불변식을 **보존**하면서 슬롯 유지+respawn UX를 얻음. (tmux dead-pane과 동형 — 죽은 건 process, 슬롯은 메모리에 남음.)

## OSS 선례 (조사)
| OSS | inner 종료 후 | 재실행 | 비고 |
|---|---|---|---|
| tmux `remain-on-exit` | dead pane 유지 | `respawn-pane -k` | SlotId 유지·Run만 교체 (가장 근접) |
| GNU screen `zombie` | dead window | resurrect 키, `onerror`=비정상만 | 정상/비정상 구분 선례 |
| Zellij command pane | exit status 표시·hold | Enter rerun, `close_on_exit=true`로 (가) | (나) 기본+(가) 옵션 |
| VS Code terminal | (Terminated) 탭 유지 | Relaunch 버튼 | dead 슬롯+명시 재실행 핸들 |
| abduco/dtach | 세션 `+`(inner 죽고 세션 유지) | 사용자 재spawn | **persist 데몬 모델 최근접** |
| supervisord/systemd | Unit failed/dead, PID와 분리 | `restart`(명시) | exit-reason 분류·자동재시작 정책 선례 |

## ★ 사용자 결정 대기 — 선택지 ★
합의(위)는 깔되, 아래는 골라야 함:

**S-1. 실동작 채택 범위 (가 vs 나, 시점)**
- (a) **추상만 지금**(타입·이벤트=(나)), 실동작은 (가)(inner 종료=슬롯 닫힘) 유지 → dead-slot 실구현은 후속. *보수적.*
- (b) **claude/codex 슬롯부터 KeepDeadSlot 실동작 도입**(Rust Slot 격상) → 죽어도 슬롯 유지·respawn. *적극적.*

**S-2. `on_exit_policy` 기본값 (백엔드별 차등 가능)**
- claude/codex = `KeepDeadSlot` / shell = `CloseSlot` 또는 KeepDead / api = `Idle`(stream end=Completed). → 차등으로 갈지, 일괄로 갈지.

**S-3. respawn 1차 지원 범위**
- same command(claude→claude resume) / different command(claude→codex 교체) / drop-to-shell / restore-with-resume — 어디까지 1차로.

**S-4. dead 슬롯 보존 수준**
- 마지막 화면(scrollback)·layout·exit status를 dead 슬롯에 얼마나 들고 있을지.

## 권고 (단정 아님 — 선택지)
judge 종합 권고 = **데이터 모델·exit-reason 분류는 GPT 안 그대로, 구현 경로는 "cmd /c 유지 + Rust AgentSlot 수명 격상"**(Gemini 정답). 이게 (나)의 UX를 ADR-0001/0005 불변식·Windows PTY 손상 없이 얻는 유일한 길. S-1은 (a)(추상 먼저)로 시작해 §0(저위험·장기는 미리)에 맞추고, S-2는 claude/codex=KeepDeadSlot 지향 권고.

## 다음
S-1~S-4 사용자 선택 → **ADR-0017**(세션/죽음 구조: Slot≠Run≠Transport, death=Run 종료사건, Windows=Rust Slot 격상) 확정 → 변수/타입 스키마를 ADR-0016과 합쳐 코더로 구현(동작은 단계적).
