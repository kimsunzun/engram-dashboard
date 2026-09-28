# TRD — 챗 화면 4건: 스크롤 따라가기 · Esc 끊기 · 도구 호출 묶기 · claude 글자 스트리밍 (S21)

> 상태: **7판(2026-09-28) — 사용자 결정 ⑬: Esc 끊기를 누르면 곧바로 「중단하는 중…」을 보이고 멈출 때까지 Esc 를 무시한다 · 백엔드 무변경(ADR-0244 · §3-1 · §3-2 · §3-6 · §7 FE-1.4 · §8-2 F2 ⑩ · §9 · §10 · §11 ⑬) · 사용자 결정 ⑭: 터미널 모드(PTY)는 끊기 명령을 지원하지 않는다 — Ctrl-C 를 보내지 않는다(ADR-0245 · §3-1 · §3-3 · §7 B3c · §9 · §10 · §11 ⑭) · 7판 문서 묶음(앞선 리뷰 후속 — §3-1 · §4-3 · §4-7 ⑨ · §8-2 F2 ⑦ · F3 ④ · §12) · 코드 무변경.** 6판 이전 기록은 아래 그대로다. **6판(2026-09-28) — 사용자 결정 ⑫: claude 끊김 합성 줄 = 말풍선 대신 끊김 표시 행 · 그 턴의 중단 행 생략(ADR-0243 · §3-4 · §6 · §7 B3 · FE-2c · §8-2 F2 ② · §9 · §10 · §11 ⑫) · 6판 doc 리뷰 FIX 반영(§12) · 코드 무변경.** 5판 이전 기록은 아래 그대로다. **5판(2026-09-27) — 4판 리뷰 FIX 반영 · 사용자 결정 U7 · 업스트림 태그 판독으로 사실 정정 · 5판 light 재검 FIX 반영 · 채취 둘째 = 사용자 결정 ⓐ(그대로) · Claude Code Esc 실측 · 5판 좁은 재검 FIX 반영 · U8 중단 줄 강조 · claude 끊긴 도구 = ⓐ 와 같은 규칙(메인 · 위임) · 코드 무변경.** 5판 light 재검 FIX(§12) = `Declined` 사유 문구 「실행되지 않음」(사용자 결정) · 채취 셋째의 파일 변경 멈춤 · 끊기 서술 정정(벤더 실험 terminate API — §9) · 채취 둘째 = 사용자 결정 ⓐ(그대로 — 끊긴 뒤 늦은 `failed` 는 보통 붉은 「오류」 · §11 ⑩ · 2026-09-27) · Claude Code Esc 실측(대화형 CLI 의 Bash 도구는 Esc 에 죽는다 — §3-5 · §9) · 정합 수정. 5판 = 4판 `/review trd full` FIX(§12 — 결말을 둘로 나눔: 우리 거절 `Refused` · codex 스스로의 거부 `Declined`, 사유 줄이 출처별 · FE-1.3 주석 · 같은 id 는 기억 한 칸) + 사용자 결정 U7(채팅 승인 = 다음 과제 `docs/tracking.md` T-38 — §1) · 리뷰 갈림 2 건 = 알려진 한계로 수락(§11 ⑨ · §12) + 업스트림 `rust-v0.156.1` 소스 판독에 따른 사실 정정(거절된 명령은 이어받은 이력에 없다 · 거절된 파일 변경은 `declined` 로 닫힌다 · 끊긴 명령은 계속 돌다 늦게 끝난다 — §4-7 · §9) · 귀속 스위치 제거(귀속은 늘 짓는다). 4판 = 3판 `/review trd full` FIX(§12) + 사용자 결정 U2-a(codex 「거부」 = 따로 표기 — §1) + 그 귀속 설계(§4-7 ②-2 · §7 B5 채취 셋째). 3판 = 사용자 결정 U1–U6 반영 · U2 설계 추가(§4-7 · §7 B5). 2판 = `/review trd full` 1 라운드 FIX 반영(두 리뷰어 FIX · 불일치 없음 · light 재검 PASS). PRD 는 건너뛰었다(동작은 직전 세션 인계 메모에서 합의 — 사용자 2026-09-27). 설계 결정은 오케스트레이터가 내렸고 이 문서는 그것을 구현 가능한 명세로 옮긴다. 결정과 다르게 가야 할 곳은 「★메인 확인 필요★」로 올렸고, 2판에서 메인 판정을 받아 본문에 반영했다(모음·판정 = §11 — 3판 ⑥ 은 판정 대기 · ⑦ 과 4판 ⑧⑨ 는 5판에 닫았다 · 5판 ⑩ 은 사용자 결정 ⓐ 로 닫았다). 3판이 「★사용자 확인★」으로 올린 U2 세부 한 건(`declined` 를 어떻게 보이나)은 사용자가 결정했다(§1 U2-a).
>
> **입력:** [조사 보고서](../../research/chat-ux-four-features-2026-09-27.md)(사실의 정본 — 여기서 되풀지 않고 `조사 §n` 으로 가리킨다). 판독 기준 = 브랜치 `v0.3.2/feat/chat-ux` 머리 `3f06e08`. 이 문서의 `파일:줄` 은 전부 그 커밋에서 직접 열어 확인했다 — 3판이 더한 것은 `095e027` 에서, 4판이 더한 것은 `a0c5674` 에서 열었다(`3f06e08` 뒤 커밋은 문서뿐이라 코드 줄은 같다 — `git log 3f06e08..a0c5674 -- src crates src-tauri` 0 줄). 업스트림 codex = openai/codex main `41f9084` 의 `codex-rs/app-server/src/bespoke_event_handling.rs`(`gh api` 로 받아 판독 — 조사 §8 의 메인 대조와 같은 판 · 아래 `beh.rs:줄`). ★5판이 더한 업스트림 줄은 태그 `rust-v0.156.1`(= 커밋 `b412ff32c417` · 설치본 codex-cli 0.156.1 과 같은 판)의 `codex-rs/` 아래 경로이고 끝에 `@v0.156.1` 을 단다★ — 소스 판독이지 실행 확인이 아니다(확신 = 가능성 높음). 그 판의 `app-server/src/bespoke_event_handling.rs` 는 main 과 줄이 다르다(예: 우리 -32601 경로 = main `beh.rs:2026-2032` · 태그 `beh:2025-2031@v0.156.1`). 벤더 = claude 2.1.280(이 PC 설치본) · codex app-server 스키마(t3code 생성본 `packages/effect-codex-app-server/src/_generated/schema.gen.ts` 판독).
>
> **앵커:** ADR-0004(백엔드 지식 격리) · ADR-0012(시험대) · ADR-0044/0045(stream-json · 정제는 백엔드) · ADR-0051(행 종류 ↔ 레일) · ADR-0053(ScrollArea seam) · ADR-0055/0167(명령 레지스트리 · `help` 부재 = 창 안 전용) · ADR-0056(탭 keep-alive) · ADR-0113/0127(턴 관측) · ADR-0155(명령 선언은 생산자 옆) · ADR-0173(주황 경고 토큰 `--status-blocked`) · ADR-0203(codex 이력 item — 어휘표 한 벌) · ADR-0231/0234/0235(턴 도중 입력).
>
> 표기: **[고름]** = 사용자 체감이 없는 내부 구현이라 이 문서가 골랐다. **U#** = 사용자 결정(§1).

---

## 0. 결론 (먼저)

| 기능 | 무엇을 | 덩치 · 층 | 멈춤 조건 |
|---|---|---|---|
| **F1 스크롤 따라가기** | 두 슬롯(RichSlot · DomSlot)의 무조건 `scrollTop = scrollHeight` 를 공용 훅 하나 + DOM 없는 순수 코어로 바꾼다. 붙음 플래그는 스크롤 이벤트와 위로 가는 입력으로만 바뀐다 | 프론트만 | 없음 |
| **F2 Esc 끊기** | JSON 채팅 슬롯 안에서 맨 Esc = 도는 턴 끊기. 명령 `agent.interrupt`(창 + 데몬 버스). claude JSON 끊기를 새로 짓는다(U1) | 프론트 + agent crate | ★claude 부분은 스파이크(§3-5)가 출구 조건을 못 채우면 멈추고 사용자에게 돌아간다★ |
| **F3 도구 호출 묶기** | 연속 도구 행 ≥2 를 렌더 시점 순수 함수로 묶는다. 종류는 각 번역기가 `ToolCall` 에 싣는 중립 `category`. 묶음 머리 「오류 N」은 두 백엔드 다 — codex 는 도구 끝 상태를 새 선 변형 `ToolResult` 로 옮긴다(U2 · §4-7) | 선 타입 1 필드 + 새 변형 1 + 두 번역기 + codex 통로 거부 귀속 + 프론트 | 채취 둘째(끊긴 뒤 늦은 `failed`) = 사용자 결정 ⓐ 그대로 붉은 「오류」 · 멈춤 아님(§11 ⑩) · ★B5 채취에서 0 아닌 종료가 `completed` 로 오면 멈추고 메인에 올린다 · 채취 셋째(우리 거절)의 명령 · 파일 변경 끝이 승인 `itemId` 와 다른 id 로 오거나 · 안 오거나 · `failed`/`declined` 가 아니거나 · 그 턴이 `failed` 로 끝나거나 · `error` 알림이 오면 멈춘다(§4-7 ⑨)★ |
| **F4 claude 글자 스트리밍** | `--include-partial-messages` + 번역기 상태(메시지 id · 열린 블록 · 흘린 블록). 흘린 블록의 완결 본문은 버린다 | 백엔드만 · 선 타입·프론트 무변경 | 없음 |

- **구현 순서**(§7): 백엔드 한 워커 = F4 → F2 스파이크 → F2 구현 → F3 백엔드(B4) → U2 codex 끝(B5)(각 단계 빌드 초록 · 커밋). 프론트 두 워커 = FE-1(F1 → F2 프론트 → F3 접착) ∥ FE-2(F3 프론트 — 지역 타입 · 가드로 B4 · B5 를 안 기다린다). 공유 파일은 FE-1.0 이 먼저 한 번에 친다. ★B5 는 FE-2 의 가드 커밋 뒤에 **착수**(지시)한다★(새 변형이 프론트 `never` 망라를 건다 · 생성물은 B5 의 첫 protocol 시험 실행에서 공유 트리에 써지므로 커밋이 아니라 착수 조건이다 — §11 ⑥). 채취 둘째(끊긴 뒤 늦은 `failed`)의 사용자 결정은 받았다 — ⓐ 그대로(§11 ⑩ · 2026-09-27) · 착수 조건이 아니다. B4 · B5 · FE-2 뒤 I1 이 생성물 타입으로 바꾼다.
- **사용자 결정 U1–U8 = 완료(2026-09-27 · §1).** U2 를 뺀 나머지는 추천안 그대로다. ★U2 만 추천과 달리 「이번에 한다」★ → codex 도구 끝을 새 선 변형 `ToolResult{id, outcome}` 로 번역하고 실패·거부만 낸다 · 두 턴 분류기에서 신호 없음 · `PROTOCOL_VERSION` 은 안 올린다(§4-7). U6 과 함께 단축키 시스템은 보류(`docs/tracking.md` T-36). **U7(5판) = 채팅 안의 승인은 다음 과제**(`docs/tracking.md` T-38) — 이 라운드에서도 대시보드는 codex 승인 질문을 자동 거절하고, 거절된 호출을 「거부됨」으로 보이기만 한다. **U8(5판 후속) = 턴 결말 「중단됨」 행을 살짝 강조** — 굵게 + 테마 강조색 `var(--accent)` · 아이콘 · 문구 그대로(§1 · §7 FE-2).
- **★사용자 확인★ = 결정(2026-09-27 · §1 U2-a)**: codex 「거부」(실행되지 않은 호출) = **따로 표기(B)** — 주황 「거부됨」 배지 · 줄 안 한 줄 사유 · 요약 「거부 N」(오류와 따로). **5판 = 결말 둘**(4판 리뷰 FIX): 우리 거절 = `Refused`(사유 「대시보드가 승인 요청을 처리하지 않아 실행되지 않음」) · codex 스스로의 거부 등 기억에 없는 거부 = `Declined`(사유 「실행되지 않음」 — ★사용자 결정 2026-09-27 · 5판 light 재검★ · 5판 첫 문구 「codex 가 실행을 거부함」을 바꿨다 — §1 U2-a) — 배지 · 색 · 셈은 같다. codex 는 우리 거절을 명령은 `failed` 로, 파일 변경은 `declined` 로 닫으므로(조사 §8-1 · 5판 태그 판독 — §4-7 사실) 통로가 거절한 item id 를 기억했다가 그 끝을 `Refused` 로 바꿔 낸다(§4-7 ②-2). ★귀속은 늘 짓는다★ — 4판의 스위치(설치본이 `declined` 를 보내면 안 짓는다)는 걷었다(옳은 사유를 고르려면 늘 든다).
- **★메인 확인 필요★ — 2판 5 건 = 판정 완료**(§11): ①②⑤ 수락 · ③ 턴 열림 문(§3-4)으로 해소 · ④ 스파이크가 끊김을 못 가를 때만 쓰는 예비. 가장 큰 것은 claude 끊긴 턴을 `MessageDone` 대신 `TurnEnd{Interrupted}` 로 닫는 것(②) — `types.rs:70` 주석을 함께 고친다. **3판 새 2 건**: ⑥ B5 착수 ↔ FE-2 가드 커밋 순서 = 대기 · ⑦ claude 결과 중립화 = 닫힘(`docs/tracking.md` T-37 로 적립됐다). **4판 새 2 건 = 닫힘(5판)**: ⑧ 수락(거부 귀속 집합 = 통로 `State` 가 아니라 리더(`Reader`) 전용 칸) · ⑨ 알려진 한계로 수락(사용자 — U7 과 한 묶음 · 바른 해법 = T-38). ★5판 정정★: 재활성화 뒤 이력의 우리 거절은 「오류」가 아니다 — 명령 행은 **없고**, 파일 변경은 「거부됨 · 실행되지 않음」으로 보인다(우리 사유를 잃지만 거짓은 아니다 — §4-7 ②-2 한계). **5판 light 재검 새 1 건**: ⑩ 채취 둘째(끊긴 뒤 늦게 `failed` 로 닫히는 명령의 표시) = 닫힘 — 사용자 결정 2026-09-27 ⓐ 그대로(그 행의 붉은 「오류」 · 머리 「오류 1」 · 바로 아래 「중단됨」 행이 맥락을 준다 · 새 장치 없음). **5판 좁은 재검 새 1 건**: ⑪ claude 의 Esc 로 끊긴 도구(`tool_result` `is_error:true`) = 닫힘 — 메인 결정(사용자 위임) · ⓐ 와 같은 규칙(붉은 「오류」 · 머리 「오류 1」 · 아래 강조된 「중단됨」 행 · 새 장치 없음).

---

## 1. 사용자 결정 (U1–U8) — 2026-09-27 결정 완료

각 항목 = 사용자 결정 · 추천안 · 한 줄 트레이드오프 · 답에 따라 바뀌는 자리. U2 를 뺀 나머지는 추천안 그대로다(§0 과 같다). U2-a 는 U2 의 세부로 4판에 더했다(추천안 그대로). U7 은 4판 리뷰 갈림을 판정하며 5판에 더했다(메인 추천 그대로). U8 은 5판 후속에 사용자가 먼저 제기해 정했다.

| # | 질문 | 사용자 결정(2026-09-27) | 추천 | 대안 | 트레이드오프 | 바뀌는 자리 |
|---|---|---|---|---|---|---|
| **U1** | claude JSON 끊기를 이번에 짓나 | **짓는다** — 「claude 쪽 기능 확인해서 적용」(스파이크 먼저 — §3-5 · 설계 그대로) | **짓는다** | codex 만 먼저 | 짓지 않으면 Esc 가 claude 슬롯에서 아무 일도 안 한다(능력 `false`) — 기본 백엔드가 claude 라 체감 대부분이 빈다. 짓는 비용 = 스파이크 + 번역기 한 갈래 + 통로 주입 | 대안이면 §3-4 · §3-5 · §7 B2–B3 의 claude 부분을 뺀다. 버스 명령 · 프론트 키는 그대로 |
| **U2** | codex 묶음에 오류·끝 표시를 이번에 하나 | ★**이번에 한다 — codex 묶음에도 오류 수**(추천과 다름)★. 사용자 근거: 관례(codex TUI 의 「· N failed」 · t3code)는 요약의 개수 + 펼치면 어느 호출이 실패했나이고, codex 번역기는 끝 상태를 싣는 `item/completed` 만 버리고 있다 | 안 한다 — codex 묶음은 개수만 | codex `item/completed` 를 결과 사건으로 번역(**채택**) | 하면 새 선 변형이 필요하고 **옛 셸이 새 데몬을 만나면 결과마다 「표시할 수 없는 신호」 줄을 그린다**(조사 §3-3 — 실패·거부만 내서 줄인다 · §4-7 ③ · ⑤). 안 하면 codex 묶음 머리에 오류 수가 없다 | 새 선 변형 `ToolResult` + codex 번역기 `ItemOrigin::Completed`·`History` 갈래(`backend/codex/decoder.rs:623`) + 누산기 갈래 → **§4-7 · §7 B5** |
| **U2-a** | codex 「거부」(실행되지 않은 호출)를 어떻게 보이나 — 3판 ★사용자 확인★ | **따로 표기(B)** — 주황 「거부됨」 배지 · 줄 안 한 줄 사유 「대시보드가 승인 요청을 처리하지 않아 실행되지 않음」 · 묶음 요약 「거부 N」(오류 수에 안 든다). 사용자 근거 = 조사 §8-3(우리 앱의 거절은 전부 우리 정책 — 승인 창이 없다 — 이라 빨강은 없는 버그를 찾게 한다). ★5판(4판 리뷰 FIX)★: 사유는 출처별 둘 — 우리 거절(`Refused`) = 위 문구(메인) · 기억에 없는 거부(`Declined` — codex 스스로의 거부 등) = 「실행되지 않음」 · 배지와 「거부 N」 셈은 같다. ★`Declined` 문구 = 사용자 결정 2026-09-27(5판 light 재검)★ — 메인이 5판에 정한 「codex 가 실행을 거부함」을 바꿨다: 프론트 문자열은 백엔드 이름 없는 중립 낱말이다(`structuredAccumulator.ts:25` · §6 ADR-0004 행) · 옛 문구는 문서화된 경로에서 거짓이었다(복원된 우리 파일 변경 거절 · 상한에서 밀려난 우리 파일 변경 거절 · 벤더 준비 실패의 Declined 보고 — §4-7 ②-2 · §9) · claude `ToolResult`(T-37)가 오면 codex 이름을 물려받는다 | 따로 | 오류로 함께 센다(A — Zed · vibe-kanban · paseo 선례 · 조사 §8-2) | B 는 **귀속이 든다** — codex 는 우리 거절을 명령은 `failed` 로(조사 §8-1), 파일 변경은 `declined` 로(5판 태그 판독) 닫아 벤더 status 만으로는 출처를 못 가른다. A 는 추가 작업이 없지만 권한에 막힌 것과 진짜 실패가 같아 보인다 | §4-7 ②-2(귀속 — B5 · codex 통로) · ⑧(배지 · 사유 줄 둘 · 요약) · §7 FE-1.0 키 넷(`toolGroupDeclined` · `toolDeclined` · `toolRefusedReason` · `toolDeclinedReason`) · 스위치 `DECLINED_MARK` |
| **U3** | 「맨 아래로」 버튼 | **바닥이 아니면 늘** | **바닥이 아니면 늘 · 셰브론 · 개수 없음 · 보이기 150 ms 늦춤** | 위로 올린 동안 새 내용이 왔을 때만 | 피어 전부가 추천안이다(조사 §1-2). 대안은 「읽다 멈춘 자리」에서 버튼이 안 떠 돌아갈 길을 못 찾는다 | 상수 `JUMP_BUTTON_MODE` 하나(§2-4). 대안이면 코어의 `unseenGrowth` 를 버튼 조건에 건다 |
| **U4** | 보내면 다시 바닥에 붙나 | **붙는다** | **붙는다** | 그대로 둔다 | 붙지 않으면 위를 읽다 보낸 글의 답이 화면 밖에서 흐른다. 붙으면 읽던 자리를 잃는다(paseo 선례 = 붙는다) | 상수 `REPIN_ON_SEND` 하나(§2-4) |
| **U5** | 도구 호출 사이의 (비지 않은) 생각 블록 | **흡수** | **묶음에 흡수** | 묶음을 끊는다 | 끊으면 생각이 도구마다 끼는 claude 턴에서 묶음이 거의 안 선다. 흡수하면 펼쳐야 생각이 보인다(codex TUI 선례 = 흡수) | `groupToolRuns` 의 흡수 판정 한 줄(§4-3) |
| **U6** | Esc 가 먹히는 범위 | **칸 안 어디든** · 단축키 시스템은 보류(`docs/tracking.md` T-36) | **그 채팅 칸 안 어디든**(입력창 + 대화 본문 · 칸 루트가 클릭으로 포커스를 받는다) | 입력창 포커스일 때만 | 추천안은 본문을 클릭해 읽다가도 끊을 수 있다. 대안은 피어(Claude Code · codex · paseo)와 같고 포커스 변화가 없다 | 상수 `ESC_SCOPE` 하나 + 루트 `tabIndex` 한 줄(§3-2) |
| **U7** | codex 승인 질문을 채팅 안에서 답하게 하나(채팅 승인 UI) — 4판 리뷰 갈림(§12) 판정 중에 나왔다 | **다음 과제로 뺀다**(`docs/tracking.md` T-38) — 사용자: 「다음 과제로 빼자. 지금 많이 엮여있어」 | 뺀다(메인 추천 — 거부 귀속 장치를 더 키우지 않는 것과 한 묶음) | 이번 라운드에 짓는다(기각) | 빼면 대시보드는 계속 codex 승인 질문을 자동 거절하고, 귀속의 알려진 한계 넷(이력의 거절된 명령 행이 없다 · 상한 16 에서 밀려난 명령은 「오류」 · 이력의 파일 변경 거절과 상한에서 밀려난 파일 변경 거절은 「거부됨 · 실행되지 않음」 — 우리 사유를 잃는다)이 남는다(§4-7 ②-2 · §9). 지으면 승인 요청 · 답 경로 · UI 가 새로 들고 이 라운드의 귀속 · 턴 관측 · 대기 표시와 엮인다 | 이번 라운드에는 없다 — §4-7 ②-2 한계 · §9 · §11 ⑨ · §12 가 T-38 을 가리킨다 |
| **U8** | 턴 결말 「중단됨」 행을 더 눈에 띄게 하나 — 오늘 `OutcomeRow`(`src/components/slot/StructuredTextView.tsx:289-316`)는 `CircleStop` 아이콘 + 「응답이 중단됐습니다」를 `text-muted` 로 그린다 | **살짝 강조** — 굵게 + 테마 강조색 `var(--accent)`(dark `#4a9eff` · light `#0066cc` · e-ink 검정) · 아이콘 · 문구 그대로. 사용자: 「너무 눈에 띄지 않음?」 → 「색깔은 살짝 강조해야될것같음」 · 문구 · 색은 보고 나서 고친다(「나중에 보고 개선하면 되지」) | —(사용자가 먼저 제기) | 오늘 그대로 `text-muted` | 빨강(실패)도 주황(`--status-blocked` = 거부)도 아니라 셋이 갈린다 · 「중단·모름은 붉게 칠하지 않는다」(`StructuredTextView.tsx:286-287`) 그대로 · `unknown` · `failed` 무변경. 알려진 위험 = 채팅 링크가 같은 강조색을 쓸 수 있다(보고 나서 고친다 — 사용자) | `OutcomeRow` 의 `interrupted` 갈래(§7 FE-2) · §8-2 F2 ⑨ · §10 0237 |

- 두 번 Esc(글 지우기 · 되감기)는 **이번 범위 밖**이다.
- **단축키 시스템은 보류**(U6 과 함께 · `docs/tracking.md` T-36). Esc 는 지금 명령 `agent.interrupt` 를 부르는 **지역 술어** 하나이고(§3-2), T-36 이 키바인딩 표를 세우면 그 표의 한 줄로 옮긴다 — 끊는 동작은 명령에 있어 손댈 것이 없다. ADR-0237(가안)이 이것을 거부한 대안 「단축키 시스템을 지금 만든다」로 적는다(§10).

---

## 2. F1 스크롤 따라가기

### 2-1. 바뀌는 자리

| 파일 | 지금 | 바뀜 |
|---|---|---|
| `src/components/slot/RichSlot.tsx:101` · `:288-291` | `scrollRef` + `[items]` 마다 무조건 바닥 | `const follow = useScrollFollow(viewId)` · `<ScrollArea ref={follow.viewportRef}>`(`:398`) · 효과 삭제 |
| `RichSlot.tsx:153` · `:251` | 구독 초기화 · 비우기(onReset) | 두 자리에서 `follow.pin()` — 비운 뒤 오는 이력이 바닥에 착지. ★구독 효과 deps 는 `[viewId, agentId]` 그대로(`:283` — CLAUDE.md micro-rule)★ — `follow`·`pin` 을 넣지 않는다(§2-3: 손잡이는 안정적이다) |
| `RichSlot.tsx:293-331`(`send`) | — | `if (REPIN_ON_SEND) follow.pin()`(U4) |
| `RichSlot.tsx:398-412` | ScrollArea 자식 = 본문 · 로딩 막 | 버튼 `<JumpToBottom follow={follow} />` 를 로딩 막 옆에 둔다 — absolute 기준이 Root 라 스크롤되지 않는다(`:400-408` 주석과 같은 근거) |
| `src/components/slot/DomSlot.tsx:81` · `:180-185` · `:194-197` | 같은 무조건 효과 | 같은 교체 · `follow.pin()` 은 구독 초기화(`:106-107`)와 onReset(`:155`) |
| 새 `src/components/slot/scrollFollow/followCore.ts` | — | 순수 코어(§2-2) |
| 새 `…/scrollFollow/useScrollFollow.ts` · `followRegistry.ts` · `JumpToBottom.tsx` | — | 훅 · 슬롯 id → 손잡이 모듈 맵 · 버튼 |
| 새 `src/commands/scrollCommands.ts` | — | `slot.scrollToBottom`(§2-5) |

### 2-2. 순수 코어 — `followCore.ts` (DOM 0)

```ts
export const FOLLOW_THRESHOLD_PX = 48 // [고름] 피어 범위 40–64 의 가운데(t3code 40 · paseo 64)
export interface Metrics { top: number; height: number; client: number } // scrollTop · scrollHeight · clientHeight
export interface FollowState {
  pinned: boolean
  frozen: boolean            // client === 0 (display:none 탭) — 재는 값이 무효
  lastTop: number            // 마지막으로 본 scrollTop
  expectTop: number | null   // 우리가 쓰고 아직 이벤트로 못 본 값
  savedTop: number | null    // 얼 때 적은 scrollTop — 풀릴 때 되살린다. 처음·풀린 뒤 = null
  unseenGrowth: boolean      // 떨어진 동안 내용이 자랐다(U3 대안용)
}
export type FollowInput =
  | { type: 'attach'; m: Metrics }   // 뷰포트 노드가 (다시) 붙었다
  | { type: 'scroll'; m: Metrics }   // scroll 이벤트
  | { type: 'resize'; m: Metrics }   // 내용·뷰포트 ResizeObserver
  | { type: 'intentUp'; m: Metrics } // 위로 가는 사용자 입력(§2-3)
  | { type: 'pin' }                  // 비우기 · 버튼 · 보냄 · LLM
  | { type: 'unpin' }                // F3 묶음 펼침(§4-5)
export function step(s: FollowState, i: FollowInput): { state: FollowState; scrollTo: number | null }
```

규칙(이 표가 시험 표다):

1. **얼음**: 얼지 않은 상태에서 `m.client === 0` 이 오면(잰 값을 싣는 입력 넷 어느 것이든) `frozen = true` · **`savedTop = expectTop ?? lastTop`**(그 순간의 `m.top` 은 이미 무효일 수 있어 마지막으로 유효하게 본 값을 적는다) · `scrollTo = null`. 얼어 있는 동안 `scroll`·`resize`·`attach`·`intentUp` 은 아무 칸도 안 바꾼다(규칙 2 의 풀림만 예외). `pin`·`unpin` 은 얼어 있어도 붙음만 바꾼다(적용은 풀릴 때 · `pin` 이 함께 내리는 `unseenGrowth` 는 규칙 10).
2. **풀림**(`frozen` 이고 `m.client > 0` 인 `resize`·`attach`): `frozen = false` · `lastTop = m.top` · `expectTop = null`(숨기 전 쓰기의 이벤트는 얼어 있는 동안 버려졌고 위치를 다시 쟀다). 붙어 있으면 바닥으로 쓴다. 떨어져 있으면 `savedTop !== null` 이고 `m.top` 이 `savedTop` 과 다를 때만(규칙 7 의 0.5 px 폭) `savedTop` 으로 쓴다(`null` 이면 쓰지 않는다) — `display:none` 을 지나 `scrollTop` 이 보존되는지 모르므로(조사 §1-1) 되살리기를 늘 건다(보존되면 무동작). 끝에 `savedTop = null`.
3. **우리 쓰기는 절대 풀지 않는다**: `scroll` 의 `m.top` 이 `expectTop` 과 1px 안이면 `lastTop` 만 옮기고 `expectTop = null`.
4. **사용자 스크롤**: 그 밖의 `scroll` — 바닥 거리 `d = height − top − client`. 끝에 늘 `lastTop = top` · `expectTop = null`(브라우저는 한 프레임의 위치 변화를 이벤트 하나로 합친다 — 우리 쓰기 뒤 사용자가 움직였으면 그 쓰기의 이벤트는 따로 오지 않는다).
   - 붙어 있을 때: `d > 문턱` 이고 `top < lastTop`(위로 움직였다) → `pinned = false`. 그 밖 → 그대로(문턱 안의 위 움직임은 내용 줄어듦의 클램프일 수 있다 — 풀지 않는다).
   - **떨어져 있을 때 = 위 의도 걸쇠**: 되붙기는 **아래로 움직여(`top > lastTop`) 문턱 안에 든** `scroll` 에서만 — `pinned = true` · `unseenGrowth = false`. 위로·제자리·문턱 밖 아래로는 그대로 떨어져 있다. ★문턱 안의 작은 위 휠이 규칙 5 로 푼 뒤, 그 휠이 만든 `scroll`(위로 · 문턱 안)이 곧바로 되붙이지 않게 하는 것이 이 걸쇠다★. 그 밖에 걸쇠를 푸는 길은 명시 `pin`(버튼 · 보냄 U4 · 비우기 · 명령)뿐이다.
5. **위로 가는 입력**(`intentUp`): `height > client`(스크롤할 것이 있다)면 `pinned = false`. 문턱 안의 작은 휠도 곧바로 푼다 — 그래야 성장이 끌어내리지 않는다(되붙기는 규칙 4 의 걸쇠가 막는다).
6. **성장**(`resize`): ★붙음을 다시 재지 않는다★(조사 §1-3 — 재면 스트리밍 성장이 「위로 올렸다」로 읽힌다). 붙어 있으면 `scrollTo = max(0, height − client)`. 떨어져 있으면 `unseenGrowth = true`.
7. **쓰기 예고**: 쓰기는 `|m.top − 목표| > 0.5` 일 때만 낸다 — 그때 `expectTop = scrollTo` · **`lastTop = scrollTo`**. 0.5 px 안이면 이미 그 자리라 `scrollTo = null`(쓰기도 예고도 없다) — 이벤트가 안 오므로 예고를 남기면 뒤의 사용자 스크롤을 우리 것으로 오인한다. `lastTop` 을 쓰기가 닿을 자리로 옮기는 것은 쓰기의 이벤트가 오기 전에 사용자가 위로 끈 경우 때문이다: 둘이 이벤트 하나로 합쳐져 오는데 옛 자리를 기준으로 두면 「위로」가 안 읽혀 붙은 채 남고 다음 성장이 도로 끌어내린다. 쓰기 자신의 이벤트는 `expectTop` 이 따로 알아본다(규칙 3).
8. 처음 상태 = `pinned: true` · `frozen: false` · `lastTop: 0` · `expectTop: null` · `savedTop: null` · `unseenGrowth: false`.
9. **새 노드**(`attach` · 얼지 않고 `m.client > 0`): `lastTop = m.top` · `expectTop = null`(옛 노드에 쓴 위치의 이벤트는 새 노드로 오지 않는다). 붙어 있으면 바닥으로 쓰고, 떨어져 있으면 쓰지 않는다.
10. **`pin` · `unpin`**: `pin` = `pinned = true` · `unseenGrowth = false`(얼어 있어도 — 불변식 `pinned ⇒ !unseenGrowth`). `unpin` = `pinned = false` 만. 둘 다 잰 값을 싣지 않아 `scrollTo = null` — 바닥 쓰기는 훅이 `pin` 뒤에 `resize` 한 번(지금 잰 값)을 먹여 규칙 6 으로 낸다(얼어 있으면 규칙 1 이 막고 풀릴 때 규칙 2 가 쓴다).

### 2-3. 훅 — `useScrollFollow(slotId)`

- 돌려주는 것: `{ viewportRef: (el: HTMLDivElement | null) => void, pinned: boolean, unseenGrowth: boolean, scrollable: boolean, pin(), unpin() }`(`FollowHandle` = 그중 `pinned` · `pin` · `unpin`). 세 불리언은 값이 바뀔 때만 React 상태를 올린다(스크롤 이벤트마다 리렌더하지 않는다). `scrollable` = 마지막으로 유효하게 잰(`client > 0`) 값의 `height − client > 0.5` — 버튼 조건(§2-4)이지 붙음 판정이 아니다(코어는 이것을 안 본다 · 숨은 동안은 그대로 둔다).
- ★`viewportRef`·`pin`·`unpin` 은 렌더마다 같은 함수다★ — 코어 상태·노드를 ref 에 두고 그 ref 만 읽는다(마운트 수명 동안 정체성 불변). 그래서 구독 효과(deps `[viewId, agentId]`)가 안에서 불러도 옛 함수를 쥘 걱정이 없고, ★그 deps 에 넣지 않는다★(넣으면 손잡이 정체성이 흔들릴 때 재구독 → replay 가 돈다).
- **콜백 ref** 라 뷰포트가 내려갔다 새 노드로 붙어도(RichSlot 빈 상태 `:397`) 옛 노드의 청취자·관찰자를 떼고 새 노드에 `attach` 를 먹인다.
- 관찰 대상 둘(ResizeObserver): 뷰포트 자신(입력창 위 대기 목록 `:458` 이 뷰포트를 줄인다 · 창 크기 · 탭 표시)과 **뷰포트의 첫 자식 요소**(Radix 내용 래퍼 — `StructuredTextView.tsx:552-557` 주석의 `display:table` 래퍼). [고름] 내용 ref 를 StructuredTextView·`<pre>` 로 내려보내지 않는다 — 두 슬롯이 손대지 않고 같은 훅을 쓴다. 대가 = Radix 내부 구조에 기댄다(§9).
- **휠**: `wheel` 에서 `deltaY < 0` 이고(`ctrlKey` 는 확대/축소라 거른다), 대상에서 가장 가까운 `[data-radix-scroll-area-viewport]` 가 이 뷰포트가 아니면서 `scrollTop > 0` 이면 **안쪽 스크롤러가 먹는 입력**이라 무시한다(ThoughtRow 의 자기 ScrollArea — `chat/ThoughtRow.tsx:53-58`). 그 속성은 Radix dist 가 Viewport 에 싣는다(`node_modules/@radix-ui/react-scroll-area/dist/index.mjs` 판독). 전역 청취자를 걸지 않는다 — 뷰포트에만 건다(`use-stick-to-bottom` 기각 사유 — 조사 §1-2).
- **키**: 뷰포트의 `keydown` 에서 `PageUp`·`Home`·`ArrowUp` → `intentUp`(같은 안쪽 스크롤러 거름 · 편집 칸(`input`·`textarea`·`select`·contentEditable) 안의 키와 IME 조합 중(`isComposing`) 키는 거른다). ★Radix Viewport 엔 `tabIndex` 가 없어 대화 글을 눌러도 포커스가 body 에 남고 keydown 이 뷰포트에 오지 않는다★ — 그래서 훅이 붙일 때 뷰포트에 `tabIndex = -1` 을 준다(클릭 · 코드로만 포커스 · 탭 순서 밖). 글을 누르면 뷰포트가 포커스를 받아 keydown 이 뷰포트를 과녁으로 오고, 슬롯 루트의 Esc 처리기(§3-2)로 그대로 번진다. 포커스 테두리는 코드베이스 관례(포커스 받는 목록 컨테이너의 inline `outline: none`)대로 지운다. 뗄 때 원래 `tabindex`(있었으면 그 값 · 없었으면 속성 삭제)와 inline `outline` 을 되돌린다.
- **쓰기**: 코어가 낸 `scrollTo` 를 `el.scrollTop` 에 쓴다. RO 콜백은 레이아웃 뒤 · 페인트 전이라 깜빡임이 없다.
- **관측 표면**: 뷰포트에 `data-scroll-follow="pinned" | "free"` 를 훅이 직접 적는다(React 상태 경유 없음).
- **시험 seam**(ADR-0012): `useScrollFollow(slotId, { ResizeObserver? })` — 시험이 가짜 관찰자를 꽂는다(jsdom 엔 RO 가 없다 — `DomSlot.test.tsx:12-17` 이 이미 전역 가짜를 둔다).

### 2-4. 버튼 · 보냄 (U3 · U4)

- `JumpToBottom`: **`!pinned && scrollable`** 이 150 ms 이어지면 보인다(t3code — 탭 전환 깜빡임 방지) · lucide `ChevronDown` · 개수 없음 · 누르면 `pin()`(즉시 쓰기 — 부드러운 스크롤은 중간 이벤트를 만든다). `aria-label`·`title` = `t('slot.scrollToBottom')`. DOM `data-jump-to-bottom="1"`. 자리 = ScrollArea 의 자식 · 가운데 `bottom-7`(RichSlot 의 이름 라벨이 영역 바닥 20 px 을 덮는다) · DomSlot 에도 같은 버튼.
  - `scrollable` 을 거는 이유: 떨어진 뒤 뷰포트가 커지거나 내용이 줄어 다 들어오면 이미 「바닥」인데 붙음은 다시 재지 않는다(규칙 6 · ADR-0242 결정 2) — 붙음 규칙은 그대로 두고 버튼만 가린다.
- 상수 둘(`scrollFollow/useScrollFollow.ts` 머리): `JUMP_BUTTON_MODE: 'whenFree' | 'whenUnseen' = 'whenFree'`(U3 · `'whenUnseen'` = `!pinned && unseenGrowth && scrollable`) · `REPIN_ON_SEND = true`(U4 — RichSlot 이 읽는다).

### 2-5. LLM 경로 (CLAUDE.md 「LLM-우선 제어」)

- `followRegistry.ts`: `Map<slotId, FollowHandle>` 모듈 맵. 훅이 마운트 때 올리고 내릴 때 **자기 손잡이일 때만** 지운다(StrictMode 이중 마운트). 새 전역 핸들 없음.
- `slot.scrollToBottom { slotId }`(`scrollCommands.ts`) — ★`help` 없음★: 스크롤 위치는 **창마다 따로 있는** 프론트 상태라 `renderModeCommands.ts:8-20` 의 규칙(ADR-0167)이 그대로 든다. 손잡이가 없으면 throw(「이 창에 그 슬롯의 대화·텍스트 뷰가 없다」). 돌려주는 값 = `{ pinned: true }`. 숨은 탭이면 붙음만 세우고 쓰기는 보일 때 한다.
- 읽기 = DOM 속성(`data-scroll-follow`) — `cdp.mjs` eval 로 본다.

### 2-6. 알려진 한계 (고치지 않는다)

- **DomSlot 머리 자르기(200 KB · `DomSlot.tsx:36` · `:142`)**: 떨어져 읽는 동안 앞이 잘리면 본문이 위로 밀린다(표류). 이번 범위 밖.
- 글자를 끌어 선택하는 중 성장 · 스크롤바를 문턱 안에서 끄는 중 성장은 바닥으로 끌린다. 거슬리면 `pointerdown` 을 의도 입력으로 더한다(후보).

### 2-7. 시험

- `followCore.test.ts`(표 시험 — 위 규칙 1–10 각각 + 조합 · 쓰기의 이벤트가 오기 전 위로 끌기 → 푼다): 우리 쓰기 뒤 이벤트는 안 푼다 · 떨어진 뒤 아래로 문턱 안에 들면 되붙기 · ★걸쇠 — 문턱 안 작은 위 휠(`intentUp`) → 그 휠의 위 `scroll`(문턱 안) → 여전히 떨어짐 · 이어 아래로 문턱 안 `scroll` → 붙음★ · 떨어진 채 제자리·위 `scroll` 은 되붙이지 않음 · 명시 `pin` 은 걸쇠를 푼다 · 붙은 채 문턱 안 위 `scroll`(클램프)은 안 푼다 · 성장 중 풀리지 않음 · 얼 때 `savedTop = expectTop ?? lastTop` · 얼음 동안 스크롤·RO·`intentUp` 무시 · 풀릴 때 붙음=바닥 / 떨어짐=`savedTop` / 떨어짐 + `savedTop` 없음 = 쓰기 없음 · 풀린 뒤 `savedTop = null` · 스크롤할 것 없는 휠은 안 푼다 · 이미 바닥인 쓰기는 예고 없음.
- `useScrollFollow.test.tsx`(가짜 RO · `Object.defineProperty` 로 `scrollHeight`·`clientHeight`·`scrollTop`): 노드 교체 뒤 재부착 · 안쪽 Radix 뷰포트 휠 무시 · `data-scroll-follow` 값 · 리렌더 뒤 `viewportRef`·`pin`·`unpin` 정체성 불변 · `tabIndex -1` 부여와 뗄 때 복원 · 포커스 받은 뷰포트의 ArrowUp · `scrollable` 이 다 들어오면 거짓(붙음은 그대로).
- `scrollCommands.test.ts`: 손잡이 있음/없음 · `help` 가 없다(버스 미승선 — `renderModeCommands.test.ts` 의 같은 단언 모양).
- 기존 `RichSlot.test.tsx` · `DomSlot.test.tsx` 는 무조건 스크롤을 단언하지 않는다(`rg scrollTop src --glob '*.test.*'` 의 슬롯 쪽 적중은 RO 가짜뿐) — 회귀로 그대로 돈다.

---

## 3. F2 Esc 끊기

### 3-1. 바뀌는 자리

| 파일 | 바뀜 |
|---|---|
| `src/commands/agentCommands.ts`(`:126-158` 옆) | `agent.interrupt { agentId }` — `agentClient.interruptAgent`(`src/api/protocolClient.ts:988-992`)를 부른다. ★`help` 없음★ — 같은 이름을 데몬이 버스에서 답한다(`:130-133` 의 `cancelQueuedInput` 주석과 같은 사유). 입력 임대 거절은 `:146-155` 와 같은 `CONFLICT:` 접두로 편다 — [고름] 그 매핑을 작은 함수로 뽑아 두 명령이 함께 쓴다. 돌려주는 값 = `{ outcome: 'requested' }` |
| `src/components/slot/RichSlot.tsx:373-392`(루트) | `onKeyDownCapture` 로 Esc 처리(§3-2) · U6(칸 안 어디든)이라 `tabIndex={-1}` + `outline-none` |
| 새 `src/components/slot/interruptKey.ts` | §3-2 의 조건 전부를 담은 순수 술어 하나 |
| 오버레이 뿌리 넷 | `data-engram-overlay` 속성 하나씩 — `SlotContextMenu.tsx`(루트) · `AgentMonitoringPicker.tsx:100`(백드롭) · `AgentList.tsx`·`PresetPalette.tsx` 의 행 메뉴 뿌리. [고름] 문서 전역 Esc 로 닫히는 셋(조사 §2-1)과 우클릭 메뉴를 한 표지로 잰다 |
| `crates/engram-dashboard-agent/src/commands.rs` | 버스 선언 `agent.interrupt`(§3-3) |
| `crates/engram-dashboard-agent/src/transport/pty.rs`(7판 · ADR-0245 · §7 B3c) | `interrupt()` = `Unsupported`(날 `0x03` 을 더는 쓰지 않는다) · 능력 `control.interrupt = false` — 터미널 모드의 Esc · Ctrl-C 는 사람이 칠 때 터미널이 스스로 처리한다 |
| `src/components/slot/slotFocus.ts`(새 · FE-1.2 리뷰 FIX — 7판 기록) | `keepFocusInSlot(button)` — 칸 안 버튼이 눌린 뒤 사라지거나 잠겨도 초점을 그 칸의 `tabindex="-1"` 조상(스크롤 뷰포트 · RichSlot 루트)으로 옮긴다. 초점이 문서 body 로 떨어지면 그 칸의 Esc 가 루트 capture 에 닿지 않는다(U6). 그 버튼이 초점을 쥐었을 때만 옮긴다 — 다른 칸의 초점은 건드리지 않는다 |
| `src/components/slot/scrollFollow/JumpToBottom.tsx` · `src/components/slot/QueuedInputList.tsx`(FE-1.2 — 7판 기록) | 「맨 아래로」 · 대기 행 ✕ · 「외 N개」 버튼이 제 동작 뒤 `keepFocusInSlot` 을 부른다 |
| `src/store/interruptStore.ts`(새 · FE-1.4 · 7판 · ADR-0244) · `src/components/slot/chat/WaitRow.tsx` | 에이전트별 「끊는 중」 상태 · 끊는 중인 꼬리의 「중단하는 중…」(§3-2 「끊는 중」) |
| `crates/engram-dashboard-agent/src/transport/stdio.rs` · `backend/claude/mod.rs` | claude 끊기(§3-4 · U1) |

- **전역 단축키 가드(`src/commands/keybindings.ts:11-35`)는 손대지 않는다** — load-bearing 이다. Esc 는 `BINDINGS`(`:54-57`)에 넣지 않는다.
- PTY·xterm 슬롯(`TerminalSlot`)은 바이트 하나 안 바뀐다 — Esc 는 지금처럼 PTY 로 간다.

### 3-2. 키 조건 (RichSlot 루트 `onKeyDownCapture`)

발화 = 아래 **전부**:

- `e.key === 'Escape'` · 수식키 없음(`ctrl`·`alt`·`shift`·`meta` 모두 거짓) · `!e.repeat`
- IME 조합 중 아님 — `!e.nativeEvent.isComposing && e.keyCode !== 229`(`:476` 과 같은 판정)
- `!e.defaultPrevented`(Radix 레이어는 문서 capture 에서 먼저 먹는다) · `document.querySelector('[data-engram-overlay]') === null`
- `streaming`(`:370`) · `!agentUnavailable`(`:130`) · `agent?.capabilities?.control?.interrupt === true`
- 중단하는 중이 아님 — `!ctx.interrupting`(7판 · ADR-0244 · 아래 「끊는 중」)
- (`ESC_SCOPE = 'input'` 일 때만) `e.target === textarea`

[고름] 위 조건 전부를 **순수 술어 하나** `isInterruptEscape(e, ctx)`(`interruptKey.ts` — `ctx` = `streaming` · `agentUnavailable` · `canInterrupt` · `interrupting`(7판 · ADR-0244) · `overlayOpen`(루트 처리기가 위 `querySelector` 로 재서 넘긴다 — 술어는 DOM 을 안 읽는다) · `scope` · 입력창 노드)에 모은다. 루트 처리기는 그 술어가 참이면 명령을 부르기만 한다. 이 술어가 T-36(단축키 시스템 — 보류)이 세울 키바인딩 표 한 줄의 `when` 으로 그대로 옮겨 간다(§1 U6).

발화하면 `e.preventDefault()` 하고 `fireAndForget('agent.interrupt', { agentId })`(`src/commands/dispatch.ts:15`) — 사람 키와 LLM 이 같은 핸들을 흔든다. ★낙관 상태를 바꾸지 않는다★ — 끊겼다는 것은 턴 끝 사건이 알린다(claude 의 응답 수신은 「멈췄다」가 아니다 — §3-5 S4). ★7판 정정(ADR-0244 · 사용자 결정 ⑬)★: **표시에 한해서만** 뒤집는다 — 아래 「끊는 중」 항목. 턴이 멈췄다는 사실은 그대로 턴 끝 사건이 알린다. ★입력창 글은 건드리지 않는다.★

- **끊는 중(7판 · ADR-0244 — 사용자 결정 ⑬ · 세부는 메인 · 위임)**: 상태 = `src/store/interruptStore.ts` — 에이전트별 대기 중인 요청 하나 + 이 창에 마운트된 그 에이전트의 채팅 뷰 수(`watch`). ★창마다 따로다★ — 팝아웃은 다른 창의 상태를 보지 않는다. ① **세움** — 창 명령 `agent.interrupt` 가 보내는 같은 틱에 동기로 세운다(답 · 프레임보다 먼저 · 사람의 Esc 와 창 명령을 부르는 LLM 이 한 상태를 쓴다 · CLAUDE.md 「LLM-우선 제어」). 이 창에 그 에이전트의 채팅 뷰가 하나도 없으면 세우지 않는다(걷어 줄 쪽이 없다). ② **둘째 호출** — 끊는 중에 다시 부르면 다시 보내지 않는다: `{ outcome: 'requested' }` 또는 대기 중인 요청의 결과(거절 포함)를 돌려준다. ③ **걷음** — ⓐ 창 명령이 어떤 오류로 끝나든(입력 임대 거절만 `withLeaseConflict` 가 `CONFLICT:` 접두를 단다 · 턴이 없을 때는 통로의 「unsupported」 오류 · 연결 오류도 같다). ⓑ RichSlot — 라이브 프레임의 턴 경계 수(누산기가 턴 경계 `MessageDone` · `TurnEnd` 를 결말과 무관하게 센다 · 줄지 않는다 — 뷰가 첫 `'live'` 뒤에 받은 프레임이 그 수를 올리면 걷는다 · ★턴 끝 상태의 전이(거짓 → 참)로 보지 않는다★ — 전이로 보면 「보냄 → 여는 프레임 전 Esc → [`TurnEnd`, 다음 턴의 `TextDelta`] 한 묶음」을 놓친다 · ★이 화신의 첫 `'live'` 전 프레임은 세지 않는다★ — 새 뷰의 이력 · 새 화신의 이력 속 옛 턴 끝이 살아 있는 끊기를 걷으면 안 된다) · `streaming` 참 → 거짓(뒷받침 — 턴을 열지 않고 대기에 든 보냄만 덮는다: `Queued` → Wait 꺼짐) · 뷰가 **처음** `'live'` 로 따라잡았을 때 열린 턴이 없으면(끊긴 턴이 live 전 플러시 안에서 이미 끝났다) · onReset(새 화신) · 에이전트 부재(Killed · Exited · Failed · 명부에서 빠짐)나 연결 끊김 — 잠깐이라도(다시 붙은 뒤의 Esc 는 다시 끊는다). ⓒ 제 구독이 떨어지거나 오류가 난 뷰는 세기에서 빠진다(제 `watch` 를 푼다) — 그래서 상태는 **성한 뷰가 하나도 남지 않을 때만** 걷힌다(마지막 뷰 풀림). ★둘째 뷰의 구독 오류는 다른 성한 뷰의 상태를 걷지 않는다★. 시한은 없다 — 턴 끝이 끝내 안 오면 남는다(오늘 Wait 가 걸려 남는 것과 같다). ④ **보임** — 끊는 중에는 채팅 꼬리의 Wait 행이 정지 아이콘과 함께 `t("chat.interrupting")`(「중단하는 중…」)을 흐린 글 + 깜빡임으로 보인다(U8 결말 행의 강조색 · 굵게가 아니다 — 아직 결말이 아니다 · 메인) · 술어가 거짓을 돌려준다(위 조건 「중단하는 중이 아님」 — Esc 무시). 이 표시는 「멈춰 달라고 했다」이지 「멈췄다」가 아니다 — 멈춤은 중단 행 · 끊김 표시 행(ADR-0243)이 알린다. ★덮지 않는 것★: 데몬 버스로 직접 보낸 끊기(`engram agent.interrupt …`)는 창 명령을 안 거쳐 표시도 가드도 없다(§9) · 한 번 누름의 잔여 경합(§3-4)은 그대로다 · B 가 눈에 보이게 시작한 뒤의 Esc 는 B 의 보통 끊기다(앞 턴 끝에서 상태가 걷힌다). 백엔드는 바뀌지 않는다 — 앞선 「턴마다 끊기 한 번(`TurnGate` 의 `sent` 표식)」 제안은 이것으로 대체됐다(B3 그대로).

- 프론트 `streaming` 은 백엔드의 턴 상태와 양 끝에서 어긋난다 — 보낸 직후 낙관적으로 켜지고(`awaiting`) 턴 끝 사건이 올 때까지 늦게 꺼진다. 앞 끝(턴이 아직 안 열렸다)의 Esc 는 통로가 `Unsupported` 로 거절하고(codex · claude 턴 열림 문 — §3-4) `fireAndForget` 가 warn 으로 삼킨다 — 무동작이고 사용자는 다시 누르면 된다. 뒤 끝은 §3-4 의 잔여 경합이다.

- capture 인 이유: 입력창 `onKeyDown` 이 `e.stopPropagation()` 을 먼저 한다(`:471-472`) — bubble 로는 입력창의 Esc 가 루트에 안 온다. capture 는 입력창과 본문을 한 처리기로 덮는다.
- 상수 `ESC_SCOPE: 'slot' | 'input' = 'slot'`(U6 확정 = `'slot'`).

### 3-3. 명령 — 창 + 데몬 버스 (`agent.cancelQueuedInput` 선례)

선례를 끝까지 따라가면 **선언은 agent crate, 호스팅은 데몬**이다(`commands.rs:1-7`). 데몬 crate 는 바뀌지 않는다 — 입력 임대 검문(`daemon control/commands.rs` 의 `admit_input`)이 `INPUT_AFFECTING` 을 읽는 일반 경로다. ★6판 정정(B3a 착지)★: **제품 코드는 그대로이고, 손으로 관리하는 시험용 이름 목록만 바뀐다** — `daemon/src/control/commands.rs` 의 `BUS_ONLY` · `daemon/src/connection_core.rs` 와 `daemon/tests/control_agent.rs` 의 명령 목록 이름 골든 · `agent/tests/ts_export.rs` 의 내보내기 목록. [메인] `agent.interrupt` 는 `agent.cancelQueuedInput` 처럼 **버스 전용**이다(`CLI_AGENT_VERBS` 에 없다) — CLI 는 일반 호출꼴 `engram agent.interrupt …` 로 닿는다(인자 모양 = `engram commands agent.interrupt`).

- `commands.rs` `declare_commands!`(`:31-226`): `catalog_version: 5 → 6`(`:41` · v6 주석 한 줄 — 이름이 늘었다) · 새 항목
  ```text
  /// 도는 턴을 끊는다(≠ kill — 프로세스는 산다). `outcome` = `requested`(끊기를 보냈다 — 턴이 실제로 멈췄는지는 턴 끝 사건이 알린다).
  #[effect(Write)] #[since(6)]
  "agent.interrupt" => args AgentInterruptArgs { target: String } -> ok AgentInterruptOk { outcome: String } errors [NOT_FOUND, CONFLICT];
  ```
- `INPUT_AFFECTING`(`:234`)에 `"agent.interrupt"` — WS `Interrupt` 가 임대를 보는 것(`connection_core.rs:1222-1230`)과 같게.
- `AgentCommandHost`(`:263-291`)에 `fn interrupt_agent(&self, id: AgentId) -> Result<(), PtyError>` · `impl for AgentManager`(`:306`) = `AgentManager::interrupt`(`manager.rs:2875-2877`) · 시험 `FakeHost`(`:1343`).
- `make_table`(`:531`)에 한 줄 · `verb_interrupt`: 공백 검문 → `resolve` → `host.interrupt_agent` → `Ok` = `requested` · 산 세션 없음 = NOT_FOUND · `PtyError::Unsupported`(codex 「중단할 턴이 없다」(`backend/codex/transport.rs:4045-4061`) · claude 턴 열림 문 닫힘(§3-4) · 능력 없는 통로 — 7판부터 PTY 통로도 여기 든다(ADR-0245)) = CONFLICT(「끊을 턴이 없다 · 이 에이전트는 끊기를 지원하지 않는다」) · 그 밖 = INTERNAL.
- 생성물: `crates/engram-dashboard-agent/bindings/AgentInterruptArgs.ts` · `AgentInterruptOk.ts` · `commands.schema.json`(CI sync 게이트 대상).
- 시험: `tests/command_declarations.rs:61-70` 에 `INPUT_AFFECTING.contains(&"agent.interrupt")` · 표 시험(결말 셋 매핑).
- **덩치 판정**: 선언 1 · trait 메서드 1 · 동사 1 · 시험 — `cancelQueuedInput` 과 같은 몫이라 과하지 않다. 프론트만 가는 대안(버스 없이 창 명령만)은 필요 없다 — 적어 두면: `help` 없는 창 명령만 남기고 버스 LLM 은 끊기를 못 부른다.

### 3-4. claude JSON 끊기 (U1) — 설계

**제어 줄**: `{"type":"control_request","request_id":"interrupt:<uuid v4>","request":{"subtype":"interrupt"}}\n` — 모양은 공식 Agent SDK(조사 §2-1). `cancel_line`(`claude/mod.rs:698-724`)처럼 typed struct 로 직렬화한다. ★`cancel_queued` 같은 선택 능력은 싣지 않는다★ — 우리가 원하는 뜻은 codex 와 같다: **도는 턴만 멈추고, 이미 받아 둔 대기 입력은 다음 턴으로 간다**(ADR-0235 결정 4 — `Interrupted` 는 멈춤을 세우지 않는다 · 결정 5 — 남은 글은 다음 한 턴에). 그것이 CLI 응답의 `still_queued`(끊은 뒤에도 돌 대기 메시지 목록)의 뜻인지는 스파이크가 확인한다(§3-5 S3).

**줄이 통로에 닿는 길** — ①(메인 수락): 지시는 「`cancel_async_message` 가 닿는 길을 따라 하라」였다. 그 길은 `MidTurnPolicy::SessionClassified { cancel_line }`(`types.rs:215`)을 세션이 입력 자물쇠 안에서 `send_input` 하는 것이다(`session.rs:563-580`). 끊기는 그 길에 맞지 않는다: ① 능력 `control.interrupt` 는 **통로의 caps** 에서 온다(`session.rs:656-657` → `stdio.rs:425-455`) ② 끊기는 입력 id 에 묶이지 않아 입력 자물쇠가 지킬 순서가 없다. 그래서 **`structured` 주입과 같은 모양**을 쓴다(`stdio.rs:52-56` — 「구조화냐」를 backend 가 주입):

```rust
// transport/stdio.rs — 통로는 이 바이트의 뜻을 모른다(바보 파이프 유지)
/// `None` = 「지금은 끊을 턴이 없다」(backend 가 판정) — 통로는 그대로 `Unsupported` 로 옮긴다.
pub type InterruptLine = Arc<dyn Fn() -> Option<Vec<u8>> + Send + Sync>;
impl StdioTransport {
    /// 「지금 턴을 멈춰 달라」는 줄을 만드는 backend 함수를 꽂는다. 없으면 오늘처럼 `Unsupported`.
    pub fn with_interrupt(mut self, line: InterruptLine) -> Self { self.interrupt = Some(line); self }
}
// interrupt(): Some(f) => match f() { Some(b) => self.input.push(b) · None => Unsupported("끊을 턴이 없다") }
//              · None => 오늘의 Unsupported(`:371-376`)
// capabilities(): control.interrupt = self.interrupt.is_some()(`:441`) — 능력은 「끊을 수 있는 통로」이지 「지금 턴이 있다」가 아니다
```

- [고름] `open` 인자를 늘리지 않고 빌더로 둔다 — `StdioTransport::open(` 호출이 시험 포함 9 곳이다(`rg "StdioTransport::open\(" crates`). 기존 단언(`stdio.rs:507` · `session.rs:1249` — 주입 없는 통로는 `Unsupported`·`false`)은 그대로 참이다.
- 줄은 입력 큐(`stdio.rs:46-47` — 라이터 스레드 하나)에 들어가 사용자 줄과 **통째로** 직렬화된다(줄 섞임 없음). 입력 자물쇠(`input_order`)는 안 탄다 — 락 순서 불변식(ADR-0006)에 새 간선이 없다.
- `backend/claude/mod.rs` `open_spawn`(`:424-433`): `open` 은 튜플 `(StdioTransport, Option<u32>)` 을 돌려준다(`stdio.rs:72-76`) — 풀어 낸 뒤 꽂는다: `let (t, pid) = StdioTransport::open(spec, true, Some(decoder))?; let t = t.with_interrupt(interrupt_line(Arc::clone(&turn_gate)));`. claude 지식(줄 모양 · 턴 열림 판정)은 `backend/claude` 에만 산다(「백엔드 확장」).
- 응답 `control_response`(`request_id` 머리 `interrupt:`)는 **번역하지 않는다** — `cancel_response_event`(`:1215-1235`)는 `cancel:` 머리만 보므로 이미 `None` 이다. 응답이 왔다고 턴이 멈춘 것은 아니다(§3-5 S4).

**턴 열림 문** — codex 는 턴이 없으면 끊기를 거절한다(`backend/codex/transport.rs:4045-4061` — `Unsupported("… 중단할 턴이 없다")`). claude 도 같은 버스 계약을 따른다(§3-3 매핑 = CONFLICT):

- `backend/claude` 안의 화신 공유 값 `turn_gate: Arc<TurnGate>` — `DeliveryAck` 공유와 같은 모양이다(`open_spawn` `:424` 에서 하나 만들어 decoder(`stream_decoder` `:758`)와 끊기 줄 함수에 같은 `Arc` 를 준다). 선 타입·세션·통로는 이 값을 모른다 — `types.rs` 에 두지 않는다(`DeliveryAck` 는 세션이 읽어 거기 있다 · 이 값은 backend 만 읽는다).
  ```rust
  struct TurnGate { open: AtomicBool /* , interrupt_sent: AtomicBool — ④ 예비일 때만 */ }
  ```
- **세우는 쪽 = 라이브 decoder**: 한 라이브 줄이 턴 진행 신호 사건(`classify_turn` `:538-545` → `Progress` — 사용자 되울림 `Structured{user}` · `assistant` 블록 · 흘린 `TextDelta` · `command_lifecycle` `started` 의 `Delivered`)을 하나라도 내면 `open = true`. **`result` 에서 `open = false`**(오늘 번역 뒤). 이어받기(`LineSource::Transcript`)는 건드리지 않는다. [고름] 「턴 중」의 정의를 턴 관측과 같은 분류기 하나에서 뽑는다 — 두 정의가 갈리지 않는다.
- **읽는 쪽 = 끊기 줄 함수**: `open` 이면 `Some(줄)`(④ 예비면 `interrupt_sent = true` 도), 아니면 `None` → 통로 `Unsupported` → 버스 CONFLICT · WS `Interrupt` 는 오류 응답(`connection_core.rs:1222-1230`). ③ 은 이것으로 해소된다(메인 판정) — 턴이 열리기 전의 Esc 는 정직하게 거절되는 무동작이고, 미루는 큐를 두지 않는다. ★단 S4 가 「되울림이 `system/init` 보다 먼저 오고 그 틈의 끊기는 무시된다」를 보이면 문이 여는 지점을 **그 턴의 `system/init` 뒤 첫 진행 줄**로 늦춘다★(같은 값 · 같은 거절 — 여는 줄만 바뀐다).
- ★**잔여 경합 — 좁힐 뿐 닫지 못한다**★: 문을 읽고 줄을 큐에 넣는 사이, 또는 줄이 CLI 에 닿기 전에 그 턴의 `result` 가 나오고 **CLI 가 스스로 다음 턴을 열면**(대기 중이던 B · 우편이 나른 턴) 그 줄은 **다음 턴**을 끊는다. 프론트 `streaming` 이 턴 끝 사건까지 늦게 꺼지므로(§3-2) 늦은 Esc · 버스 호출이 이 틈에 든다. claude `control_request` 에는 턴 id 가 없어(codex `turn/interrupt` 는 `turn_id` 를 싣는다) **벤더 프로토콜로는 닫을 수 없다**. 증상 = B 가 한 번 끊긴 턴으로 닫힌다(중단 행 — 6판: claude 는 끊김 표시 행 · 중단 행은 대비로만 · ADR-0243 · 오류 아님 · 멈춤 불변 — 우편이 멈추지는 않는다). 스파이크 S7 이 크기를 잰다.

**끊긴 턴의 `result` 분류** — 지금 번역기(`:944-970`)는 `is_error || subtype.starts_with("error")` 면 `Error(RESULT_FAILURE_DETAIL…)` 를 낸다. 끊긴 턴의 `result` 가 `is_error: true` 에 `terminal_reason` = `aborted_streaming` | `aborted_tools` 로 올 수 있다(SDK `types.py` · SDK issue #429 — 메인 대조). 그대로 두면 Esc 한 번이 **오류 행을 그리고 오류 뒤 멈춤(`last_end_failed`)을 세워** 우편을 멈춘다 — 의도와 반대다. 그래서:

- `interrupted(result)` 판정(스파이크가 확정 — S2): `subtype == "interrupted"`(오늘도 오류 아님 — `:951-956`, 유지) **또는** `terminal_reason ∈ {"aborted_streaming","aborted_tools"}`. ④(메인 판정 = 예비로만 둔다): 두 칸 모두 끊김을 가르지 못할 때**만** 「이 결과 앞에 우리가 끊기를 보냈다」 표식 `TurnGate.interrupt_sent` 를 짓는다 — 끊기 줄 함수가 **문이 열려 있을 때만**(줄을 돌려줄 때만) 세우고, 번역기가 **그 턴의 `result`** 에서 읽고 지운다(문을 닫는 같은 자리). 그래서 한가할 때의 끊기 호출은 표식을 세우지 못해 다음 턴의 진짜 오류를 가리지 않는다. 남는 대가 = 끊기와 겹친 **같은 턴의** 진짜 오류는 끊김으로 접힌다 · 위 잔여 경합에 걸리면 표식은 앞 턴의 `result` 에서 쓰이고, 줄이 끊은 다음 턴은 오류로 읽힐 수 있다(그 턴에 오류 뒤 멈춤이 선다). 스파이크가 두 칸 중 하나로 가르면 이 칸은 짓지 않는다.
- 참이면 `Usage`(오늘처럼) 뒤 **`OutputEvent::TurnEnd { turn_id: None, outcome: TurnOutcome::Interrupted }`** 하나 — `Error` 도 `MessageDone` 도 내지 않는다.
  - 턴 분류기(`classify_turn` `:553-557`)가 이미 `Interrupted → Ended(Other)` 로 적는다 → 턴은 끝나고 · 오류 뒤 멈춤을 세우지도 풀지도 않는다(ADR-0234 「뜻 모를 턴 끝은 멈춤을 건드리지 않는다」).
  - 누산기(`structuredAccumulator.ts:213-220`)가 `outcome: 'interrupted'` 행(「응답이 중단됐습니다」 — `ko.ts:137`)과 구분선을 그린다 — codex 끊김과 같은 모양. 프론트·선 타입 무변경. ★6판 정정(ADR-0243 · 사용자 결정 ⑫ 2026-09-28)★: 선 타입은 그대로지만 **프론트는 바뀐다(FE-2c)** — claude 는 그 턴에 합성 줄의 표시 행(`interruptNote` — 아래 항목)이 이미 있으면 이 중단 행을 그리지 않는다(구분선은 그대로). 표시 행이 없으면(줄이 안 옴) 중단 행이 그대로 선다 · codex 는 변함없다.
  - ②(메인 수락 — 턴 관측 `Ended(Other)` · `last_end_failed` 불변 · 초인종 울림 · 누산기 결말 행 · 대기 입력 골든 무영향 확인): `types.rs:70`(「`MessageDone` 을 이것으로 이주시키지 않는다 — claude 는 그대로 `MessageDone` 을 쓴다」)과 `claude/mod.rs:551`(「이 decoder 는 `TurnEnd` 를 내지 않는다」)이 이 변경과 부딪힌다. 이주가 아니라 **끊김 한 갈래만** `TurnEnd` 로 가는 것이고 같은 doc 이 「어느 쪽을 내는지는 각 decoder 가 정한다」고도 적는다. 또 오늘 `subtype:"interrupted"` 는 `MessageDone`(구분선만)으로 닫히는데 이 변경 뒤엔 중단 행이 붙는다(오류 아님은 그대로). 두 주석은 이 변경과 함께 고친다.
- **합성 사용자 줄 `[Request interrupted by user]`(6판 · ADR-0243)** — B2 실측: 끊긴 턴마다(14/14) CLI 가 `result` **앞에** `user` 줄 `content:[{"type":"text","text":"[Request interrupted by user]"}]`(도구 중이면 `…for tool use]`)을 내고 transcript 에도 남긴다(조사 `docs/research/claude-interrupt-spike-2026-09-28.md` §1 · §10-1). 오늘은 `Structured{kind:"user"}` → 사용자 말풍선이다.
  - **알아보기(번역기 — `claude/mod.rs`)**: `user` 줄의 `message.content` 가 **배열이고 `text` 블록 정확히 하나**이며 그 글이 `[Request interrupted by user` 로 시작하고 **`isReplay: true` 가 아니면** `Structured{kind:"user"}` 대신 **`Structured{kind:"interrupted", json:{"text":"<원문>"}}`** 로 낸다. 라이브 · 이어받기(`LineSource::Transcript`) 둘 다. 평문 문자열 content 는 보지 않는다(오늘 `consume_line` 이 배열 아닌 content 를 버린다 — `claude/mod.rs:973-976`@HEAD · 관측된 적 없다). ★`isReplay: true` 는 뺀다★(메인) — 사용자가 그 접두로 시작하는 글을 치면 우리 uuid 의 되울림이 오고, 그것을 바꾸면 누산기의 대기 행 검사 · uuid dedup 을 건너뛰고 진행 신호도 사라진다 · 진짜 합성 줄에는 `isReplay` 가 없다(조사 §0 · §1). 벤더 문자열 판별이라 문구가 바뀌면 오늘의 말풍선으로 돌아간다(잃는 것은 없다). 위치(`result` 바로 앞)로 알아보지 않는다 — 이어받은 transcript 에는 `result` 가 없다(`parse_transcript_events` 주석 `claude/mod.rs:1536`@HEAD 「실측 2026-08-17 — 최근 transcript 12개 전부 0건」 · 메인 2026-09-28 확인 — S1 스파이크 세션 transcript · fixture `claude_transcript.jsonl` 의 `result` 는 손으로 지은 것이다).
  - **분류(`classify_turn`)**: 이 사건은 **`None`**(진행 아님) — 끝에 대한 표시다. 진행으로 세면 `result` 뒤에 올 때 「턴 중」이 다시 켜져 30 분 우편이 막힌다(ADR-0127). 그래서 턴 열림 문도 열지 않는다(문은 진행 신호에서만 연다 · 줄이 올 때 문은 이미 열려 있다 — 실측 14/14 · claude 2.1.280 · 가능성 높음). 끊김 판정은 그대로 `result` 의 `terminal_reason` 이 한다(이 줄에 기대지 않는다).
- **끊김 직전의 잘린 `assistant` 줄**(SDK issue #338): F4 뒤에는 흘린 블록의 잘린 완결 본문은 버려지고(사용자는 흘린 만큼을 이미 봤다) 안 흘린 블록은 잘린 본문 그대로 나간다. 끝은 위 중단 행이 표시한다(6판: claude 는 끊김 표시 행 · 중단 행은 대비로만 — ADR-0243).

### 3-5. 스파이크 (B2) — 출구 조건

**방법**: Phase 0 하네스(`.claude/handoff/attachments/20260925-midturn-phase0/claude_harness.js`)로 claude 2.1.280 을 **최종 스폰 인자**(F4 가 먼저 착지 — `--include-partial-messages` 포함)로 띄우고 `{t,dir,line}` 을 기록한다. 결과 = 새 보고서 `docs/research/claude-interrupt-spike-2026-09-28.md` + fixture `backend/claude/fixtures/interrupt_s1.jsonl`(`fixtures/README.md` 의 가공 규칙).

| # | 시행 | 기록 | 통과 조건 |
|---|---|---|---|
| S1 | 글 흐르는 중 · 도구 도는 중 각각 끊기 | 끊은 뒤 오는 줄 전부 · `result` 의 `subtype`·`is_error`·`terminal_reason` | 턴 끝 줄(`result`)이 **반드시** 온다 |
| S2 | S1 의 `result` | 위 칸 | `subtype` 또는 `terminal_reason` 이 끊김을 가른다(아니면 ④ 예비 표식을 짓는다 — §3-4) |
| S3 | 도구 중 B 를 써서 `queued` 를 본 뒤 끊기 | 응답의 `still_queued` · B 의 `command_lifecycle` | B 가 `still_queued` 에 있고 · 뒤이어 B 의 `started` 가 새 턴에서 온다(B 가 돈다) · `cancelled`/`discarded` 가 오지 않는다 |
| S4 | 새 턴의 사용자 되울림 직후 · `system/init` 전에 끊기(하네스가 직접 쓴다 — 문을 거치지 않는다) | 되울림과 `init` 의 순서 · 응답 · 턴이 멈췄나 | 멈추면 통과(문은 되울림에서 연다). 되울림이 `init` 보다 먼저 오고 그 틈의 끊기가 무시되면(SDK issue #429 — 2.1.241 보고, 2.1.280 미검) 문이 여는 지점을 `init` 뒤 첫 진행 줄로 늦춘다(§3-4 — ③ 판정의 구현 조정 · 멈춤 사유 아님) |
| S5 | S1 중 잘린 `assistant` 줄 | 흘렸나 · 완결 줄이 왔나 | 기록만(§3-4 끝 문단의 전제 확인) |
| S6 | 턴이 없을 때 끊기(하네스가 직접 — 문이 막는 경로지만 S7 의 경합이 이 줄을 한가한 CLI 에 닿게 할 수 있다) | 응답 · 그 뒤 줄 | 턴 신호를 켜는 줄(`assistant`·`user`·`stream_event` 글)이 오지 않는다 — 오면 경합으로 닿은 한가 끊기가 「턴 중」을 켜 30 분 막힘 경로가 된다 |
| S7 | 도구 중 B 를 대기시키고 A 의 `result` 직후(B 의 턴이 열리기 전후) 끊기 | B 의 턴이 끊겼나 · B 의 `result` 모양 · 우리 문 값의 궤적 | 기록만 — §3-4 잔여 경합의 실물 크기. B 가 끊겨도 오류 행 · 오류 뒤 멈춤이 없으면 받아들인다(있으면 S2 의 판정이 그 모양을 못 가른 것 — 멈춘다) |
| S8 | 문 궤적 — S1·S3 채취본을 decoder 에 먹여 문을 여닫는 줄을 적는다 | 열림 = 첫 진행 줄 · 닫힘 = `result` | 턴 밖(한가 구간 · `result` 뒤)에 문을 여는 줄이 없다 |

**S1 기록 추가**(2026-09-27 · 통과 조건 무변경): 도구 중 끊기에서 도는 Bash 도구가 끊은 뒤 출력을 멈췄나 · 그 프로세스가 살아 있나를 기록한다. 대조 = 대화형 Claude Code CLI 는 Esc 에 도는 Bash 도구를 죽인다(실측 2026-09-27 — §9). 우리 stream-json `control_request` 끊기가 같은지는 미검이고 이 기록이 가른다 — 어느 쪽이든 출구 조건은 그대로다. 같은 끊기에서 그 도구의 `tool_result` 의 `is_error` 값과 본문 모양도 기록한다 — 대조 = 이 세션의 대화형 CLI 기록에서 `[Request interrupted by user for tool use]` 바로 앞 `tool_result` 가 `is_error:true` · 본문 「The user doesn't want to proceed with this tool use…」(실측 2026-09-27 · 우리 stream-json 경로는 미검 · 같을 가능성 높음). 기록만이다 — 출구 조건은 그대로고 멈춤 사유가 아니다(표시 규칙은 그 값에 기대지 않는다 — §4-7 ④ · §11 ⑪).

**스파이크 결과에 기대는 불변식**(결과가 지저분하면 **작업을 멈추고 사용자에게 돌아간다**):

- 「턴 관측 정리 = 두 지점뿐」·30 분 fail-open(ADR-0127) — S1 · S6 · S8. 끊은 턴이 끝 줄 없이 남으면 `in_turn` 이 30 분 붙는다.
- 「오류 뒤 멈춤」 `last_end_failed`(ADR-0231 N11 · ADR-0234) — S2. 끊김이 실패로 읽히면 Esc 한 번이 우편을 멈춘다.
- 「대기 입력 상태 = 링 사건 한 줄기」 환원 규칙과 claude 수명주기 번역표(`claude/mod.rs:1164-1181` — `cancelled` → `Dropped{Unknown}` 묘비) · ADR-0235 결정 2 「글은 늘 보인다」 — S3. CLI 가 끊으며 대기분을 버리면 B 가 말풍선 없이 목록에서 사라진다.
- ADR-0226 첫 제출 래치 — 기대지 않는다(claude 는 보내기 전에 센다 · 끊기는 래치를 안 건드린다).

### 3-6. 시험

- 프론트: `agentCommands.test.ts` — `agent.interrupt` 인자 검문 · `interruptAgent` 호출 · 임대 거절 → `CONFLICT:` · `help` 없음. `interruptKey.test.ts` — 술어 표(아래 조건 하나씩 거짓 · 전부 참). `RichSlot.test.tsx` — 조건 표(수식키 · repeat · 조합 · 오버레이 표지 · `defaultPrevented` · 안 돎 · 능력 거짓 · 부재) 각각 미발화 / 전부 참이면 한 번 발화 · 입력창 글 유지 · 본문 클릭 뒤 Esc 발화(U6). ★7판(FE-1.4 · ADR-0244)★: 스토어 — 세움 · 턴 끝에 걷힘 · 거절에 걷힘 · 에이전트마다 따로. `agentCommands.test.ts` — 보낼 때 세우고, 창 명령이 어떤 오류로 끝나든(임대 `CONFLICT:` · 통로의 「unsupported」 · 연결 오류) 걷는다 · 이 창에 그 에이전트의 채팅 뷰가 없으면 세우지 않는다 · 끊는 중의 둘째 호출은 다시 보내지 않는다(`requested` 또는 대기 중 요청의 결과). `interruptKey.test.ts` — `interrupting` 이 참이면 다른 조건이 전부 참이어도 거짓. `RichSlot.test.tsx` — Esc 한 번 → 명령 한 번 · 곧바로 「중단하는 중…」 · 턴 끝 전 둘째 Esc → 명령 추가 호출 없음 · 턴 끝(`MessageDone` · `TurnEnd{Interrupted}`) 뒤 표시가 사라지고 다음 턴의 Esc 는 다시 발화 · 거절 뒤 표시가 사라지고 다음 Esc 가 다시 발화 · (리뷰 요청 둘) A 의 끝과 B 의 시작이 한 플러시에 와도 걷힌다(있음 — FE-1.4 코더) · 보냄 → 여는 프레임 전 Esc → [`TurnEnd`, 다음 턴의 `TextDelta`] 한 묶음이어도 걷힌다(경계 수 규칙) · 턴을 열지 않고 대기에 든 보냄(`Queued` → Wait 꺼짐)은 `streaming` 뒷받침으로 걷힌다 · 끊는 중에 kill → 부재가 되는 즉시 걷힌다(재스폰이면 onReset 도 — kill 전용 시험은 없고 부재 · reset 시험이 덮는다) · 이 화신의 첫 `'live'` 전 프레임(새 뷰의 이력 · 새 화신의 이력) 속 옛 턴 끝으로는 걷히지 않는다 · 첫 `'live'` 따라잡기에 열린 턴이 없으면 걷힌다 · 둘째 뷰의 구독 오류는 성한 뷰의 상태를 걷지 않는다. `StructuredTextView.test.tsx` — 끊는 중이면 꼬리가 정지 아이콘 + 「중단하는 중…」 · 아니면 오늘의 Wait.
- 백엔드: `stdio.rs` — 주입 함수가 `Some` 이면 `interrupt()` 가 줄 한 벌을 큐에 넣고 · `None` 이면 `Unsupported` 에 큐 무변경 · 주입 있으면 caps 참(문 값과 무관) · 주입 없으면 오늘 단언. `claude/mod.rs` — 끊기 줄 골든(`cancel_line_bytes_golden_and_its_answer_round_trips` `:4040` 모양) · 스파이크 fixture 로 끊긴 `result` → `[Usage?, TurnEnd{Interrupted}]` · `classify_turn` → `Ended(Other)` · `interrupt:` 응답 → 사건 없음 · 기존 `result_error_handbuilt.jsonl` 은 여전히 `Error` + `MessageDone`(진짜 오류 회귀).
- 백엔드 — 턴 열림 문: 스폰 직후 함수 = `None` · 사용자 되울림 · `assistant` · 흘린 델타 · `Delivered` 줄 뒤 = `Some` · `result` 뒤 = `None` · `Dropped` · `control_response` · `system` 같은 신호 없는 줄로는 안 열린다 · 이어받기 원문으로는 안 열린다 · (④ 예비를 지을 때만) 한가할 때의 호출은 표식을 안 세워 다음 턴의 `is_error` 결과가 여전히 `Error` · 표식은 그 턴 `result` 에서 지워진다.
- ★B3 이 **의도적으로** 고쳐 쓰는 기존 시험 둘★ — 회귀로 읽지 말 것: `result_interrupted_subtype_emits_only_done_no_error`(`claude/mod.rs:3390`) · `result_interrupted_subtype_with_is_error_false_emits_only_done`(`:3402`). 둘 다 `subtype:"interrupted"` 에 `["done"]` 을 단언하는데 ② 뒤 기대값은 `TurnEnd{Interrupted}` 한 벌(Error 없음 그대로)이다. 이름도 기대에 맞게 바꾼다. §5-5 의 11 번(기존 fixture 무수정)과는 다른 축이다 — 그쪽은 F4 가 fixture 사건열을 안 바꾼다는 증거이고 이 둘은 F2 가 뜻을 바꾸는 손 시험이다.

---

## 4. F3 도구 호출 묶기

### 4-1. 선 타입 — `category` (Rust ↔ TS 접점, ★워커 간 고정★)

**agent 도메인** `crates/engram-dashboard-agent/src/types.rs`:

```rust
/// 도구 호출의 중립 종류 — 각 backend 번역기가 정한다(ADR-0004). 벤더 도구 이름·item 타입은 여기 안 온다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCategory { Read, Search, List, Edit, Command, Web, Agent, Mcp, Other }
// OutputEvent::ToolCall(:46-53) 에 칸 하나: `category: ToolCategory` — Option 아님(번역기는 늘 하나를 고른다)
```

**wire** `crates/engram-dashboard-protocol/src/messages.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub enum ToolCategory { Read, Search, List, Edit, Command, Web, Agent, Mcp, Other }
// StructuredEvent::ToolCall(:719-726) 에:
//   #[serde(default, skip_serializing_if = "Option::is_none")]
//   #[ts(optional)]
//   category: Option<ToolCategory>,
```

- **TS 접점(고정)** — 생성물 `crates/engram-dashboard-protocol/bindings/ToolCategory.ts` = `export type ToolCategory = "Read" | "Search" | "List" | "Edit" | "Command" | "Web" | "Agent" | "Mcp" | "Other";` · `StructuredEvent.ts` 의 ToolCall 에 `category?: ToolCategory`. ★FE-2 는 B4 를 기다리지 않는다★ — 자기 파일 `src/components/slot/structuredAccumulator.ts` 에 **같은 아홉 낱말의 지역 리터럴 합** `export type ToolCategory = "Read" | … | "Other"` 을 선언해 쓰므로(생성물 import 없음) FE-2 의 모든 커밋이 B4 전에 `tsc` 초록이다. 교체는 §7 의 **I1**(주인 · 시점이 거기 있다)에서 한다.
- **호환**: 옛 데몬 → 칸이 없다 → 프론트 `Other`. 옛 셸 → 모르는 칸을 무시한다(셸은 tag1 JSON 을 해석하지 않고 나른다 — `rg StructuredEvent src-tauri/src` 0 줄). `PROTOCOL_VERSION` 은 올리지 않는다 — `TurnEnd`(`:743-763` 주석)와 같은 판단(데몬→셸 한 방향 · 옛 쪽엔 오독할 것이 없다).
- **데몬 변환** `crates/engram-dashboard-daemon/src/connection_core.rs:777-789` — `category: Some(map(category))`(일대일 match).
- **링 무게** `output_core.rs:1225-1230` `estimate_cost_bytes`: 고정 크기 enum 이라 무게 0 — 구조 분해만 `category: _` 로 넓힌다.
- **모든 생성 자리 · 망라 패턴을 한 커밋에**(컴파일러가 가리킨다): `claude/mod.rs:1108` · `codex/decoder.rs:1136` · `codex/decoder.rs:1606` 시험(`..` 없는 구조 분해 패턴 — 칸이 늘면 깨진다) · `output_core.rs` 시험(`:2152-2172` · `:2248`) · `daemon/src/agent_conn.rs:592`·`:618` · `connection_core.rs:5150-5160` · `protocol/src/messages.rs:1047` 시험. ts-rs 생성물은 `cargo test -p engram-dashboard-protocol` 이 굽는다(CI sync 게이트) — ★어느 워커도 `*/bindings/` 아래 파일을 손으로 쓰지 않는다★(§7).
- `OutputChunk::ToolCall`(`messages.rs:907` — S14 스냅숏 잔재)은 건드리지 않는다.

### 4-2. 백엔드 분류

- **claude**(`claude/mod.rs` `consume_block` `tool_use` 갈래 `:1092-1115`) — 도구 이름 표 `fn tool_category(name: &str) -> ToolCategory`:
  - `Read` ← `Read` · `NotebookRead` / `Search` ← `Grep` · `Glob` / `List` ← `LS` / `Edit` ← `Edit` · `MultiEdit` · `Write` · `NotebookEdit` / `Command` ← `Bash` · `PowerShell` · `BashOutput` · `KillShell` · `KillBash` / `Web` ← `WebFetch` · `WebSearch` / `Agent` ← `Task` · `Agent` / `Mcp` ← `mcp__` 로 시작 / 나머지(`TodoWrite` 등) `Other`.
  - [고름] `Glob` 을 `Search` 로 — Claude Code 자신이 「파일을 찾는다」로 묶는다. 표는 정확 일치(대소문자 구분)라 모르는 새 도구는 `Other` 로 떨어진다.
- **codex**(`codex/decoder.rs` `tool_call` `:1126-1146`) — item 타입과 `commandActions`:
  - `commandExecution` → `commandActions[].type`(판독: `read` · `listFiles` · `search` · `unknown`). 전부 `read` = `Read` · 전부 `listFiles` = `List` · `search` 가 있고 `unknown` 이 없다 = `Search` · `read`·`listFiles` 만 섞임 = `Read` · `unknown` 이 하나라도 있거나 배열이 없거나 비었다 = `Command`.
  - `fileChange` = `Edit` · `webSearch` = `Web` · `mcpToolCall` = `Mcp` · `collabAgentToolCall` = `Agent` · `dynamicToolCall` = `Other`.
  - ★크기 상한 토막은 문제가 안 된다★: 종류는 `tool_call` 이 받는 **온전한 item**(`&Value`)에서 먼저 정하고 `args_json` 만 상한을 지난다(`bounded_args_json` `:1159`). 지시의 「토막이면 item 타입으로 폴백」은 그래서 생기지 않는다 — `commandActions` 가 없을 때의 폴백(`Command`)만 남는다. 이력 복원(`ItemOrigin::History`)도 같은 함수를 지난다.

### 4-3. 프론트 — 묶기 함수 (렌더 시점 · 누산기는 묶음을 만들지 않는다)

- **누산기** `structuredAccumulator.ts:34`(`tool` 변형)에 `category: ToolCategory` · `:143-151` 에서 `category: normalizeToolCategory((ev as { category?: unknown }).category)` — 아는 9 낱말이면 그대로, 없거나 모르면 `'Other'`. 방어 읽기라 생성물이 오기 전·옛 데몬·더 새 데몬 모두에서 선다.
- **새** `src/components/slot/chat/toolRuns.ts`(순수):

```ts
export type DisplayRow =
  | { kind: 'item'; item: StructuredItem }
  | { kind: 'toolGroup'; key: string; members: StructuredItem[]; calls: ToolItem[]; live: boolean }
/** turnOpen = StructuredTextView 의 `streaming`(RichSlot `:370`). rowKind = StructuredTextView 의 `rowKindOf` 그대로. */
export function groupToolRuns(items: readonly StructuredItem[], turnOpen: boolean,
  rowKind: (item: StructuredItem) => ChatRowKind): DisplayRow[]
/** vendorErrorIds = claude 파싱(`buildToolResultMap`)의 `isError` 호출 id(`vendorErrorIdsOf(results)`). codex 는 항목의 `resultMark` 로 온다(§4-7 ⑧). */
export function summarizeGroup(calls: readonly ToolItem[], vendorErrorIds: ReadonlySet<string>,
  declinedMark: 'own' | 'error' = DECLINED_MARK):
  { counts: ReadonlyArray<readonly [ToolCategory, number]>; errors: number; declined: number }
/** 한 호출의 판정 — 묶음 요약과 펼친 행 배지가 이 함수 하나를 쓴다(§4-7 ⑧). */
export function toolCallVerdict(mark: ToolResultMark | null | undefined, vendorError: boolean,
  declinedMark?: 'own' | 'error'): 'error' | 'declined' | null
```

- ★구현 판(FE-2b-1)★: `groupToolRuns` 의 셋째 인자 `rowKind` 는 구현이 더했다 — `rowKindOf` · `parseToolResult` 는 `StructuredTextView` 안에 있고, 묶기 함수가 그 판정을 받아 쓰므로 벤더 결과 파싱이 한 곳에 남는다(ADR-0051 — 레일 계산과 같은 판정). 요약 줄 칸(순서 · 톤)은 `summaryParts(summary)` · 사유 줄 키는 `declinedReasonKey(mark)` · 벤더 오류 id 는 `vendorErrorIdsOf(buildToolResultMap(items))` · 종류 정규화는 누산기의 `normalizeToolCategory` 한 벌. `members` 는 그리는 멤버(도구 행 · 비지 않은 생각)만 담는다 — 묶음 안의 skip 행은 행 목록에서 빠진다.

- **묶음 규칙**:
  - 후보 = 첫 `tool` 행부터 마지막 `tool` 행까지. 그 사이에 올 수 있는 것 = 그리지 않는 행(`rowKindOf` = `skip` — usage · claude `tool_result` 운반 행 · 빈 생각 · `StructuredTextView.tsx:377-395`)과 **비지 않은 생각**(U5 흡수 — 대안이면 이 한 줄이 「끊는다」로 바뀐다).
  - 그 밖의 **그리는** 행은 전부 끊는다 — 글 · 사용자 말풍선 · 구분선(턴 끝) · 결말 · 오류 · 모르는 사건 · 그 밖의 `structured`.
  - `tool` 이 ≥2 일 때만 묶음. 하나면 오늘처럼 그 행 그대로.
  - 마지막 `tool` 뒤의 생각·skip 행은 멤버가 아니다(흐름으로 돌아간다).
  - `live` = `turnOpen` 이고 묶음 뒤에 오는 행이 전부 skip·비지 않은 생각뿐이다. ★뒤에 생각이 왔다고 접지 않는다★ — 그 뒤에 도구가 이어지면 다시 펼쳐지는 깜빡임이 된다.
  - `key` = 첫 호출의 백엔드 id(`tool:<id>`) · 없으면 `item:<itemId>` — 누산기는 같은 사건열을 같은 `itemId` 로 재구성하므로(`structuredAccumulator.ts:11-15` 멱등 불변식) replay 뒤에도 같은 키다. 앞 묶음이 이미 같은 `tool:<id>` 를 쓰면 뒤 묶음은 `item:<itemId>` 다(구현 판). ★한계★: 링에서 밀려난 뒤 시작하는 재구독 replay 는 사건열이 달라 `item:` 키가 바뀌고, 묶음 머리 호출이 밀려나면 `tool:` 키도 바뀐다 — 그 묶음의 고른 펼침은 잃고 자동 규칙으로 돌아간다. ★한계(7판 · 모양만)★: 슬롯의 대화 뷰가 내려가 있는 동안 에이전트가 재스폰하면 새 화신 비우기(`clear` — RichSlot onReset)가 불리지 않아, 그 슬롯에 남은 `item:<n>` 키의 고른 펼침이 새 화신의 같은 키 묶음에 이월될 수 있다(표시만 바뀐다 · 잃는 정보는 없다).
- **렌더**(`StructuredTextView.tsx:514-561`): `items.map(renderItem)` 을 `groupToolRuns(items, streaming)` 의 행 목록으로 바꾼다. ★ADR-0051 불변식★ — 레일 위치(`chat/railPositions.ts`)는 **행 목록**으로 계산한다: 묶음 = `'assistant'` 한 행 · 항목 = 오늘 `rowKindOf`. 묶음은 `ChatRow rail` 하나로 그리고, 펼쳤을 때 멤버는 그 안에서 행 컴포넌트(`ToolItemRow` · `ThoughtRow`)만 그린다 — 안쪽에 `ChatRow` 레일을 두지 않으므로 레일 계산과 DOM 이 한 몸으로 남는다. `isRenderedItem`(`:402-404` — RichSlot 이 쓴다)은 항목 단위 그대로.
- ★`StructuredTextView` 는 순수 렌더로 남는다(`:5` 책임 주석 · `:527` ADR-0050/0051 순수성)★ — 이 컴포넌트에 state · effect · 스토어 구독을 더하지 않는다. `groupToolRuns` 는 렌더 중 파생이고, 펼침 상태(`useToolGroupStore`) 읽기와 토글은 새 자식 컴포넌트 **`ToolGroupRow`**(`chat/ToolGroupRow.tsx` — props = `row` · `vendorErrorIds`(`vendorErrorIdsOf` — 한 렌더에 한 번 지어 모든 묶음이 나눠 쓴다) · `runPos` · `isLast`(마지막 묶음인가 — 렌더 중 파생) · `renderMember`(펼친 멤버 한 줄 — 레일 없는 행 컴포넌트를 `StructuredTextView` 가 돌려주는 렌더 콜백 seam) · `slotId?` · `onGroupToggle?` — ★7판 정정 = FE-2b-2 구현 판(옛 `results` 대신)★)가 진다. `StructuredTextView` 는 `slotId`·`onGroupToggle` 을 그대로 내려보낼 뿐이다.
- **요약 줄**: 아이콘(lucide `Layers`) · 종류별 `t('chat.toolGroup<Kind>', {count})` 를 고정 순서(검색 · 읽기 · 목록 · 편집 · 명령 · 웹 · 에이전트 · MCP · 기타)로 ` · ` 로 잇고 · 오류가 있으면 끝에 `t('chat.toolGroupErrors', {count})`(붉은 톤) · 거부가 있으면 그 뒤 `t('chat.toolGroupDeclined', {count})`(주황 톤 `var(--status-blocked)` — 우리 거절 · codex 스스로의 거부를 함께 센다 · 오류 수에 안 든다 · U2-a · §4-7 ⑧). 오류 수 = 항목 `resultMark === 'failed'`(codex — §4-7) 이거나 `buildToolResultMap`(`:117-125`)의 `isError`(claude) 인 호출 — 두 백엔드 다 선다(U2). 한 호출은 오류 · 거부 중 많아야 하나에 든다(`toolCallVerdict` — 거부 표식이 벤더 `isError` 를 이긴다 · §4-7 ⑧). 셰브론 · `aria-expanded`.
- DOM 표지: 묶음 뿌리 `data-tool-group={key}` · `data-tool-group-open="1"|"0"` · `data-tool-group-count={calls.length}`.
- [고름] 단일 도구 행 아이콘은 `category !== 'Other'` 면 종류 아이콘, 아니면 오늘 이름 휴리스틱(`:150-167`) — 옛 데몬에서 오늘 모습 그대로.

### 4-4. 펼침 상태 · 명령

- **새** `src/store/toolGroupStore.ts`(zustand):
  ```ts
  interface ToolGroupState {
    bySlot: Record<string, { agentId: string; open: Record<string, boolean>; mount: number | null }>
    bind(slotId: string, agentId: string): () => void   // 다른 에이전트면 그 슬롯 칸을 비운다 · 돌려받은 함수 = 해제
    clear(slotId: string): void                         // 새 화신 비우기(onReset) 전용
    setOpen(slotId: string, key: string, open: boolean): boolean   // false = 그 슬롯이 지금 묶여 있지 않다 · 아무것도 안 적는다
  }
  ```
  유효 펼침 = `open[key] ?? live` — **사용자 토글이 자동 접힘을 이긴다**(양방향). 재구독·replay 는 지우지 않는다. 웹뷰 새로고침은 인메모리라 초기화된다(레이아웃과 같은 수준 — CLAUDE.md 「LLM-우선 제어」).
  - ★구현 판(FE-2b-1 · 리뷰 FIX · 메인 결정)★: 묶임 = 그 슬롯의 대화 뷰가 이 창에 **지금 마운트돼 있다**. `bind` 가 해제 함수를 돌려주고, 해제는 묶임만 풀고 고른 값은 남긴다(다시 마운트하면 그대로 붙는다). 해제는 자기 묶임에만 먹는다 — 두 번 불러도 · 새 `bind` 뒤의 옛 해제도 새 묶임을 풀지 않는다(묶임마다 표식). `setOpen` 은 `void` 를 넓혀 `boolean` 을 돌려준다 — `false` = 한 번도 안 묶였거나 해제됐다 · 아무것도 적지 않는다. 명령이 이 값으로 「이 창에 그 뷰가 없다」를 답한다 — 렌더 모드 교체(`LayoutLeaf.tsx:270-280`) · 팝아웃 이동(ADR-0057)으로 뷰가 내려간 뒤 성공으로 답하지 않는다(ADR-0167 · 형제 `slot.scrollToBottom` 의 손잡이 명부와 같은 결).
- RichSlot 접착(FE-1.3): `StructuredTextView` 에 `slotId={viewId}` · 마운트 효과에서 `bind(viewId, agentId)` 하고 그 정리(cleanup)에서 돌려받은 해제 함수를 부른다 · onReset(`:249-266`)에서 `clear(viewId)`. `StructuredTextView` 의 새 prop 은 선택이다 — 없으면 자동 규칙만 쓴다(FE-2 가 FE-1 을 기다리지 않는다).
- **명령** `chat.toolGroup.setExpanded { slotId, groupKey, expanded: boolean }`(새 `src/commands/chatCommands.ts`) — ★`help` 없음★(창마다 따로 있는 프론트 상태 — `renderModeCommands.ts:8-20` · ADR-0167). 인자 검문은 `requireSlotId` 모양으로 throw. `groupKey` 는 DOM `data-tool-group` 에서 읽는다.

### 4-5. F1 과의 맞물림

- 접기·펼치기·자동 접힘으로 높이가 바뀌면 F1 의 내용 RO 가 받는다 — 붙어 있으면 바닥으로 간다.
- ⑤(메인 수락 · t3code 선례): 사람이 **마지막이 아닌** 묶음을 펼치면 `follow.unpin()` 을 부른다. 붙은 채 위쪽 묶음을 펼치면 바닥으로 다시 내리면서 누른 머리가 화면 위로 밀려난다(Claude Code 가 2.1.83 에서 고친 「스크롤이 튄다」와 같은 부류 — 조사 §3-2). 마지막 묶음 토글은 붙음을 그대로 둔다. 빼도 다른 곳은 안 바뀐다(`StructuredTextView` 의 `onGroupToggle` prop 하나). 푼 뒤는 §2-2 규칙 4 의 걸쇠가 지킨다 — 아래로 스크롤해 문턱에 들거나 명시 `pin` 이 올 때까지 떨어져 있다.

### 4-6. 시험

- `toolRuns.test.ts`: 도구 1 개 = 묶음 없음 · 2 개 = 묶음 · `tool_result` 운반·usage·빈 생각 끼어도 이어짐 · 비지 않은 생각 흡수(U5) · 글/말풍선/구분선/결말/오류/모르는 사건이 끊음 · 마지막 도구 뒤 생각은 멤버 아님 · `live` 참(턴 중 · 뒤가 생각뿐) / 거짓(끊는 행이 옴 · 턴 끝) · 키 = 첫 id · id 없으면 `itemId` · 같은 사건열 두 번 = 같은 결과(replay).
- `summarizeGroup`: 고정 순서 · 0 인 종류 생략 · 오류 수(codex `resultMark` · claude 파싱 둘 다) · 거부 수.
- U2(codex 끝 · 실패) 시험 = §4-7 ⑨.
- `StructuredTextView.test.tsx`: 레일 위치가 행 목록과 DOM 에서 일치(ADR-0051 — 기존 레일 시험 모양) · 사용자 토글이 자동 접힘을 이김 · `data-tool-group*` 값.
- `structuredAccumulator.test.ts`: `category` 있음·없음·모르는 낱말 → 정규화.
- `toolGroupStore` · `chatCommands.test.ts`(`help` 없음).
- Rust: claude 이름 표(각 낱말 + `mcp__x__y` + 모르는 이름) · codex `commandActions` 조합 표(fixture `tool_end_m7.jsonl` 의 `unknown` → `Command` 포함) · 상한을 넘는 item 도 종류는 온전한 item 에서 · 데몬 변환 일대일 · 직렬화 왕복(`category` 없는 옛 JSON 도 읽힌다).

### 4-7. codex 도구 끝 · 실패 (U2 — 사용자 결정 「이번에 한다」)

**무엇을**: codex 가 도구 item 의 `item/completed` 에 싣는 끝 상태를 중립 사건 하나로 옮겨, 묶음 머리 「오류 N」과 펼친 행의 배지가 codex 에서도 선다. 결과 **본문**은 옮기지 않는다 — 표시는 개수와 배지뿐이다(관례 = codex TUI 의 「· N failed」 · t3code).

**사실**(확실 — 직접 확인 · 5판의 태그 `@v0.156.1` 판독은 가능성 높음 — 각 항목에 적는다):

- codex 는 도구 item 마다 `item/started`(`status: inProgress`) 뒤 **같은 item id** 로 `item/completed` 를 끝 상태와 함께 낸다. 끝 어휘 = `commandExecution` `completed | failed | declined`(+ `exitCode`) · `fileChange` 같은 넷(`PatchApplyStatus`) · `mcpToolCall`·`dynamicToolCall` `completed | failed` · `collabAgentToolCall` `completed | failed | interrupted` · `webSearch` 는 status 칸이 없다(t3code 생성본 `schema.gen.ts:1928-1938` · `:2004-2009` · `:2278-2283` · `:2356-2366` · item 모양 `:21559-21632`).
- 번역기는 도구 item 의 `ItemOrigin::Completed` 에서 **일부러 아무것도 안 낸다**(`backend/codex/decoder.rs:262-263` · `:623` — 한 호출을 두 번 그리지 않으려고). 그래서 끝 상태가 프론트에 닿는 길이 없다. 이력 문(`ItemOrigin::History`)은 끝난 item 을 받지만 `ToolCall` 만 낸다(같은 `:623`).
- ★**끝이 턴 끝 뒤에 올 수 있다(실측)**★: `codex/fixtures/steer_m6.jsonl` 27 줄에 시작한 명령이, 32 줄에서 그 턴이 끊긴 뒤 **다음 턴의 `turn/completed`(49 줄) 뒤** 50 줄에서 옛 turn id · `status: completed` · `exitCode: 0` 으로 닫힌다(`fixtures/README.md:19` — 끊기 17 초 뒤).
- ★**우리가 거절한 명령은 `declined` 가 아니라 `failed` 로 닫힌다**★(업스트림 소스 판독 · 조사 §8-1 메인 대조): 우리 통로는 서버 요청을 전부 `METHOD_NOT_FOUND` 로 거절한다(`backend/codex/transport.rs:3054-3087` · 들어오는 요청마다 `:3629`). app-server 는 그 클라이언트 오류를 `ReviewDecision::denied("approval request failed")` + 끝 상태 `Failed` 로 바꾸고(`beh.rs:2026-2032`), 명령 item 을 `status:"failed"` · `exitCode:null` · `aggregatedOutput:null` 로 닫는다(`beh.rs:1446-1493` — 조사 §8-1 은 item 모양 `:1468-1483`). ★item 에 사유 칸이 없다★. 명령의 `declined` 는 명시적 Decline/Cancel(우리는 안 보낸다) · guardian 심사 · 네트워크 정책 거부에서만 온다. 5판 — 설치본과 같은 판의 소스에서도 같다: 그 failed/null/null 끝은 **app-server 가 지어 내는 알림**이다(`beh:1446-1491@v0.156.1` · 우리 -32601 경로 `beh:2025-2031@v0.156.1` — 가능성 높음 · 실행 확인은 B5 채취 셋째).
- ★**우리가 거절한 파일 변경은 `declined` 로 닫힌다**★(5판 · 태그 판독 · 가능성 높음): 4판은 「끝 상태를 모른다」였다. core 가 거절된 패치를 `declined` 로 닫는다(`core/src/tools/handlers/apply_patch.rs:595` · `:608-623` · `core/src/tools/approvals.rs:460` · `core/src/tools/events.rs:333-343` — 전부 `@v0.156.1`). 파일 변경의 시작(Begin)은 승인 **전에** 나므로 시작 행이 선다. 시작 · 승인 요청 · 끝은 같은 id 를 싣는다 — 승인 요청 `item_id = event.call_id`(`beh:638-647@v0.156.1`) · 시작 item id = `ctx.call_id`(`core/src/tools/events.rs:256@v0.156.1` — 둘 다 5판 light 재검 편집에서 다시 열어 확인). fileChange item 의 칸은 `id` · `changes` · `status` 셋뿐이고(`app-server-protocol/src/protocol/v2/item.rs:326-330@v0.156.1`) 사유 칸이 없다. 이력에도 `declined` 로 남는다. ★벤더의 `declined` 는 거절 전용 낱말도 아니다★ — 도구 끝 경로의 벤더 주석이 「승인 거절과 일부 실행 준비 실패(setup failure 등)가 같은 거부 경로를 타, 사용자 아닌 실패 일부가 Declined 로 보고될 수 있다」고 적는다(`core/src/tools/events.rs:445-452@v0.156.1` — 이 편집에서 다시 열어 확인).
- ★**거절된 명령은 이어받은 이력에 없다**★(5판 · 태그 판독 · 가능성 높음 — 실행 확인 아님): 이력 페이지는 저장된 core `ItemCompleted` 에서만 item 을 짓는다(`app-server-protocol/src/protocol/thread_history_projection.rs:77-85@v0.156.1`). 승인 요청 · `ItemStarted` · ExecCommandBegin/End 는 저장되지 않는다(`rollout/src/policy.rs:142-192@v0.156.1`). 거절된 unified-exec 명령은 core 가 Begin 전에 돌아가 item 을 아예 안 낸다(`core/src/unified_exec/process_manager.rs:530-539@v0.156.1` · Begin = `:594`). 라이브의 failed/null/null 끝은 위 app-server 합성 알림뿐이다. → 재활성화 뒤 그 행은 「오류」도 「거부됨」도 아니고 **없다**.
- **끊긴 명령은 계속 돌다 늦게 끝난다**(5판 · 태그 판독 · 가능성 높음): 도는 명령이 있는 턴을 끊으면 턴 끝에 그 명령의 끝이 오지 않는다(`core/src/tools/parallel.rs:237-265@v0.156.1`). unified-exec 프로세스는 살아 있고(`core/src/unified_exec/process_manager.rs:598-625@v0.156.1`), 나중에 끝나면 **실제 `exitCode`** 와 함께 `completed` 또는 `failed` 로 닫는다 — 다음 턴 도중일 수 있다(`core/src/unified_exec/async_watcher.rs:186-247@v0.156.1` · 위 `steer_m6` 실측과 맞다). 우리 거절 모양(`exitCode:null`)과는 겹치지 않는다.
- **순서**(4판이 같은 파일에서 더 확인 — 확실): 보통 명령 승인(`approvalId` 없음 · 명령 표시)은 `item/started` 를 **요청보다 먼저** 낸다(`beh.rs:754-771` → 요청 `:785-806`) — 거절된 명령도 `ToolCall` 행이 선다. 끝은 우리 답을 받은 그 처리기가 **곧바로** 낸다(`:2058-2074` — 벤더에 결정을 넘기는 `:2076-2085` 보다 먼저) · 시작한 item 에만 낸다(`:1458-1466`). 하위 명령 승인(`approvalId` 있음 — zsh 갈래)은 부모 item 의 끝을 누르고(`:2043-2056`) · 네트워크 승인(명령 표시 없음)은 이 처리기에서 시작도 끝도 내지 않는다(`:741-753`). 파일 변경 승인 처리기는 요청 전에 `item/started` 를 내지 않는다(`:638-666`) — 파일 변경의 시작은 core 가 승인 전에 낸다(위 5판 판독).
- ★**지금 화면에는 아무것도 안 뜬다**★(가능성 높음 — 실측 아님 · 조사 §8-1): 거절 경로는 사건을 내지 않고(`refuse` 는 제어 큐가 넘칠 때만 `Error` 를 낸다 `transport.rs:3075-3086`), 끝 알림은 번역기가 버린다(`decoder.rs:262-263` · `:623`). 붉은 경고 박스(`StructuredTextView.tsx:484-490`)는 codex `error` 알림(`decoder.rs:747-764`)이나 그 넘침에서만 선다.

**① 선 모양 — 새 변형 `ToolResult`** [고름]

```rust
// agent types.rs — OutputEvent 에 한 변형
/// 도구 호출 하나의 끝 결과 — 앞선 `ToolCall` 을 `id` 로 가리킨다(새 행이 아니다). 결말은 중립 enum(ADR-0004).
ToolResult { id: String, outcome: ToolOutcome },
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolOutcome {
    Completed,
    Failed,
    /// 에이전트(벤더)가 스스로 실행을 거부했다.
    Declined,
    /// 우리(호스트)가 승인 요청을 거절해 실행되지 않았다 — 번역기는 내지 않고 통로가 바꿔 쓴다(②-2 · 5판).
    Refused,
}

// protocol messages.rs — StructuredEvent 에 같은 모양(단위 변형이라 serde·ts-rs 는 문자열)
ToolResult { id: String, outcome: ToolOutcome },
```

- TS 접점(고정): 생성물 `crates/engram-dashboard-protocol/bindings/ToolOutcome.ts` = `"Completed" | "Failed" | "Declined" | "Refused"` · `StructuredEvent.ts` 에 `{ "type": "ToolResult", id: string, outcome: ToolOutcome }`. `Refused` 는 5판에 더했다(4판 리뷰 FIX — §12) — 아직 안 나간 타입이라 호환 비용이 없다.
- `id` 는 `Option` 이 아니다 — 가리킬 호출이 없으면 사건을 내지 않는다(②).
- 기각한 모양 둘:
  - **`ToolCall` 에 선택 `status` 칸 + 누산기 id 병합** — 끝에서 `ToolCall` 을 한 번 더 내야 한다. 두 턴 분류기가 `ToolCall` 을 진행으로 세므로(`claude/mod.rs:541` · `codex/mod.rs:1206`) 턴 끝 뒤에 온 끝(위 실측)이 「턴 중」을 다시 켜 30 분 막힘 경로가 된다. 옛 셸은 끝마다 **같은 도구 행을 하나 더** 그린다 — 「표시할 수 없는 신호」 줄보다 나쁘다(틀린 내용이다).
  - **`Structured{kind:…}` 탈출구** — 같은 두 분류기가 `Structured` 를 통째로 진행으로 센다(대기 입력 사건을 탈출구에 안 싣는 사유와 같다 — `types.rs:94-97`). json 을 claude `tool_result` 모양으로 지으면 옛 셸도 배지를 그리지만, 그것은 벤더 모양을 codex 번역기까지 넓혀 조사 §3-1 의 누수를 키운다.

**② 상태 매핑 — codex 번역기**: 함수 하나 `tool_outcome(item, kind) -> Option<ToolOutcome>` 를 `Completed`·`History` 두 문이 같이 쓴다(ADR-0203 「두 번째 어휘표를 만들지 않는다」).

| 벤더 `status` | → | 비고 |
|---|---|---|
| `failed` | `Failed` | 다섯 도구 변형 모두 · 우리가 거절한 id 면 통로가 `Refused` 로 바꾼다(②-2) |
| `declined` | `Declined` | `commandExecution` · `fileChange` 만 스키마에 있다 — 다른 변형에서 오면 드리프트 계수 · 우리가 거절한 id 면 통로가 `Refused` 로 바꾼다(②-2) |
| `completed` | `Completed` | ★내지 않는다★(③) |
| `inProgress` | — | 이력 페이지의 아직 도는 item |
| `interrupted`(`collabAgentToolCall`) | — | 턴 결말 행이 이미 끊김을 보인다 |
| 칸 없음(`webSearch`) | — | 일상 |
| 그 밖 문자열 · 문자열 아님 | — | `observe(tool-status:…, Drift)` |

- ★**판정은 벤더 `status` 하나다 — `exitCode` 를 읽지 않는다**★ [고름]: 0 아닌 종료를 우리가 실패로 다시 가르면 벤더와 다른 판정이 선다(t3code 도 `status` 만 본다 — `apps/server/src/provider/Layers/CodexAdapter.ts:1013-1019`). `mcpToolCall.error` · `dynamicToolCall.success` 도 같은 이유로 안 읽는다. codex 가 0 아닌 종료를 늘 `failed` 로 적는지는 **미검**이다(가능성 높음 — t3code 시험이 `exitCode: 1` 을 `failed` 와 짝짓는다 `CodexAdapter.test.ts:1545-1552`) → B5 채취가 확인한다(⑨).
- **발행**: `Started` 문 = 오늘 그대로(`ToolCall`) · `Completed` 문 = `tool_outcome` 이 `Failed`/`Declined` 면 `ToolResult` 하나(번역기는 `Refused` 를 내지 않는다 — ②-2), 그 밖엔 오늘처럼 사건 0 + 일상 계수 · `History` 문 = `[ToolCall, ToolResult?]` 이 순서. `id` 는 `ToolCall` 과 같은 `bounded_id`(`decoder.rs:1141`) — 걸러져 없으면 가리킬 행이 없으므로 내지 않고 `Malformed` 계수.

**②-2 우리가 거절한 호출의 귀속**(U2-a · 4판 · 5판 결말 분리) [고름 — 자리 = 리더 전용 칸 · §11 ⑧ 수락]: codex 는 우리가 거절한 명령을 `failed` 로, 파일 변경을 `declined` 로 닫으므로(위 사실) 번역기 혼자서는 「우리 거절」을 못 가른다 — 명령은 진짜 실패와, 파일 변경은 codex 스스로의 거부와 모양이 같다. 통로가 거절한 item id 를 기억했다가, 그 item 의 라이브 `item/completed` 에서 번역기가 낸 `ToolResult{Failed}` 또는 `ToolResult{Declined}` 를 `Refused` 로 바꿔 낸다(메인 결정 5판 — 벤더 status 가 어느 쪽이든). 기억에 없는 id 의 `status:"declined"` → `Declined`(② 표 — codex 스스로의 거부)는 그대로다.

- **결말 둘 = 사유 둘**(5판 · 4판 리뷰 FIX — §12 4판 1): `Refused` = 우리 거절 · 사유 「대시보드가 승인 요청을 처리하지 않아 실행되지 않음」 · `Declined` = 기억에 없는 거부(codex 스스로의 거부 — guardian 심사 · 네트워크 정책 등 · 벤더 준비 실패 · 아래 한계의 복원 · 상한 밀림) · 사유 「실행되지 않음」(★사용자 결정 2026-09-27 · 5판 light 재검★ — §1 U2-a). 배지 · 색 · 「거부 N」 셈은 같고 둘 다 오류 수에 안 든다(⑧). 4판은 `Declined` 하나에 우리 사유를 붙여, codex 스스로의 거부에 거짓 사유가 붙었다. 5판 첫 문구 「codex 가 실행을 거부함」도 복원 · 상한 밀림의 우리 거절과 벤더 준비 실패에서 거짓이었다 — 「실행되지 않음」은 그 경로 전부에서 참이고(가능성 높음) 이유를 말하지 않을 뿐이다.
- **같은 모양을 내는 다른 길**(5판 · 태그 판독 · 가능성 높음): failed/null/null 끝은 우리 거절만의 것이 아니다 — 스레드 unload · 되돌림으로 승인 콜백이 버려질 때(`beh:2032-2038@v0.156.1`) · guardian 시한 초과(`beh:376-396@v0.156.1` — 자동 심사를 켤 때만 · 우리 스폰은 안 켠다)도 같다. 전부 「실행되지 않음」이지만 기억에 없으니 `Failed`(「오류」)로 보인다.
- **기억하는 요청 = 둘뿐**: `item/commandExecution/requestApproval` · `item/fileChange/requestApproval`(t3code 생성본 `schema.gen.ts:36713` · `:36718` · params `:20998-21012` · `:3033-3040` — 둘 다 필수 `itemId` · `threadId` · `turnId`). 조건 셋:
  - `params.itemId` 가 문자열이고 `MAX_ID_BYTES`(`decoder.rs:93` — 128) 안이다(밖이면 번역기가 가리킬 `ToolResult` 를 안 낸다).
  - 명령 승인은 `approvalId` · `networkApprovalContext` 가 없거나 `null` 일 때만. 하위 명령 승인은 부모 item 의 실행 거절이 아니고(부모의 끝은 스크립트 전체의 결과다) · 네트워크 승인은 그 처리기가 끝을 안 낸다(위 사실 `beh.rs:2043-2056` · `:741-753`).
  - 거절 줄이 실제로 제어 큐에 들어갔다 — `refuse` 가 `closed` 로 일찍 돌아가거나 넘침(`dropped`)이면 적지 않는다(상대가 답을 못 받아 끝이 안 온다).
  - 그 밖의 요청(`item/tool/call` · `item/tool/requestUserInput` · `item/permissions/requestApproval`(그 item 이 우리가 그리는 도구 행인지 모른다) · `mcpServer/elicitation/request` · 옛 `execCommandApproval`·`applyPatchApproval`(칸이 `callId`)) · `itemId` 없음 → 귀속 없음(오늘처럼 거절만).
- **자리 = 리더(`Reader` — `transport.rs:3041-3049`) 전용 칸 하나** `refused: RefusedItems` — `first_turn`(`:3048`)과 같은 결이다. 상태 락(`State` `:385`) 밖에 둔다(메인 수락 — §11 ⑧):
  - 쓰는 쪽(`:3629` → `refuse`)과 읽는 쪽(같은 `handle_line` 의 `item/completed` 알림)이 **둘 다 리더 스레드**다. 요청은 번역기에 가지 않는다(`protocol.rs:86` — 번역기가 받는 것은 알림뿐). 한 스레드가 줄 순서대로 쓰고 읽고, 업스트림도 우리 답을 받은 뒤 끝을 낸다 — 기억이 늘 끝보다 먼저 선다.
  - `State` 는 라이터 · `send_input` · 리더가 나눠 쥐는 임계 구역이다(`:382-384` doc). 한 스레드만 만지는 칸을 거기 두면 락 구간만 늘고 「소유권 분할」 표에 칸이 는다. **새 락 없음 · `Announcer.order → {상태 락, replay → …}` 에 새 간선 없음**(바꿔 쓰기는 emit 전 · 락 없이).
  - `refuse` 는 `&self` → `&mut self` 에 인자 `params: Option<&Value>`(`:3629` 의 `..` 를 풀어 넘긴다 — `handle_line` 은 이미 `&mut self`). 메서드 이름 상수 둘은 `protocol.rs` `method`(`:210-243`)에 더한다. 시험의 `Reader { … }` 짓기 셋(`:4372` · `:4420` · `:4511`)에 칸 하나.
- **모양 · 상한** [고름]: `VecDeque<String>` · 상한 `REFUSED_ITEM_SLOTS = 16`(`EARLY_COMPLETION_SLOTS`(`:301`)와 같은 결의 상수) · 넘치면 가장 오래된 것을 버린다(debug 로그). ★이미 기억에 있는 `itemId` 는 다시 넣지 않는다★(5판 · 4판 리뷰 low) — 해가 없는 방어 규칙이다: 업스트림은 승인마다 요청 → 끝을 번갈아 내므로(명령 — 끝 `complete_command_execution_item` 이 벤더 `submit` 보다 먼저 · `beh:2055-2080@v0.156.1`) 한 id 에 거절이 둘 걸려 있는 일은 없다(가능성 높음 · 판독). 벤더가 바뀌어도 칸이 새지 않게 둔다. 보통은 0–1 개다 — 끝이 거절 직후에 온다(위 사실). 남는 것은 끝이 안 오는 item 뿐이다(도중에 죽은 프로세스 · 승인 대기 중 끊긴 턴 — 요청이 사유 `turnTransition` 으로 취소되고 시작 행이 열린 채 남는다 `beh:2024@v0.156.1` · `beh:754-770@v0.156.1` — 우리 거절은 즉시라 거의 안 든다). **알려진 한계**(§12 4판 3 = 사용자 수락 · 바른 해법 = T-38): 밀려난 명령의 늦은 끝은 「오류」로, 밀려난 파일 변경의 끝은 「거부됨 · 실행되지 않음」으로 보인다(우리 사유를 잃지만 거짓은 아니다 — 아래 한계 ②④).
- **바꿔 쓰기 · 비우기** — `handle_line` 알림 갈래에서 번역(`dec.decode`) 뒤 · 경계 뽑기 전 한 줄 `self.refused.attribute(&method, params.as_ref(), &mut events)`:
  - `method == item/completed` 이고 `params.item.id` 가 집합에 있으면 **꺼낸다** — 끝 상태와 무관하다(끝 줄이 곧 비우는 자리다). 꺼냈으면 `events` 안의 `ToolResult { id: 그 id, outcome: Failed | Declined }` 를 `Refused` 로 바꾼다(명령 = `failed` · 파일 변경 = `declined` — 위 사실). 번역기가 아무것도 안 냈으면(`completed` 등) 기억만 지운다.
  - ★턴 끝에서는 비우지 않는다★ [고름]: 끝이 턴 끝 뒤에도 온다(위 실측 `steer_m6` 50 줄). 비우면 그 늦은 끝이 붉은 「오류」가 된다. item id 는 호출마다 다르므로(가능성 높음 — 명령 item id = 벤더 호출 id `call_id` · `beh.rs:760` · `:789`) 남은 항목이 다른 호출에 붙지 않고, 남는 수는 상한이 막는다.
  - 새 화신 = 새 리더 = 빈 집합 — 리더는 통로를 열 때 한 번 짓는다(`:3965-3975`).
- **번역기는 이 집합을 모른다** [고름]: 번역기는 알림 줄만 받는 순수 번역기로 남는다(`Box<dyn OutputDecoder>` 의 `decode(bytes)` 하나 — 번역기 시험은 `failed` → `Failed` · `declined` → `Declined` 그대로 · 번역기는 `Refused` 를 내지 않는다). 기각 = claude `TurnGate` 처럼 공유 `Arc<Mutex<…>>` 를 번역기에 주입(한 스레드인데 락이 하나 늘고 번역기 시험이 통로 상태에 묶인다).
- **ADR-0004**: 승인 메서드 이름 · `itemId`/`approvalId`/`networkApprovalContext` 읽기는 전부 `backend/codex` 안이다. 통로 밖(코어 · 데몬 · 프론트)은 여전히 중립 `ToolOutcome::Refused` · `Declined` 만 본다. 턴 관측은 바뀌지 않는다 — 바꿔 쓰기는 사건 종류를 안 바꾸고 `ToolResult` 는 결말과 무관하게 `None` 이다(⑥).
- **스위치는 없다 — 귀속은 늘 짓는다**(5판): 4판은 「설치본이 우리 거절에 이미 `declined` 를 보내면 귀속을 짓지 않는다」였다. 결말이 둘로 갈려(위) 벤더가 우리 거절을 `declined` 로 닫아도 옳은 사유를 고르려면 귀속이 든다 — 파일 변경은 이미 그렇다(위 사실). 바꿔 쓰기는 조건 없이 `Failed | Declined → Refused` 라 채취 셋째(⑨)가 정하는 코드는 없다 — 채취 셋째는 fixture 채취와 멈춤(설치본이 판독과 다르면 메인에 반환)뿐이다(5판 light 재검).
- **알려진 한계 넷 — 이어받은 이력 · 상한**(§11 ⑨ · §12 4판 2 · 3 = 사용자 수락 · 바른 해법 = `docs/tracking.md` T-38 — 대시보드가 자동 거절을 그만두면 이 귀속이 필요 없어질 가능성이 높다): 집합은 화신마다 비고 벤더 item 에 사유 칸이 없다. 같은 화신 안의 재구독 replay 는 링의 `Refused` 를 그대로 받아 맞다. 재활성화 뒤는(5판 정정 · 태그 판독 · 가능성 높음) ①③, 같은 화신 안의 상한 밀림은 ②④:
  - ① **이력의 명령** — 이력에 우리가 거절한 명령 행이 **없다**(위 사실 — 「오류」도 「거부됨」도 아니다). 라이브 때 「거부됨」이던 행이 사라지고 묶음 개수와 「거부 N」이 하나씩 준다. 4판의 「`failed` 그대로 「오류」로 보인다」는 틀렸다.
  - ② **상한에서 밀려난 명령** — 늦은 `failed` 끝이 기억에 없어 `Failed`(「오류」)다(위 모양 · 상한).
  - ③ **이력의 파일 변경** — 이력에 `declined` 로 남는다 → 기억이 비어 `Declined` 가 된다 — 배지 · 셈은 같고 사유만 「실행되지 않음」으로 바뀐다(「거부됨 · 실행되지 않음」). 우리 사유를 잃지만 거짓은 아니다(5판 light 재검 — 5판 첫 문구 「codex 가 실행을 거부함」은 여기서 거짓이었다).
  - ④ **상한에서 밀려난 파일 변경** — `declined` 끝이 기억에 없어 ③ 과 같은 「거부됨 · 실행되지 않음」이다(5판 light 재검에 더했다 — 목록에서 빠져 있었다).
  - item 모양(`failed` + `exitCode:null` + `aggregatedOutput:null`)으로 가르는 안은 기각(메인 · 5판 — §10 0241): 같은 모양을 내는 다른 길(위 — 승인 콜백 버려짐 · guardian 시한 초과)을 우리 거절로 읽어 거짓 사유를 붙인다. 거절된 명령은 이력에 없어 ① 도 못 고친다 — 얻는 것은 ② 하나뿐이다.

**③ 실패 · 거부만 낸다** [고름]: `Completed` 는 선 어휘로만 둔다(④ 의 후속이 쓸 자리). 끝마다 내면 codex 도구 호출마다 사건이 하나씩 늘고 옛 셸의 「표시할 수 없는 신호」 줄이 **모든** 호출에 붙는데, 화면이 얻는 것은 없다 — 표시는 오류·거부 수와 배지뿐이다. ★`ToolResult` 가 없다 = 성공으로 읽지 말 것★ — 없음은 「모름」이다(옛 데몬 · 링에서 밀려남 · 끊겨 끝이 안 옴).

**④ claude 는 이번에 바꾸지 않는다** [고름] — claude 번역기가 `tool_result` 블록에서 같은 `ToolResult` 를 함께 내는 안은 짓지 않는다:

- 결과 **본문**은 여전히 벤더 블록(`Structured{kind:"user"}`)에서 읽는다 — 펼친 행의 Out(`StructuredTextView.tsx:262`) · `rowKindOf` 의 skip(`:390`). 상태만 싣는 중립 사건은 그 파싱을 옛 데몬 폴백으로 내리지 못하고, **같은 오류 비트가 두 곳에 산다**(한쪽이 낡는다).
- claude 는 **모든** 호출에 결과를 낸다 — 기본 백엔드라 링 사건과 옛 셸의 표시 불가 줄이 codex 실패 수가 아니라 claude 호출 수만큼 는다.
- 누수를 **키우지는 않는다** — codex 는 벤더 모양을 안 탄다.
- ★**끊긴 claude 도구 = ⓐ 와 같은 규칙**★(메인 결정 · 사용자 위임 2026-09-27 · §11 ⑪): Esc 로 끊긴 도구의 `tool_result` 가 `is_error:true` 로 오면(대화형 CLI 실측 · stream-json 미검 — §3-5 S1 기록) 그 행은 오늘 붉은 `Error` 배지(⑧ `isErr = result?.isError === true`)를 그대로 달고, 묶음이 서면 머리가 「오류 1」 을 센다 · 바로 아래 강조된 「중단됨」 행(U8)이 맥락을 준다(★6판★ claude 는 그 자리를 끊김 표시 행이 채운다 — 중단 행은 대비로만 · ADR-0243). 새 장치 없음 — 끊긴 턴 안의 `is_error` 를 다시 가르지 않는다(codex 채취 둘째 ⓐ 와 한 규칙 — ⑨).
- 후속(추적 항목 후보 — §11 ⑦): 본문을 싣는 중립 결과(`ToolResult` 에 본문 칸 · `Completed` 도 냄) + 프론트 파싱을 옛 데몬 폴백으로 내리기. claude 이어받기 원문 경로와 한 사용자 줄의 여러 블록 처리(`structuredAccumulator.ts:175-190`)를 함께 옮겨야 해 이번 범위가 아니다.

**⑤ 판차 — 두 방향**

- **거를 값이 없다(확인)**: 셸이 데몬에 알리는 것은 인증 첫 프레임의 `protocol_version` 하나이고(`crates/engram-dashboard-net/src/auth.rs:33-38`), 데몬은 **같지 않으면 끊는다**(`net/src/ws.rs:348-363`). `Hello` 의 `capabilities` 는 데몬→셸 방향이고 지금 `None` 이다(`daemon/src/connection_core.rs:2214-2220`). 그래서 한 판 안에서 옛 셸과 새 셸을 가를 값이 없고, 연결별 변환(`connection_core.rs:765` `output_event_to_wire`)에서 걸러 낼 근거가 없다. 판을 올리면(6 → 7 · `protocol/src/lib.rs:121`) 옛 셸은 아예 못 붙어 거를 필요가 없어지지만, 개발 일상 조합(옛 데몬 + 새 셸 — discovery 가 재사용을 거부한다 `discovery/src/lib.rs:446`)을 표시 한 줄 때문에 끊는다.
- **결정: 판을 올리지 않고 거르지 않는다** — `TurnEnd`(`protocol/src/messages.rs:743-763`) · `QueuedInput`(`:782-785`)와 같은 판단(데몬→셸 한 방향 · 옛 쪽엔 오독할 것이 없다). 새 변형 doc 에 그 한 줄을 단다.
- 새 데몬 + 옛 셸: codex 실패·거부 호출마다 누산기 기본 갈래가 「표시할 수 없는 신호」 줄을 남긴다(연속이면 한 줄의 수만 는다 — `structuredAccumulator.ts:249-254`). 그 프레임은 `false` 를 돌려 대기 표시를 안 건드린다.
- 옛 데몬 + 새 셸: 사건이 안 온다 → codex 오류 표시가 없을 뿐이다.

**⑥ 턴 관측 · 오류 뒤 멈춤**: 두 분류기(`claude/mod.rs:538` · `codex/mod.rs:1203`)에 `OutputEvent::ToolResult { .. } => None`.

- ★`Progress` 금지★ — 위 실측대로 끝은 턴 끝 뒤에도 오고, 진행으로 세면 「턴 중」이 다시 켜져 30 분 fail-open 까지 우편이 막힌다(CLAUDE.md 「턴 관측 정리」 · 「대기 입력 상태」 끝 문단). `None` 이라 도착 순서에 기대지 않는다.
- ★`Failed` 금지★ — 도구 실패는 에이전트의 평범한 한 걸음이다. 세면 실패한 검색 한 번이 오류 뒤 멈춤(`last_end_failed`)을 세워 다음 깨끗한 턴까지 우편이 멈춘다(ADR-0231 N11).
- 통로의 도구 끝 계기(`is_tool_item` — `decoder.rs:282-288`)는 원 줄의 item 타입을 읽으므로 이 사건과 무관하다 — `tool_end_m7` 시험(`codex/transport.rs:9653`)이 무수정 초록이어야 한다.
- 프론트 누산기의 `ToolResult` 갈래는 `turnDone` 을 안 건드리고 대기 표시도 풀지 않는다(`feed` 반환 `false` — ⑧).
- 통로가 `Refused` 로 바꿔 쓴 사건(②-2)도 같은 `None` 이다 — 바꿔 쓰기는 결말 칸만 바꾸고, 사건이 보통 문 · 관측 없는 문 중 어디로 나가는지(`transport.rs:3659-3670` 의 `ours` 판정)도 그대로다.

**⑦ 링 · 생성물**

- `estimate_cost_bytes`(`output_core.rs:1217`)에 `ToolResult { id, .. } => id.len()`(결말은 고정 크기). 실패·거부 호출당 사건 1 · 수십 바이트 — `REPLAY_MAX_EVENTS = 4096`(`:1291`)에 견주면 무시할 크기다.
- 망라 match 전부(컴파일러가 가리킨다 · B5 한 커밋): 두 분류기 · `output_core.rs:993-1003`(변형 이름) · `:1217` · `daemon/src/connection_core.rs:765`(새 `tool_outcome_to_wire` 일대일 — `_` 갈래 없이) · 시험 `claude/mod.rs:3012-3030` · `agent/tests/stdio_smoke.rs:77-86` · `daemon/src/bin/saturation_pilot.rs:986-995`.
- 생성물 `ToolOutcome.ts`(새) · `StructuredEvent.ts` — `cargo test -p engram-dashboard-protocol` 이 굽는다(CI sync). ★새 변형이라 프론트 `never` 망라(`structuredAccumulator.ts:243`)가 걸린다★ — 그래서 FE-2 가 가드를 먼저 커밋하고 B5 는 그 뒤에 **착수**한다(§7) — `crates/engram-dashboard-protocol/tests/ts_export.rs:9-20` 이 protocol 시험을 돌릴 때마다 `bindings/StructuredEvent.ts` 를 다시 쓰므로, 새 변형은 B5 의 커밋이 아니라 첫 시험 실행에서 공유 트리에 내려앉는다.

**⑧ 프론트**

- 누산기 `tool` 항목에 칸 `resultMark: ToolResultMark | null` — `ToolResultMark = 'failed' | 'declined' | 'refused'`(`TurnOutcomeMark` 와 같은 결: 정상 완료는 표식이 없다). [고름 · 5판] 표식이 출처를 안다 — 사유 줄을 가르려면 든다.
- `ToolResult` 갈래: 뒤에서부터 같은 `id` 의 `tool` 항목을 찾아 copy-on-write 로 `resultMark` 를 바꾼다(`Failed` → `'failed'` · `Declined` → `'declined'` · `Refused` → `'refused'` · `Completed` → `null` · 모르는 낱말 → `null` + warn). 못 찾으면(링에서 밀려남) 버린다. **새 항목을 만들지 않는다** → `rowKindOf` · 레일 · `isRenderedItem` 무변경. `turnDone` 을 안 건드리고 ★**`false` 를 돌려준다** — 붙였든 못 찾았든★. 같은 사건열 = 같은 결과(멱등 — `ToolCall` 이 늘 먼저 온다: 라이브는 `started` → `completed`, 이력은 한 item 안에서 그 순서).
- ★**결말 붙이기는 「새 내용이 왔다」가 아니다**★(3판 리뷰 Designer FIX · 사용자 동의 2026-09-27): `feed` 의 반환값은 `RichSlot.tsx:222`(`if (understood) setAwaiting(false)`)에서 대기 표시를 푸는 신호로만 쓰인다(소비자는 `:213` 한 곳). codex 는 끊어도 도는 명령을 죽이지 않아 지난 턴 도구의 끝이 늦게 온다(실측 `steer_m6.jsonl` — 17 초 뒤). 그 늦은 끝이 사용자가 새 글을 보낸 직후 · 새 턴이 답하기 전에 들면, `true` 를 돌려줄 경우 새 턴의 Wait 표시가 답 없이 꺼진다. 그래서 `ToolResult` 갈래는 `false` 를 돌려준다 — 행의 다시 그리기는 그대로다(`RichSlot` 이 반환값과 무관하게 `setItems([...acc.snapshot()])` 를 먼저 부른다 `:215`). `RichSlot` 의 코드는 고치지 않는다(FE-1 파일 · 반환값의 뜻이 「대기를 풀어도 되는 프레임」으로 넓어질 뿐이다 — `feed` 의 doc 을 그렇게 고친다). 단 그 호출부 주석 `RichSlot.tsx:218-221`(「알아들은 프레임에만 … 못 알아들었으면 아직 아무것도 못 들은 것이다」)은 `false` 를 「못 알아들음」으로만 읽어 `ToolResult` 의 `false`(알아들었지만 대기를 풀지 않는 프레임)와 어긋난다 → FE-1.3 이 새 뜻에 맞춰 고친다(§12 4판 4 · §7). Wait 자체의 규칙은 바꾸지 않는다(임시 구조 · 재설계는 `docs/tracking.md` T-12). I1 이 `case 'ToolResult':` 로 바꿀 때도 반환값은 그대로 `false`.
- 요약: `summarizeGroup`(§4-3)의 `errors` = `resultMark === 'failed'` 이거나 `vendorErrorIds` 에 든 호출 · `declined` = `resultMark` 가 `'declined'` 또는 `'refused'`.
- ★판정은 한 함수 — 구현 판(FE-2b-1)★: 묶음 요약 셈과 펼친 행 배지는 둘 다 `toolCallVerdict(mark, vendorError, declinedMark)`(`chat/toolRuns.ts`)에서 온다 — `'failed'` = 오류 · `'declined'`/`'refused'` = 거부(`DECLINED_MARK='error'` 면 오류) · 표식 없음이면 벤더 `isError` 가 오류. ★거부 표식이 벤더 `isError` 를 이긴다★ — 표식은 중립 끝 결과이고 벤더 본문 파싱은 그 결과를 아직 안 싣는 백엔드의 대체다(오늘 한 호출에 둘이 같이 오는 경로는 없다 — claude 는 `ToolResult` 를 안 낸다 · T-37 뒤에 겹칠 수 있다). 그래서 아래 `isErr` 식은 이 판정 `=== 'error'` 로 읽는다.
- 펼친 행 `ToolItemRow`(`StructuredTextView.tsx:209`)에 prop `mark` — `isErr = result?.isError === true || mark === 'failed'`(오늘 배지 · 붉은 테 그대로 · codex 는 Out 칸이 없다) · `mark` 가 `'declined'` 또는 `'refused'` = 아래 「거부됨」 모양.
- **「거부됨」 모양 = 사용자 결정 U2-a(따로 표기 · B) · 5판 = 출처별 사유** — 도구가 돌다 실패한 것이 아니라 실행되지 않은 것이다. 우리 거절(`refused`)은 전부 우리 정책이고(승인 UI 없음 — `backend/codex/transport.rs:3054-3087` · 채팅 승인은 다음 과제 T-38 — U7), 기억에 없는 거부(`declined`)는 벤더가 낸 것이다 — 벤더 판단 · 준비 실패 · 복원 · 상한 밀림 중 무엇인지 모르므로 사유가 이유를 말하지 않는다(②-2). ★두 표식은 배지 · 색 · 셈이 같고 사유 줄만 다르다★. 선례 = t3code 「Declined」(`apps/web/src/components/chat/MessagesTimeline.logic.ts:86-96`) · cline 경고 톤(조사 §8-2).
  - **배지**(두 표식 공용): 붉은 `Error` 배지 자리(`StructuredTextView.tsx:250-254`)에 `t('chat.toolDeclined')`(「거부됨」). 글자 · 테두리 색 = `var(--status-blocked)` — 주황 경고 토큰(`src/styles/theme.css:1-3` — dark `#d29922` · light `#9a6700` · e-ink 는 본문색 · ADR-0173). 줄 상자 테두리도 같은 토큰(반투명). 머리(`RowHeader`)는 기본 톤 그대로다 — 붉게 칠하지 않는다. [고름] 새 토큰을 짓지 않고 기존 경고 토큰을 쓴다(e-ink 무력화가 이미 들어 있다).
  - **사유 줄**(출처별 — 5판): 같은 줄 상자 안, 머리 버튼 바로 아래 한 줄(`text-xs` · `text-muted`) — `refused` = `t('chat.toolRefusedReason')`(「대시보드가 승인 요청을 처리하지 않아 실행되지 않음」) · `declined` = `t('chat.toolDeclinedReason')`(「실행되지 않음」 — 사용자 결정 2026-09-27 · 5판 light 재검). **접힌 채로도 보인다**. DOM 표지 `data-tool-declined-reason` = 그 사유 키(`"chat.toolRefusedReason"` | `"chat.toolDeclinedReason"`). ★출처는 이 키로 가른다 — 글로 가르지 말 것★: 「실행되지 않음」은 우리 사유의 꼬리와 같아 글 포함 검사로는 두 출처가 안 갈린다. [고름] 문구는 출처마다 고정이다 — 벤더 item 에 사유 칸이 없다(위 사실 · 한계 = §9).
  - **요약**: `chat.toolGroupDeclined`(「거부 {count}」) — `refused` · `declined` 를 함께 센다 · 「오류 N」 뒤 · 같은 주황 토큰 · **오류 수에 안 든다**(`summarizeGroup` 의 `errors` 와 `declined` 가 따로 센다 — 위).
  - ★ADR-0051 — 사유 줄은 `ToolItemRow` 의 DOM 안이다★: 새 항목도 새 행도 아니다 → `rowKindOf` · `railPositions` · `isRenderedItem` 무변경 · 레일 점은 줄 하나에 하나 그대로. 묶음을 펼친 멤버도 같은 `ToolItemRow` 라 사유 줄이 그 안에 선다(`ToolGroupRow` 안에 `ChatRow` 레일을 두지 않는 규칙 — §4-3 — 그대로).
  - DOM 표지: 줄 뿌리 `data-tool-mark="failed" | "declined"`(`refused` 도 `"declined"` — 출처는 `data-tool-declined-reason` 이 가른다 · 표식 없으면 속성 없음) — §8-2 가 읽는다.
- 상수 `DECLINED_MARK: 'own' | 'error' = 'own'`(`chat/toolRuns.ts` 머리 · **U2-a 확정 = `'own'`** — 위 모양). `'error'` 는 대안 A 의 스위치로 남긴다: 거부(`refused` · `declined` 둘 다)를 오류로 함께 세고 붉은 `Error` 배지 · 사유 줄 없음.

**⑨ 시험**

- 번역기(인라인 줄 — `decoder.rs:1594-1622` 모양): 실패 명령 `item/completed`(`failed` · `exitCode: 1`) → `[ToolResult{Failed}]` · 거부 패치(`fileChange` `declined`) → `Declined` · 실패 MCP(`mcpToolCall` `failed` + `error`) → `Failed` · `completed`(`exitCode` 가 0 이 아니어도) → 사건 0 · 기존 `item_completed_emits_nothing_but_still_inspects_the_item_type`(`:2283`) 무수정 초록 · `inProgress`·`interrupted`·`webSearch` → 0 · 모르는 status → 0 + 드리프트 계수 · id 없음/상한 초과 → 0 + `Malformed` · 이력 문 = `[ToolCall, ToolResult]` 순서.
- **실측 fixture** 새 `codex/fixtures/tool_fail_u2.jsonl` — `codex_harness.js`(`.claude/handoff/attachments/20260925-midturn-phase0/`)로 0 아닌 종료 명령 한 턴을 떠서 `fixtures/README.md` 가공 규칙대로(표에 한 행). 시험 = 그 줄들 → `ToolCall` 뒤 `ToolResult{Failed}` 하나. ★그 채취에서 0 아닌 종료가 `completed` 로 오면 멈추고 메인에 올린다★ — 그러면 codex 명령 실패는 `status` 로 안 보이고, 판정을 `exitCode` 로 넓힐지는 사용자 체감이다. MCP 실패 · 벤더 스스로의 `declined` 는 실측 조건(MCP 서버 · guardian 심사)이 무거워 위 인라인 줄로만 덮는다 — README 의 「손으로 지은 줄은 없다」는 그대로 참이다(인라인 줄은 fixture 가 아니다). 우리 거절은 아래 채취 셋째가 실측한다.
- **채취 둘째 — 끊긴 뒤 실패하는 명령**(3판 리뷰 Architect low · 5판 light 재검): 같은 하네스로 도는 codex 명령을 끊고(`turn/interrupt`), 그 명령이 이어서 실패하게 한다(예: 잠시 잠든 뒤 0 아닌 종료). 늦게 오는 그 `item/completed` 의 `status` 를 기록한다. ★**사용자 결정 2026-09-27 = ⓐ 그대로 보인다**(§11 ⑩)★: 5판 태그 판독상(가능성 높음 — 위 사실 「끊긴 명령은 계속 돌다 늦게 끝난다」) 끊어도 명령 프로세스는 살아 있다가 실제 `exitCode` 로 닫히므로, 0 아닌 종료로 끝나는 명령이면 늦은 끝은 `failed` 다. 그 끝은 그 행에 보통 붉은 「오류」로 붙고 묶음 머리가 「오류 1」 을 센다. 바로 아래 그 턴의 「중단됨」 행이 맥락을 주고 명령은 실제로 실패했으므로 거짓 표시가 아니다(사용자 — 「일단 밑에 중단됨 줄이 있는데 뭐가 문제라는거야?」). **새 장치 없음** — 누산기에 규칙을 더하지 않고 `ToolResult` 붙이기 경로(⑧)도 그대로다. 근거 = 피어(`rust-v0.156.1` 소스 판독 · 로컬 클론 — 가능성 높음): codex TUI 는 끊을 때 프로세스를 죽이지 않고 도는 exec 칸을 그 자리에서 실패(붉은 ✗)로 닫으며(`codex-rs/tui/src/chatwidget/turn_runtime.rs:314-352` — `finalize_active_cell_as_failed` `:330`) 백그라운드 터미널을 죽이는 수동 `/stop` 을 따로 둔다 · t3code · paseo 는 늦은 status 를 그대로 흘리고 끝내지도 않는다(t3code `apps/server/src/provider/Layers/CodexAdapter.ts:1013-1020` · paseo `packages/server/src/server/agent/providers/codex-app-server-agent.ts:4684-4692`). 거부한 대안 = §10 0241. 채취는 확인으로 그대로 한다 — 늦은 끝의 `status` 를 기록하고, `failed` 끝은 예상대로라 받아들인다(멈춤 아님). 채취가 전혀 다른 것을 보이면(늦은 끝이 아예 안 옴 · 다른 id 로 옴 · `failed` 가 아닌 늦은 status) B5 가 반환에 기록해 메인에 올린다 — `interrupted`·`completed` 면 기록해 싣는다(0 아닌 종료가 `completed` 로 오면 채취 첫째의 멈춤(위)과 같은 표류다 · `interrupted` 는 벤더가 끊김을 도구 끝에 적은 것이다). 표시 규칙이 그것에 기대지 않으므로 멈춤이 아니다. fixture 로 남길지는 B5 가 반환에 싣는다.
- **채취 셋째 — 우리 거절**(U2-a · 조사 §8-3 「남은 확인」): 같은 하네스로 뜬다 — `codex_harness.js:115` 가 `approvalPolicy:'on-request'` · `sandbox:'workspace-write'`(우리 스폰 `backend/codex/mod.rs:132-133` 과 같다)로 띄우고, `:63-64` 가 서버 요청을 우리 `refuse` 와 같은 `-32601` 로 거절한다. 프롬프트는 승격이 필요한 동작을 강제한다 — ⓐ cwd 밖 임시 경로에 **셸 명령으로** 파일 쓰기(명령 승인) ⓑ 편집 도구로 cwd 밖 파일 고치기(파일 변경 승인) · 각각 한 턴.
  - 기록: ① 원 승인 요청 줄 ② 우리 오류 답 줄 ③ 그 뒤 약 20 줄 — 명령 · 파일 변경 각각의 `item/started`(있나 · 요청보다 앞인가)와 `item/completed` 의 `status` · `exitCode` · `aggregatedOutput`, 그리고 `error` 알림 · `turn/completed` 의 `failed` 여부 ④ 이어지는 `agentMessage` 글 ⑤ B5 뒤 GUI 스크린숏(§8-2 F3 ④). (가능하면 · 기록만) 그 스레드를 이어받아 이력 페이지를 본다 — 거절된 명령 item 이 **없는지** · 파일 변경 item 이 `declined` 로 남는지(②-2 한계 — 5판 태그 판독의 실행 확인 · §11 ⑨).
  - ★귀속은 끝 status 와 무관하게 짓는다(스위치 없음 — ②-2)★ — 바꿔 쓰기가 조건 없는 `Failed | Declined → Refused` 라 이 채취가 정하는 코드는 없다. 하는 일 = fixture 채취와 아래 멈춤(5판 light 재검). 예상 = 명령 끝 `failed`(업스트림 판독) · 파일 변경 끝 `declined`(5판 태그 판독) → 둘 다 `Refused`. 명령 끝이 `declined` 로 와도 같다(그 status 도 바꿔 쓴다) — 반환에 적는다. 그 줄들을 fixture `codex/fixtures/refuse_u2a.jsonl`(명령 · 파일 변경 · README 표 한 행)로 남기고 통로 시험이 리더에 먹인다 → 각각 `ToolCall` 뒤 `ToolResult{Refused}` 하나.
  - 파일 변경 경로가 태그 판독(시작이 승인 전에 선다 · 끝 = `declined`)과 다르면 기록해 반환에 싣는다. 시작 없이 끝만 오면 누산기가 가리킬 행이 없어 버린다(⑧ — 오늘처럼 안 보인다).
  - ★**명령 · 파일 변경 각각** 아래 중 하나면 멈추고 메인에 반환한다★(5판 light 재검 — 파일 변경을 멈춤에 넣었다): ⓐ 끝의 item id 가 승인 요청의 `itemId` 와 다르다 ⓑ 끝이 안 온다 ⓒ 끝 status 가 `failed`/`declined` 가 아니다(`completed` 등) · 그 밖에 그 턴이 `failed` 로 끝나거나(오류 뒤 멈춤이 선다) `error` 알림이 온다. 파일 변경도 멈춤인 이유 = 그 귀속이 이제 하중을 진다: 놓치면 우리 거절이 벤더 `Declined`(「거부됨 · 실행되지 않음」)로 **경보 없이** 보인다(명령은 놓치면 붉은 「오류」로 드러난다). 업스트림 판독은 id 가 맞는다(승인 요청 `item_id = event.call_id` `beh:638-647@v0.156.1` · 시작 item id = `ctx.call_id` `core/src/tools/events.rs:256@v0.156.1` — 위 사실) — 이 채취는 설치본의 표류를 잡으려고 있다.
- **통로 — 거부 귀속**(②-2 · `codex/transport.rs` 시험 · 리더를 짓는 기존 모양 `:4372` · 거절 줄 단언 `:4700` 모양): ① 명령 승인 요청(`approvalId` 없음) → 거절 줄 하나 → 같은 `itemId` 의 `item/completed`(`failed` · `exitCode:null`) → 나간 사건 = `ToolResult{Refused}` ② 거절 없이 온 `failed` 끝 → `Failed` ③ **두 출처** — 한 리더에서 ① 과 나란히, 거절 없이 온 `status:"declined"` 끝 → `Declined`(거절한 id 는 `Refused` · 안 한 id 는 `Declined` — 사유가 갈리는 자리) ④ 파일 변경 승인 거절 → `declined` 끝 → `Refused`(5판 태그 판독의 모양) · 같은 거절 뒤 `failed` 끝이어도 `Refused` ⑤ 귀속 없음 — `item/tool/call` · `item/permissions/requestApproval` · `approvalId` 있는 명령 승인 · `networkApprovalContext` 있는 승인 · `itemId` 없음 · 129 바이트 id 뒤의 `failed` 끝 = 사건 없음(번역기가 `bounded_id`(`decoder.rs:1225-1227` — 128 B 넘는 id 를 거른다)로 걸러 `ToolResult` 를 안 내고 `Malformed` 만 센다 — ② 발행 · 기억도 안 한다 — 조건 첫째) ⑥ 끝이 꺼낸다 — 같은 id 의 둘째 `failed` 끝 = `Failed` · 둘째 `declined` 끝 = `Declined` · `completed` 끝도 꺼내 뒤 `failed` 끝 = `Failed` ⑦ 상한 — 거절 17 개 → 첫 id 의 끝 = `Failed` · 둘째 id = `Refused` ⑧ 턴 끝에서 안 비움 — 거절 → `turn/completed` → 끝 = `Refused` ⑨ 넘침 · 닫힘 — 제어 큐가 `OUTBOX_LIMIT` 로 차 있거나 `closed` 면 기억하지 않는다 → 끝 = `Failed` ⑩ 새 리더 = 빈 집합 ⑪ 실측 `refuse_u2a.jsonl`(채취 셋째의 명령 · 파일 변경 줄 → 각각 `Refused`) ⑫ 같은 id 는 한 칸(5판 · 4판 리뷰 low) — 같은 `itemId` 를 두 번 거절 → 그 id 의 끝 = `Refused` → 같은 id 의 둘째 `failed` 끝 = `Failed` · 기억 칸 수 = 1(해가 없는 방어 규칙 — 업스트림은 한 id 에 거절을 둘 걸지 않는다 · ②-2 모양 · 상한). 번역기 시험(위)은 그대로다 — 번역기는 집합을 모른다.
- 두 분류기: `ToolResult` → `None`(네 결말 각각).
- 턴 끝 뒤 끝: `steer_m6.jsonl` 49–50 줄 모양으로 `turn/completed` 뒤 `failed` 끝 → 턴 표 `in_turn` 거짓 유지 · `last_end_failed` 불변.
- wire: 직렬화 골든 `{"type":"ToolResult","id":"i-1","outcome":"Failed"}` · `…"outcome":"Refused"}` · 왕복 · 데몬 변환 일대일(네 결말).
- 프론트: 누산기 — 같은 id 에 붙음 · 항목 수 불변 · 없는 id 는 버림 · 턴 끝 뒤 도착해도 `turnDone` 불변 · replay 두 번 = 같은 스냅숏 · `Refused` → `'refused'` · `Declined` → `'declined'` · 모르는 outcome → 표식 없음 · `ToolResult` 의 `feed` 반환 = `false`(붙인 경우 · 못 찾은 경우 둘 다 — 스냅숏은 붙인 경우에만 바뀐다). ★늦은 끝 창(§4-7 ⑧ · FE-1.3 이 `RichSlot.test.tsx` 에)★: 한 턴이 끝난 뒤 새 글을 보내 대기가 켜진 상태에서 지난 턴 도구의 `ToolResult{Failed}` 만 오면 → Wait 표시가 남고(대기 참 유지) 그 행에 배지가 붙는다 · 이어 새 턴의 첫 진짜 항목(`TextDelta` 등)이 오면 → 대기 플래그(`awaiting`)가 풀린다. ★Wait 표시가 여기서 꺼지지는 않는다(7판 정정)★ — 열린 턴이 턴 경계까지 Wait 를 켜 둔다(`RichSlot.tsx` `streaming = awaiting || (!turnDone && items.length > 0 && !historyPending)`). ★ⓐ 고정(§11 ⑩ · ⑪ · FE-2 — `structuredAccumulator.test.ts` · `StructuredTextView.test.tsx`)★: 끊긴 `TurnEnd{Interrupted}` 뒤 그 턴 행의 늦은 `ToolResult{Failed}` → 그 행 표식 `'failed'` · 묶음 요약 「오류 1」 · 끊긴 결말 행은 그대로 남는다 · claude 짝 = 끊긴 턴의 도구 `tool_result` 가 `is_error:true` 면 붉은 배지. `summarizeGroup` — codex 실패 · claude 파싱 오류 · 거부 셈 = `refused` + `declined`(오류 수에 안 든다) · `DECLINED_MARK` 두 값. `StructuredTextView` — 배지 두 종 · codex 행에 Out 없음 · 거부 행(U2-a): 배지 글 「거부됨」 · 색 토큰 `var(--status-blocked)`(붉은 클래스 없음) · 사유 줄이 접힌 채로도 있다 · `data-tool-mark="declined"` · 머리는 기본 톤 · 레일 위치가 사유 줄 없는 같은 목록과 같다(ADR-0051 — 사유 줄은 행이 아니다) · ★두 출처(5판)★: `refused` 행 사유 = `chat.toolRefusedReason` · `declined` 행 사유 = `chat.toolDeclinedReason` · `data-tool-declined-reason` 이 그 키(출처 단언은 이 키로 — 「실행되지 않음」은 우리 사유의 꼬리라 글 포함으로는 안 갈린다 · `declined` 행의 보이는 글 = 「실행되지 않음」) · 두 행의 배지 · 색 · `data-tool-mark` 는 같다 · 한 묶음에 둘이면 요약 「거부 2」 · 오류 칸 없음 · 묶음 요약 「오류 1 · 거부 1」 순서 · 거부만 있으면 「오류」 칸 없음 · `DECLINED_MARK='error'` 면 거부 둘 다 오류로 — 「오류 2」 · 붉은 배지 · 사유 줄 없음.
- 골든 영향: 대기 입력 골든(`queued_input_golden.json`) 무영향(이 사건은 명부 환원에 안 든다) · 기존 codex fixture 셋(`empty_turn_m9` · `steer_m6` · `tool_end_m7`)은 도구 끝이 전부 `completed` 라 사건열이 그대로다.

---

## 5. F4 claude 글자 스트리밍

### 5-1. 인자

- `claude/mod.rs:184-194` StreamJson 갈래에 `--verbose` 뒤 `args.push("--include-partial-messages")`. 터미널 갈래(`:171-180`)는 그대로.
- 시험: 골든 `json_mode_build_spec_uses_headless_stream_json_args`(`:2674-2704`)에 그 낱말을 더하고, 터미널 모드에 없다는 단언 하나.

### 5-2. 번역기 상태 · 규칙

`ClaudeStreamDecoder`(`:768-794`)에 칸 하나 — [고름] 줄 버퍼와 같은 수명(화신 하나):

```rust
/// 라이브 부분 메시지 추적 — `stream_event` 가 채우고 완결 `assistant` 줄이 읽는다.
#[derive(Debug, Default)]
struct PartialMessage {
    /// 지금 메시지의 id(`message_start`). `None` = 추적 중인 메시지가 없다.
    id: Option<String>,
    /// 시작했고 아직 멈추지 않은 블록 번호(`content_block_start` ~ `content_block_stop`).
    open: Option<u64>,
    /// 글 델타를 하나라도 흘린 블록 번호들.
    streamed: BTreeSet<u64>,
}
```

`consume_line`(`:882-1005`)은 인자를 하나 더 받는다: `partial: Option<&mut PartialMessage>` — 라이브(`decode` `:836-840` · `flush` `:865`)는 `Some(&mut self.partial)`, 이어받기(`parse_transcript_events` `:1351`)는 `None`.

| 줄 | 규칙 |
|---|---|
| `stream_event` + `parent_tool_use_id` 가 null 아님 | **통째로 버린다**(`message_start` 포함 — 하위 에이전트 흐름이 부모 상태를 덮지 않게 · 조사 §4-2) |
| `message_start` | `*partial = { id: message.id, open: None, streamed: {} }` — 턴 도중 접힌 입력이 여는 새 메시지도 여기서 새로 선다 |
| `content_block_start{index}` | `open = Some(index)` |
| `content_block_delta{index, delta:{type:"text_delta", text}}` | `id` 가 있고 `text` 가 비지 않으면 `TextDelta{text, turn_id: None, message_id: id.clone()}` · `streamed.insert(index)` · `id` 가 없으면(`message_start` 없이 온 늦은 델타) **버린다** |
| 그 밖의 델타(`thinking_delta` · `input_json_delta` · `signature_delta`) | 버린다 — 생각·도구는 완결 줄에서 오늘처럼 |
| `content_block_stop{index}` | `open == Some(index)` 면 `open = None` |
| `message_delta` · `message_stop` · 모르는 `event.type` | 아무것도 안 낸다 — ★`MessageDone`/`TurnEnd`/`Structured` 금지★(§5-3) |
| 완결 `assistant` 줄 | `message.id == partial.id` 이고 `open = Some(k)` 이고 `streamed ∋ k` 면 그 줄의 **`text` 블록만** `TextDelta` 를 내지 않는다. 나머지 블록(`tool_use` · `thinking` · …)과 그 밖의 모든 경우(안 흘린 블록 · id 불일치 · `partial` 없음)는 오늘 그대로(`consume_block` `:1045-1128`) |
| `result` | 오늘 번역 뒤 `*partial = PartialMessage::default()` |

- **「(메시지 id, 블록 번호)」를 여는 블록으로 잡는 이유** [고름]: 완결 줄에는 블록 번호가 없다. 계약은 「비지 않은 블록마다 완결 메시지 하나, 그 블록의 `content_block_stop` 보다 먼저」다(조사 §4-2 — 실측 + 공식 문서 = 확실). 그러니 완결 줄이 오는 순간 열려 있는 블록이 곧 그 줄의 블록이다. 빈 블록(완결 줄 없이 멈춤)은 자리를 차지하지 않는다. 벤더가 순서를 바꾸면(멈춘 뒤 완결) `open` 이 비어 **오늘처럼 전문을 낸다 — 잃지 않고 겹친다**(보이는 쪽으로 틀린다).
- **기본은 합치지 않는다**(codex 와 같다 — 조사 §4-4). 단 §9 링 압박의 수치 문턱을 넘으면 B1 안에서 합친다(§9).
- 문서 갱신: `:762-767`(「decoder 자신의 상태 = 줄 재조립뿐」)을 이 칸으로 고친다.

### 5-3. 불변식

- **턴 끝은 `result` 한 줄 그대로**(조사 §4-4 함정) — `message_stop` 을 끝으로 옮기면 도구 호출마다 턴이 끝난다.
- **스트림 부속 줄을 `Structured` 로 내지 않는다** — claude 턴 분류기는 `Structured` 를 통째로 진행으로 센다(`:538-545`). 턴 끝 뒤의 진행은 30 분 막힘 경로다(CLAUDE.md 「대기 입력 상태」 끝 문단).
- 흘린 델타는 `TextDelta` 라 진행이다 — 턴 **안**에서만 나온다: `result` 가 상태를 지우고, `message_start` 없이 온 늦은 델타는 버리므로 턴 끝 뒤에 「턴 중」을 다시 켤 길이 없다.
  - ★이 「턴 끝 뒤 진행 없음」은 벤더 전제 하나에 기댄다 — **벤더가 턴 밖에서 최상위(`parent_tool_use_id` null) `stream_event` 를 내지 않는다**★. 한가할 때 `message_start` + 글 델타가 오면 번역기는 그것을 새 메시지로 받아 `TextDelta` 를 내고, 「턴 중」(과 §3-4 턴 열림 문)이 켜져 30 분 막힘 경로가 된다. 번역기로는 막을 수 없다(턴 도중 접힌 입력의 새 `message_start` 와 모양이 같다). 그래서 B1 채취가 확인한다(§5-4 ③).
- **프론트 무변경**: 누산기 `TextDelta` 갈래는 마지막 글 항목에 이어 붙이고 중복 제거가 없다(`structuredAccumulator.ts:130-141`) — 제거는 번역기 몫이고 위 표가 진다. 흘린 첫 델타와 완결 본문이 같은 글 항목으로 모이므로 완결 버림이 빠지면 글이 두 벌이 된다(회귀 시험).

### 5-4. 이어받기 기록 (transcript)

- [고름] `None` 경로는 `stream_event` 줄을 **통째로 건너뛴다**(오늘의 `_ => {}` `:1003`). 기록에 그 줄이 있든 없든 완결 `assistant` 줄이 전문을 내므로 한 벌이다 — 지시의 「기록마다 새 상태」보다 단순하고 기록 모양에 기대지 않는다.
- **확인 단계**(B1 끝): 플래그를 켠 세션 하나를 이어받아 ① 그 `~/.claude/projects/<slug>/<sid>.jsonl` 에 `"stream_event"` 가 있는지 ② 완결 `assistant` 글 줄이 남아 있는지 기록한다. ② 가 거짓이면(기록이 부분 줄만 남긴다) 이어받은 화면의 답이 빈다 → **멈추고 메인에 올린다.** ① 이 참이면 그 줄을 fixture 로 떠서 「건너뛴다」를 못 박는다.
- **한가 구간 확인**(B1 채취 · §5-3 전제): 라이브 채취에서 각 `result` 뒤 다음 입력을 쓰기 전까지(≥ 30 초 한 구간 포함) ③ 최상위 `stream_event` 줄이 **0** 인지 기록하고, 그 구간을 fixture 에 담아 「`result` 뒤 · 다음 사용자 줄 전 = `TextDelta` 0」을 시험으로 단언한다(§5-5 의 13 번). 0 이 아니면 **멈추고 메인에 올린다.**
- **링 압박 기록**(B1 채취 · §9): 턴마다 흘린 글 델타 수 · 글 글자 수를 적는다. 긴 답(≥ 2,000 자) 한 턴을 반드시 포함한다.

### 5-5. 시험 · fixture

- **새 fixture** `backend/claude/fixtures/partial_stream_p1.jsonl` — 우리 실제 스폰 인자(`MAX_THINKING_TOKENS=8000` 포함)로 떠서 `fixtures/README.md` 의 가공 규칙(잡음 줄 · `system/init` 손질 · 개인정보 치환)을 적용한다. 담을 것: 글만인 답 · 글 → 도구 → 결과 → 글 · 생각이 있는 답 · 턴 도중 접힌 입력(새 `message_start`). README 표에 한 행.
- 시험(조사 §4 의 경우 + 지시의 추가):
  1. 글 델타마다 `TextDelta`(메시지 id = `message_start` 의 것) · 그 블록의 완결 글은 안 나온다.
  2. 델타 없는 글 블록 → 완결 전문(폴백).
  3. `tool_use` 는 완결 줄에서 `ToolCall` 한 번 · `input_json_delta` 는 사건 0.
  4. 생각 델타 무시 · 완결 생각 블록 → `Structured{thinking}` 오늘처럼.
  5. `parent_tool_use_id` 있는 `stream_event` 전부 버림(그 `message_start` 가 상태를 안 바꾼다).
  6. `message_start`/`message_delta`/`message_stop` → 사건 0(`MessageDone`·`TurnEnd`·`Structured` 없음).
  7. 새 `message_start` 가 상태를 갈아 끼운다(도구 루프 · 접힌 입력).
  8. `result` 뒤 늦은 델타 → 사건 0.
  9. 줄이 청크 둘에 걸친 `stream_event`(한글 포함) → 한 번.
  10. 이어받기 원문에 `stream_event` 가 섞여도 완결 글 한 벌.
  11. ★회귀 — 부분 줄 없는 기존 fixture 전부(`claude_text` · `claude_tool` · `claude_transcript` · `lifecycle_m1` · `cancel_m3` · `drain_m7` · `slash_m13` · `transcript_queued_m5` · `result_error_handbuilt`)의 사건열이 바이트 단위로 같다★ — 기존 시험을 고치지 않고 통과시키는 것이 증거다(고쳐야 하면 설계가 틀렸다).
  12. `classify_turn`: 흘린 `TextDelta` 는 진행 · 부속 줄은 신호 없음.
  13. 채취한 한가 구간(`result` 뒤 · 다음 사용자 줄 전)에서 사건 중 진행 신호 0(§5-4 한가 구간 확인).
  14. (§9 문턱을 넘어 합치기를 지을 때만) 한 `decode()` 안의 이웃 `TextDelta` 는 같은 `message_id` 끼리만 하나로 합쳐지고 · 사이에 다른 사건이 끼면 끊기며 · 합친 글 = 원래 글들의 이음(청크 경계가 달라도 최종 글 항목이 같다).
- ADR 후보: 이 중복 제거 규칙은 load-bearing 이다(프론트에 제거가 없고, 빠지면 모든 claude 답이 두 벌이다) — §10.

---

## 6. 건드리는 불변식 (한눈에)

| 불변식(CLAUDE.md 「핵심 불변식」·ADR) | 기능 | 어떻게 지키나 |
|---|---|---|
| 턴 관측 · 30 분 fail-open(ADR-0127) | F2 · F3 · F4 | F2 = 끊긴 턴은 `TurnEnd` 로 닫힌다(S1 · S6) · 턴 밖 끊기는 턴 열림 문이 거절(S8) · F3 = `ToolResult` 는 두 분류기에서 `None` — 턴 끝 뒤에 와도(실측 `steer_m6` 50 줄) 「턴 중」을 안 켠다(§4-7 ⑥ — 통로가 `Refused` 로 바꿔 써도 같다) · F4 = 부속 줄은 신호 없음 · 늦은 델타 버림 · 한가 구간 최상위 `stream_event` 0(§5-4 — 벤더 전제) · F2(6판) = `Structured{kind:"interrupted"}` → `classify_turn` `None`(ADR-0243) |
| 오류 뒤 멈춤 `last_end_failed`(ADR-0231 · 0234) | F2 · F3 | F2 = 끊김 = `Ended(Other)` — 세우지도 풀지도 않는다(S2) · F3 = 도구 실패는 `Failed` 신호가 아니다(§4-7 ⑥) |
| 대기 입력 한 줄기 · 글은 늘 보인다(ADR-0231 · 0235 결정 2) | F2 | 대기분이 다음 턴에 돈다(S3) · `interrupt:` 응답은 번역 안 함 |
| 락 순서(ADR-0006) | F2 | 끊기 줄은 입력 큐만 탄다 — 새 락 간선 없음 |
| codex 통로 락 순서 · 소유권 분할(CLAUDE.md 「핵심 불변식」 — `Announcer.order → {상태 락, replay → …}` · 통로 입력 상태의 정본 = `backend/codex/transport.rs` 의 `State`) | F3(U2-a) | 거부 귀속 집합은 **리더(`Reader`) 전용 칸**이다 — `State` 칸 목록 · 락 순서 무변경 · 새 락 없음. 쓰기(거절)와 읽기(끝 줄)가 둘 다 리더 스레드이고, 바꿔 쓰기는 emit 전에 락 없이 한다(§4-7 ②-2 · §11 ⑧ 메인 수락 — `State` 칸 안과 다르다) |
| 백엔드 지식은 `backend` 한 곳(ADR-0004) | F2 · F3 · F4 | 줄 모양 · 도구 이름 표 · `commandActions` 해석 · codex 끝 상태 매핑 · 승인 요청 이름 · 칸(거부 귀속) · 부분 메시지 규칙 전부 `backend/{claude,codex}` · 통로·프론트는 중립 값만 · 프론트 사유 문구도 백엔드 이름이 없다(`Declined` = 「실행되지 않음」 — §4-7 ⑧ · 5판 light 재검)(claude `tool_result` 파싱은 오늘 그대로 — 누수를 늘리지 않는다 · §4-7 ④) · 벤더 접두 판별(`[Request interrupted by user`)은 `backend/claude` 안(6판 · ADR-0243) |
| replay→live · 누산기 멱등 | F3 · F4 | 묶음은 렌더 파생(누산기 상태 아님) · `ToolResult` 는 id 로 기존 항목에 붙고 새 항목을 안 만든다 · 델타도 같은 `TextDelta` 어휘 |
| wire 판 기준(`protocol/src/lib.rs:121` 의 doc) | F3 | `ToolResult` 로 `PROTOCOL_VERSION` 을 안 올린다 — 데몬→셸 한 방향 · 한 판 안에서 셸을 가를 값이 없다(§4-7 ⑤) |
| 행 종류 ↔ 레일(ADR-0051) | F3 | 레일을 행 목록으로 계산 · 묶음 안에 레일 행 없음 |
| 제어 표면 하나 · 새 전역 핸들 금지(「LLM-우선 제어」) | F1 · F2 · F3 | 레지스트리 명령 셋 · 모듈 맵 · DOM 표지 |
| 전역 단축키 가드(`keybindings.ts:11-35`) | F2 | 손대지 않는다 |
| PTY 무변경 | F2 · F4 | 터미널 인자·`TerminalSlot` 무변경 |

---

## 7. 구현 순서 · 워커 분할 — 파일 겹침으로 가른다

★**어디서 멈춰도 빌드가 서게 짠다** — 각 단계가 끝나면 그 워커의 게이트가 초록이고 커밋이 있다. 자료구조를 바꾸는 단계는 그 생성 자리를 같은 커밋에서 다 맞춘다.★ 위임 전 되돌릴 지점 = 이 TRD 커밋.

### 백엔드 — 워커 하나, 순차 (`backend/claude/mod.rs` 를 F2 · F3 · F4 가 다 건드린다)

| 단계 | 내용 | 파일 | 끝났을 때 |
|---|---|---|---|
| **B1** F4 | 인자 · `PartialMessage` · `consume_line` 인자 · fixture · 시험 1–13 · §5-4 확인(이어받기 · 한가 구간 · 링 압박 기록) · §9 문턱을 넘으면 합치기 + 시험 14 | `claude/mod.rs` · `claude/fixtures/*` | `cargo test -p engram-dashboard-agent -- --test-threads=4` 초록 · 기존 fixture 시험 무수정 · 델타 수 기록이 반환에 실림 |
| **B2** F2 스파이크 | §3-5 S1–S8 · 보고서 · fixture 채취 | 스크래치 하네스 · `docs/research/…` · `claude/fixtures/interrupt_s1.jsonl` | ★출구 조건 미달 → 멈추고 메인에 반환(B3 의 claude 부분 착수 금지)★ |
| **B3** F2 구현 | ⓐ 버스 `agent.interrupt`(스파이크와 무관 — B2 가 멈춰도 이것은 간다) ⓑ `with_interrupt`(`Option` 반환) · 턴 열림 문 · 끊기 줄 · `result` 분류 · `TurnEnd{Interrupted}` · 두 주석 갱신 · 기존 시험 둘 의도적 수정(§3-6) · (S2 가 못 가를 때만) ④ 표식 · **합성 줄 알아보기(6판 · ADR-0243 — §3-4)**: `[Request interrupted by user` 로 시작하는 글 하나짜리 `user` 줄 → `Structured{kind:"interrupted", json:{"text"}}` · `classify_turn` `None` · 시험(두 문구 · 글 블록 둘 이상 또는 다른 글은 오늘처럼 `user` · `isReplay:true` 인 같은 글은 오늘처럼 `user` · 라이브와 이어받기 둘 다 · 분류 `None` · 픽스처 `interrupt_s1` 의 합성 줄 둘) · 주석 갱신(6판 doc 리뷰): `types.rs` `QueuedInput` doc(`:94-95` — 「claude 턴 분류기가 `Structured` 를 통째로 진행 신호로 센다」) · `classify_turn` doc 에 `kind:"interrupted"` 예외 | `commands.rs` · `agent/bindings/*`(생성물) · `tests/command_declarations.rs` · `transport/stdio.rs` · `claude/mod.rs` · `types.rs`(주석) · (B3a 시험 전용) `daemon/src/control/commands.rs`(시험) · `daemon/src/connection_core.rs`(시험) · `daemon/tests/control_agent.rs` · `agent/tests/ts_export.rs` | 위 게이트 + `cargo test -p engram-dashboard-daemon -- --test-threads=4` |
| **B3c** PTY 끊기 제거(7판 · ADR-0245 · 사용자 결정 ⑭) | `PtyTransport::interrupt()` → `Unsupported`(0x03 을 쓰지 않는다 · 입력 큐 무변경) · `capabilities()` 의 `control.interrupt = false` · 시험(거절 · 아무것도 안 씀 · 능력 거짓 · 데몬 쪽 PTY 에이전트 끊기 = 오류) · 주석 · 문서 정리 — 「PTY=0x03 주입」 · 「Ctrl+C」 로 적힌 자리(예: `session.rs` 의 `interrupt` doc · 데몬 `connection_core.rs` 의 `interrupt()` 서술 · 셸 `src-tauri/src/commands/agent.rs` 의 끊기 주석 · `docs/reference/structure/session-path-ownership.md` 의 두 줄(7판 doc 수정에서 고쳤다) — 전수는 `rg -e 0x03 -e "Ctrl[-+]C" crates src-tauri docs/reference` 로 다시 본다) · ★의도적으로 고쳐 쓰는 기존 시험 — 회귀로 읽지 말 것★: `crates/engram-dashboard-daemon/tests/ws_e2e.rs` 의 `case16_ws_interrupt_ack_process_alive`(`:1208`@a75569b — PTY 끊기가 ack 되고 프로세스가 산다) → `case16_ws_interrupt_refused_on_terminal_process_alive`(`:1210`@02e7f18 — PTY 끊기가 거절되고 프로세스가 산다) · ★착지 = `02e7f18`★ | `transport/pty.rs` · 그 시험 · `session.rs`(주석) · `backend/mod.rs` · `transport/stdio.rs` · `protocol/src/messages.rs`(주석만) · 데몬(주석 · 시험 — `tests/ws_e2e.rs`) · `src-tauri/src/commands/agent.rs`(주석) · `docs/reference/structure/session-path-ownership.md` | `cargo test -p engram-dashboard-agent -- --test-threads=4` · `cargo test -p engram-dashboard-daemon -- --test-threads=4` 초록 |
| **B4** F3 백엔드 | `ToolCategory` 두 벌 · wire 칸 · 데몬 변환 · 생성 자리 전부 · claude 표 · codex 판정 · 생성물 | `types.rs` · `protocol/src/messages.rs` · `protocol/bindings/*`(생성물) · `daemon/src/connection_core.rs` · `daemon/src/agent_conn.rs` · `output_core.rs` · `claude/mod.rs` · `codex/decoder.rs` | `cargo test --workspace -- --test-threads=4` 초록 · 생성물 sync |
| **B5** U2 codex 끝(§4-7) | `ToolOutcome` 두 벌 · `ToolResult` 변형(agent · wire) · 데몬 변환 · 망라 match 전부 한 커밋(§4-7 ⑦) · codex `tool_outcome` · 두 분류기 `None` · 번역기 주석 셋 갱신(`decoder.rs:262-263` · `:523-525` · `:623` 앞) · 새 변형 doc 의 판 한 줄 · 시험 · 실측 fixture `tool_fail_u2.jsonl` · 생성물 · **거부 귀속(U2-a — §4-7 ②-2 · 늘 짓는다)**: 채취 셋째 먼저(fixture 채취 + 멈춤 — 바꿔 쓰기가 조건 없는 `Failed`·`Declined` → `Refused` 라 이 채취가 정하는 코드는 없다 · 멈춤이 걸리면 귀속 설계가 틀리므로 귀속 코드보다 먼저 뜬다) → `ToolOutcome::Refused`(두 벌 · 생성물) · 리더 칸 `refused`(같은 id 는 한 칸) · `refuse` 인자 · 메서드 상수 둘 · `handle_line` 한 줄(`failed`·`declined` → `Refused`) · 통로 시험 ①–⑫ · fixture `refuse_u2a.jsonl` | `types.rs` · `protocol/src/messages.rs` · `protocol/bindings/*`(생성물) · `daemon/src/connection_core.rs` · `daemon/src/bin/saturation_pilot.rs` · `output_core.rs` · `claude/mod.rs`(분류기 · 시험 이름표) · `codex/decoder.rs` · `codex/mod.rs` · `codex/transport.rs` · `codex/protocol.rs` · `codex/fixtures/*` · `agent/tests/stdio_smoke.rs` | B4 게이트 + `npx tsc --noEmit` · `npm test`(생성물이 프론트 망라를 건다) · ★착수 조건 — FE-2 의 가드 커밋이 트리에 있다 — 메인이 B4 반환 뒤 확인하고, 없으면 B5 지시를 FE-2 뒤로 미룬다(생성물이 첫 시험 실행에서 써지므로 커밋 조건으로는 늦다 · §11 ⑥)★ · ★채취에서 0 아닌 종료가 `completed` 면 멈추고 반환(§4-7 ⑨)★ · 채취 둘째 = 사용자 결정 ⓐ(§11 ⑩)의 확인 — 늦은 `failed` 끝은 예상대로(멈춤 아님) · 늦은 끝이 안 오거나 다른 id 로 오거나 `failed` 아닌 늦은 status(`interrupted`·`completed`)면 기록해 반환(멈춤 아님 · §4-7 ⑨ 채취 둘째) · ★채취 셋째: 명령 · 파일 변경 각각 끝의 item id ≠ 승인 `itemId` · 끝이 없다 · 끝 status 가 `failed`/`declined` 가 아니다 · 그 턴이 `failed` 로 끝나거나 · `error` 알림이 오면 멈추고 반환 · 태그 판독과의 차이는 기록해 반환(§4-7 ⑨ 채취 셋째 — 스위치 없음)★ |

### 프론트 — 워커 둘

- **FE-1.0**(FE-1 의 첫 커밋 · FE-2 는 이 커밋 뒤에 시작): 공유 파일 둘을 한 번에 친다.
  - `src/i18n/ko.ts` 키(전부): `agent.interrupt: '응답 중단'` · `slot.scrollToBottom: '맨 아래로'` · `chat.toolGroupSetExpanded: '도구 묶음 펼치기·접기'` · `chat.toolGroupSearch: '검색 {count}'` · `chat.toolGroupRead: '읽기 {count}'` · `chat.toolGroupList: '목록 {count}'` · `chat.toolGroupEdit: '편집 {count}'` · `chat.toolGroupCommand: '명령 {count}'` · `chat.toolGroupWeb: '웹 {count}'` · `chat.toolGroupAgent: '에이전트 {count}'` · `chat.toolGroupMcp: 'MCP {count}'` · `chat.toolGroupOther: '기타 {count}'` · `chat.toolGroupErrors: '오류 {count}'` · `chat.toolGroupDeclined: '거부 {count}'` · `chat.toolDeclined: '거부됨'`(배지 — 두 출처 공용) · `chat.toolRefusedReason: '대시보드가 승인 요청을 처리하지 않아 실행되지 않음'` · `chat.toolDeclinedReason: '실행되지 않음'`(뒤 넷 = §4-7 ⑧ · U2-a · 사유 둘은 5판 · `toolDeclinedReason` 문구 = 사용자 결정 2026-09-27)(키는 두 단 — `src/i18n/index.ts` 의 `StringKey`).
  - `src/commands/contributions.ts:8-13` 에 `import './scrollCommands'` · `import './chatCommands'` + 두 파일을 머리 주석만 있는 빈 모듈로.
- **FE-1**(RichSlot 소유 · 순차): FE-1.1 F1(§2 전부 · DomSlot) → FE-1.2 F2 프론트(`agentCommands.ts` · RichSlot Esc · 오버레이 표지 넷) → FE-1.3 F3 접착(`slotId` · `bind`/`clear` · `onGroupToggle` → `unpin`, FE-2 의 스토어 착지 뒤 · `RichSlot.test.tsx` 에 늦은 `ToolResult` 창 시험 — §4-7 ⑨ 프론트 · `RichSlot.tsx:218-221` 주석을 `feed` 반환의 새 뜻(`false` = 대기를 풀지 않는 프레임 — 못 알아들은 프레임 + `ToolResult`)에 맞춰 고친다 — §4-7 ⑧ · §12 4판 4).
- **FE-1.4 끊는 중 표시 · Esc 무시**(7판 · ADR-0244 · §3-2 「끊는 중」 — FE-1.3 뒤 · 백엔드 무변경): 파일 = `src/commands/agentCommands.ts`(보낼 때 세움 · 거절에 걷음) · `src/store/interruptStore.ts`(새 — 대기 중 요청 · 뷰 수 `watch`) · `src/components/slot/RichSlot.tsx`(뷰 등록 · 첫 `'live'` 뒤 프레임이 누산기의 턴 경계 수를 올리면 걷음 · `streaming` 뒷받침(대기에 든 보냄) · 첫 따라잡기에 열린 턴 없음 · onReset · 부재 · 연결 끊김 · 제 구독 오류면 `watch` 풀기 · 술어 `ctx` 와 뷰에 넘김 — §3-2 「끊는 중」 ③) · `src/components/slot/interruptKey.ts`(`ctx.interrupting`) · `src/components/slot/StructuredTextView.tsx`(꼬리에 넘김 — ★순수 렌더 유지 · 스토어를 구독하지 않고 prop 하나로 받는다★ — §4-3) · `src/components/slot/chat/WaitRow.tsx`(정지 아이콘 + 「중단하는 중…」 · 흐린 글 + 깜빡임) · `src/i18n/ko.ts`(`chat.interrupting: '중단하는 중…'`) · 시험(§3-6 7판). `StructuredTextView.tsx` 는 FE-2 파일 · `ko.ts` 는 FE-1.0 의 공유 파일이지만 FE-2c 착지(`b0b5440`) 뒤라 겹침이 없다.
- **FE-2**(FE-1 과 병렬): `chat/toolRuns.ts` · `chat/ToolGroupRow.tsx` · `store/toolGroupStore.ts` · `StructuredTextView.tsx`(U2-a 거부 행 — 주황 배지 · 출처별 사유 줄 둘 · `data-tool-mark` · `data-tool-declined-reason` · §4-7 ⑧) · `structuredAccumulator.ts`(지역 `ToolCategory` 합 — §4-1 · U2 의 지역 `ToolOutcome` 합 + 가드 — 아래) · `commands/chatCommands.ts`. B4 · B5 를 기다리지 않고, 생성물 `ToolCategory.ts` · `ToolOutcome.ts` 를 import 하지 않는다.
  - ★**U2 가드 — 생성물에 변형이 오기 전후 둘 다 `tsc` 초록이어야 한다**★: `consume` 의 `switch` 앞에 `if (isToolResultEvent(ev)) return this.consumeToolResult(ev)` · `isToolResultEvent(ev: unknown): ev is LocalToolResultEvent` · `LocalToolResultEvent = { type: 'ToolResult'; id: string; outcome: ToolOutcome }`(지역 합 `'Completed' | 'Failed' | 'Declined' | 'Refused'` — `Refused` 는 5판). ★가드 가지 안에서 `ev` 의 칸을 읽지 말고 도우미에 통째로 넘긴다★ — 변형이 오기 전에는 그 가지의 `ev` 가 `never` 로 좁혀져 칸 읽기가 `tsc` 오류다. 변형이 온 뒤에는 거짓 가지에서 그 변형이 빠져 `never` 망라(`:243`)가 그대로 선다 · 생성물의 결말 낱말이 지역 합보다 많으면 빠지지 않아 거기서 빨갛다(낱말 표류 경보). 두 상태 모두 TS 5.9.3 `--strict --noUnusedLocals --noUnusedParameters` 스크래치 파일로 확인했다(3판). 도우미는 `outcome` 을 런타임에 다시 거른다(더 새 데몬의 모르는 낱말 → 표식 없음).
  - 이 가드 커밋이 B5 의 착수 조건이다(§11 ⑥) — FE-2 는 가드를 FE-2 의 이른 커밋에 싣는다.
  - **U8 중단 줄 강조**(§1 U8 · `StructuredTextView.tsx:289-316` `OutcomeRow`): `interrupted` 갈래만 굵게 + `var(--accent)` · 아이콘(`CircleStop`) · 문구 그대로 · `failed` · `unknown` 은 오늘과 같다. 시험(`StructuredTextView.test.tsx`) = 중단 행에 굵게 + 강조색 클래스 또는 스타일 · 실패 행 · 모름 행은 오늘 모양 그대로.
  - **FE-2c 끊김 표시 행**(6판 · ADR-0243 — FE-2b-2 뒤 · [고름] B3 를 기다리지 않는다 — 입력은 기존 `Structured` 의 새 `kind` 문자열뿐이라 생성물이 안 바뀐다): `structuredAccumulator.ts` — `Structured{kind:"interrupted"}` → 항목 `{ kind:'interruptNote'; text: string; itemId: number }`([고름] `json` 을 풀어 `text` 가 문자열이 아니면 오늘 탈출구 항목으로) · `kind === 'user'` 갈래(uuid dedup · `turnDone = false`)를 타지 않는다 · `TurnEnd{Interrupted}` 에서 **현재 턴(마지막 구분선 뒤 항목)에 `interruptNote` 가 있을 때만** 결말 행을 건너뛴다(구분선은 그대로 · 순서 전제 = 표시 행은 그 턴의 `result` 앞에 온다 — 실측 14/14 · 구분선 뒤에 오면 다음 턴에 앉아 그 턴의 대비 중단 행을 지울 수 있다) — ★「claude 인가」로 가르지 않는다 · 표시 행의 유무로만 가른다★. `rowKindOf` = `'assistant'`(결말 행과 같게 · ADR-0051 의 renderItem null 규칙과 일치) · `StructuredTextView.tsx` — 표시 행 = U8 중단 행과 같은 모양(`CircleStop` · 굵게 · `var(--accent)`) · 글은 원문 그대로(마크다운 없이). 시험 = 표시 행 + `TurnEnd{Interrupted}` → 결말 행 없음 · 구분선 있음 / 표시 행 없이 `TurnEnd{Interrupted}` → 중단 행(대비 · codex 모양) / 앞 턴의 표시 행이 다음 턴의 결말 행을 지우지 않는다 / 표시 행이 사용자 말풍선이 아니다 · 렌더 모양(굵게 · 강조색 · 원문 글자 그대로).
- **I1 바인딩 교체**(통합 한 커밋): 주인 = 메인이 『코더(단순)』에 맡긴다 · 시점 = **B4 · B5 · FE-2 가 모두 커밋된 뒤**. `structuredAccumulator.ts` 의 지역 합 둘을 지우고 `import type { ToolCategory } from '../../../crates/engram-dashboard-protocol/bindings/ToolCategory'`·`ToolOutcome` 도 같은 모양(같은 파일의 기존 생성물 import 모양 `:18-21`)으로 바꾼 뒤 그 이름을 다시 내보낸다 — 다른 파일의 import 는 안 바뀐다. U2 가드는 `case 'ToolResult':` 갈래로 바꾼다(도우미는 그대로). 게이트 = `npx tsc --noEmit` · `npm test`(프론트가 쓰는 낱말이 생성물에 없으면 여기서 빨갛다 · 생성물이 낱말을 더 가졌는지는 눈으로 대조한다).
- 겹침 점검: FE-1 ∩ FE-2 = ∅(FE-1.0 뒤) · RichSlot = FE-1 만 · StructuredTextView = FE-2 만 · B5 = 백엔드 파일 + 생성물뿐(프론트 소스 0 — 거부 귀속도 `codex/transport.rs` · `codex/protocol.rs` 안) · I1 = FE-2 · B5 끝난 뒤라 겹침 없음 · FE-1.4(7판) = FE-2c 뒤라 `StructuredTextView` · `ko.ts` 겹침 없음.
- **커밋 규율(워커 공통)**: ★어느 워커도 `*/bindings/` 아래 파일(`crates/engram-dashboard-protocol/bindings/` · `crates/engram-dashboard-agent/bindings/` · `src-tauri/bindings/`)을 손으로 쓰거나 고치지 않는다★ — 시험이 굽고 CI sync 게이트가 주인이다. 한 트리를 나눠 쓰는 워커는 커밋할 때 **자기 경로만** 스테이징한다(`git add <자기 파일>` — `git add -A`·`git add .` 금지). 남의 미커밋 변경(생성물 포함)을 자기 커밋에 싣지 않는다.

### 메인이 먼저 못 박는 접점

| 접점 | 모양 | 쓰는 쪽 → 읽는 쪽 |
|---|---|---|
| TS `ToolCategory` | `"Read"\|"Search"\|"List"\|"Edit"\|"Command"\|"Web"\|"Agent"\|"Mcp"\|"Other"` · ToolCall `category?` | B4(생성물) · FE-2(같은 모양의 지역 합) → I1 이 하나로 |
| TS `ToolOutcome` · `StructuredEvent` `ToolResult` | `"Completed"\|"Failed"\|"Declined"\|"Refused"` · `{ type:"ToolResult"; id: string; outcome: ToolOutcome }` | B5(생성물) · FE-2(같은 모양의 지역 합 + 가드 — 위) → I1 이 하나로 |
| `StructuredItem` `tool` | `{ kind:'tool'; name; argsJson; id; category: ToolCategory; resultMark: 'failed'\|'declined'\|'refused'\|null; itemId }` | FE-2 내부 |
| `StructuredTextView` props | `{ items; streaming?; slotId?: string; onGroupToggle?: (isLast: boolean) => void }` | FE-2 → FE-1.3 |
| `useToolGroupStore` | §4-4 | FE-2 → FE-1.3 |
| `FollowHandle` | `{ pinned: boolean; pin(): void; unpin(): void }` | FE-1.1 → FE-1.3 |
| 버스 `agent.interrupt` | `{target}` → `{outcome:"requested"}` · NOT_FOUND · CONFLICT | B3 → (LLM · CLI) |
| 선 `Structured` `kind:"interrupted"`(6판 · ADR-0243) | `{ type:"Structured"; kind:"interrupted"; json: string }` — `json` 은 오늘처럼 직렬화된 JSON 글(`types.rs:91` `json: String`)이고 그 내용 = `{"text":"<합성 줄의 원문>"}` | B3 → FE-2c |
| `StructuredItem` `interruptNote`(6판 · ADR-0243) | `{ kind:'interruptNote'; text: string; itemId: number }` | FE-2c 내부 |

---

## 8. 검증 계획

### 8-1. 기계 게이트 (`/qa`)

- 백엔드 = 각 단계 표의 명령 · 마지막 `cargo test --workspace -- --test-threads=4` · `cargo fmt --check` · 생성물 sync(`protocol/bindings` · `agent/bindings`). B5 는 생성물이 프론트 망라를 걸어 `npx tsc --noEmit` · `npm test` 도 돈다.
- 프론트 = `npm test` · `npx tsc --noEmit`.
- 격리 게이트 = CLAUDE.md 「빌드·검증 명령」 그대로(`use tauri` 0 줄 등) — 이 라운드는 crate 경계를 옮기지 않는다.

### 8-2. GUI 실측 (`scripts/cdp.mjs` · `/qa full` 격리 인스턴스 · ★셸에서 직접 띄우지 않는다★)

| 기능 | 무엇을 재나 |
|---|---|
| F1 | ① 떨어진 채 탭을 돌렸다 돌아오기 — `scrollTop` 이 `display:none` 을 지나 남나(조사 §7 미검 — 코어의 되살리기가 필요했는지 기록) · 붙은 채 돌아오면 바닥 ② 창 새로고침·재구독 replay 폭주 뒤 바닥 착지 ③ 스트리밍 중 위로 휠 → 안 끌려 내려옴 · `data-scroll-follow="free"` ④ 펼친 생각 블록 안 휠 → 바깥 안 풀림 ⑤ 대기 목록이 서며 뷰포트가 줄 때 바닥 유지 ⑥ 버튼 150 ms 뒤 등장 · 누르면 바닥 · 떨어진 채 창을 키워 다 들어오면 버튼이 사라짐 ⑦ `__engramCmd.run('slot.scrollToBottom', {slotId})` ⑧ 스트리밍 중 대화 글을 누른 뒤 ArrowUp 한 번 → 안 끌려 내려옴 · `document.activeElement` 가 그 뷰포트 · 포커스 테두리 없음 · (FE-1.2 뒤) 같은 포커스에서 Esc 가 끊기로 번짐 |
| F2 | ① 실 codex 도구 중 Esc → 중단 행 · 대기 글이 다음 턴 · 끊은 명령이 계속 도는지 기록(§9 벤더 동작 — 5판) ② (B3 뒤) 실 claude 글 흐르는 중 · 도구 중 Esc → 중단 행 · 오류 행 없음(턴 단위 Error 행) · 대기 글이 돈다 · 끊긴 도구 줄 = 붉은 오류 배지 · 묶음이 서면 「오류 1」 · 그 아래 강조된 중단 줄 — ★6판(FE-2c 뒤 · ADR-0243)★ claude 는 사용자 말풍선 없음 · 「응답이 중단됐습니다」 대신 **강조된 끊김 표시 행(원문 `[Request interrupted by user]` · 도구 중이면 `…for tool use]`)** 하나 · 그 세션을 다시 열어도 그 표시 행이 남는다 — 스크린숏(§4-7 ④ · §11 ⑪ ⑫) ③ 한글 조합 중 Esc = 조합만 취소 ④ 안 돌 때 Esc = 무동작 · 턴이 열리기 전(보낸 직후) Esc = 통로의 「unsupported」 오류 · 거절 답까지 「중단하는 중…」이 잠깐 선다(7판 · §3-2 ③ⓐ · ⑩ 과 같다) · (B3 뒤) `engram agent.interrupt …`(일반 호출꼴 — 6판) 를 한가할 때 = CONFLICT ⑤ 우클릭 메뉴 열린 채 Esc = **아무 일도 없다**(끊기 없음 · 메뉴는 그대로 — `SlotContextMenu` 는 Esc 처리기가 없고 바깥 `mousedown` 으로만 닫힌다 `SlotContextMenu.tsx:110-114` · Esc 닫기는 이번에 더하지 않는다) ⑥ 입력창 글 유지 ⑦ 본문 클릭 뒤 Esc(U6) · 「맨 아래로」 · ✕ 를 누른 뒤 Esc(FE-1.2 초점 FIX — `slotFocus.ts` · 7판) ⑧ `engram agent.interrupt …`(일반 호출꼴 — 버스 전용 · 6판 · §3-3) ⑨ (FE-2 뒤 · U8) 중단 행 = 굵게 · 강조색(`var(--accent)`) — dark · light 테마 각각 스크린숏 · 실패 행 · 모름 행은 그대로 ⑩ (FE-1.4 뒤 · 7판 · ADR-0244) 실 claude · codex 도는 중 Esc → 곧바로 꼬리에 정지 아이콘 + 「중단하는 중…」 · 멈추기 전 Esc 연타 → 무시(끊기 추가 호출 없음 — CDP 로 `agentClient.interruptAgent` 호출 수를 세어 한 번 · WS `Interrupt` 는 데몬 로그 줄이 없다 `connection_core.rs:1222-1230`) · 꼬리 톤 = 흐린 글 + 깜빡임 · ★관측 기록★ claude 는 강조된 끊김 표시 행(FE-2c)이 `TurnEnd` 보다 먼저 와서 그 사이 `CircleStop` 행 둘(표시 행 + 흐린 「중단하는 중…」)이 함께 설 수 있다 — 창 = 표시 행 → `TurnEnd` 로 수십 ms 이하(스파이크 S1a 11 ms · S4a1 25 ms · S4a2 6 ms · S4b 4 ms — 조사 `docs/research/claude-interrupt-spike-2026-09-28.md` §2-1 · §5) · 대개 눈에 안 보인다(스크린숏 요구 없음) · 턴 끝(중단 행 · 끊김 표시 행)에서 표시가 사라진다 · 한가할 때 창 명령 `agent.interrupt` 를 직접 부름(LLM 경로 — 통로의 「unsupported」 오류) · 보낸 직후 턴이 열리기 전 Esc(같은 오류) → 창 명령이 어떤 오류로 끝나든 표시가 남지 않고 다음 Esc 가 다시 발화 · 스크린숏 |
| F3 | ① 실 claude 읽기·검색 ≥2 턴 — 도는 동안 펼침 · 글이 오면 접힘 · 요약 문구 ② 실 claude 실패 도구 → 「오류 1」 ③ (B5 뒤) 실 codex 0 아닌 종료 명령(예: `exit 3`)이 든 묶음 → 머리 「오류 1」 · 펼치면 그 행에 배지 · Out 칸 없음 ④ (B5 뒤 · U2-a) 실 codex 에 읽기 하나 + cwd 밖 쓰기 **셸 명령** 하나를 시키는 프롬프트(묶음이 서게 도구 ≥2) → 우리 거절 뒤 그 명령 행 = 주황 「거부됨」 배지 + 사유 줄 「대시보드가 승인 요청을 처리하지 않아 실행되지 않음」(묶음을 펼치면 그 행의 IN/OUT 이 접혀 있어도 보인다 · 묶음이 접혀 있으면 머리에 「… · 거부 1」 — 7판 문구) · 붉은 테 · 「Error」 · 붉은 경고 박스 없음 · 묶음 머리 「… · 거부 1」(「오류」 칸 없음) · `data-tool-mark="declined"` · `data-tool-declined-reason="chat.toolRefusedReason"` · 재구독 replay 뒤에도 같다 · 스크린숏(§4-7 ⑨ 채취 셋째 ⑤) · (같은 프롬프트에 편집 도구로 cwd 밖 파일 고치기를 더해) 파일 변경 거절 행도 같은 배지 · **같은 우리 사유**(codex 는 이 경로를 `declined` 로 닫는다 — `data-tool-declined-reason="chat.toolDeclinedReason"`(글 「실행되지 않음」 만)이면 귀속이 파일 변경을 놓친 것이다 · 글 포함으로 가르지 말 것 — 우리 사유의 꼬리가 같은 낱말이다 · §4-7 ②-2) ⑤ codex 묶음 개수 ⑥ 토글 · 실패 배지가 재구독 replay 뒤에도 남음 ⑦ 같은 이력 replay → 같은 묶음 ⑧ (B5 뒤) 실 codex 도구 ≥2 턴(묶음이 서게 — ④ 와 같다)에서 도는 명령을 Esc 로 끊은 뒤 그 명령이 늦게 실패 → 그 행에 붉은 「오류」 · 묶음 머리 「오류 1」 · 바로 아래 그 턴의 「중단됨」 행(사용자 결정 ⓐ — §11 ⑩ · §4-7 ⑨ 채취 둘째) · 스크린숏 ⑨ 턴 끝 뒤 새 글을 보낸 직후 지난 턴 도구의 늦은 끝이 와도 Wait 표시가 새 턴의 첫 답까지 남음(§4-7 ⑧) |
| F4 | ① 실 claude JSON 에서 글이 점점 늘어난다 ② 블록 끝에 글이 두 벌 안 됨 ③ 도구·생각 한 번씩 ④ 그 세션 이어받기 → 글 한 벌 |

---

## 9. 위험 · 알려진 한계

- **F1 내용 노드 = Radix 첫 자식**: Radix 가 래퍼 구조를 바꾸면 성장을 못 본다(뷰포트 RO 는 계속 돈다). 증상 = 스트리밍이 바닥에 안 붙음 → 훅 시험이 첫 자식 부재를 경고로 남긴다. 대안(내용 ref 를 내려보내기)은 두 슬롯 수정이 필요하다.
- **F4 링 압박**: 토큰 단위 `TextDelta` 로 링(`REPLAY_MAX_EVENTS = 4096` — `output_core.rs:1291`)이 더 빨리 찬다 → 긴 대화의 앞 이력이 replay 에서 더 일찍 밀린다. ★수치 출구 문턱★: B1 채취(§5-4 링 압박 기록)의 긴 답(≥ 2,000 자) 한 턴의 흘린 델타 수가 **`REPLAY_MAX_EVENTS` 의 10 %(≈ 410)를 넘으면** B1 안에서 합치기를 짓는다 — 한 `decode()` 호출이 돌려주는 사건 목록 안에서 이웃한 같은 `message_id` 의 `TextDelta` 를 하나로 잇는다(backend 만 · seq 는 emit 때 매겨지므로 구멍이 안 생긴다 · 시험 14). 넘지 않으면 합치기는 후속으로 남긴다(조사 §4-4). 판정 수치는 B1 반환에 싣는다.
- **F4 렌더 비용**: 델타마다 `setItems`(`RichSlot.tsx:215`) + F1 RO. codex 가 오늘 같은 경로를 이미 탄다.
- **F4 늦은 완결 줄 = 두 벌**(B1 리뷰): 완결 `assistant` 줄이 그 블록의 `content_block_stop` 뒤에 오면(벤더 계약 위반 — §5-2) 글이 두 벌 나온다. 의도된 동작이다(「잃기보다 겹친다」 — 완결 줄에 블록 번호가 없고, 꼬리 비교는 ADR-0240 이 거부했다). 시험 `a_text_block_that_was_not_streamed_falls_back_to_the_completed_text`(멈춘 뒤 완결 갈래)가 못 박는다.
- **F4 흘린 글은 되돌릴 수 없다**(B1 리뷰): CLI 가 실패한 스트림을 다시 시도하거나 스트리밍 아닌 요청으로 물러나면 실패한 시도가 흘린 앞부분이 화면에 남는다. 벤더 동작 미확인.
- **F4 compact 미채취**(B1 리뷰): `/compact` · 자동 compact 는 플래그를 켠 채로 채취하지 않았다 — compact 가 최상위 `stream_event` 를 내면 요약이 채팅에 흘러들 수 있다. 미확인 — GUI QA(§8-2)에서 확인한다.
- **F2 턴 열림 전 끊기**(③ · S4): 턴이 열리기 전(보낸 직후 · S4 결과에 따라 `init` 전까지)의 Esc 는 CONFLICT 로 거절되는 무동작이다 — 다시 누르면 된다.
- **F2 잔여 경합**(§3-4 · S7): 턴 끝 직후의 늦은 Esc · 버스 호출이 CLI 가 스스로 연 다음 턴을 끊을 수 있다. 벤더 줄에 턴 id 가 없어 닫을 수 없다.
- **F2 받아들여졌으나 듣지 않은 끊기**(7판 · ADR-0244 · 코드 리뷰): `requested` 는 「줄에 실었다」이다. 받아들여진 끊기가 실제로 듣지 않으면(예: codex 가 `turn/interrupt` 를 큐에 넣고 상대의 답 없이 `Ok` 를 돌려준다) 턴이 스스로 끝날 때까지 그 창의 Esc 와 창 명령은 무동작이다 — 시한은 두지 않는다(의도). 빠져나가는 길 = 그 뷰를 닫는다(마지막 뷰 unmount 로 걷힌다) · 버스 직접 호출(`engram agent.interrupt …`).
- **F2 새로 붙은 뷰의 이른 걷힘**(7판 · ADR-0244 · 받아들임): 새로 붙은 뷰의 첫 따라잡기가 앞 턴의 끝에서 끝나면(다른 뷰가 막 보냈고 새 턴의 프레임이 아직 없다) 「열린 턴 없음」으로 일찍 걷힌다 — 오늘 동작으로 돌아갈 뿐이다(Esc 가 다시 보낼 수 있다). 창이 짧아 받아들인다.
- **F2 버스 직접 끊기엔 「끊는 중」이 없다**(7판 · ADR-0244): 데몬 버스로 직접 보낸 끊기(`engram agent.interrupt …`)는 창 명령을 거치지 않아 표시도 Esc 무시도 없다 · 그 에이전트의 채팅 뷰가 없는 창에서 부른 창 명령도 상태를 세우지 않는다 — 그 경로들에는 멈추는 중의 둘째 끊기 위험(불확실 · 미검 — §11 ⑬)이 그대로 남는다. 한 번 누름의 잔여 경합(바로 위)도 7판에서 그대로다.
- **F2 터미널 모드는 끊기 명령이 없다**(7판 · ADR-0245): PTY 에이전트에는 버스 `agent.interrupt` 가 CONFLICT · WS `Interrupt` 가 오류로 답한다 — LLM · CLI 는 이 명령으로 터미널 모드 에이전트를 끊을 수 없다. 사람은 터미널 칸에서 Esc · Ctrl-C 를 직접 친다. ★LLM 갭(사용자 결정 ⑭ 로 수락)★: 버스에는 날 입력 명령도 없어 LLM 이 터미널 모드 에이전트를 끊을 길이 없다 — CLAUDE.md 「LLM-우선 제어」의 갭이다(CLAUDE.md 는 고치지 않았다 · ADR-0245 영향). 되살리지 말 것 — 사용자 결정 ⑭(「터미널모드는 지가 알아서 하잖아」)가 뺐고, 날 Ctrl-C 는 턴이 돌든 말든 나가 TUI 는 짧은 창 안의 두 번째 Ctrl-C 에 끝난다(codex = 소스 판독 — 조사 `docs/research/interrupt-feedback-peers-2026-09-28.md` codex TUI 행 · 가능성 높음 · claude = 알려진 동작 · 여기서 미측정 · 가능성 높음).
- **F2 codex 끊기(`turn/interrupt`)는 이미 도는 명령을 멈추지 않는다**(벤더 동작 · 5판 · 태그 판독 · 가능성 높음 — §4-7 사실 「끊긴 명령은 계속 돌다 늦게 끝난다」): Esc 는 턴을 끊지만 unified-exec 로 이미 도는 명령 프로세스는 살아서 끝까지 돌고, 그 끝은 다음 턴 도중에 올 수 있다. 끝내는 길은 벤더에 있다 — 실험 API `thread/backgroundTerminals/terminate` · `clean` · `list`(`app-server-protocol/src/protocol/common.rs:745-762@v0.156.1` — `#[experimental]` · 5판 light 재검 편집에서 다시 열어 확인 · 인자 세부는 판독하지 않았다). 이 라운드의 범위 밖이다 — 사용자 결정 ⓐ(§11 ⑩)가 ⓒ(끊을 때 끝낸다)를 거부했고 후속 후보로만 남는다(아직 추적 항목 없음). 5판의 「우리 쪽 끊기 경로로는 고칠 수 없다」는 과했다 — `turn/interrupt` 로는 못 멈출 뿐이다.
- **대조 — Claude Code 는 Esc 에 도는 Bash 도구를 죽인다**(실측 2026-09-27 · 확실): 90 초 동안 줄을 찍는 명령이 10 초째 Esc 를 누른 순간 쓰기를 멈췄고 뒤에 그 프로세스가 없었다 · 화면은 곧바로 「Interrupted」. ★범위 = 대화형 Claude Code CLI 의 Bash 도구다 — 우리 stream-json `control_request` 끊기가 도는 Bash 도구를 죽이는지는 미검★(B2 스파이크 S1 이 기록한다 — §3-5).
- **F2 능력 표시**: claude JSON 의 `control.interrupt` 가 참이 되면 트리 `canInterrupt`(`mergeTreeNodes.ts:94`)도 참 — 오늘 소비자가 없다.
- **F3 펼침 상태는 인메모리**(새로고침에 초기화).
- **F3 판차**(U2 · §4-7 ⑤): 새 데몬 + 옛 셸 = codex 실패·거부 호출마다 「표시할 수 없는 신호」 줄 · 옛 데몬 + 새 셸 = codex 오류 표시 없음. 한 판 안에서 셸을 가를 값이 없어 거르지 못한다 — `TurnEnd` 와 같은 수용이다.
- **F3 codex 판정 = 벤더 `status`**: 0 아닌 종료를 벤더가 무엇으로 적는지 미검(B5 채취가 확인 — `completed` 로 오면 멈춘다) · 「못 찾음」을 종료 1 로 알리는 검색 명령이 벤더가 `failed` 로 적으면 「오류」로 보인다.
- **F3 늦은 끝**: 끊긴 호출의 끝이 다음 턴 뒤에 오면(실측 `steer_m6`) 지난 턴의 행에 배지가 붙는다(오늘 설계). 5판 태그 판독(가능성 높음): 끊긴 명령은 실제 `exitCode` 로 닫히므로 0 아닌 종료면 그 끝은 `failed` 다 — 끊긴 턴 묶음에 붉은 「오류」가 서고 그대로 둔다(사용자 결정 ⓐ 2026-09-27 · 바로 아래 「중단됨」 행이 맥락을 준다 · §11 ⑩ · §4-7 ⑨ 채취 둘째). 그 프레임이 「보낸 직후 · 첫 응답 전」 창에 들어도 대기 표시는 꺼지지 않는다 — `ToolResult` 갈래가 `feed` 에서 `false` 를 돌려줘 `RichSlot.tsx:222` 가 대기를 풀지 않는다(§4-7 ⑧ · 3판 리뷰 Designer FIX 로 수용하던 틈을 닫았다). ★같은 부류가 `Usage` 에는 남는다★ — 이번 범위가 아니고 Wait 재설계(`docs/tracking.md` T-12)의 몫이다.
- **F3 거부 귀속은 업스트림 매핑에 기댄다**(U2-a · §4-7 ②-2): 「우리 거절 → 같은 item id 의 끝이 곧바로(명령 = `failed` · 파일 변경 = `declined`)」는 업스트림 소스 판독이다(main `41f9084` · 5판 태그 `rust-v0.156.1` — 설치본과 같은 판의 소스지만 실행 확인이 아니다 · 가능성 높음). 설치본의 실제 동작은 B5 채취 셋째가 확인한다. 벤더가 바꾸면 — 끝을 안 내거나 다른 id 로 내면 기억은 상한에서 밀려나고 표시는 없음 · 명령은 「오류」 · 파일 변경은 「거부됨 · 실행되지 않음」 — **경보 없이 조용히 틀린다**(status 가 `failed`↔`declined` 로 바뀌는 것은 둘 다 `Refused` 로 바꿔 쓰므로 무해하다). 설치본(0.156.1)의 이 표류는 B5 채취 셋째의 멈춤이 명령 · 파일 변경 둘 다 잡는다(§4-7 ⑨ — 5판 light 재검) · 그 뒤의 벤더 판 변경에는 여전히 경보가 없다.
- **F3 거부 귀속의 알려진 한계 넷**(§12 4판 2 · 3 = 사용자 수락 · 바른 해법 = `docs/tracking.md` T-38 — U7 · §4-7 ②-2): ① 이어받은 이력에 우리가 거절한 명령 행이 없다(「오류」도 「거부됨」도 아니다 — 5판 정정) ② 상한 16 에서 밀려난 명령의 늦은 끝은 「오류」다 ③ 이력의 파일 변경 거절은 「거부됨 · 실행되지 않음」으로 보인다 ④ 상한 16 에서 밀려난 파일 변경 거절도 같다(5판 light 재검에 더했다). ③④ 는 우리 사유를 잃지만 거짓은 아니다 — 5판 첫 문구 「codex 가 실행을 거부함」에서는 거짓이었다.
- **F3 기억에 없는 거부의 사유는 이유를 말하지 않는다**: 4판의 거짓 사유(모든 `Declined` 에 우리 사유)는 5판에 결말을 둘로 나눠 닫았고, `Declined` 사유는 5판 light 재검에 「실행되지 않음」이 됐다(사용자 결정). 그 문구는 기억에 없는 `declined` 의 출처 전부 — codex 스스로의 거부(guardian · 네트워크 정책) · 벤더 준비 실패(벤더 주석이 일부 실행 준비 실패도 Declined 로 보고될 수 있다고 적는다 `core/src/tools/events.rs:445-452@v0.156.1`) · 복원 · 상한에서 밀려난 우리 파일 변경 거절(한계 ③④) — 에서 참이지만(가능성 높음) 왜 안 돌았는지는 알려 주지 않는다. 벤더 스스로의 거부가 우리 앱에서 얼마나 나오는지는 모른다 — 사용자 codex 설정에 달렸을 수 있다(불확실).
- **F3 claude 결과 파싱은 그대로**(§4-7 ④): 프론트가 벤더 `tool_result` 모양을 읽는 자리(조사 §3-1)는 이번에 걷지 않는다 — 후속(§11 ⑦).
- **F2 벤더 합성 줄 문구 표류(6판 · ADR-0243)**: 벤더가 `[Request interrupted by user` 문구를 바꾸면 알아보기가 빗나가 오늘의 말풍선 + 대비 중단 행으로 돌아간다(정보 손실 없음 · 잘못 알아본 줄도 원문 그대로라 거짓 뜻을 말하지 않는다) — 아래 F3 의 도구 이름 표류와 같은 부류다. ★알려진 한계(메인 · 수락)★: transcript 의 `user` 줄에는 `isReplay` 가 없어, 사용자가 그 접두로 시작하는 글을 쳤다면 다시 연 이력에서는 그 글이 끊김 표시 행으로 보인다(라이브에서는 `isReplay` 제외가 막는다 · ADR-0243 영향).
- **F3 벤더 도구 이름 표류**: claude 가 도구 이름을 바꾸면 `Other` 로 떨어진다(묶음은 그대로 선다 — 요약 문구만 「기타」).

---

## 10. ADR 후보 (번호 = `/adr` 가 채번 · 가안은 0237 부터 — `docs/decisions` 마지막 = 0236 확인)

| 가안 | 결정 | 거부한 대안(사용자·메인이 확정) |
|---|---|---|
| **0237** | 채팅 칸 Esc = 도는 턴 끊기 · 명령 `agent.interrupt`(창 + 버스) · 범위 = 칸 안 어디든(U6) · 조건은 지역 술어 하나(§3-2) — ★ADR-0235 결정 10(「UI 정지 버튼·단축키는 지금 넣지 않는다」)을 번복★(그 ADR 에 개정 도장) · 표시 세부 결정 — 턴 결말 「중단됨」 행 = 굵게 + 강조색 `var(--accent)`(U8 · 빨강 = 실패 · 주황 = 거부와 갈린다 · 아이콘 · 문구 그대로) | 전역 단축키 표(가드를 연다) · **단축키 시스템을 지금 만든다**(보류 — `docs/tracking.md` T-36 · Esc 는 지금 명령 `agent.interrupt` 를 부르는 지역 술어이고 나중에 키바인딩 표 한 줄로 옮긴다) · 입력창만(U6 — 사용자가 칸 안 어디든을 골랐다) · 두 번 Esc · 중단 행을 `text-muted` 그대로(U8 — 사용자: 「너무 눈에 띄지 않음?」) |
| **0238** | claude JSON 끊기 = 통로 주입 제어 줄(턴 열림 문 — 턴 밖이면 `Unsupported`) · 끊긴 `result` → `TurnEnd{Interrupted}`(오류 아님 · 멈춤 불변) · 대기분은 다음 턴(`cancel_queued` 안 씀) · 잔여 경합은 벤더 한계로 문서화 | 세션 입력 자물쇠 경로(①) · `MessageDone` 유지(②) · 턴 열림 전 끊기를 미루는 큐(③) · 대기분 취소 |
| **0239** | `ToolCall.category` 중립 선 칸 — 번역기가 정하고 프론트는 모르면 「기타」 | 프론트 이름 표(벤더 지식 누수) · 백엔드가 묶음을 만든다(표시 관심사를 선에) |
| **0240** | claude 부분 메시지 중복 제거 = 열린 블록이 흘렸으면 완결 글을 버린다 · 턴 끝은 `result` 그대로 | 교체(새 선 변형 + replay 교체) · 꼬리 비교(paseo — 중복 모서리) · 무조건 합치기(§9 문턱을 넘을 때만 짓는다) |
| **0241** | 도구 끝 = 새 선 변형 `ToolResult{id, outcome}` — codex 가 실패·거부만 낸다 · 판정은 벤더 `status` · 두 턴 분류기 `None` · `PROTOCOL_VERSION` 안 올림(판차 수용) · claude 는 이번에 안 바꾼다(U2 · §4-7) · **거부 = 따로 표기 · 결말 둘**(U2-a · 5판 — `Refused` = 우리 거절 · 사유 「대시보드가 승인 요청을 처리하지 않아 실행되지 않음」 / `Declined` = 기억에 없는 거부(codex 스스로의 거부 등) · 사유 「실행되지 않음」(사용자 결정 2026-09-27) — 같은 주황 배지 · 둘 다 「거부 N」 · 오류 아님) · **우리 거절 귀속** = codex 통로가 거절한 승인 요청의 item id 를 기억해 그 끝(명령 `failed` · 파일 변경 `declined`)을 `Refused` 로(리더 전용 칸 · 상한 16 · 같은 id 는 한 칸 · 끝 줄이 비움 · 늘 짓는다 · §4-7 ②-2) · **알려진 한계 넷 수락**(이력의 거절된 명령 행 없음 · 상한 밀림의 명령 = 「오류」 · 이력 · 상한 밀림의 파일 변경 = 「거부됨 · 실행되지 않음」 — 바른 해법 = 채팅 승인 T-38 · U7) · **끊긴 뒤 늦게 `failed` 로 닫히는 명령 = 그대로 붉은 「오류」**(사용자 결정 ⓐ 2026-09-27 · 바로 아래 「중단됨」 행이 맥락 · 새 장치 없음 · 피어 = codex TUI · t3code · paseo 모두 끊을 때 명령을 끝내지 않는다 — §4-7 ⑨ 채취 둘째 · §11 ⑩) · **claude 끊긴 도구(`tool_result` `is_error:true`) = 같은 규칙**(붉은 `Error` 배지 · 머리 「오류 1」 · 아래 강조된 「중단됨」 행(6판: claude 는 끊김 표시 행 — ADR-0243) — 메인 결정 · 사용자 위임 2026-09-27 · 두 백엔드 한 규칙 · 새 장치 없음 · §4-7 ④ · §11 ⑪) | `ToolCall` 선택 status + id 병합(끝이 진행 신호가 됨 · 옛 셸 중복 행) · `Structured` 탈출구(claude 모양 흉내 포함) · 판을 올려 거르기 · 모든 끝을 냄 · claude 도 상태만 냄 · `exitCode` 로 판정 · **거부를 오류로 함께 센다**(A — 사용자: 권한에 막힌 것과 진짜 실패가 같아 보이고, 빨강은 없는 버그를 찾게 한다 · 조사 §8-3) · **`Declined` 하나에 사유 하나**(4판 리뷰 — codex 스스로의 거부에 우리 사유가 붙는 거짓 사유 결함) · **`Declined` 사유 「codex 가 실행을 거부함」**(사용자 2026-09-27 · 5판 light 재검 — 프론트 문자열은 백엔드 이름 없는 중립 낱말이어야 한다 `structuredAccumulator.ts:25` · §6 ADR-0004 행 · 문서화된 경로에서 거짓이다: 복원된 우리 파일 변경 거절 · 상한에서 밀려난 우리 파일 변경 거절 · 벤더 준비 실패의 Declined 보고 `core/src/tools/events.rs:445-452@v0.156.1` · claude `ToolResult`(T-37)가 오면 codex 이름을 물려받는다) · **채팅 승인을 이번에 짓는다**(사용자 U7 — 「다음 과제로 빼자. 지금 많이 엮여있어」 · T-38) · **item 모양(`failed` + `exitCode:null` + `aggregatedOutput:null`)으로 거부 판정**(메인 5판 · light 재검 보정 — 같은 모양을 내는 다른 길(승인 콜백 버려짐 · guardian 시한 초과 — §4-7 ②-2)을 우리 거절로 읽어 거짓 사유를 붙인다. 거절된 명령은 이어받은 이력에 없어 복원을 돕지도 못한다 — 얻는 것은 라이브에서 상한 16 에 밀려난 명령 하나뿐이다) · **출처를 이력 복원까지 보존**(4판 리뷰 Designer — 사용자가 알려진 한계로 수락 · 바른 해법은 T-38) · 설치본이 `declined` 를 보내면 귀속을 안 짓는 스위치(5판 — 결말 분리 뒤엔 옳은 사유를 고르려면 늘 든다) · ([고름] — 메인 수락 §11 ⑧) 집합을 통로 `State` 에 · 번역기에 공유 집합 주입 · 턴 끝에서 비움 · **끊긴 턴의 늦은 결과를 숨긴다**(ⓑ-1 — 진짜 실패가 안 보이고 새 로직이 든다) · **늦은 실패를 그 행의 「중단됨」으로 보인다**(ⓑ-2 — 규칙이 든다(사용자: 별도 장치 없이) · 이력 페이지는 턴 결말을 싣지 않아(`backend/codex/decoder.rs:1179-1183` · ADR-0203) 다시 열면 적용이 안 된다) · **끊는 순간 아직 도는 행을 모두 「중단됨」으로**(ⓑ-3 — 번역기가 성공엔 아무것도 안 내(§4-7 ③) 프론트가 성공으로 끝난 codex 행과 아직 도는 행을 못 가른다 · 새 성공 신호나 기억이 든다) · **끊을 때 명령을 끝낸다**(ⓒ — 실험 API `thread/backgroundTerminals/*` · 끊을 때 쓰는 피어가 없다 · 결과 표시에 결정이 따로 든다 · 후속 후보 · 아직 추적 안 함) · **claude 끊김 `tool_result` 를 본문 글로 판별**(벤더 문자열에 기대 깨지기 쉽다 · §11 ⑪) · **끊긴 claude 턴 안의 `is_error` 를 「중단됨」으로 바꿔 단다**(ⓑ-2 와 같은 규칙 장치 — 사용자가 codex 에서 거부 · §11 ⑪) |
| (선택) **0242** | 스크롤 따라가기 = 직접 쓴 순수 코어 · 성장 뒤 다시 재지 않는다 | `use-stick-to-bottom`(Radix 휠 불일치 의심 · 전역 청취자) · CSS 만 · 가상화 |
| **0243**(6판 · 채번 완료) | claude 끊김 합성 줄 = 말풍선이 아니라 원문을 싣는 끊김 표시 행(라이브 · 다시 연 이력) · 그 표시가 있는 턴엔 「응답이 중단됐습니다」 행을 그리지 않는다(claude 한정 · codex 무변경) · 턴 끝 분류(0238) 그대로(사용자 ⑫ 2026-09-28) · 알아보기 = 번역기의 벤더 문자열 접두 → `Structured{kind:"interrupted"}` · `classify_turn` `None` · 프론트는 표시 행 유무로 가른다(메인 · 위임 — §3-4 · §7 B3 · FE-2c) · ADR-0237 결정 6 · ADR-0238 결정 4 · 7 증상 · 영향 · ADR-0241 결정 12 에 부분 개정 도장 | 번역기에서 숨긴다(메인의 앞선 추천 — 이력에 표지가 안 남는다) · 라이브만 숨기고 이력에서만 중단 행 · 말풍선 그대로(사용자) · 위치로 알아본다(이력에 `result` 가 없다) · 프론트가 에이전트 종류로 가른다(backend 지식 누수 · 대비를 잃는다) · 새 선 변형(0241 선례 — `Structured` 의 새 `kind` 는 wire 타입 · 생성물 · 판차 처리가 필요 없다)(메인) |
| **0244**(7판 · 채번 완료) | Esc 끊기를 누르면 곧바로 「중단하는 중…」(꼬리 Wait 행 · 정지 아이콘) · 멈출 때까지 Esc 무시(사용자 ⑬ 2026-09-28 — A · paseo 모양) · 백엔드 무변경 · 에이전트별 「끊는 중」 상태 = 창 명령이 세우고 턴 끝 · 거절에 걷는다 · 표시는 「멈춰 달라고 했다」뿐이고 멈춤은 턴 끝 사건이 알린다(메인 · 위임 — §3-2 · §7 FE-1.4) · 끊는 중의 둘째 호출은 다시 보내지 않는다(메인) · ADR-0237 결정 1 · 4 에 부분 개정 도장 | B 표시 없이 연타만 무시 · C 그대로 둔다(사용자 — 사유는 선택과 피어 조사뿐) · 백엔드 턴마다 끊기 한 번(`TurnGate` 의 `sent` 표식 — 표시(사용자 A)에 프론트 상태가 어차피 필요하고 그것이 **뷰가 있는 창에서만** 창 경로 연타를 막는다 — 그 한정을 붙여도 사유는 선다(메인) · 버스 직접 경로 · 뷰 없는 창의 둘째 끊기는 §9 알려진 한계로 받는다 · 실측으로 위험이 서면 다시 연다)(메인) |
| **0245**(7판 · 채번 완료) | 터미널 모드(PTY)에서 Ctrl-C 주입을 뺀다 · Esc · Ctrl-C 는 사람이 칠 때 터미널이 알아서 처리한다(사용자 ⑭ 2026-09-28) · 대응 = `interrupt()` `Unsupported`(버스 CONFLICT · WS `Interrupt` 오류) · 능력 `control.interrupt = false`(메인) · LLM 이 터미널 모드를 끊을 길이 없다 = 「LLM-우선 제어」 갭 · 사용자 수락(§9) · ADR-0017 결정 5 의 interrupt(Ctrl-C) 부분 · ADR-0030 의 PTY 능력 목록에 부분 개정 도장 | Ctrl-C 를 두고 끝날 위험을 문서화(사용자 — 빼기를 골랐다) · Ctrl-C 대신 Esc(TUI 의 Esc 뜻이 상태마다 갈리고 명령은 그 상태를 모른다 · 사람이 이미 키를 직접 보낸다)(메인) |

- **0241 을 0239 에 넣지 않고 새 번호로 두는 이유**: 거부한 대안이 서로 다른 축이다(0239 = 종류를 누가 판별하나 · 0241 = 끝 결과의 선 모양과 판차). 한 ADR 에 섞으면 한쪽을 번복할 때 다른 쪽까지 폐기 도장이 번지고, 다음 세션이 어느 대안이 어느 결정의 것인지 못 가른다.
- 함께 고칠 문서: CLAUDE.md 「LLM-우선 제어」 남은 갭에 「LLM 이 터미널 모드 에이전트를 끊을 길이 없다(ADR-0245 · 사용자 수락)」 — 착지 라운드의 `/review doc` 몫(지금은 CLAUDE.md 를 고치지 않는다 · 7판) · `types.rs:70` · `types.rs:94-95`(`QueuedInput` doc — `Structured` 통째 진행 전제 · 6판 · B3) · `classify_turn` doc(`kind:"interrupted"` 예외 · 6판 · B3) · `claude/mod.rs:551` · `:762-767` 주석 · `codex/decoder.rs:262-263` · `:523-525` · `:623` 앞 주석(끝에서 결과를 낸다 — B5) · `codex/transport.rs:3052-3053`(`refuse` doc — 거절한 승인 item 을 기억한다 · B5) · CLAUDE.md 「핵심 불변식」(claude 끝 어휘에 끊김 `TurnEnd` 한 갈래 · 「대기 입력 상태」 끝 문단의 「턴 끝 뒤에 오는 사건」 예에 `ToolResult` — 선택) — 착지 라운드에서 `/review doc`.

---

## 11. ★메인 확인 필요★ 모음 · 판정(2판 ①–⑤ · 3판 ⑥⑦ · 4판 ⑧⑨ · 5판 ⑩⑪ · 6판 ⑫ · 7판 ⑬⑭)

- **3판 ★사용자 확인★(U2 — `declined` 를 어떻게 보이나) = 닫힘**: 사용자 결정 2026-09-27 = **따로 표기(B)** — 주황 「거부됨」 · 줄 안 한 줄 사유 · 요약 「거부 N」(오류와 따로). 반영 = §1 U2-a · §4-7 ②-2(귀속) · ⑧(모양) · ⑨(시험 · 채취 셋째) · §7 B5 · FE-1.0 · §8-2 F3 ④ · §10 0241.
- **4판 리뷰 갈림 2 건(§12 4판 2 · 3) = 닫힘**: 사용자 결정 2026-09-27 = **알려진 한계로 수락**(메인 추천 — U7 「채팅 승인은 다음 과제」와 한 묶음 · 귀속 장치를 더 키우지 않는다). 바른 해법 = `docs/tracking.md` T-38(대시보드가 자동 거절을 그만둔다). 반영 = §1 U7 · §4-7 ②-2 한계 · §9 · §10 0241 · 아래 ⑨(판정의 틀도 거기 적는다).

| # | 무엇 | 왜 올리나 | 메인 판정 |
|---|---|---|---|
| ① | claude 끊기 줄을 `MidTurnPolicy`·세션 입력 자물쇠가 아니라 **통로 주입**(`StdioTransport::with_interrupt`)으로 보낸다(§3-4) | 지시는 `cancel_async_message` 경로를 따르라였다. 그 경로로 가면 능력(`control.interrupt`)이 통로 caps 에서 오는 구조와 어긋나고, 입력 id 가 없어 자물쇠가 지킬 순서도 없다 | **수락** — 리뷰어 확인: ADR-0004 격리 유지 · 새 락 간선 없음 · 입력 임대 검문은 이미 덮인다 |
| ② | 끊긴 claude 턴을 `MessageDone` 대신 `TurnEnd{Interrupted}` 로 닫는다 · 오늘의 `subtype:"interrupted"` 도 같이(§3-4) | `types.rs:70` · `claude/mod.rs:551` 주석과 부딪히고, 오늘 구분선만 그리던 경우에 중단 행이 붙는다. 이득 = codex 와 같은 중단 표시 · 오류 뒤 멈춤 불변 | **수락** — 리뷰어 확인: 턴 관측 `Ended(Other)` · `last_end_failed` 불변 · 초인종 울림 · 누산기가 이미 결말 행을 그림 · 대기 입력 골든 무영향 |
| ③ | S4 에서 init 전 끊기가 무동작이면: 받아들인다(문서화) vs init 을 볼 때까지 끊기를 미룬다(§3-5) | 미루기는 번역기↔통로 사이에 새 공유 상태가 필요하다 | **해소** — 턴 열림 문(§3-4): 턴이 열리기 전의 Esc 는 `Unsupported`(무동작 · 정직한 답) · 사용자가 다시 누른다 · 미루는 큐 없음. S4 가 되울림 뒤 · `init` 전 틈을 보이면 문이 여는 줄만 늦춘다 |
| ④ | S2 에서 `subtype`·`terminal_reason` 이 끊김을 못 가르면 「우리가 보냈다」 표식으로 가른다(§3-4) | 끊기와 겹친 진짜 오류를 끊김으로 접어 가릴 수 있다 | **예비로만 유지** — 스파이크가 두 칸 모두 끊김을 못 가를 때만 짓는다 · 문이 열려 있을 때만 세우고 그 턴의 `result` 에서 지운다 → 한가할 때의 호출이 다음 턴의 진짜 오류를 가리지 못한다 |
| ⑤ | 사람이 마지막이 아닌 도구 묶음을 펼치면 따라가기를 푼다(§4-5) | 지시(「RO 가 처리한다」)에 더한 것 — 빼면 붙은 채 펼친 묶음 머리가 화면 위로 밀려난다 | **수락** |
| ⑥ | B5 는 FE-2 의 U2 가드 커밋 뒤에 **착수**(지시)한다(§7) | 새 변형의 생성물이 프론트 `never` 망라(`structuredAccumulator.ts:243`)를 걸어, 가드 없이 들어가면 그때부터 트리의 `tsc` 가 빨갛다 · ★생성물은 B5 의 커밋이 아니라 첫 protocol 시험 실행에서 공유 트리에 써진다(`protocol/tests/ts_export.rs:9-20`) — 그래서 커밋 조건이 아니라 착수 조건이다(3판 리뷰 Architect low)★ — 두 워커 사이에 순서가 하나 생긴다. 대안 = B5 가 누산기 한 갈래를 함께 친다(FE-2 파일과 겹친다 — 기각) | 대기 |
| ⑦ | claude 결과 중립화(본문 포함 `ToolResult` · 프론트 파싱을 옛 데몬 폴백으로)를 후속 추적 항목으로 적는다(§4-7 ④) | 이 문서는 `docs/tracking.md` 를 안 건드린다 — 적립은 메인 몫 | **닫힘** — `docs/tracking.md` T-37 로 적립됐다(`a0c5674`) |
| ⑧ | 거부 귀속 집합을 통로 `State` 가 아니라 **리더(`Reader`) 전용 칸**에 둔다(§4-7 ②-2 · §6) | 지시는 「`State` 에 칸이 는다」를 예상했다. 쓰는 쪽(거절 — `transport.rs:3629`)과 읽는 쪽(같은 `handle_line` 의 끝 줄)이 둘 다 리더 스레드라 락이 지킬 것이 없다. `State` 에 두면 락 구간과 「소유권 분할」 표의 칸만 는다. 대안 = `State` 칸(`refuse` 가 이미 잡는 락 구간에서 적고, 끝 줄에서 한 번 더 잡는다 — 옳지만 얻는 것이 없다). 함께 고른 것: 턴 끝에서 비우지 않는다(늦은 끝 — `steer_m6` 50 줄) · 상한 16 · 번역기는 집합을 모른다 | **수락**(메인 2026-09-27) — 5판이 같은 id 는 한 칸을 더했다(4판 리뷰 low) |
| ⑨ | 재활성화 뒤 이력(`ItemOrigin::History`)의 우리 거절 — 4판은 「오류」로 보인다고 적었다. ★5판 정정(태그 판독 · 가능성 높음)★: 명령 행은 이력에 **없고**, 파일 변경은 「거부됨 · 실행되지 않음」으로 보인다(우리 사유를 잃는다 · 거짓은 아니다 — 문구 = 5판 light 재검 사용자 결정 · §4-7 사실 · ②-2 한계) | 사용자 눈에 보이는 불일치다(라이브 때 「거부됨」이던 호출이 사라지거나 사유가 바뀐다). 집합은 화신마다 비고, 벤더 item 에 사유 칸이 없다. 4판 리뷰가 갈렸다(§12 4판 2 — Designer = 출처를 이력 복원까지 보존 · Architect = 알려진 한계로 수용 가능) | **닫힘 — 알려진 한계로 수락**(사용자 2026-09-27 · 메인 추천 · U7 과 한 묶음). ★판정의 틀★: 사용자는 4판의 틀(「오류」로 바뀐다)에서 판정했다. 판정 직후 메인이 정정한 사실(우리가 거절한 명령 행은 이력에 없어 다시 열면 사라진다 · 거절된 파일 변경은 `declined` 로 닫힌다)을 사용자에게 알렸고, 사용자는 이의를 달지 않았고 이어서 `Declined` 사유 문구를 정했다(§1 U2-a — 「실행되지 않음」). 바른 해법 = `docs/tracking.md` T-38. 모양 판정 대안(`exitCode:null`·`aggregatedOutput:null`)은 기각 — 같은 모양의 다른 길을 오판한다 · 이력엔 그 행이 없다(§10 0241 · 메인 · 5판) |
| ⑩ | B5 채취 둘째 — 끊긴 뒤 늦게 `failed` 로 닫히는 명령을 어떻게 보이나(§4-7 ⑨ 채취 둘째 · 5판 light 재검) | 5판 태그 판독상 걸릴 가능성이 높다 — 끊어도 명령 프로세스는 살아 있다가 실제 `exitCode` 로 닫힌다(§4-7 사실 · §9 F2). 오늘 설계대로면 끊긴 턴의 「중단됨」 행 옆 묶음 머리에 붉은 「오류 1」 이 선다 — 「끊기는 실패가 아니다」 규칙(`structuredAccumulator.ts:25-27`)과 부딪히는 사용자 눈에 보이는 판정이다. 선택지(메인이 피어 조사 뒤 올린다 — 이 문서는 고르지 않는다): ⓐ 그 행에 붉은 「오류」 배지(오늘 설계) ⓑ-1 끊긴 턴의 늦은 결과를 숨긴다 ⓑ-2 늦은 실패를 그 행의 「중단됨」으로 보인다 ⓑ-3 끊는 순간 아직 도는 행을 모두 「중단됨」으로(★계기가 다르다★ — 늦은 끝이 아니라 끊는 순간에 단다) ⓒ 끊을 때 도는 명령 프로세스를 끝낸다(벤더 실험 API `thread/backgroundTerminals/terminate` — §9) | **닫힘 — 사용자 결정 2026-09-27 = ⓐ 그대로**(사용자 — 「일단 밑에 중단됨 줄이 있는데 뭐가 문제라는거야?」 · 바로 아래 「중단됨」 행이 맥락 · 명령은 실제로 실패했다 · 새 장치 없음). ⓑ-1 · ⓑ-2 · ⓑ-3 · ⓒ 를 거부한 사유 = §10 0241. 채취 둘째는 확인으로 남는다(멈춤 아님 — §4-7 ⑨) |
| ⑪ | claude 의 Esc 로 끊긴 도구를 어떻게 보이나(5판 좁은 재검) | 대화형 Claude Code CLI 는 Esc 에 도는 Bash 도구를 죽이고(실측 2026-09-27 — §9), 이 세션의 기록에서 `[Request interrupted by user for tool use]` 바로 앞 `tool_result` 가 `is_error:true`(「The user doesn't want to proceed with this tool use…」)다 · 우리 stream-json 경로는 미검(같을 가능성 높음 — §3-5 S1 이 기록). 그러면 그 행은 오늘 붉은 `Error` 배지(§4-7 ⑧ `isErr = result?.isError === true`)가 서고 묶음 머리가 「오류 1」 을 센다 | **닫힘 — 메인 결정(사용자 위임 2026-09-27)** · ⓐ(§11 ⑩)와 같은 규칙: 그대로 보인다 — 그 행의 붉은 「오류」 · 머리 「오류 1」 · 바로 아래 강조된 「중단됨」 행(U8). 근거 = 사용자의 ⓐ 사유(「일단 밑에 중단됨 줄이 있는데 뭐가 문제라는거야?」)를 그대로 적용 · 표시 선택은 사용자가 위임했다(「너가 생각하는 방향으로 하고 나중에 보고 개선하면 되지」) · 두 백엔드가 한 규칙 · 새 장치 없음. 거부한 대안 = claude 끊김 `tool_result` 를 본문 글로 판별(벤더 문자열 — 깨지기 쉽다) · 끊긴 턴 안의 `is_error` 를 「중단됨」으로 바꿔 단다(사용자가 codex ⓑ-2 로 거부한 규칙 장치와 같다) — §10 0241 · 반영 = §3-5 · §4-7 ④ · ⑨ · §8-2 F2 ② · ★6판 덧붙임★: claude 에서 「강조된 「중단됨」 행」 자리는 이제 합성 줄의 끊김 표시 행이 채운다(⑫ · ADR-0243 — 그 줄이 안 오면 중단 행이 그대로 선다) · 이 판정의 나머지는 그대로다 |
| ⑫ | claude 끊김 합성 사용자 줄(`[Request interrupted by user]` · `…for tool use]`)을 어떻게 보이나(6판 · B2 멈춤 사유 — 조사 `docs/research/claude-interrupt-spike-2026-09-28.md` §10-1) | 오늘 경로로는 라이브에서도 이어받은 이력에서도 사용자 말풍선이 된다 — §3-4 「프론트 무변경」 · U8 의 화면이 말풍선 하나만큼 달라진다. 사용자 체감이라 사용자 판정이 필요했다 | **닫힘 — 사용자 결정 2026-09-28**(Claude Code 의 「Interrupted · What should Claude do instead?」 화면을 가리킨 뒤): 「이게 정석이긴한데」 · 「어쨋든 그냥 표기하는게좋을것같음. 물론 말풍선은 제거하고, 그리고 오히려 응답이 중단되었습니다.를 빼는게 맞는것같은데? Claude에 한해서」 → claude 한정 · 말풍선 대신 원문을 싣는 끊김 표시 행(라이브 · 이력) · 그 턴의 「응답이 중단됐습니다」 행은 안 그린다 · 턴 끝 분류는 그대로. 알아보기 · 분류 · 누산기 규칙 · 행 모양 = 메인 결정(사용자 위임 「너가 생각하는 방향으로 하고 나중에 보고 개선하면 되지」 · 뒤집을 수 있다). 거부한 대안 = §10 0243 · 반영 = §3-4 · §7 B3 · FE-2c · 접점 표 · §8-2 F2 ② · ADR-0243 |
| ⑬ | Esc 끊기 뒤 멈출 때까지 화면이 그대로인 틈(B2 실측 9 ms – 1.0 s)의 연타를 어떻게 하나(7판 · B3 동시성 리뷰 — 실측 아님) | 그 틈의 둘째 Esc 도 끊기 줄로 나가고(claude 턴 열림 문이 아직 열려 있다), 조사 `docs/research/claude-interrupt-spike-2026-09-28.md` §5 의 관측(끊기는 그때 걸려 있던 일에 걸린다)대로면 대기 중인 다음 메시지 B 에 걸려 B 가 시작하자마자 끊길 수 있다(불확실 — 멈추는 중의 둘째 끊기는 시험한 적 없다). 화면 동작이라 사용자 판정이 필요했다 · 피어 조사 = `docs/research/interrupt-feedback-peers-2026-09-28.md` | **닫힘 — 사용자 결정 2026-09-28**(메인이 A 즉시 표시 + 무시 · B 표시 없이 연타만 무시 · C 그대로를 피어 조사와 함께 올린 뒤): 「ㅇㅇ 중단하는중 나오고 그동안 esc 무시.」 → A(paseo 모양). 백엔드 무변경 · 막는 자리 = 프론트 · 「끊는 중」 상태의 세움 · 걷음 · 표시 자리 = 메인 결정(위임 · 뒤집을 수 있다). 앞선 「턴마다 끊기 한 번(`TurnGate` 의 `sent`)」 제안은 대체됐다. 거부한 대안 = §10 0244 · 반영 = §3-1 · §3-2 · §3-6 · §7 FE-1.4 · §8-2 F2 ⑩ · §9 · ADR-0244 |
| ⑭ | 터미널 모드(PTY)의 끊기 — `PtyTransport::interrupt()` 가 날 `0x03`(Ctrl-C)을 쓰고 능력 참을 알리던 원래 설계(ADR-0017 결정 5)를 둘까(7판) | 버스 `agent.interrupt`(LLM · CLI)가 생기며 턴이 돌든 말든 Ctrl-C 가 나간다 · TUI 는 짧은 창 안의 두 번째 Ctrl-C 에 끝난다(codex = 소스 판독 — 조사 codex TUI 행 · 가능성 높음 · claude = 알려진 동작 · 여기서 미측정 · 가능성 높음) — 두 번 부르면 「≠ kill — 프로세스는 산다」 계약이 깨진다. PTY 끊기를 쓰는 UI 는 없다 | **닫힘 — 사용자 결정 2026-09-28**: 「터미널모드는 지가 알아서 하잖아. 아예 기능에서 빼야지」 · 「아니 내말은 ctrl c를 빼라는거지 터미널모드에서는 알아서 하라고해.」(메인이 A 「터미널 모드는 끊기 명령을 거절」 · B 「Ctrl-C 를 두고 위험을 문서화」를 올린 뒤 · 둘째 원문이 「기능에서 빼」를 Ctrl-C 주입으로 좁혔다 — 사람이 치는 키는 그대로) → Ctrl-C 주입을 뺀다 · 키는 사람이 터미널에 직접 친다(사용자). 대응 = `Unsupported`(버스 CONFLICT · WS `Interrupt` 오류) · 능력 거짓(메인). 거부한 대안 = §10 0245 · 반영 = §3-1 · §3-3 · §7 B3c · §9 · ADR-0245 |


## 12. 리뷰 기록 · 구현 때 반영할 것

- **`/review trd full` 1 라운드(2026-09-27)** — 두 리뷰어(cross-family Designer · doc-aware Architect-breaker) 모두 FIX · 불일치 없음 · 합 11 건(겹친 것 1) → 2판에 전부 반영. **light 재검(doc-aware) = PASS**, 남은 low 4 건은 아래 — 구현 워커 지시서에 넣는다.
  1. §3-4 S4 보정(「init 뒤 첫 진행 줄에서만 연다」)은 비트가 하나 더 든다(「마지막 `result` 뒤 init 을 봤나」) · `system/init` 이 턴마다 오는지 S4 가 기록한다.
  2. §2-2 규칙 2 의 「떨어진 채 녹는데 `savedTop` 이 비었다」 갈래는 규칙 1 이 얼 때마다 채우므로 손으로 만든 상태로만 시험된다 — 시험에 그렇게 적는다 · 규칙 8 에 `lastTop`·`unseenGrowth` 초기값을 적는다.
  3. §9 · 시험 14 의 이웃 델타 합치기는 펌프 한 번 읽기에 여러 줄이 올 때만 줄어든다 — 문턱을 넘으면 B1 은 **실 펌프 청크 기준** 합친 뒤 사건 수를 보고한다(fixture 청크 기준 아님).
  4. §8-2 F2 ④ 문구: 턴이 열리기 전 Esc 는 WS `Interrupt` 경로라 날 「unsupported」 오류 문자열이 온다(`CONFLICT:` 접두는 임대 거절만) — §3-2 문구가 맞다. 유휴 버스 명령의 CONFLICT 줄은 그대로 맞다.
- **`/review trd full` 3판 라운드(2026-09-27)** — Designer FIX 1(늦은 ToolResult 가 Wait 를 지움 — 채택, 사용자 동의) · Architect PASS + low 2(반영). 반영 자리 = §4-7 ⑧(`ToolResult` 의 `feed` 반환 `false`) · §4-7 ⑨(늦은 끝 창 시험 · 채취 둘째) · §9 F3 늦은 끝 · §0 · §7 · §11 ⑥(B5 착수 조건) · §8-2 F3 ⑧⑨.
- **`/review trd` 4판 라운드** — 치렀다(바로 아래). 4판이 더한 것 = 사용자 결정 U2-a(거부 = 따로 표기) · 우리 거절 귀속(§4-7 ②-2 — 리더 칸 · 상한 · 비우기 · 스위치) · 거부 모양(⑧ — 주황 배지 · 사유 줄 · 요약 · ADR-0051) · 시험 · 채취 셋째(⑨) · §6 · §7 B5 · FE-1.0 키 · §8-2 F3 ④ · §9 · §10 0241 · §11 ⑧⑨. 업스트림 판독(`beh.rs`)은 4판이 `gh api` 로 받아 직접 열었다.
- **`/review trd full` 4판 라운드(2026-09-27) — 두 리뷰어 FIX · 5판에 반영(1 · 4 · 5) · 갈림 2 건(2 · 3) = 사용자 판정 「알려진 한계로 수락」** — 판정이 한 곳에서 갈려 사용자 판정을 받았다(2026-09-27).
  1. ★**둘 다 지적 · 메인 채택(문구는 메인이 정함 — 사용자에게 보고)**★ vendor `declined`(guardian · 네트워크 정책)에도 「대시보드가 승인 요청을 처리하지 않아 실행되지 않음」이 붙는다 — 거짓 사유. **고칠 것:** 결말을 둘로 나눈다(`ToolOutcome` 은 아직 안 나간 타입이라 호환 비용 0) — 우리 거절 = `Refused`(그 사유 줄) · vendor `declined` = `Declined`(같은 주황 「거부됨」 · 사유 「codex 가 실행을 거부함」 — ★5판 light 재검에서 사용자가 「실행되지 않음」으로 바꿨다★ §1 U2-a). 두 출처 시험. → **반영(5판)**: §4-7 ①(`Refused`) · ② 표 · ②-2(결말 둘 · 명령 `failed` · 파일 변경 `declined` 둘 다 `Refused` 로 — 메인 결정) · ⑧(표식 셋 · 사유 줄 둘 · `data-tool-declined-reason` = 사유 키) · ⑨(두 출처 시험 — 통로 ③ · 프론트) · §7 FE-1.0 키 · §10 0241. 스위치는 걷었다(귀속은 늘 든다).
  2. ★**갈림 → 사용자 판정**★ 재활성화(이력 복원)에서 우리 거절이 「오류」로 바뀐다(§11 ⑨). Architect = 알려진 한계로 수용 가능 · Designer = 중간 결함(같은 대화를 다시 열면 배지·요약 수가 바뀐다 — 출처를 이력 복원까지 보존하라). → **사용자 판정(2026-09-27) = 알려진 한계로 수락**(메인 추천 — U7 「채팅 승인은 다음 과제」와 한 묶음 · 귀속 장치를 키우지 않는다). 바른 해법 = `docs/tracking.md` T-38(대시보드가 자동 거절을 그만둔다). ★5판 사실 정정(태그 판독 · 가능성 높음)★: 이력 복원에서 우리가 거절한 명령은 「오류」가 아니라 행이 **없다** · 파일 변경은 「거부됨 · 실행되지 않음」이다(§4-7 사실 · ②-2 한계 — 문구 = 5판 light 재검). ★판정의 틀(5판 light 재검에 적었다)★: 사용자는 위 4판의 틀(「오류」로 바뀐다)에서 판정했다. 판정 직후 메인이 정정한 사실(거절된 명령 행은 이력에 없어 다시 열면 사라진다 · 거절된 파일 변경은 `declined` 로 닫힌다)을 사용자에게 알렸고, 사용자는 이의를 달지 않았고 이어서 `Declined` 사유 문구를 정했다(§1 U2-a).
  3. ★**갈림 → 사용자 판정**★ 거절 기억 상한 16 — 밀려난 거절의 끝이 빨간 「오류」로 보인다. Architect = 수용(오표시 방향이 안전) · Designer = 중간 결함(상한 근거 없음 · 모름으로 두거나 대기 중 항목은 보존). → **사용자 판정 = 알려진 한계로 수락**(2 와 같은 묶음 · 바른 해법 = T-38). 판정의 틀은 2 와 같다(4판 — 밀려나면 「오류」 · 판정 직후 알린 사실도 2 와 같다). 그 사실대로면 밀려난 파일 변경은 「오류」가 아니라 「거부됨 · 실행되지 않음」이다 · 명령은 그대로 「오류」다(§4-7 ②-2 모양 · 상한 · 한계 ②④).
  4. (low · Architect) `RichSlot.tsx:217-221` 주석(「`understood === false` = 못 알아들은 프레임」)이 `ToolResult` 의 `false` 와 어긋난다 → FE-1.3 이 주석을 고친다. → **반영(5판)**: §7 FE-1.3 범위 · §4-7 ⑧(주석 줄 = `:218-221` — 주석 넷 줄 · `3f06e08` 에서 확인).
  5. (low · Architect) 같은 `itemId` 를 두 번 거절하면 기억에 두 칸 — 이미 있으면 넣지 않는다. → **반영(5판)**: §4-7 ②-2 모양 · 상한 · 통로 시험 ⑫.
  - ★**2·3 을 한 번에 푸는 대안(메인 제안)**★: 기억 장치 대신 **item 모양으로 판정** — 업스트림은 우리 거절을 `status:"failed"` · `exitCode:null` · `aggregatedOutput:null` 로 닫는다(`bespoke_event_handling.rs:1468-1483 · 2026-2032` — 조사 §8-1). 「종료코드·출력 없는 실패 = 실행 안 됨」으로 읽으면 이력 복원에서도 같고 상한도 없다. 대가 = 실행 자체가 실패한 경우(스폰 실패 등)와 못 가를 수 있다. → **기각(메인 · 5판 · 사유 순서는 light 재검이 고쳤다 — §10 0241)**: 같은 모양을 내는 다른 길(승인 콜백 버려짐 · guardian 시한 초과 — §4-7 ②-2)을 우리 거절로 읽어 거짓 사유를 붙인다. 거절된 명령 item 은 이어받은 이력에 없어(§4-7 사실 — 태그 판독) 복원도 돕지 못한다 — 얻는 것은 라이브의 상한 밀림 명령 하나뿐이다.
- **`/review trd` 5판 라운드** — **light 재검 = FIX → 반영(바로 아래)**. 5판이 더한 것 = 4판 리뷰 FIX 1 · 4 · 5(위) · 사용자 결정 U7(§1 — T-38) · 갈림 2 건 닫음(§11 ⑨ · 위 2 · 3) · §11 ⑦⑧ 닫음 · 업스트림 태그 `rust-v0.156.1`(`b412ff32c417`) 소스 판독에 따른 사실 정정(§4-7 사실 — 거절된 파일 변경 = `declined` · 거절된 명령은 이력에 없다 · 끊긴 명령의 늦은 끝 · ②-2 — 같은 모양의 다른 출처) · 스위치 제거 · §6 · §7 B5 · FE-1.0 · FE-1.3 · §8-2 F2 ① · F3 ④ ⑧ · §9 · §10 0241. 태그 판독은 메인이 했고 실행 확인은 아니다(가능성 높음) — 이 문서의 편집자는 `core/src/tools/events.rs:445-452@v0.156.1` 한 곳만 `gh api` 로 다시 열었다.
- **`/review trd light` 5판 재검(2026-09-27) — FIX → 반영 · 채취 둘째 사용자 결정 대기(→ 같은 날 닫힘 = ⓐ 그대로 · 아래 「5판 후속」)**:
  - ★사용자 결정★ `Declined` 사유 = 「실행되지 않음」(5판 메인 문구 「codex 가 실행을 거부함」을 바꿨다 — 프론트 중립 낱말 · 옛 문구는 문서화된 경로에서 거짓 · claude `ToolResult`(T-37)가 codex 이름을 물려받는다). `Refused` 사유는 그대로다. 한계는 종류가 바뀌었다 — 복원 · 상한 밀림의 우리 파일 변경 거절 = 「거부됨 · 실행되지 않음」(우리 사유를 잃지만 거짓 아님) · 빠져 있던 상한 밀림 파일 변경을 더해 한계 넷. → §0 · §1 · §4-7 ②-2 · ⑧ · ⑨ · §6 · §7 FE-1.0 · §8-2 · §9 · §10 0241 · §11 ⑨ · §12 · `docs/tracking.md` T-38.
  - (med) 채취 셋째 — 파일 변경 경로도 멈춤(끝 id ≠ 승인 `itemId` · 끝 없음 · `failed`/`declined` 아님 — §4-7 ⑨ · §7 B5) · (med) §9 끊기 서술 과장 정정(벤더 실험 API `thread/backgroundTerminals/*` · 범위 밖) · 채취 둘째 = ★사용자 결정 대기 — B5 착수 전★(§11 ⑩ · §7 B5 착수 조건 ② · 선택지 셋은 메인이 피어 조사 뒤 올린다).
  - (low) 판정의 틀 기록(§11 ⑨ · 위 4판 2 · 3) · 통로 시험 ⑤ 기대값 = 사건 없음(`decoder.rs:1225-1227` 을 열어 확인) · 채취 셋째가 정하는 코드 없음(§4-7 ②-2 · §7 B5) · FE-1.0 키 넷 · §1 머리 문장 · 통로 시험 ⑫ 근거 = 방어 규칙 · 0241 모양 판정 기각 사유 순서 · 조사 §8 정정 포인터.
  - 이 편집에서 다시 연 업스트림(`gh api` · 전부 `@v0.156.1`) = `beh:638-647` · `beh:2055-2080` · `app-server-protocol/src/protocol/common.rs:745-762` · `core/src/tools/events.rs:256` · `:445-452`.
- **5판 후속(2026-09-27) — 채취 둘째 = 사용자 결정 ⓐ(그대로) · Claude Code Esc 실측**: 끊긴 뒤 늦은 `failed` 끝은 보통 붉은 「오류」 · 머리 「오류 1」 · 바로 아래 「중단됨」 행이 맥락(새 장치 없음 · 피어 근거 · 거부한 대안 ⓑ-1 · ⓑ-2 · ⓑ-3 · ⓒ = §10 0241 · ⓒ 는 후속 후보로 아직 추적 안 함) · B5 착수 조건 ② 삭제 · 대화형 Claude Code CLI 의 Bash 도구는 Esc 에 죽는다(실측 · 우리 stream-json 경로는 미검 — B2 S1 기록). → 머리 · §0 · §3-5 · §4-7 ⑨ · §7 B5 · §8-2 F3 ⑧ · §9 · §10 0241 · §11 ⑩ · 조사 §2-2.
- **5판 좁은 재검 FIX 반영 · U8 중단 줄 강조 · claude 끊긴 도구 = ⓐ 와 같은 규칙(메인 · 위임)(2026-09-27)**: ① (med) claude 끊긴 도구(`tool_result` `is_error:true`) = ⓐ 와 같은 규칙 — 메인 결정 · 사용자 위임(§11 ⑪ · §3-5 S1 기록 추가 · §4-7 ④ · §8-2 F2 ② · §10 0241) ② 사용자 결정 U8 — 「중단됨」 행 = 굵게 + `var(--accent)`(§0 · §1 · §7 FE-2 · §8-2 F2 ⑨ · §10 0237) ③ (low) 채취 셋째 멈춤 목록을 §0 · §7 B5 · §4-7 ⑨ 셋이 같게(그 턴 `failed` · `error` 알림) ④ (low) 채취 둘째 — `failed` 아닌 늦은 status(`interrupted`·`completed`)는 기록해 반환(멈춤 아님) ⑤ (low) §11 ⑩ 선택지 이름표 = 0241 의 ⓑ-1 · ⓑ-2 · ⓑ-3 · ⓒ(ⓑ-3 은 계기가 다르다) ⑥ (low) §8-2 F3 ⑧ 도구 ≥2 ⑦ (low) ⓐ 고정 프론트 시험(§4-7 ⑨ — FE-2).
- **6판(2026-09-28) — B2 멈춤 사유(합성 사용자 줄) = 사용자 결정 ⑫ · ADR-0243 · 리뷰 전**: claude 끊김 합성 줄 → 번역기가 `Structured{kind:"interrupted"}` 로 알아본다 · `classify_turn` `None` · 누산기 `interruptNote` · 그 턴의 결말 행 생략(표시 행 유무로 가른다) · 행 모양 = U8 중단 행. → 머리 · §3-4 · §7 B3 · FE-2c · 접점 표 · §8-2 F2 ② · §10 0243 · §11 ⑪(덧붙임) · ⑫ · 조사 `docs/research/claude-interrupt-spike-2026-09-28.md` §10-1 포인터 · ADR-0237 · ADR-0238 부분 개정 도장.
- **6판 doc 리뷰(2026-09-28) — FIX → 반영**: 알아보기에서 `isReplay:true` 제외(메인 — §3-4 · §7 B3 시험) · 평문 문자열 변형 삭제 · 「턴이 열려 있을 때만」 = 실측 14/14 · 순서 전제(FE-2c) · ADR-0241 결정 12 부분 개정 도장 + §4-7 ④ · §10 0241 의 6판 표시 · `types.rs:94-95` · `classify_turn` doc 을 B3 주석 갱신 · 「함께 고칠 문서」에 · 잔여 경합 증상 · 잘린 `assistant` 줄의 6판 구절 · §6 두 행 · §9 F2 위험 · FE-2c `rowKindOf` · transcript 에 `result` 없음의 근거 · §3-3 데몬 crate 정정(B3a — 시험용 이름 목록만 · `agent.interrupt` 버스 전용 · 메인) · §8-2 F2 ④ ⑧ 일반 호출꼴. 메인이 더한 것 둘 = §3-3 의 B3a 착지 정정 · 다시 연 이력의 `isReplay` 알려진 한계(§9 F2 · ADR-0243 영향). 재검 low(같은 날): ADR-0238 도장 범위를 결정 4 · 7 증상 · 영향으로 넓힘 · §10 0243 행에 0241 도장과 「새 선 변형」 · §7 B3 파일 칸에 B3a 시험 파일 넷.
- **7판(2026-09-28) — 사용자 결정 ⑬ · ADR-0244 · 리뷰 전**: Esc 끊기를 누르면 곧바로 「중단하는 중…」 · 멈출 때까지 Esc 무시 · 백엔드 무변경(B3 그대로 · `TurnGate` `sent` 제안 대체). → 머리 · §3-1 · §3-2(「낙관 상태」 표시 한정 번복 · 「끊는 중」 항목) · §3-6 · §7 FE-1.4 · §8-2 F2 ⑩ · §9 · §10 0244 · §11 ⑬ · 조사 `docs/research/interrupt-feedback-peers-2026-09-28.md` 상태 줄 · ADR-0237 부분 개정 도장. 같은 판 문서 묶음(앞선 리뷰 후속): §4-7 ⑨ Wait 문구(첫 `TextDelta` 에 Wait 가 꺼지지 않는다 — 대기 플래그만 풀리고 열린 턴이 턴 경계까지 Wait 를 켠다) · §4-3 `ToolGroupRow` props = FE-2b-2 판(`vendorErrorIds` · `renderMember` — 옛 `results` 대신) · §4-3 알려진 한계(대화 뷰가 내려간 동안의 재스폰 → `item:` 키 펼침 이월 · 모양만) · §3-1 FE-1.2 초점 FIX 파일 셋(`slotFocus.ts` · `JumpToBottom.tsx` · `QueuedInputList.tsx`) · §8-2 F2 ⑦ · F3 ④ 문구.
- **7판 추가(2026-09-28) — 사용자 결정 ⑭ · ADR-0245 · 리뷰 전**: 터미널 모드(PTY) 끊기 제거 — `interrupt()` = `Unsupported` · 능력 `control.interrupt = false` · Ctrl-C 를 보내지 않는다. → 머리 · §3-1(`transport/pty.rs` 행) · §3-3(CONFLICT 매핑) · §7 B3c · §9 · §10 0245 · §11 ⑭ · ADR-0017 부분 개정 도장(ADR-0017 에 `- 관련:` 줄이 없어 링크 자리로 한 줄을 더했다). ADR-0001 에는 도장을 박지 않았다 — 본문에 interrupt 조항이 없다(「kill 과 interrupt(Ctrl-C) 분리」는 ADR-0017 결정 5 가 ADR-0001 을 가리키며 적은 것이다).
- **7판 doc 리뷰(2026-09-28) — FIX → 반영**: codex 「Ctrl-C 두 번 종료」 근거를 조사 문서 codex TUI 행에 올리고(`interaction.rs:99-114` · `:204-217` · `:503-510` · 가능성 높음) claude 상태를 「알려진 동작 · 여기서 미측정 · 가능성 높음」으로 맞춤(ADR-0245 · §9 · ⑭) · 「되살리지 말 것」에 사용자 사유 · 「끊는 중」의 걷음(라이브 프레임 턴 경계 · `streaming` 뒷받침 · onReset · 마지막 뷰 unmount) · 뷰 없으면 안 세움 · 둘째 호출 뜻(메인) · 톤(흐린 글 + 깜빡임 · 메인) — ADR-0244 a–c · §3-2 · §7 FE-1.4 · 걷음 = 「창 명령이 어떤 오류로 끝나든」(창 경로의 턴 없음 오류는 「unsupported」 — `CONFLICT:` 는 임대 거절뿐) · §3-6 리뷰 시험 둘(한 플러시 · kill) · ADR-0244 의 `sent` 기각 사유 정정(§10 0244) · ADR-0030 부분 개정 도장 + 0245 「추상 계약」 항목을 「능력은 통로 caps · PTY 값만 거짓」으로 · B3c 에 의도적 시험 수정(`ws_e2e.rs` case16) · `src-tauri/src/commands/agent.rs` · 문서 쓸기(`session-path-ownership.md` 두 줄 — 고쳤다) · ADR-0245 관련의 계약 포인터(`commands.rs` 선언 doc) · ADR-0244 관련의 스파이크 절 번호(§0 · §2 = 9 ms – 1.0 s · §5 = 걸쇠) · §8-2 F2 ⑩ 관측 수단(CDP 로 `interruptAgent` 호출 수) · FE-1.4 의 `ko.ts` = FE-1.0 공유 파일 · ADR-0245 태그 분리([사용자] 빼기 · [메인] 대응)와 A · B 제시 · 좁힘 · LLM 갭(「LLM-우선 제어」 · 사용자 수락 · §9) · §3-2 조건 목록에 「중단하는 중이 아님」(FE-1.4 코더). 같은 라운드에 더한 코드 리뷰 후속: 상태는 창마다(팝아웃은 다른 창을 안 본다) · `sent` 기각 사유에 「뷰가 있는 창에서만」 한정(사유는 선다 — 메인) · 걷음 자리에 에이전트 부재 · 구독 오류 · §9 「받아들여졌으나 듣지 않은 끊기」 · §8-2 F2 ⑩ 의 `CircleStop` 두 행 관측 · 톤(흐린 글 + 깜빡임). 마지막 판(FE-1.4 최종 코드에 맞춤 · doc-aware 재검): 「착지 중」 표시를 걷고 규칙으로 적음(이 화신의 첫 `'live'` 전 프레임은 세지 않는다) · 걷음 자리 최종본(첫 따라잡기에 열린 턴 없음 · 부재 · 연결 끊김 · 제 구독 오류 뷰의 `watch` 풀기 · 둘째 뷰 오류는 성한 뷰를 안 걷음) · 세움 = 보내는 같은 틱 · `[고름]` `ctx` 에 `interrupting` · §9 새로 붙은 뷰의 이른 걷힘. 끝 판(FE-1.4 최종 코드 · doc 리뷰 low): 걷음 = 누산기의 턴 경계 수가 오를 때(전이 아님 — 한 묶음 [`TurnEnd`, 다음 `TextDelta`] 를 잡는다) · `streaming` 뒷받침 = 대기에 든 보냄만 · 부재에 `Failed` · §3-6 시험 둘 · F2 ④ 문구(「unsupported」 · 잠깐 「중단하는 중…」) · B3c 착지 `02e7f18` 과 시험 이름 `@a75569b` → `@02e7f18` · 주석만 바뀐 파일 셋 · F2 ⑩ 두 `CircleStop` 창 = 수십 ms 이하 · §10 함께 고칠 문서에 CLAUDE.md 「LLM-우선 제어」 갭.
