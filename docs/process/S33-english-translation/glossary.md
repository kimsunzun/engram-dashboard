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
| 창구 | endpoint (or channel) | any receiving/sending entry: daemon route, write path, LLM surface; never "window" |
| A와 B 제외 (수식어 공유) | excluding A and B that … | a shared qualifier covers both items |
| 화신 / 화신 표식 | incarnation / incarnation marker | |
| 자취 / 주인 | trace / owner | command registry |
| N라운드 (리뷰) | round N | "3라운드" = round 3, not three rounds |
| 0 자리채움 | zero placeholder | the whole value is zero, not zero-padded |
| 규격대로 뚫다 | cross as the spec allows | not "against its spec" |
| 살림 (데몬) | lifecycle management | not "liveness" |
| 파서 (폴더를 파서) | create (a folder) | 파다, not "parse" |
| 「…」이 없다 (quote + negation) | there is no 「…」 | keep the negation in the sentence, not only in the gloss |
| X를 들어 주다 | lift / waive X for us | not "make X apply" |
| 거두다 / 거둔 (입력, ✕) | withdraw (with ✕) | not "take in" or "collect" |
| 갈려 나가다 | be replaced | not "ground away" |
| N판 (문서) / N회차 (리뷰) | Nth edition / round N | "deep 4회차 전원 PASS" = all reviewers passed in round 4 |
| 명단 (목록) | list | 명단이 아니다 = not a complete list; roster only for 로스터 |
| 가능성 높음 / 불확실 / 확실 | likely / uncertain / certain | confidence labels, not "confidence high" |
| 쌍둥이 (같은 이름) | twins | "@all 쌍둥이는 둘 다 받는다" = both twins receive it |
| 목적 함수 (platform) | purpose-specific function | not "objective function" (ADR-0266/0295) |
| 크롬 보고 | chrome reporting | app window chrome, not Google Chrome |
| 가리다 | tell apart / distinguish (or cover) | decide by context |
| 끄는 | turning off (끄다) or dragging (끌다) | decide by context |
| 녹 / 빨강 (측정) | green / red | "M9 · M10 녹" = M9 and M10 passed; not "folded in" |
| \<id\> 앞 (파킹분·드레인) | addressed to \<id\> | not "in front of" (no queue position meant) |
| N차 (개정) | Nth amendment | amendment generation, not a review round |
| 칸 | area (UI / layout region) · field (struct, JSON, wire) · entry (list) | never "slot" — slot is 슬롯 |
| 바람 (숨은 창의 최대화) | desire | the deferred-maximize record; 거두다 on it = clear the desire |
| 재기 | measurements | UI metrics; not timers |
| 떠 두다 | copy aside / capture | `copy_aside`; "captured at launch" for spawn settings |
| 맡기다 / 맡김표 / 되물림 | hand over / claim ticket / take back | prewarmed-window handover |
| 네트워크 행 | network layer | the `net` crate, not a row |
| 독 (락) | poison | not "dock" |
| 배치 (항목·사본) | placement | in the queued-input code; not "batch" (배치 of a layout = layout) |
| 부재 막 / 막 | absence curtain / curtain | the slot overlay; not "veil" |
| 물높이 | fill level | byte budget in transport; not "water level" |
| 앞잡이 줄 / 앞잡이 버퍼 | read-ahead queue / read-ahead buffer | `Policy::frame_buffer`, drained by `salvage_read_ahead`; not "front queue" |
| 몫 | share (byte / permit budget) · role, part, job (otherwise) | do not use "share" for a duty |
| 창 상한 | window cap | transport resume window |
| 유예 (bounded wait) | grace period | a wait with a timeout before a fallback; the flush sense stays "deferral" |
| 칸 (connection-count permit) | permit | upgrade / auth-wait / operating permits in transport |
| 도착순 줄 / 우선 줄 | arrival-order lane / priority lane | |
| 수신 큐 | inbound queue | |
| 회귀망 | regression net | |
| 통로 | link | transport connection |
| 감독 | supervisor | transport peer supervisor task |

Kept in Korean: the ADR lint tokens (`- 상태:` + vocab, `- 관련:`, section headings), text in 「」, names of CLAUDE.md and `.claude/` sections.
