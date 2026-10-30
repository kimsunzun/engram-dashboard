# TRD 리뷰 결과 (2026-10-20 · `/review trd full` · 대상 = `docs/process/S25-preset-library/trd.md` @ `b2e68ece`)

- 리뷰어 = Claude Architect-breaker(doc-aware — CLAUDE.md · ADR · 코드 대조) **FIX 15건** · codex Designer(effort xhigh) **FIX 9건**.
- ★**codex 슬롯은 blind 가 아니다**★ — 사용자 지시(2026-10-19 「클로드하고 같은조건으로 데이터줘서 리뷰해」)대로 Claude 와 같은 자료(CLAUDE.md · ADR · PRD)를 줬다. 리뷰 스킬 기본(Designer = blind)과 다르다.
- **취합 판정 = FIX(둘 다 FIX)** — 단 독립 수정 항목이 5 를 크게 넘어 규칙상 「분리 · 재검토(BLOCK 쪽)」 검토 대상이고, PRD 리뷰가 BLOCK(잠정)이라 PRD 를 고친 뒤 TRD 를 다시 맞추는 순서가 맞다. **반영 안 함 — 사용자 판정 대기.**
- 메인이 원문으로 확인한 것: Claude #10(이름 발급기 = `format!("{base}({n})")` — 띄어쓰기 없음 · `crates/engram-dashboard-agent/src/manager.rs:228`) · Claude #4(데몬의 `reqwest` 는 시험용 의존 — `crates/engram-dashboard-daemon/Cargo.toml:199-200`).

## 둘이 같이 짚은 것

1. **같은 작업 폴더를 쓰는 codex 터미널 에이전트들** — 검사와 파일 깔기 사이에 다른 에이전트가 끼어들 수 있고(배타적 생성 계약 없음), 다른 에이전트 표식이 붙은 파일의 규칙 · 「다른 에이전트」 가 있는 것인지 도는 것인지가 없다. 복제가 작업 폴더를 복사하면 원본의 `AGENTS.override.md` 때문에 복제본이 영영 막히거나 덮어쓴다 · 스킬 정션이 같은 폴더의 다른 codex 에게 보인다 · 에이전트를 지울 때 생성 파일 정리가 없다(§1-5 · §1-3 8 ⑤ · `agent.duplicate`). (Claude #1 · codex #2)
2. **백엔드 전용 설정 지식이 데몬 `library::validate` 에 샌다** — 백엔드별 스키마 출처 · codex `-c` 점 키 해석 · `claude --version` · 「합친 claude settings」 가 데몬 쪽 → CLAUDE.md 「백엔드 확장」 위반 · backend 가 내주는 검사 포트로(§1-4 `validate.rs` · §1-5). (Claude #3 · codex #5)
3. **휴지통 비우기(사람만)의 전송이 안 정해졌고 전제가 틀렸다** — §1-8 「새 wire 메시지 없음」 대 §6-5 「비우기용 wire 하나」 · 에이전트도 같은 사용자 프로세스라 portfile 토큰으로 WS 에 닿는다 · 2판에 `PROTOCOL_VERSION` 올림이 빠짐 · §5 단계에 그 메시지를 세우는 단계가 없다. (Claude #7 · codex #4)
4. **프라이밍 파일 출처가 둘** — `builtin\` 의 `.md` 대 지금 `prompts/agent-priming.md`(`ENGRAM_PRIMING_FILE`) · ADR-0092 를 폐기 목록에 안 넣음 · 개발 · 시험 · `ENGRAM_DATA_DIR` 에서 `builtin\` 찾는 법 · 배포 zip 싣기 · 없을 때 동작(지금은 「프라이밍 없이」 ↔ 새 안은 매 실행 막힘)(§1-2 · §1-5). (Claude #8 · codex #6)

## 한쪽만 짚은 것 — 굵은 것

5. **삭제 때 참조를 `.trash` 경로로 고치는 흐름 ④ 와 되살리기 「참조가 원래 경로에 남아 저절로 풀린다」 가 서로 어긋난다**(§1-4 ④ · §1-7 `library.restore`). (codex #1 — PRD 리뷰 codex #1 과 같은 축)
6. **참조 색인의 에이전트 쪽이 낡는다** — `agent.duplicate` · `agent.new` · wire `CreateProfile` · `DeleteProfile` 이 라이브러리 서비스를 안 거쳐, 앱 안 이동이 복제한 에이전트의 붙임을 못 고치고 실행이 막힌다(§1-4 `catalog.rs`). (Claude #2)
7. **스키마 내려받기 — 데몬 운영 의존에 HTTP/TLS 클라이언트가 없다**(`reqwest` 는 dev 전용) · 오프라인일 때 검사를 건너뛰나 막나 미정 · 훑기 쓰기 락을 쥔 채 네트워크를 기다려 실행 · `library.list` 가 30초 기한에 걸릴 수 있다(§1-4 · §1-12). (Claude #4)
8. **cmd.exe 위험 · 경고 통로 없음** — codex 두 모드 다 `cmd.exe /c` 를 거치는데 가드가 터미널 칸에만 · 「빼고 경고」 의 경고 통로가 없다(경고 범주 없음 결정) · 명령줄 8191자 예산 · 새 경로 인자가 기존 `path_is_cli_safe` 가드를 안 탄다(§1-5). (Claude #5)
9. **막힘 창의 수명이 셸과 안 엮였다** — 트레이 숨기기(ADR-0229)가 그 창을 모름 · 창 파괴 정리가 팝업 label 만 봐서 출력 채널 등록이 샌다 · 라우트 미정 · 창 안의 상세 카드 · 추가 팝오버가 그 작은 창에 잘린다 · 줄 좌표 · DPI 를 셸에 넘기는 계약 없음(§1-9 · §1-10). (Claude #6 · codex #8)
10. **범위가 다른 새로 고침을 한 훑기로 합친다**(폴더 A 를 훑는 중 B 요청 · 루트 바꾸기)(§1-4 ①). (codex #3)

## 중간 · 작은 것

11. 컬렉션 커서의 권위가 `collection.applyMoves` 공개 인자로 샌다 · 따라잡기를 웹뷰가 몰아 「프론트는 렌더링만」 위반 · 폴더 이동을 짝 하나로 적나 자손마다 적나(1,000 상한)(흐름 ⑥ · §1-7 · §1-9). (codex #7 · Claude #13)
12. `preset.get` 은 메모리 카탈로그, `reorder` 는 디스크 기준 `baseRev` → 바깥 편집 뒤 새로 고침 전까지 CONFLICT 반복 · `agent.reorderPresets` 의 rev 출처 없음(흐름 ③). (Claude #9)
13. 복제 이름 「<이름> (2)」 가 이름 발급 규약(`이름(N)` — 띄어쓰기 없음 · ADR-0115/0120/0123)과 다르다 → 두 번째 복제가 「bob (2)(1)」.  (Claude #10 · 메인 확인)
14. 스킬 충돌 검사가 이름을 대소문자 구분 비교 — Windows · macOS 정션 폴더는 대소문자 무시로 부딪힘 · 경로에 못 쓰는 글자의 `name`(§1-3 5). (Claude #11)
15. 흐름 ④ 실행 중 부분 실패(폴더 안 파일이 열려 있으면 이름 바꾸기 실패) · 부팅 재시도가 매번 돌 수 있음. (Claude #12)
16. 다음 주제 몫 칸(`LaunchOrigin` · `profile_changes`)과 `LaunchPlan.real` 이 이번 실행 포트에 미리 실림(§1-5). (codex #9)
17. `file_identity` 가 platform `windows` feature 에 없는 `Win32_Storage_FileSystem` 를 요구(§1-6). (Claude #14)
18. T6 「지난 적용값」 지침 해시의 저장 자리 없음 · 12a 가 1단계에서 지우는 `presetCommands.ts` 를 옮긴다(Claude #15).
- 막연한 우려(판정 무관): 같은 `std::sync::RwLock` 재귀 읽기(쓰기 대기 시 교착 가능) · `agent.setWorkspace` 가 실행 예약을 잡으면 동시 활성화가 말없이 버려짐.

## 깨끗하다고 본 것

crate 배치(라이브러리 = 데몬 · 어휘 = agent · OS 코드 = platform · `junction` 4g 명단) · 셸 → agent 의존 안 생김 · 실행 전처리 자리(`register_for_spawn` → `epoch_for_spawn` 인접 유지) · 첫 제출 래치 · kill 인과 손 안 댐 · 명령 표면 대체로 완비 · 1판 단독으로 선다 · 공용 편집 부품 하나 · 저장소 포트와 가짜 · 서드파티 선택 관문.
