# S33 glossary — Korean → English

Every translation batch uses these renderings. A batch that needs a new term adds a row here in the same commit.

| Korean | English | Note |
|---|---|---|
| 동사 (2동사) | verb (2 verbs) | ADR-0001 metaphor for separate operations |
| 격리 | isolation · isolating X from Y | |
| 합류 (스레드) | join / joining | thread join |
| 자원 폐쇄 | resource closing | |
| 전이 | transition | terminal 전이 = terminal transition |
| 발행 / 단독 발행 | emit / emitted by X alone | events and notifications |
| 발행 주체 | emitter | |
| 원자 게이트 | atomic gate | |
| 정확히 1회 | exactly once | |
| 과도기 | transitional | |
| 자연 종료 | natural exit | |
| 중복·경합 | duplicates and races | |
| 확장 enum | extensible enum | |
| 교체 가능 / 교체성 | swappable / swappability | |
| 대원칙 | core principle | |
| 결합 | coupling / coupled to | |
| 주입 | inject | |
| 명령 조립 / 인자 조립 | command assembly / argument assembly | |
| 전송 경로 / 전송 방식 | transport path / transport method | |
| 실측 | measured | |
| 재편 | restructure | |
| 판정 | decide / decision | 판정 금지 = must not decide |
| 프론트 | frontend | |
| 셸 | shell | the Tauri app process |
| 데몬 | daemon | |
| 우편 | mail | agent-to-agent messaging |
| 슬롯 | slot | |
| 게이트 | gate | |
| 미리 깐다 | lay down early | |
| 폐기 (in prose, not the status line) | discarded | the status word `폐기` stays Korean |
| 깡통 (에이전트) | empty shell | reserved agent with no process |
| 예약 / 대기 | reserved / waiting | |
| 죽음 | death | |
| 복원 / 부팅 복원 / 콜드 부팅 | restore / boot restore / cold boot | |
| 되살리기 / 깨우기 | revive / wake-up | |
| 폭주 (가드) | runaway (guard) | |
| 강등 / 무손상 강등 | degrade / degrade without damage | not "damage is contained" |
| 회귀 | regression | |
| UI 실행 | launching the UI | the app start, not "a UI action" (S33 batch 2 review) |
| X 이하 | X and everything below it | includes X itself |
| 단일 발행 (파일) | single writer | exclusive ownership, not "one publication" |
| 상주 / 생사 | resident / liveness | |
| 재입양 / 재발견 | re-adoption / rediscovery | |
| 맹아 | seed | |
| 보강 (절) | addendum | |
| 구독 (X 구독) | subscribes to X | "eventBus.ts(agentClient 구독)" = eventBus subscribes through agentClient; check the direction |
| 자동 재시작 (supervision) | auto-restart | check who restarts what: "데몬 사망 시 UI 자동 재시작" = the UI restarts the daemon |
| X 독립 (단위) | independent per X | not "X-independent" (that reads as "unrelated to X") |
| 스택 흔적 0 | no stack-specific content | tech-stack neutrality, not stack traces |
| A + B N곳 | A and B, N places in total | the count covers the whole list |
| 자리만 | placeholder only | |
| 적대 리뷰 / 탈출구 | adversarial review / escape hatch | |
| A→B→B 불량 시 C | A→B→(C if B is poor) | one step, not two |
| X은 Y가 … 건너뛰어짐 | X is skipped because Y … | the topic (X) is what gets skipped |
| 명부 / 로스터 | registry / roster | registry = all agents, roster = alive now |
| 장부 / 보관함 / 봉투 | ledger / mailbox / envelope | |
| 파킹 / 반려 | parking / bounced | |
| 유예 (flush) | deferral / deferred | not "grace" |
| 창구 (데몬) | endpoint | not "window" |
| A와 B 제외 (수식어 공유) | excluding A and B that … | a shared qualifier covers both items |
| 화신 / 화신 표식 | incarnation / incarnation marker | |
| 자취 / 주인 | trace / owner | command registry |
| N라운드 (리뷰) | round N | "3라운드" = round 3, not three rounds |
| 0 자리채움 | zero placeholder | the whole value is zero, not zero-padded |
| 규격대로 뚫다 | cross as the spec allows | not "against its spec" |
| 살림 (데몬) | lifecycle management | not "liveness" |
| 파서 (폴더를 파서) | create (a folder) | 파다, not "parse" |

Kept in Korean: the ADR lint tokens (`- 상태:` + vocab, `- 관련:`, section headings), text in 「」, names of CLAUDE.md and `.claude/` sections.
