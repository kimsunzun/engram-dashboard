# S9 최종본 재검토 — fable

**검토자:** fable (pane 8), 2026-06-11
**대상:** `session-restore-lld.md` (개정본, §7-R spike 결과 + §11 sid 확보 메커니즘)
**확신 수준:** [확실] / [가능성 높음] / [불확실]

---

## 종합 판정: GO — 단 §11에 운영 허점 1건(PID shim)과 §6 결정(프론트 재구독 권고)을 닫고 진행

1차 검토 지적이 충실히 반영됐고(분리 통제, atomic write, fallback 새 id, spike 확장), spike 실측으로 최대 불확실성(fork)이 해소됐다. 남은 것은 §11의 PID 동일성 가정 검증 1건과 §6 결정이다.

---

## (1) §11 sid 확보 메커니즘 — 구조 타당, 허점 1건 + 보강 3건

**타당성 [확실]:** "최초는 지정(캡처 제로) + 변경 시에만 결정적 파일로 재확보 + 어느 경우든 무손상"은 wezterm 실패(사후 캡처 비결정성)의 올바른 반전이다. 특히 sid 변경 감지가 *보조* 신호이고 정확성이 이 파일에 의존하지 않는 구조가 핵심 강점.

**허점 1-a. PID 동일성 가정 — PtyManager의 child PID가 그 파일의 PID라는 보장이 없다 [가능성 높음]**

Windows에서 `claude`는 설치 방식에 따라 shim(`claude.cmd` → cmd.exe → node)을 경유할 수 있다. 그 경우 **PtyManager가 보유한 child PID = shim 프로세스**이고, `sessions/<pid>.json`의 PID는 **실제 claude(node) 프로세스** — 둘이 다르면 `<우리 pid>.json` 조회가 영구 미스다. spike가 이 파일의 존재·갱신은 확인했지만, **"PTY로 spawn한 우리 child PID와 파일명 PID가 일치"를 같은 조건에서 확인했는지**가 §7-R에 없다.

**→ spike 추가 1줄:** PtyManager 경로로 spawn → `sessions/<child_pid>.json` 존재 확인.
**→ 불일치 시 결정적 우회(추측 아님):** `sessions/*.json` 전체에서 `sessionId == 우리가 지정한 claude_session_id`인 파일을 1회 스캔해 실제 PID를 학습 → 이후 그 파일 watch. 우리가 지정한 sid는 유일하므로 이 스캔은 결정적이다(§8이 금지한 "최신 파일 추정"과 다름). 대안: Job Object 프로세스 열거로 node 자손 PID 직접 확보.

**보강 1-b. 관측 즉시 persist [확실]:** §11-2의 sid 갱신은 **관측 즉시 atomic save**로 명시하라. `/clear` → 관측 → persist 전 크래시면 다음 복원이 pre-clear 세션을 살린다. 잔여 윈도(관측 자체의 지연) 동안의 같은 시나리오는 막을 수 없으니 "복원이 /clear 직전으로 돌아갈 수 있음"을 알려진 한계로 §11-4에 1줄 문서화 — 사용자가 clear한 대화라 피해는 경미하다.

**보강 1-c. watch는 파일이 아니라 디렉토리 [확실]:** 대상 파일이 교체(rename/recreate)되면 파일 단위 watcher는 핸들을 잃는 구현이 흔하다. `sessions/` 디렉토리를 watch하고 파일명 필터링. 읽기는 claude가 쓰는 중일 수 있으니 공유 위반 시 짧은 재시도.

**보강 1-d. 부수입: `status` 필드 [불확실 — 보너스로만]:** 파일에 status가 있다면 spike #6/#7의 "프롬프트 대기 좀비" 감지 보조 신호로 쓸 수 있다. 단 비공식 필드이므로 감지 *보조*로만, 트리거로는 금지.

기타: §7-R에 spike #5(`--resume`+`--session-id` 조합) 결과가 빠져 있다 — fork 안 함 확정으로 무의미해졌으니 "불요(#4로 대체)"로 한 줄 닫아라.

---

## (2) ★§6 상태머신 — 권고: 프론트 재구독 + epoch 필드★ [확실]

**구독자 승계(이주)가 아니라 프론트 재구독을 권한다.** 근거:

1. **확정 설계 2개에 대한 최소 변경.** 재구독안에서 백엔드 변경은 "재시작 시 같은 AgentId로 새 PtySession을 맵에 교체 삽입"뿐 — PtySession의 terminal 상태·자원 정리 서사(확정 LLD §9·§11)가 그대로 산다. 승계안은 백엔드에서 가장 예민한 영역(drain↔subscribers)에 **신규 동시성 경로**(옛 drain의 마지막 send와 sink 이주의 경합, SinkId 재배치)를 추가한다 — 1단계에서 락 규칙을 그토록 다듬은 곳이다.
2. **프론트는 어차피 재시작을 알아야 한다.** 배너/배지(§4 UI 신호)는 승계안에서도 필요하므로 "프론트 무지" 이점은 성립하지 않는다. 재구독은 이미 검증된 경로(frontend C2 반영으로 terminal.reset + replay가 idempotent)를 그대로 탄다 — agent 전환과 동일 코드.
3. **실패 모드의 가시성.** 재구독 누락은 "멈춘 터미널 + 배지"로 보인다. 승계 경합 버그는 조용한 출력 유실/중복으로 나타난다. 운영에서 전자가 압도적으로 싸다.
4. 비용은 IPC 1회 + 빈 replay 재생뿐 — 재시작은 어차피 시각적 단절(배너)이 있는 이벤트다.

**구현 키 — `epoch: u32`:** AgentInfo·agent-status-changed payload에 재spawn마다 증가하는 epoch를 추가하고, 프론트 subscribe effect 의존성을 `[agentId, epoch]`로. status 변화만으로는 effect가 재실행되지 않으므로 epoch가 재구독의 결정적 트리거가 된다. 멀티 창도 각자 이벤트 수신 → 각자 재구독으로 균일.

**부속 결정 2건:**
- **재시작 실행 주체:** drain thread가 terminal 전이 후 직접 respawn하지 말 것 — drain이 spawn/join/맵 교체까지 하면 자기 수명과 얽힌다. drain은 manager에 신호(채널/큐)만 보내고, **전용 restart 태스크**가 사다리(resume 2회→fresh→정지)·backoff·respawn을 수행. 백엔드 LLD 개정 시 스레드 목록에 1행 추가.
- **stale unsubscribe 무해화:** 재시작 직후 프론트의 옛 sinkId unsubscribe가 새 세션에 도착 — retain은 not-found여도 에러 없이 통과하도록(이미 그렇게 설계됨, 명시만).

replay 리셋(§6) 동의. 배너는 프론트가 reset 후·replay 전후 순서를 정해 쓰도록 frontend LLD 개정에 포함.

---

## (3) sessions/<pid>.json 비공식 의존 — 수용 가능 [확실], 단 4개 조건부

수용 가능한 이유는 설계 자체에 있다: **정확성의 1차 근거가 아니라 drift 보정용 2차 신호**이고, 파일이 사라지거나 포맷이 바뀌어도 "최초 지정값 → resume 시도 → 실패 시 fallback"으로 무손상 강등된다. 비공식 의존이 위험해지는 건 정확성이 거기 걸릴 때인데, 이 설계는 안 걸려 있다.

조건:
1. **version 필드 게이트:** 파일의 version이 기대 밖이면 추적만 끄고(로그) 강등 — 파싱 실패를 추적 중단 신호로 처리.
2. **watcher feature-flag/설정 토글:** claude 업데이트로 포맷이 깨졌을 때 코드 배포 없이 끌 수 있게.
3. **spawn 직후 self-test:** `<pid>.json` 존재 + sessionId == 지정값 확인, 불일치 시 큰 로그 — 가정 붕괴(포맷 변경·1-a)를 첫날 감지.
4. **문서에 등급 명시:** "best-effort enrichment, correctness 의존 금지"를 §11-4에 이미 쓴 대로 유지 — 향후 누군가 이 파일로 기능을 *확장*하려 할 때의 방어선.

---

## (4) 놓친 것

1. **[보안, 확실] `profile.env`가 평문 JSON으로 persist된다.** 사용자가 env에 API 키/토큰을 넣는 순간 `agents.json`에 평문 저장 — 백업/동기화로 유출 표면이 된다. 최소: 문서에 "env에 자격증명 금지" 명시 + 저장 시 알려진 패턴(`*_KEY`, `*_TOKEN` 등) 경고. 이상적: 자격증명 류는 persist 제외 목록.
2. **CLAUDE_CONFIG_DIR 환경변수** — `~/.claude` 위치가 이 변수로 이동 가능하다. sessions 경로 해석 시 고려(우리 env 주입과 상호작용 포함). [가능성 높음]
3. §7-R spike #5 결과 누락 (상기 — 한 줄로 닫기).
4. epoch 필드 (§(2) — 백엔드·프론트 LLD 개정 항목에 포함).
5. restart 전용 태스크 (§(2) 부속 — 스레드 목록 개정).
6. 재시작 배너와 replay의 순서 명시 (frontend LLD 개정 항목).

---

## 진행 순서 동의 + 한 줄 수정

§9의 "② 상태머신 개정 → ③ 코어 구현" 순서에 동의하며, ②에 들어갈 백엔드 LLD 개정 항목을 확정하면: **(a) 재시작 시 같은 AgentId 새 PtySession 맵 교체 + epoch, (b) restart 전용 태스크(스레드 목록), (c) stale unsubscribe 무해, (d) StatusSink에 restore-result/epoch 반영.** 프론트 LLD 개정: **(e) [agentId, epoch] 재구독, (f) 배너 순서, (g) Restarting 표시(또는 Exited→Running 재진입 허용).** 이 7건이면 §6 충돌은 닫힌다.
