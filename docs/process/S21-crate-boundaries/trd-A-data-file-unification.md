# TRD — 경계 리팩터링 A: 데이터 파일 규칙을 하나로 (S21)

> 상태: **6판(2026-10-10) — 리뷰 4 라운드 반영(작은 증분) · 재리뷰 전.** 코드는 한 줄도 바뀌지 않았다.
> **6판 개정(2026-10-10 — 리뷰 4 라운드: doc-aware PASS · codex FIX 1 — 메인 직접 반영)**
> - 만들고 띄우기의 부분 성공 = 지금의 계약(되돌리지 않음 · 문구가 회복 길) 그대로 · 저장 거절 갈래 ⓐ ⓑ · U5b-1 정지 지점 모양 — §3-9 (메인 결정(위임) · 잠정)
> - U5b-1 지시서에 넣을 것 여섯(doc-aware 잔손질) — §6-4 끝
>
> **5판 개정(2026-10-10 — 리뷰 3 라운드 결과 `.claude/handoff/attachments/A-trd-review-r3-2026-10-10.md` 의 메인 결정을 그대로 적용 · 전부 「메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기」):**
> - **E1** 내부 입구는 저장 때의 **어떤 `Err`**(쓰기 IO 실패 · 저장 직전 재판정 거절)에도 메모리 적용 + dirty — 4판의 「`ReadOnly` 면 적용하지 않는다」와 부팅 복원의 「이번 쓰기 거절이면 `Failed`」를 뒤집는다 · 디스크는 매 쓰기의 재판정이 지킨다 · 적재 거절(읽기 전용)은 명부가 비어 대상이 없다 · 부르는 쪽 있는 입구는 그대로 · 래치 머리 전제가 다시 참이 된다(ADR-0226 개정 확인은 영속 시점만) — R17 · §3-9 · §4 5 · 10 · §6-1 · §6-3 · §6-4 · §7 · §8-1 11.
> - **E2** D3 의 「`Ok` 일 때만」 목록에서 ADR-0172 가 실패 때도 요구하는 알림 둘(`agent/src/commands.rs:975` · `:1008`)을 뺀다 — §3-9.
> - **E3** 부팅 복원을 내부로 치는 배관 = 수동 깨우기와 함께 닿는 한 공유 함수 `spawn_agent_watching_link`(`manager.rs:1191`)에 정책 인자 하나(가안 `entry: Entry`) · 사본 금지 · 표 줄 고침 · 두 입구가 같은 함수에 닿는 시험 — §3-9 · §6-1.
> - **E4** U5b-1 뒤의 옛 API = U5a 그대로(§6-4 의 뜻으로 통일 — 「옛 판은 새 판을 불러 로그만」을 버린다) · 매니저의 `Result` 판(create · delete · rename · reparent · set_auto_restore)을 U5b-1 ③ 에 · U5b-1 뒤 멈춘 트리가 동작상 안전함을 §6-4 에 적었다 — §6-1 · §6-4.
> - **E5** 상태 기록기의 파일 이음매 `StateFiles`(`src-tauri/src/state/saver.rs:70`)에 떠 두기 연산을 더한다 · 실물 · 시험 구현 + 「떠 두기 실패면 계속 쓴다」 시험 — §3-6 · §6-1.
> - **잔손질(전부 반영)** — 고정 세대 단언 둘(`src-tauri/tests/layout_commands.rs:619` · `crates/engram-dashboard-agent/tests/command_declarations.rs:75`)을 U4 · U5b 에 · ADR-0163 결정 7 의 `mutate` 클로저 포인터를 §7 에 · R16 의 단일 인스턴스 뮤텍스 = 번들 식별자 단위 · `Parsed`/`Loaded` 의 `#[non_exhaustive]` 고려(§3-3) · 깨우기 경로의 거절을 `CONFLICT` 로 실으려면 `PtyError` 변형이 든다(§3-9) · 종료 때 남은 dirty 는 잃는다(§3-9) · `Claim` · 적재 거절은 첫 `load()` 에서만 선다(§3-6).
> - **그 밖(5판을 쓰며 찾은 것)** — ① 지금 운영 데몬은 `restore_all` 을 부르지 않는다(부팅 자동 복원 기본 OFF — `crates/engram-dashboard-daemon/src/lib.rs:780-786`) — 규칙은 그대로 정하되 §3-9 에 적었다 ② E2 와 같은 까닭으로 WS 활성화 뒤 브로드캐스트(`connection_core.rs:1332` · `:1700`)도 D3 밖이라 명시했다 ③ E1 로 「세션 id 는 첫 턴 전에 영속」이 디스크에서는 늦어질 수 있어 CLAUDE.md 「핵심 불변식」의 그 항목을 §7 문서 목록에 더했다.
>
> **4판 개정(2026-10-10 — 리뷰 2 라운드 결과 `.claude/handoff/attachments/A-trd-review-r2-2026-10-10.md` 의 메인 결정을 그대로 적용 · 전부 「메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기」):**
> - **C1** 실행 중 바깥 편집은 지원하지 않는다(마지막 작성자가 이긴다 — 지금과 같다) · 거부한 대안(파일 신원 · 내용 지문 추적 → 거절 · 다시 적재)과 까닭 · 셸 단일 작성자 근거 = 단일 인스턴스 플러그인(주) + 실행 잠금(보조) — R16 · §4 12 · §7.
> - **D1** `Spec` 에 주인 모양 검사 콜백 `shape` — `parse` 가 부르고 모양 실패는 어디서나 `Unusable` · 맨 판정 길이 없어진다 · 상태 기록기의 재판정도 머리 해석까지 — §3-3 · §3-4 · R7 · §4 1 · §8-2 4 가 다시 참이 된다.
> - **D2** 부르는 쪽 없는 내부 변경 = `ReadOnly` 면 적용하지 않고 · `Io` 실패면 메모리에 적용 + dirty → 다음 저장이 싣는다 · dirty 는 레지스트리 맵 옆(같은 락) · 래치 머리 전제와 ADR-0226 을 「개정 확인」으로 — §3-9 · §7. ★5판 E1 이 앞 절반을 뒤집었다 — 어떤 `Err` 에도 적용 + dirty · 래치 전제는 다시 참★
> - **D3** 변경 뒤 부수효과(브로드캐스트 · 메시징 삭제 정리 훅 · `Renamed`)는 `Ok` 일 때만 — §3-9 · U5b 시험. ★5판 E2 가 실패 때도 도는 알림 둘을 목록에서 뺐다★
> - **D4** 락 안 작업이 는다(재읽기 · 파싱 · 떠 두기 · 재시도) — 최악 약 0.4 초 잠 + IO 를 적고 수락 · ADR-0207 「개정 확인」 — §3-9 · §7.
> - **D5** 상태 기록기: `Refuse(Newer)` = 코덱 거절처럼 다음 변경까지 기다림 · `Refuse(ReadFailed)` 만 디바운스 재시도 — §3-6.
> - **D6** `agent.list` 상태에 셋째 값(이번 쓰기 거절 — 가안 `refusing` + 까닭) — 낱말은 U5b — §3-9 · §8-3.
> - **D7** agents · presets 적재 읽기에 따로 긴 잠김 예산(파일마다 약 1 초 — 셸의 데몬 찾기 마감 5 초 안) — R14 · §3-4 · §4 3.
> - **D8** 띄우기 경로의 변경 셋(`register_for_spawn` · `epoch_for_spawn` · `release_session_id`)을 표에 · `epoch_for_spawn` 은 메모리만 · 부분 성공 처리 — §3-9.
> - **D9** 저장소 구현 16 → 18(`manager.rs:3073` · `:5016`) · base 입주자 명단 사본 셋(루트 `Cargo.toml:14` · `docs/testing-strategy.md:43` · `:160`)을 U2 에 — §3-9 · §6-1 · §7.
> - **D10** U3 단위 안 순서 — `serde_json` 의존을 먼저 — §6-4.
> - **D11** U7 을 물결 2 로(물결 표 · 겹침 표 다시 맞춤) — §6-2 · §6-3.
> - **D12** §3-1 B 의 기각 사유를 ADR-0266 결정 2(OS 의존 여부로만 입주)로만 — §3-1.
> - **D13** 설정 거절이 LLM 에 `INTERNAL` 로 보인다 — 코드는 U4 에서 메인이 정한다 — §3-6 · §8-3.
> - **D14** 적재 때 없음 · 손상이었고 아직 안 쓴 파일이 첫 저장 때 쓸 만하면 덮기 전에 떠 둔다(`CopyAsideFirst`) — §3-3 R8 · §3-6 · §4 13.
> - **D15** `OsHooks` 는 운영 상수용 fn 포인터 그대로 · base 안 비공개 이음매는 클로저 인자(시험이 상태를 쥔다) — §3-3.
> - **그 밖(4판을 쓰며 찾은 것 · 이 TRD 의 제안)** — D1 을 지금 `ProfilesFile` · `PresetFile` 그대로 하면 두 구조체가 `schema_version` 을 필수로 둬 R1(버전 없음 = 1 판)이 모양 검사에서 손상으로 뒤집힌다 → 읽기 쪽 모양 구조체는 버전 키를 필수로 두지 않는다(§3-4) · D7 의 값은 셸의 데몬 찾기 마감(5 초 · 실패면 재시도 없이 `Down`)에서 거꾸로 정했다(R14 · §8-2 9) · U5b 가 커져 U5b-1(레지스트리 · manager) → U5b-2(표면) 순차 분할을 적었다(§6-1) · `boot.rs:80` 주석 인용을 `:81` 로 바로잡았다.
>
> **3판 개정(2026-10-10 — 리뷰 1 라운드 결과 `.claude/handoff/attachments/A-trd-review-r1-2026-10-10.md` 를 적용):**
> - **사용자 결정** — 거처 = **base 새 입주자 `file`**(2판의 A = 새 crate `engram-dashboard-datafile` 철회). 잠김 판정 · 다시 하기는 platform `fs`(U1 그대로)이고 base 는 platform 을 못 부르므로 **재시도(와 폴더 동기화)를 부르는 쪽이 인자로 넘긴다** — 2판이 「D」로 기각한 모양이라 그 기각 사유를 다시 봤다(§3-1). ADR-0269 결정 6 「지킬 것」(base 가 serde 를 안 끌어옴 · `file` 은 바이트만)을 고쳐 `serde_json` 을 들인다 — 새 ADR(번호 미정)에 든다(§7).
> - **메인 결정** — ① 저장 거절 = **읽기 전용 모드**: agent · preset 저장소가 거절 상태면 레지스트리 변경 요청이 적용 전에 오류로 돌아가고 그 상태가 버스 명부(`agent.list`)에 실린다(R17 · §3-9) — 2판의 「로그만 · 메모리 변경은 성공처럼」을 버린다 ② **저장 직전 재판정** · 프로세스 간 잠금은 두지 않는다 — 그 근거를 적었다(R16).
> - **리뷰 지적(Claude 12건)** — 1 ADR-0269 결정 1 · 6 · 7 · 9 행을 §7 표에 다시 쓰고 인용 「결정 2」→「결정 6」 정정 · CLAUDE.md base 항목 · 입주자 무참조 정규식 사본 여섯 자리에 `file`(§5-2 · U2) / 2 U6(mcp_config 의 `create_dir_all` 걷기)은 R11 이 든 U3 뒤(§6) / 3 U4 에 LLM 이 읽는 표면 다섯(`catalog_version` 15 → 16 포함 — §3-6 · §6-1) / 4 R10 폴더 동기화 = platform `fs::sync_dir`(§3-2 · R10) / 5 R11 서술 정정 · 기록기 · 떠 두기도 폴더를 만든다로 정함(R11) / 6 가드는 직속 자식만 끈다 — `cmd.exe` 손자 누수를 적고 `process::kill_tree` 물러서기를 권고(§3-7 · §4 · §7) / 7 `write_lock` 을 걷을 때 락 순서 사슬이 적힌 다섯 자리를 U5a 에 · 새 잎 락 「저장소 상태 칸」(R12) / 8 메시징 정규식 다섯째 사본 — base 로 바뀌어 메시징 정규식 자체가 안 바뀐다(손댈 일 없음 · §5-2) / 9 셸 → base 이사 시험 = 21(§6-5) / 10 옛 고정 임시 이름은 저장소가 직접 지운다 · `sweep_temps` API 그대로(R12) / 11 설정 `LoadNote::Newer`(§3-6) / 12 잔손질 — ChildGuard 줄 · 메모 줄 `:336` · U6 물결 표기 통일 · §2-2 상태 폴더 만들기 · ADR-0264 결정 2 그림 행(§7).
> - **그 밖** — 읽기 전용 모드가 트레이트 · 레지스트리 모양을 바꾸므로 U5 를 U5a(저장소) · U5b(읽기 전용 모드)로 갈랐다 · 새 crate 가 없어 회귀 결과 줄 증가 0 · 기준 트리를 `78ed761` 로 다시 맞췄다(코드는 `3b9725d` 와 같다).
>
> **2판 개정(2026-10-10) 요지:** 거처 A(→ 3판에서 철회) · base `retry_if_vanished` 를 1d 에서 뺌 · 새 판 규칙 네 파일 · `layout/apply.rs:247` 을 U8 에 · R7 · R8 유지 · net 몫을 transport 3-3 후속으로 · 안내 문구는 가안 · 새 ADR 은 U1 바로 앞.
> **범위 = 사용자 결정(2026-10-08~10) — §1.** TRD 1-1 의 미룬 것 D1(손상 사본) · D2(셸 쓸어 담기) · D3(원자 쓰기)와 TRD 1-3 §5-1 의 U-W(원자 쓰기 통일)를 이어받는다. 트리거였던 storage P3 는 이 브랜치에 들어와 있다(`src-tauri/src/fsutil.rs` 가 P3 모양).
> **배치 근거:** `docs/README.md` 「새 기능 **설계 착수** → `process/SN-name/`」 — 같은 단계의 형제 TRD(1-1 · 1-3 · 2-1 · 2-2 · 2-3)가 서는 폴더.
> **표기:** 「실측」 = 기준 트리 `78ed761`(브랜치 `v0.3.3/refactor/crate-boundaries` — 코드는 범위 조사 트리 `aef34d4` · 2판 기준 `3b9725d` 와 같다, 그 사이 커밋은 문서뿐 · ★4판이 새로 단 `파일:줄` 은 `3702658` 에서, 5판이 새로 달거나 고친 것은 `76875e1` 에서 다시 쟀다 — 코드는 둘 다 `78ed761` 과 같다★) · 「사용자 결정(날짜)」 = 문서에 적힌 사용자 결정만 · 「메인 결정」 = 리뷰 취합 때 메인이 정한 것(위임 「성숙한 프로그램 관행대로」 — 첨부) · 「메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기」 = 리뷰 2 · 3 라운드 취합에서 자율 모드(사용자 위임 「막히면 간단한 건 자의적으로」)로 메인이 정한 것(첨부 r2 · r3) · 「메인 판단(위임)」 = 사용자가 「성숙 프로그램 관행대로 · 메인이 정하고 보고」로 넘긴 세부(2026-10-10) · 「제안」 = 이 TRD 의 안(이름은 전부 가안 — 메인 채택 전) · 「기억 인용 · 미검」 = 관행 근거를 원문과 대조하지 않은 것.
> 앵커: ADR-0274(손상 사본 규칙) · ADR-0265 결정 4 · ADR-0266 결정 2 · 3 · 8 · ADR-0269 결정 1 · 6 · 7 · 9 · ADR-0275 결정 1 · 6 · 15 · ADR-0071(레지스트리 저장 락) · ADR-0207 · ADR-0264 결정 2 · 7 · ADR-0135(portfile 제자리 쓰기 — 바꾸지 않는다) · ADR-0282(쓰기 프로브는 base) · ADR-0061(presets 는 profiles 복제) · ADR-0226(세션 id 영속 = 첫 제출 래치) · ADR-0116 결정 3(프로필 삭제 정리 훅) · ADR-0172(실패 때도 명부 알림) · ADR-0082 · ADR-0201 · ADR-0202(부팅 복원과 수동 깨우기는 같은 동사) · ADR-0163 결정 7 · ADR-0230 · ADR-0012 · TRD `docs/process/S21-storage/trd.md` §6-2 · §6-5 · §12 R3 · 범위 조사 `.claude/handoff/attachments/A-write-atomic-set-aside-scope-2026-10-08.md` · 리뷰 1 라운드 `.claude/handoff/attachments/A-trd-review-r1-2026-10-10.md` · 리뷰 2 라운드 `.claude/handoff/attachments/A-trd-review-r2-2026-10-10.md` · 리뷰 3 라운드 `.claude/handoff/attachments/A-trd-review-r3-2026-10-10.md`.

---

## 0. 결론 (먼저)

```
① 데이터 파일 넷(설정 · 상태 · agents · presets)의 읽기 규칙 · 원자 쓰기 · 손상 사본 · 「덮어도 되나」 판정을
     구현 하나로 모은다 — base 새 입주자 `file`. 셸 · agent · 데몬이 같은 함수를 부른다.
     파일 이름 · 자리 · 스키마 · 버전 키 이름은 각 주인이 쥔다.

② 「남이 잠깐 쥐었다」 판정 · 다시 하기 고리 · 폴더 동기화는 platform fs 다 — OS 규칙이라서다.
     base 는 platform 을 못 부르므로 부르는 쪽이 그 둘을 `file` 에 넘긴다(crate 마다 상수 — agent 만 둘).
     → 셸 fsutil.rs 의 cfg!(windows) 가 걷혀 platform 밖 운영 OS 분기가 0 이 된다(ADR-0275 결정 15 를 닫는다).

③ 새 판이 쓴 파일과 읽다 실패한 파일은 덮지도 떠 두지도 않는다 — 네 파일 공통. 판정은 저장 직전마다 다시 한다
     (주인의 모양 검사까지 — 모양이 깨진 파일은 어디서나 손상이다).
     → agent 의 「새 판 agents.json 을 빈 목록으로 덮는다」 결함이 닫힌다(같은 부류인 「읽기 실패 → 덮음」도).
     → agents · presets 가 그 상태면 읽기 전용 모드: 바꾸는 요청이 적용 전에 오류로 돌아오고 agent.list 가 그 상태를 싣는다.
     → 실행 중 바깥 편집은 지원하지 않는다(마지막 작성자가 이긴다 — 지금과 같다). 단 적재 때 없음 · 손상이던 파일이
        쓸 만해져 있으면 첫 저장이 덮기 전에 떠 둔다.

④ 덤으로 공용화 둘 — agent 자식 띄우기 세 벌(codex 의 가드를 셋 모두의 규칙으로 · 가드는 자식 트리째 끈다) ·
     셸 락 오염 복구 12곳과 상태 기록기 시계를 base 로.

⑤ 단위 열(U1~U4 · U5a · U5b · U6~U9) — 어느 단위 뒤에 멈춰도 빌드 · 회귀 초록.
     병렬 = 물결 1 U1 ∥ U8 · 물결 2 U2 ∥ U7 · 물결 4 U4 ∥ U5a ∥ U6.
```

**지금 하지 않는 것:** daemon.json 읽기(net ↔ 셸) · WS 재연결 정책(→ transport 3단계) · net `instance.rs` 의 열기 재시도(net 은 transport 3-3 에서 갈린다 — 그때 platform `retry_busy` 를 쓴다 · §3-6) · base `writable.rs` `retry_if_vanished`(잠김 재시도가 아니다 — §8-1 1) · 락 오염을 패닉으로 둘지 되찾을지(ADR-0288 · ADR-0290 — 별 주제, 브랜치 `v0.3.3/feat/panic-policy`) · net `portfile.rs` 의 제자리 쓰기(ADR-0135) · 셸 실행 잠금 `state/lock.rs` 의 기다림(§2-4) · 항목 하나만 깨진 agents.json 의 관용 파싱(`docs/tracking.md` T-33 — §8-2) · 읽기 전용을 실행 중에 푸는 다시 적재(§3-9 · §8-3) · 실행 중 바깥 편집의 감지(파일 신원 · 내용 지문 — R16) · 띄우기 성공 경로의 「무리에 넣기 전에 생긴 손자」(§3-7 · §8-3).

---

## 1. 범위 — 사용자 결정 (2026-10-08~10 · 재론하지 않는다)

| id | 결정 | 이 TRD 의 자리 |
|---|---|---|
| 1a | 버전 붙은 JSON 데이터 파일 읽기(BOM 떼기 · 크기 상한 · 버전 문 · 손상 판정)를 **구현 하나 · 규칙 하나**로 — 지금 사본 = 셸 `settings/store.rs` · `state/codec.rs` · agent `persistence/mod.rs` · `persistence/presets.rs`. 버전 키 이름은 파일마다 지금 그대로(`$version` · `version` · `schema_version`) — 있는 파일이 계속 읽힌다 | §3-3 · §3-5 |
| 1b | 원자 쓰기 하나로(셸 `fsutil.rs` 의 `write_atomic` · `copy_atomic` · `sweep_temps` 대 agent persistence 두 벌) | §3-3 |
| 1c | agent 파일(`agents.json` · `presets.json`)도 **ADR-0274 규칙**(고정 `.corrupt` · 복사 · 덮어쓰기). 두 번째 손상이 첫 사본을 덮는 것을 받아들였다 | §3-5 R9 · §4 |
| 1d | 「파일이 잠깐 쥐어졌다」 재시도 → **platform 정책 하나**(OS 규칙) — 셸 fsutil rename 재시도 · net `instance.rs` · agent `usage/scratch.rs` · base `writable.rs` `retry_if_vanished` | §3-2 · ★메인 결정(2026-10-10): writable 은 뺀다 · net 은 transport 3-3 의 후속으로 미룬다(§8-1 1 · 5)★ |
| 1e | 데몬 `control/mcp_config.rs` 의 토큰 파일 평범한 `fs::write` → 통일 원자 쓰기 | §3-6 |
| 1f | platform 밖 마지막 운영 `cfg`(`fsutil.rs` 잠김 판정) → platform `fs` — ADR-0275 결정 15 의 시한부 예외를 닫는다 | §3-2 · U1 |
| 2g | agent 자식 띄우기(무리에 넣기) 세 벌 공용화 — codex 의 가드(무리 넣기가 실패하면 자식을 죽인다)를 셋 모두의 규칙으로. stdio · pty 의 누수를 고친다(메모 `docs/refactoring/architecture-discussion-2026-09-26.md:336`) | §3-7 · U7 |
| 2h | 셸 손 락 오염 복구 → base `sync`(TRD 1-1 D2 의 목록 + `theme.rs` · `state/restore.rs` · `state/placement.rs`) · `state/saver.rs` 자기 `trait Clock` → base `Clock` | §3-8 · U8 |
| 3 | 뺀다: daemon.json 읽기 · WS 재연결(→ transport 3단계) · 패닉 대 되찾기 정책(ADR-0288/0290 — A 아님) | §0 |
| 4 | 세부(버전 없음 · 새 판 · BOM · 네 파일 상한 · 재시도 수/간격 · 임시 이름 · 폴더 fsync · 폴더 만들기)는 「성숙 프로그램 관행대로 · 메인이 정하고 보고」 | §3-5(메인 판단(위임)) |
| 5 | **공용 코드 자리 = base**(2026-10-10 — 「Base에 넣어야지. 나중에 쪼갤려면 base에서 쪼개야지.」) — 파일 규칙 전부(읽기 · 버전 판정 · 원자 쓰기 · 손상 사본 · 임시 쓸기 · 쓰기 정책)를 base `file` 로 · OS 에 따라 갈리는 잠김 판정 · 재시도는 platform `fs` · 재시도를 부르는 쪽이 인자로 넘긴다 · ADR-0269 결정 6 개정(새 ADR) | §3-1 · §7 |

**알려진 결함(이 범위가 닫는다):** agent 적재가 자기 판과 다른 `schema_version` 을 만나면 빈 목록으로 시작하고(`persistence/mod.rs:115-122`), 첫 변경의 저장(`profile.rs:377-396` 의 `ProfileRegistry::new` → `mutate` → `store.save`)이 그 파일을 덮는다 — 사본도 없다. 그 파일이 새 판이면 새 판의 명부가 사라진다. 통일 규칙 R2 · R8 · R17(§3-5)이 막는다.

---

## 2. 현황 실측 (트리 `78ed761`)

### 2-1. 사본 목록

| 축 | 자리 | crate | 지금 모양 |
|---|---|---|---|
| 읽기 | `src-tauri/src/settings/store.rs:73-86` · `:131-156` | 셸 | 열기 + `fsutil::read_capped`(★잠김 재시도 없음★) · BOM 떼기 · JSON 객체 · `$version` 없음 = 1 · `1`/`1.0` 만 받음 · 그 밖 = 못 씀 |
| 읽기 | `src-tauri/src/state/codec.rs:97-133` · `:158-170` · `:176-178` | 셸 | `fsutil::read_file_capped`(잠김 재시도) · BOM · 객체 · `version` 없음 = 못 씀 · 정수 값인 수 · 초과 = `NewerVersion` · `InvalidData` = 못 씀 |
| 읽기 | `crates/engram-dashboard-agent/src/persistence/mod.rs:102-129` · `presets.rs:92-120` | agent | `fs::read` — 상한 · BOM · 재시도 없음 · serde 구조체(`schema_version` 없으면 파싱 실패 = 손상) · 불일치 = 빈 목록 · 파일 그대로 · 읽기 IO 실패 = 빈 목록 |
| 쓰기 | `src-tauri/src/fsutil.rs:62-131` · `:166-197` · `:199-280` | 셸 | `write_atomic(_unless)` · `copy_atomic` · `copy_aside` · `replace_with` · 임시 `<이름>.tmp<pid>.<번호>` · 실패면 임시 지움 · `sweep_temps` |
| 쓰기 | `persistence/mod.rs:50-76` · `presets.rs:46-68` | agent | 고정 `agents.json.tmp` · `presets.json.tmp` + `write_lock` · `create_dir_all` · 부모 폴더 fsync best-effort(`File::open(폴더)` — Windows 에서는 열기가 실패해 무동작으로 보인다) · 재시도 없음 · 실패면 임시 남김 |
| 쓰기 | `crates/engram-dashboard-daemon/src/control/mcp_config.rs:104-118` · `:157-165` | 데몬 | 토큰을 실은 mcp-config · 세션 설정 조각 — 둘 다 `create_dir_all`(`:112-114` · `:159-161`) + `fs::write` |
| 손상 사본 | `fsutil.rs:126-130` `copy_aside` | 셸 | ADR-0274 — 고정 `.corrupt` · 복사 · 덮어쓰기 · 첫 쓰기 전 |
| 손상 사본 | `persistence/mod.rs:78-87` · `presets.rs:70-79` `preserve_corrupt` | agent | 적재 때 rename(이동) → `<이름>.corrupt-<epoch ms>`(base `time::now_epoch_ms`) · 누적 · 재시도 없음 · 실패 로그만 |
| 잠김 재시도 | `fsutil.rs:292-321` `retry_denied` · `is_lock_contention` | 셸 | 5회 × 20 ms · `PermissionDenied` + Windows 원시 32 · 33 — ★`:320` 이 platform 밖 마지막 운영 `cfg!(windows)`★ |
| 잠김 재시도 | `crates/engram-dashboard-net/src/instance.rs:86-96` · `:201-266` | net | 열기 5번 · 사이 100 ms · 공유 위반(32)만 · 접근 거부(5)는 읽기 전용 속성 걷기 1회 뒤 바로 실패 · 한도 뒤 진단(`live_owner` · 한 번 더 열기) — ★A 에서 바꾸지 않는다(transport 3-3 후속)★ |
| 잠김 재시도 | `crates/engram-dashboard-agent/src/usage/scratch.rs:66-88` | agent | 임시 폴더 지우기 · 예산 200 ms · 간격 10 ms · ★`NotFound` 밖 모든 오류를 다시★ |
| (재시도 아님) | `crates/engram-dashboard-base/src/writable.rs:86-97` `retry_if_vanished` | base | `NotFound` 일 때 한 번 더 — 검사 도중 폴더가 사라지는 경합 · ★범위에서 뺐다(§8-1 1)★ |
| OS 판정을 인자로 | `src-tauri/src/state/boot.rs:105-110` | 셸 | `fsutil::sweep_temps(dir, targets, engram_dashboard_platform::process::pid_alive)` — ★파일 층이 platform 의 판정을 인자로 받는 꼴이 이미 있다★(§3-1 D 의 선례) |
| 자식 띄우기 | `crates/engram-dashboard-agent/src/transport/stdio.rs:99-137` · stderr `:217-260` | agent | ★가드 없음★ — `GroupOwner::new()?` · `adopt(pid)?`(`:130-133`) 실패면 띄운 자식이 무리 밖에서 산다 |
| 자식 띄우기 | `crates/engram-dashboard-agent/src/backend/codex/transport.rs:1402-1475` · stderr `:3993-4015` | agent | `ChildGuard`(구조체 `:1406` · `Drop` `:1414-1421`)가 첫 `?` 앞(`:1455`)에 선다 · 소스 시험 `:6613` 이 그 배치를 잰다 · ★`Drop` 은 직속 자식만 끈다(§3-7)★ |
| 자식 띄우기 | `crates/engram-dashboard-agent/src/transport/pty.rs:85-105` | agent | ★가드 없음★ — 무리 만들기 · 넣기 · `try_clone_reader` · `take_writer` 넷이 띄운 뒤의 `?` |
| 락 오염 | `settings/mod.rs:466 · 470 · 474` · `state/boot_plugin.rs:119 · 242 · 383(wait_timeout) · 444` · `theme.rs:309 · 338` · `state/restore.rs:246` · `state/placement.rs:189` · ★`layout/apply.rs:247`★ | 셸 | 손으로 쓴 `unwrap_or_else(PoisonError::into_inner)` — 마지막 하나는 처음 범위 목록 밖이었고 메인이 U8 에 넣었다(§8-1 3) |
| 시계 | `src-tauri/src/state/saver.rs:83-100` | 셸 | 자기 `trait Clock: Send + 'static { now; wall_ms }` |
| 레지스트리 저장 | `crates/engram-dashboard-agent/src/profile.rs:304-309` · `:387-407` · `preset.rs:39-44` · `:80-87` | agent | `ProfileStore::save` · `PresetStore::save` 는 `()` — 「실패는 구현 내부에서 로그만 — 호출자를 막지 않는다」 · `mutate` 가 맵에 먼저 적용하고 락을 쥔 채 저장(ADR-0071 · ADR-0207) — ★저장이 실패해도 메모리 변경은 남는다★ |

### 2-2. 동작 차이 (지금)

| 축 | 설정 | 상태 | agents · presets |
|---|---|---|---|
| 버전 없음 | 1 로 본다 | 못 씀 | 파싱 실패 → 손상 |
| 새 판 | 못 씀 → 첫 쓰기가 떠 두고 덮는다 | `NewerVersion` → 부팅이 떠 두고 실행 표식이 덮는다(TRD S21-storage §6-5 · §12 R3 「하향 지원 안 함」) | 빈 목록 · 파일 그대로 → **다음 저장이 덮는다(결함)** |
| 읽기 IO 실패 | 그 쓰기를 실패시킨다(덮지 않음) | 가드 ⅰ — 이번 실행 저장 없음(I3) | 빈 목록 → **다음 저장이 덮는다**(같은 부류 — §8-1 4) |
| BOM | 뗀다 | 뗀다 | 파싱 실패 → 손상 |
| 상한(읽기 = 쓰기) | 64 KiB | 4 MiB | 없음 |
| 손상 사본 | 첫 쓰기 전 `.corrupt` 복사 · 실패면 그 쓰기 실패 | 부팅이 `.corrupt` 복사 · 실패해도 진행(D8) | 적재 때 `.corrupt-<ms>` 로 이동 · 누적 · 실패 로그만 |
| 잠김 재시도(읽기) | 없음 | 있음 | 없음 |
| 임시 이름 · 실패 정리 · 쓸기 | `.tmp<pid>.<n>` · 지움 · 없음 | 같음 · 지움 · 부팅 때 | 고정 `.tmp` · 남김 · 없음 |
| 폴더 만들기 | 첫 쓰기가 만듦(`settings/store.rs:90-91`) | **부팅 쓰기가 만듦**(`state/boot.rs:80-82 · 90-91` — state.json · 크래시 사본) · **기록기는 안 만듦**(같은 주석 `:81`) | 저장마다 만듦(`persistence/mod.rs:51`) |
| 폴더 동기화 | 안 함 | 안 함 | best-effort(Windows 무동작으로 보임) |
| 쓰기 판정을 언제 | 쓸 때마다 다시 읽는다 | 부팅 때 한 번(그 뒤 기록기는 판정 없이 쓴다) | 판정 없음 |

### 2-3. 기록과 어긋난 사실

| 기록 | 실제 |
|---|---|
| ADR-0266 결정 8 · ADR-0275 결정 15 · TRD 1-3 §2-4 · §8 6 의 `fsutil.rs:107` | `:318-321`(`is_lock_contention`) — P3 앞 줄 번호다 |
| ADR-0266 결정 8 둘째 항목 「`set_aside_corrupt` 는 base `file` 에 남는다 — rename 하나로 OS 분기가 없다」 | ADR-0274 가 떠 두기를 복사(`copy_atomic` — 잠김 재시도를 품는다)로 바꿔 거짓이 됐다. ★3판에서 거처는 다시 base `file` 이 된다 — 재시도를 인자로 받아서다(§3-1)★ |
| TRD 1-3 §5-1 U-W 행 · ADR-0266 결정 8 · ADR-0269 「영향」의 사본 목록에 데몬 `usage_service/reject_store.rs` | 없다(ADR-0284 `9ab15231` 가 걷었다) |
| TRD 1-1 §8 D2 「`settings/store.rs:122` epoch ms(`u128`)」 | 없다(ADR-0274 가 시각 이름을 걷었다) |
| 이 TRD 2판 §3-1 의 「ADR-0269 결정 2 의 「무거운 의존을 들이지 않는다」」 | 그 문장은 결정 6 「지킬 것」 첫째 항목이다(리뷰 1 라운드) |
| `docs/reference/structure/session-path-ownership.md:821 · 887-888 · 2000` · `docs/reference/structure/agent-backend.md:237` · 주석 `profile.rs:178 · 2013` · agent `commands.rs:69` 의 `.corrupt-<ms>` · 「밀어낸다」 | 이 TRD 뒤 낡는다(U5a · U9) |

### 2-4. 남길 것

- net `portfile.rs` `write_in_place` — 잡은 핸들로 제자리에 쓴다. rename 은 우리가 쥔 공유 제한에 막힌다(ADR-0135).
- 셸 `state/lock.rs`(3 s 동안 50 ms 간격) — 앞 인스턴스가 **우리 잠금**을 놓기를 기다리는 뜻 있는 기다림이다(근거 = `REPLY_DEADLINE`). 백신 · 색인기의 잠깐 쥐기가 아니라 같은 정책으로 묶지 않는다.
- `src-tauri/src/discovery/layout.rs:235` — 시험 안의 락 오염 복구(운영 아님).
- 셸 `output_channel.rs` — ADR-0231 의 경고 + 독 걷기(ADR-0275 결정 1 의 예외).
- `usage/scratch.rs` `sweep_stale_scratch` — 기동 때 한 번, 재시도 없음.

---

## 3. 설계

### 3-1. 거처 — 결정: base 새 입주자 `file` · OS 몫은 부르는 쪽이 넘긴다

> **사용자 결정(2026-10-10 — §1 5).** 「Base에 넣어야지. 나중에 쪼갤려면 base에서 쪼개야지.」 2판의 A(새 crate)는 철회됐다. 넘기는 모양(`OsHooks` — §3-3)은 이 TRD 의 제안(가안)이다.

하나로 모을 것은 넷이다 — (i) 잠김 판정과 다시 하기 고리 · 폴더 동기화(OS 규칙) · (ii) 버전 붙은 JSON 읽기(BOM · 상한 · 버전 문 · 판정 — `serde_json` 이 필요하다) · (iii) 원자 쓰기 · 복사 · 떠 두기 · 임시 이름 · 쓸기(OS 무관한 장치지만 (i) 을 부른다) · (iv) 「덮어도 되나」 판정(순수 규칙).

**고른 것:** (i) = platform `fs`(§3-2) · (ii)~(iv) = base `file`(§3-3). base 는 platform 을 부르지 못하므로(입주 조건 ② — 워크스페이스 의존 0) `file` 의 IO 함수는 (i) 을 **인자 `OsHooks` 로 받는다** — 재시도 함수 하나 · 폴더 동기화 함수 하나. 운영 값은 crate 마다 상수다(셸 · 데몬 하나씩 · agent 둘 — 적재 읽기의 긴 예산, D7 · §3-4). 의존 = std · `serde_json`(U3 에서).

- **얻는 것:** 규칙 전체(특히 R2 · R8 의 「거절」)가 한 crate 한 모듈에 선다 — 「구현 하나 · 규칙 하나」(1a · 1b) · 새 crate · 새 게이트 없음(워크스페이스 멤버 11 그대로 · 회귀 결과 줄 증가 0) · OS 규칙은 platform 에만 있다(1d · 1f) · ADR-0269 결정 1(base 는 범용 코드를 받는 자리 · 쪼개기는 나중에 base 에서) · 결정 9(바닥 기반층 — Chromium `//base` 가 파일과 JSON 을 담는다)와 맞는다.
- **대가:** ADR-0269 결정 6 「지킬 것」 첫째 항목(base 가 serde 를 안 끌어옴 · `file` 은 바이트만)을 고친다(새 ADR) · base 입주자 여덟(게이트 ③ 정규식 사본 여섯 자리 · CLAUDE.md 개수) · 운영 `OsHooks` 넷(셸 1 · agent 2 · 데몬 1)이 각자 platform 을 이어 붙인다(아래 「D 다시 보기」의 남는 대가).

**base 입주 조건 셋에 대어 보기**(정본 = `crates/engram-dashboard-base/src/lib.rs` 헤더 「입주 조건 셋」):

1. **지금 여러 곳에서 쓰이거나 복사돼 있다** — 원자 쓰기 세 벌(셸 fsutil · agent 둘) · 손상 사본 두 꼴 · 읽기 규칙 네 벌(§2-1). 충족.
2. **도메인 지식 0 · 워크스페이스 의존 0** — `file` 은 에이전트 런타임 · wire 계약 · 데몬 살림을 모른다. 아는 것은 이름 꼴 둘(`<이름>.tmp<pid>.<번호>` · `<이름>.corrupt`)과 버전 문 규칙(키 이름은 인자)이다 — 파일 다루기의 규칙이지 도메인이 아니고, 파일 이름 · 자리 · 스키마는 주인에 남는다(ADR-0264 의 배치 정본도 그대로). platform 을 부르지 않는다 — `OsHooks` 가 있는 까닭이 이 조건이고, 게이트 ①(의존 상한)이 지킨다. ★OS 운영 코드도 없다★(base 헤더 「OS 에 따라 달라지는 운영 코드는 여기가 아니다」) — 폴더 동기화의 런타임 OS 가름은 platform `sync_dir` 로 갔다(R10). 충족.
3. **입주자끼리 무참조** — `file` 은 `time`(시각 이름은 ADR-0274 가 걷었다 — 고정 `.corrupt`) · `sync`(락이 없다 — 임시 번호는 원자 정수) · `writable`(쓰기 프로브 — 따로 선다) · `logging`(로그를 안 낸다) · `text`(fnv1a 는 셸 `codec` 으로 간다 — base 로 오지 않는다) · `path` · `testing` 을 부르지 않는다. 다른 입주자도 `file` 을 부르지 않는다. **위반 없음.** 주의 둘 — ① `file` 을 하위 모듈로 나누면 그 안에서 `crate::file::…` 로 부르지 말고 `super::` 로 부른다(게이트 ③ 이 그 꼴을 형제 참조로 오탐한다 — base 헤더 「알려진 한계」) ② `file` 의 시험도 `crate::testing` 을 부르지 않는다(지금 fsutil 시험은 std 만 쓴다 — 실측).

**D 다시 보기 — 2판이 이 모양을 기각한 사유 둘과 지금:**

1. **「base 에 어차피 `serde_json` 이 든다」** — 2판은 이것을 ADR-0269 결정 6 「지킬 것」(무거운 의존을 들이지 않는다 — base 가 serde 를 안 끌어옴)을 어기는 대가로 셌다(2판은 「결정 2」로 잘못 인용했다). → 사용자가 그 「지킬 것」을 고친다(§1 5). 무게도 실측으로 작다 — base 를 운영 의존으로 부르는 셋(셸 · agent · 데몬)이 이미 `serde_json` 을 끌고 있어(`cargo tree --locked -i engram-dashboard-base` 와 각 `-p … -e normal`, 2026-10-10) `Cargo.lock` 새 패키지가 0 이다. 들이는 것은 `serde_json` 하나(읽기 판정이 `Value` 를 본다)이고 `serde` derive · ts-rs 는 아니다 — 구조체 ↔ JSON 직렬화는 여전히 주인이 한다. base 가 무거워지면 기능 플래그로 나눈다는 ADR-0269 「영향」의 길도 그대로 열려 있다.
2. **「부를 때마다 platform 재시도를 들고 와야 하는 이음매는 의존을 피하려고만 생긴다」** — 둘 다 과했다.
   - 「부를 때마다」가 아니다 — 이음매는 `OsHooks` 값 하나이고 crate 마다 상수로 끝난다(셸 · agent · 데몬 = 셋 — agent 만 적재 읽기 몫 하나 더, D7). 주인은 그 상수를 넘기기만 한다.
   - 「의존을 피하려고만」이 아니다 — (a) 이 저장소에 같은 꼴이 이미 있다: 셸 `sweep_temps` 는 platform 의 PID 생존 판정을 인자로 받는다(`state/boot.rs:105-110`). (b) 그 이음매가 그대로 시험 이음매다: base `file` 시험은 가짜 재시도(자지 않고 잠김을 주입 · 시도를 센다)로 「재시도 도중 skip」 같은 길을 실제 OS 잠금 없이 잰다 — 지금 fsutil 이 `pause` 클로저로 하는 일이다(ADR-0012). 상태를 쥔 가짜는 공개 `OsHooks`(fn 포인터)가 아니라 base 안 비공개 이음매(클로저 인자)로 넘긴다(§3-3 · D15).
   - **남는 대가(정직하게):** 운영 `OsHooks` 넷이 각자 platform 을 이어 붙이므로, 아무것도 안 하는 재시도를 넘겨도 컴파일은 선다 → crate 마다 「운영 `OsHooks` 의 재시도가 잠김 오류를 실제로 다시 한다」 시험 하나(§5-2). 재시도 함수가 `fn` 포인터 하나라 시도의 값 타입을 지워야 한다(값을 `Option` 에 담아 꺼낸다 — §3-3).
   - 그래서 D 를 고른다. ADR-0266 결정 8 이 원자 쓰기를 platform 으로 보낸 근거(함수 안의 OS 분기 한 줄)도 이 꼴에서 사라진다 — 그 줄은 platform `is_busy` 로 떨어지고 base 는 그 결과를 받기만 한다.

**거부한 대안(새 ADR 「거부한 대안」의 재료):**

- **A — 새 crate `engram-dashboard-datafile`(2판의 결정).** 기각 = 사용자 결정(2026-10-10 — 「Base에 넣어야지. 나중에 쪼갤려면 base에서 쪼개야지.」). ADR-0269 결정 1 이 미룬 「목적별 작은 crate 를 지금 나눈다」를 이 자리에서 먼저 하는 셈이었다. 2판이 셌던 대가(워크스페이스 멤버 11 → 12 · 등록 게이트 넷 · 결과 줄 +2)도 사라진다.
- **B — 기존 둘에 나눠 넣기.** (iii) 은 platform `fs`(ADR-0266 결정 8 을 그 가족으로 넓힘) · (ii) · (iv) 는 base `file`. 기각 = **ADR-0266 결정 2(platform 의 판정 기준 = OS 의존 여부)** — (iii) 은 OS 무관 장치다(원자 쓰기 · 복사 · 떠 두기 · 임시 이름 · 쓸기 — 그 안의 OS 몫인 잠김 판정 한 줄은 `is_busy` 로 이미 platform 에 떨어졌다 · §3-2). ★메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기(리뷰 2 라운드 D12)★: 3판이 함께 들었던 사유 둘(「`.corrupt` 이름은 도메인 지식이다」 · 「부르는 쪽이 두 crate 를 각자 잇는다」)은 뺐다 — base 를 고른 근거(조건 ② 의 「이름 꼴 둘은 파일 다루기의 규칙이지 도메인이 아니다」 · `OsHooks` 를 부르는 쪽이 넘긴다)와 서로 어긋났다.
- **C — 전부 platform 에 + `serde_json` 허용.** 기각 = platform 의 서드파티 규칙(`windows` · `tracing` 뿐 — 그 crate 헤더 「의존」)이 깨지고 OS 층에 JSON 정책이 든다.
- **D′ — 판정 함수(`is_busy`)만 넘기고 다시 하기 고리와 예산은 base `file` 이 쥔다.** 기각 = platform `retry_busy` 는 scratch(U6) · net(transport 3-3)의 몫으로 남으므로 고리가 둘이 되고, 1d(「잠깐 쥐어졌다」 재시도 = platform 정책 하나)를 어긴다.

### 3-2. platform `fs` — 잠김 판정 · 다시 하기 · 폴더 동기화 (가안)

```rust
/// 남이 잠깐 쥐어서 난 실패인가 — 지금 fsutil `is_lock_contention` 의 뜻 그대로:
/// ErrorKind::PermissionDenied(모든 OS · Windows 원시 5 포함) · Windows 원시 32(공유 위반) · 33(잠금 위반).
pub fn is_busy(e: &io::Error) -> bool;

/// 첫 시도 + `retries` 번. 값은 부르는 쪽이 정한다(도메인 값은 인자로 — 들이는 규칙 1).
#[derive(Clone, Copy, Debug)]
pub struct Retry { pub retries: u32, pub pause: Duration }

/// `attempt` 를 한 번 하고, `is_busy` 면 `pause` 뒤 `retries` 번까지 다시. 잠김 아닌 오류 · 성공은 바로 돌려준다.
/// 마지막 시도 뒤에는 자지 않는다. 한도를 넘으면 마지막 잠김 오류 그대로. 로그 없음(들이는 규칙 4).
pub fn retry_busy<T>(retry: Retry, attempt: impl FnMut() -> io::Result<T>) -> io::Result<T>;
/// 기다리기를 받는 판 — 부르는 쪽 시험이 실제로 자지 않게(지금 fsutil 시험의 이음매).
pub fn retry_busy_with<T>(retry: Retry, attempt: impl FnMut() -> io::Result<T>, pause: impl FnMut(Duration)) -> io::Result<T>;

/// 폴더 안 이름 바꾸기(rename)를 영속시킨다 — POSIX = 폴더를 열어 `sync_all` · Windows = 명시 무동작(`Ok`).
pub fn sync_dir(dir: &Path) -> io::Result<()>;
```

- `cfg!(windows)` 는 `is_busy` 한 줄로 platform 안에 들어온다 — 셸 `fsutil.rs:320` 이 걷힌다(1f).
- `sync_dir` 의 OS 가름은 `#[cfg]` 로 platform 안에 둔다 — 2판 R10 은 그 가름을 「std `File::open` 이 Windows 에서 폴더를 못 연다」는 **`cfg` 없는 런타임 OS 가름**으로 두었고, 그것은 platform 헤더 게이트 ④ 의 「한계」가 「`cfg` 없이 런타임에 OS 를 가르는 자리는 못 본다 — 그런 규칙의 자리도 이 crate 다」라는 뜻으로 적어 둔 바로 그 꼴이다(리뷰 1 라운드). Windows 무동작은 지금 agent best-effort 의 실제 결과와 같다 — 동작은 바뀌지 않고 명시가 된다.
- net `instance.rs` 는 A 에서 이것을 부르지 않는다(transport 3-3 후속 — §3-6). 그때 옮겨도 net 의 「공유 위반만」과 뜻이 갈리지 않는다 — net 은 접근 거부(5)를 고리에 넣기 전에 따로 처리한다(읽기 전용 속성 1회 걷기 · ACL 이면 바로 `AccessDenied`). 새로 다시 하기에 드는 것은 33 하나고, 열기에서는 사실상 나지 않는다.
- `ERROR_DIR_NOT_EMPTY`(145)는 넣지 않는다 — scratch 지우기가 실제로 어떤 코드로 실패하는지 모른다(§8-2 1 · U6 파일럿).
- 시험: fsutil 의 판정 시험 둘(`fsutil.rs:392-431` — 원시 코드 · `cfg!(windows)` 기대 — 이 둘이 fsutil 을 4f 명단에 붙드는 시험 분기다)과 다시 하기 시험 셋(`:358-391` · `:433-447`)을 platform 으로 옮긴다 · `sync_dir` 시험 하나(있는 폴더 = `Ok` · 없는 폴더 = Windows `Ok` / 그 밖 `NotFound` — 기대값이 OS 로 갈리는 시험 분기는 platform 안이라 4f 와 무관).

### 3-3. base `file` — 데이터 파일 규칙 (가안)

```rust
// std + serde_json(U3 부터). platform 을 부르지 않는다(입주 조건 ②) — OS 에 따라 갈리는 둘은 부르는 쪽이 넘긴다(§3-1).

/// OS 에 따라 갈리는 둘 — 운영은 platform `fs` 의 것을 잇는다(crate 마다 상수 — §3-4).
/// ★fn 포인터인 것은 운영 값을 `const` 로 두기 위해서다 — 상태를 쥔 시험 가짜는 이것이 아니라 아래 비공개 이음매로 넘긴다(D15)★.
#[derive(Clone, Copy)]
pub struct OsHooks {
    /// 잠깐 쥐어진 파일의 다시 하기 — `attempt` 를 잠김이 풀리거나 예산이 다할 때까지 부르고 마지막 결과를 돌려준다.
    /// 운영 = platform `fs::retry_busy`(예산 = 아래 `BUSY_RETRIES` · `BUSY_PAUSE` — agent 적재 읽기만 따로 · R14).
    pub retry: fn(&mut dyn FnMut() -> io::Result<()>) -> io::Result<()>,
    /// 부모 폴더 동기화(R10) — 운영 = platform `fs::sync_dir`. 결과는 보지 않는다.
    pub sync_dir: fn(&Path) -> io::Result<()>,
}
/// 데이터 파일의 잠김 예산(R14) — 규칙의 집이 여기다. 운영 `OsHooks` 가 이 값으로 platform `Retry` 를 만든다.
pub const BUSY_RETRIES: u32 = 5;
pub const BUSY_PAUSE: Duration = Duration::from_millis(20);

// 비공개 이음매(D15 — 밖에 열지 않는다). 공개 IO 함수는 `OsHooks` 를 이 꼴로 풀어 `_with` 판에 넘기는 한 줄이고,
// base 자기 시험은 상태를 쥔 클로저(시도 세기 · 잠김 주입 · 자지 않기 · `sync_dir` 호출 기록)를 `_with` 에 바로 넘긴다.
type RetryHook<'a> = &'a mut dyn FnMut(&mut dyn FnMut() -> io::Result<()>) -> io::Result<()>;
type SyncDirHook<'a> = &'a mut dyn FnMut(&Path) -> io::Result<()>;
fn write_atomic_with(path: &Path, bytes: &[u8], retry: RetryHook, sync_dir: SyncDirHook, skip: &dyn Fn() -> bool) -> io::Result<WriteOutcome>;
// copy_atomic · read_file_capped 도 같은 꼴의 `_with` 를 둔다.

/// 파일마다 하나 — 주인(저장소)이 상수로 둔다(§3-4).
pub struct Spec {
    pub version_key: &'static str, // "$version" | "version" | "schema_version"
    pub current: u64,              // 이 빌드가 쓰는 판
    pub cap: u64,                  // 읽기 상한 = 쓰기 상한(바이트)
    /// 주인의 모양 검사(D1) — 버전 문을 지난 문서가 그 주인이 읽을 수 있는 모양인가. `Err(사유)` = 손상(`Unusable`).
    /// ★`parse` 가 부른다 — `classify` · `load` · `judge` 가 모두 `parse` 를 지나므로 모양을 건너뛰는 판정 길이 없다★.
    /// 받는 것 = 판(1 ..= current) · 문서. 주인은 자기 파일 구조체를 그 `&Map` 에서 바로 역직렬화하고 값은 버린다
    /// (serde_json 은 `&Map` 을 `Deserializer` 로 받는다 — 1.0.150 `value/de.rs:749`).
    pub shape: fn(version: u64, doc: &serde_json::Map<String, serde_json::Value>) -> Result<(), String>,
}
/// 모양을 따로 보지 않는 주인(설정 — 키마다 관용)의 `shape` — 늘 `Ok`.
pub fn any_shape(version: u64, doc: &serde_json::Map<String, serde_json::Value>) -> Result<(), String>;

/// 원문 하나를 본 결과 — IO 는 모른다.
pub enum Parsed {
    /// 1 ..= current · 모양 검사 통과. version < current 면 그 판 리더는 부르는 쪽 몫(지금 넷 다 current = 1 이라 없다 — R3).
    Usable { version: u64, doc: serde_json::Map<String, serde_json::Value> },
    Unusable(String),     // 손상 — 사유 문구(값은 싣지 않는다) · 모양 실패 포함
    Newer { found: u64 }, // 이 빌드보다 새 판 — 모양은 보지 않는다(새 판은 모양부터 다를 수 있다 — `codec.rs:104` 그대로)
}
pub fn parse(text: &str, spec: &Spec) -> Parsed; // BOM 하나 떼기 · JSON 객체 · 버전 문(R1~R5) · `spec.shape`(R7)

/// 파일 하나를 읽은 결과.
pub enum Loaded { Missing, Parsed(Parsed), Failed(io::Error) }
/// 읽기 결과를 판정으로 — NotFound = Missing · InvalidData(상한 · UTF-8) = 손상 · 그 밖 오류 = Failed · Ok = parse.
/// ★셸의 IO 이음매(설정 `SettingsFiles` · 상태 `BootFiles`/`StateFiles` 의 가짜)는 자기 읽기 결과를 이것에 넣는다★.
pub fn classify(read: io::Result<String>, spec: &Spec) -> Loaded;
pub fn load(path: &Path, spec: &Spec, os: OsHooks) -> Loaded; // = classify(read_file_capped(path, spec.cap, os), spec)

/// 이 실행이 그 파일을 이미 자기 것으로 잡았나(D14) — 적재 판정이 `Usable` 였거나 이번 실행이 그 파일에 한 번 썼으면
/// `Adopted`. 적재가 Missing · 손상이었고 아직 한 번도 안 썼으면 `NotYet`(그 사이 놓인 쓸 만한 파일은 우리 것이 아니다).
pub enum Claim { Adopted, NotYet }
/// 그 파일에 지금 써도 되나 — 네 파일 공통 규칙 하나(R8). ★쓰기마다 그 직전에 다시 판정한다(R16)★.
pub enum WritePolicy { Write, CopyAsideFirst, Refuse(Refused) }
pub enum Refused { Newer { found: u64 }, ReadFailed }
impl Loaded { pub fn write_policy(&self, claim: Claim) -> WritePolicy; }
pub fn judge(path: &Path, spec: &Spec, os: OsHooks, claim: Claim) -> WritePolicy; // = load(..).write_policy(claim) — 경로로 바로 읽는 주인(agent)용
pub fn check_cap(len: usize, spec: &Spec) -> io::Result<()>;     // 쓰기 상한(R6)

// 쓰기 · 읽기 장치 — fsutil 에서 옮긴다(바이트를 받는다 · 셸의 &str 은 as_bytes).
pub fn write_atomic(path: &Path, bytes: &[u8], os: OsHooks) -> io::Result<()>;
pub fn write_atomic_unless(path: &Path, bytes: &[u8], os: OsHooks, skip: impl Fn() -> bool) -> io::Result<WriteOutcome>;
pub enum WriteOutcome { Written, Skipped }
pub fn copy_atomic(from: &Path, to: &Path, os: OsHooks) -> io::Result<()>;
pub fn copy_aside(path: &Path, os: OsHooks) -> io::Result<PathBuf>; // `<이름>.corrupt` — ADR-0274
pub fn sweep_temps(dir: &Path, targets: &[&str], is_alive: impl Fn(u32) -> bool) -> io::Result<Vec<(PathBuf, io::Result<()>)>>; // 그대로 — OS 판정은 이미 인자다
pub fn read_capped(source: impl io::Read, cap: u64) -> io::Result<String>;
pub fn read_file_capped(path: &Path, cap: u64, os: OsHooks) -> io::Result<String>;
```

- **로그를 내지 않는다** — 값을 돌려주고 부르는 쪽이 자기 문구로 찍는다(지금 fsutil · codec 꼴 그대로).
- **아는 이름은 두 꼴뿐이다** — 임시 `<이름>.tmp<pid>.<번호>` · 사본 `<이름>.corrupt`(지금 fsutil 머리 그대로). 파일 이름 · 자리 · 스키마 · 「항목 하나가 못 쓸 때」 관용 · 옛 이름(`agents.json.tmp` 같은 것)은 주인이 쥔다.
- **잠김 재시도는 `os.retry` 한 길로만 지난다** — 열기 · 읽기(읽다 잠기면 새로 연다 — 지금 `read_capped_retrying`) · rename(시도마다 `skip` 을 먼저 묻는다 — 지금 `replace_with`) · 복사 원본 열기. 시도의 값 타입은 `Option` 에 담아 꺼낸다 — 재시도 함수가 `Ok` 를 돌려줬는데 값이 없으면(시도를 안 불렀다) `ErrorKind::Other` 오류다(패닉하지 않는다).
- `classify` 와 `load` 를 둘 다 두는 까닭 — 셸은 시험 가짜를 위해 IO 를 트레이트 뒤에서 하고(ADR-0012), agent 는 경로로 바로 읽는다. 판정 규칙은 `classify` 하나다(지금 `codec::unusable_read` 와 설정의 `RawFile` 가 같은 가름을 두 번 한다).
- **`#[non_exhaustive]` 고려(리뷰 3 라운드 잔손질 · 정하는 것 = U3 에서 메인)** — `Parsed` · `Loaded`(와 같은 처지의 `WritePolicy` · `Refused`)는 base 밖 주인 셋(셸 · agent · 데몬)이 match 한다. 붙이면 주인이 `_` 갈래를 둬야 해서 base 가 변형을 더해도 주인 빌드가 안 깨지는 대신 새 변형이 그 갈래로 말없이 떨어진다 · 안 붙이면 변형 추가가 주인 match 를 깨 같은 변경에서 고치게 된다(한 워크스페이스라 그 비용은 같은 커밋 안이다). 쓰기 판정처럼 빠뜨리면 데이터를 덮는 갈래는 뒤쪽이 안전 쪽이다 — U3 지시서에 이 대가를 싣는다.
- **모양 검사는 `Spec` 에 산다(D1 — 메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기)** — 버전 문만 보고 「쓸 수 있음」이라 답하는 맨 판정이 API 에 없다. 그래서 적재 · 저장 직전 재판정(agent) · 상태 기록기 재판정(머리 해석까지)이 같은 손상 정의를 쓴다. 3판은 재판정이 버전 문까지만 봐, 모양이 깨진 파일(항목 하나의 모르는 `kind` — T-33 류 · 상태 머리 해석 실패)을 「쓸 수 있음」으로 보고 `.corrupt` 없이 덮었다 — 지금 agent 는 그런 파일을 적재 때 떠 두므로(`persistence/mod.rs:113` · `:123-127`) 회귀였다. 비용 = 판정마다 주인 구조체 역직렬화 한 번(상한 안) · 적재 때는 주인이 값을 얻으려고 한 번 더 한다(검사가 값을 버리므로 — 지금 명부 1,747 B).
- **비공개 이음매(D15 — 메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기)** — 공개 `OsHooks` 는 fn 포인터 그대로(운영 값이 crate 마다 `const` 다). 상태를 쥔 시험 가짜(지금 fsutil 시험의 `pause` 클로저 · 시도 세기 · 잠김 주입)는 base 안 비공개 `_with` 판에 클로저로 넘긴다 — base 자기 시험만 쓴다. 주인 crate 의 시험은 재시도 가짜가 필요 없다: 셸은 자기 IO 트레이트 가짜(`SettingsFiles` · `BootFiles` · `StateFiles`)를, agent 는 실제 임시 폴더를 쓴다. 운영 상수의 「재시도가 실제로 다시 한다」 시험(§5-2)은 `(OS_HOOKS.retry)(&mut 시도)` 로 부르며 상태는 시도 클로저가 쥔다.
- **판정 표(R8 · D14 — `NotYet` 열은 메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기)** — `write_policy(claim)`:

  | 지금 파일 | `Adopted` | `NotYet` |
  |---|---|---|
  | Missing | `Write` | `Write` |
  | 쓸 수 있음(모양 포함) | `Write` | **`CopyAsideFirst`**(D14 — 적재 뒤 누가 놓은 쓸 만한 파일 · 사본 이름은 ADR-0274 의 고정 `.corrupt` 그대로라 내용이 멀쩡해도 그 이름이다 — 주인의 로그가 「쓸 만한 파일을 떠 뒀다」로 가른다) |
  | 손상(모양 실패 포함) | `CopyAsideFirst` | `CopyAsideFirst` |
  | 새 판 · 읽기 실패 | `Refuse` | `Refuse` |

- **저장 직전 재판정(R16)은 주인이 부르는 순서다** — 읽기(`load`/`judge` 또는 자기 IO 이음매 + `classify`) → `write_policy(claim)` → 떠 두기 → 쓰기 → 성공이면 `claim = Adopted`. 판정 표는 `file` 하나에 있고, 순서는 몇 줄이라 주인마다 쓴다(떠 두기 실패의 처리가 상태만 다르다 — R8). `claim` 을 쥐는 자리 = agents · presets 는 저장소 상태 칸(R12) · 상태는 기록기(부팅 판정과 ⑤ 의 결과를 받는다 — §3-6) · 설정은 해당 없음(쓸 때마다 지금 파일 위에 바꾼 키만 얹는다 — `settings/store.rs:284-286` — 그 파일을 덮지 않으므로 늘 `Adopted` 로 부른다).
- **하위 모듈로 나누면 `super::` 로 부른다**(§3-1 조건 ③ 주의).

### 3-4. 파일별 `Spec` · 운영 `OsHooks`

| 파일 | 주인 | 버전 키 | `current` | 상한 | 모양 검사(`shape` — D1) | 손상 사본 실패 때 |
|---|---|---|---|---|---|---|
| `shell\config\settings.json` | 셸 `settings/store.rs` | `$version` | 1 | 64 KiB(그대로) | `any_shape` — 키마다 관용(못 쓸 값은 그 키만 기본값 · `LoadNote::BadValue`) · 지금 그대로 | 그 쓰기 실패 · 원본 그대로(그대로) |
| `shell\state\state.json` · `state.crash.json` | 셸 `state/codec.rs` | `version` | `STATE_VERSION` = 1 | 4 MiB(그대로) | 머리 해석(`Head` — 지금 `codec.rs:116` 의 `Head::deserialize`) · 창 하나하나는 관용(그 창만 건너뜀) · 지금 그대로 | 진행(D8 그대로) |
| `daemon\state\agents.json` | agent `persistence/mod.rs` | `schema_version` | 1 | **16 MiB(새)** | 명부 칸(`profiles` 전체) 역직렬화 · 지금 「파싱 실패 = 손상」(`persistence/mod.rs:113`) 그대로. ★읽기 쪽 구조체는 버전 키를 필수로 두지 않는다★ — 지금 `ProfilesFile`(`:27-30`)은 `schema_version` 이 필수라 그대로 쓰면 R1(버전 없음 = 1 판)이 모양 검사에서 손상으로 뒤집힌다 · 버전 문은 `parse` 가 이미 봤다 · 쓰기 쪽은 지금처럼 `schema_version` 을 싣는다 | 그 저장 실패 · 원본 그대로(새) |
| `daemon\state\presets.json` | agent `persistence/presets.rs` | `schema_version` | 1 | **4 MiB(새)** | 같은 꼴(`presets` 칸 — 지금 `PresetFile` `presets.rs:24-27` 도 `schema_version` 필수라 같은 주의) | 같음(새) |
| 데몬 run 아래 mcp-config · 세션 설정 조각 | 데몬 `control/mcp_config.rs` | — (claude 가 읽는 형식 · 버전 문 없음) | — | — | — (읽지 않는다) | — (원자 쓰기만 — 1e) |

**운영 `OsHooks` = crate 마다 상수**(가안 모양 — `retry: |a| platform::fs::retry_busy(platform::fs::Retry { retries: file::BUSY_RETRIES, pause: file::BUSY_PAUSE }, a)` · `sync_dir: platform::fs::sync_dir`): 셸 한 곳(설정 · 상태가 함께 쓴다 — 자리 · 이름은 U2 에서) · agent `persistence` 둘 · 데몬 `control/mcp_config.rs`. ★agent 만 둘이다(D7 — 메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기)★: 저장 · 저장 직전 재판정 · 쓸기는 위 데이터 파일 예산, **적재 읽기는 따로 긴 예산**(R14 — 상수는 agent `persistence` 에 둔다: 적재 거절이 실행 내내 남는 것은 agent 의 사정(R17)이라 규칙의 집이 base 가 아니다). 셋 다 이미 base · platform 을 직접 의존한다(`cargo tree --depth 1`, 2026-10-10) — 새 간선이 없다.

### 3-5. 하나로 정한 규칙 — 세부 (메인 판단(위임) · 채택 2026-10-10 · R16 · R17 = 메인 결정 2026-10-10)

> 사용자 위임(2026-10-10 「성숙 프로그램 관행대로 · 메인이 정하고 보고」) → **메인이 아래 권고를 채택했다(2026-10-10)**. R2 · R7 · R8 은 메인이 따로 다시 확인했다(§8-1 2 · 4). R16 · R17 은 리뷰 1 라운드 취합에서 메인이 정했다(첨부). ★3판에서 고친 R10 · R11 · R12 · R14 · R15 의 바뀐 부분은 이 TRD 의 제안이다 — 재리뷰에서 메인이 채택한다★. ★4판에서 리뷰 2 라운드의 메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기가 R7(D1) · R8(D14) · R12(D2 · D6 · D14 의 칸) · R14(D7) · R16(C1 · D4) · R17(D2 · D6)에 들었다 — 각 자리에 표시했다★. ★5판에서 리뷰 3 라운드의 같은 꼴 결정이 R16(단일 인스턴스 뮤텍스의 단위 — 잔손질) · R17(E1)에 들었다★. 한 줄 근거 = 관행 또는 지금 확정된 셸 규칙. 관행 인용은 기억에 기댄 것이 있다(§8-2 3) — 결론이 그 인용 하나에 기대는 줄은 없다.

| id | 규칙 | 근거 한 줄 |
|---|---|---|
| R1 | **버전 없음 = 1(첫 판)** | 버전 표지가 나중에 붙은 형식은 「표지 없음 = 가장 옛 판」으로 읽는 것이 관행(Cargo.lock 이 `version` 줄 없는 파일을 옛 판으로 읽는다 — 기억 인용 · 미검). 지금 설정 규칙 그대로 · 상태 · agent 는 「손상」이던 것이 읽힌다(§4) |
| R2 | **새 판(`found > current`) = 읽지 않고 덮지 않고 떠 두지 않는다 — 네 파일 모두.** 메모리는 기본값 · 빈 목록, 그 파일 쓰기는 거절 — 설정은 쓸 때마다 다시 읽으므로 파일이 바뀌면 풀리고, 상태는 그 실행 동안(가드), agents · presets 는 그 실행 동안 읽기 전용(R17). ★메인 판단(위임 「성숙한 프로그램 관행대로」) — 관행 = 새 판이 쓴 파일을 옛 판이 덮지 않는다★. TRD S21-storage §6-5 · §12 R3(새 판 `state.json` 은 떠 두고 덮는다)를 뒤집고 ADR-0265 결정 4 · ADR-0274(「모르는 `$version` · `version` = 통째로 못 쓸 파일」)를 좁힌다 — 새 ADR 에 든다(§7) | SQLite 는 머리의 「쓰기 판」이 자기보다 크면 그 DB 를 읽기 전용으로 연다 · Firefox 는 새 판이 쓴 프로필을 옛 판이 열지 않는다(둘 다 기억 인용 · 미검). 상태의 크래시 사본은 이미 이렇게 한다(N6) — 그것을 넷으로 넓힌다. **알려진 결함을 닫는다** |
| R3 | **앞 판(`1 ≤ found < current`) = 그 판 리더가 있으면 읽고 없으면 손상** — 지금 넷 다 `current = 1` 이라 해당 없다. `version` 은 앞 판 리더와 함께만 올린다 | TRD S21-storage 「칸 더하기 규칙」(상태 파일)을 네 파일로 |
| R4 | **버전 값 꼴 = 정수 값인 수만**(`1` · `1.0`) — 0 · 음수 · 소수 · 문자열 · `u64` 밖 = 손상 | 지금 상태 `version_number`(`codec.rs:158-170`) 그대로 — JSON 도구가 수를 실수로 다시 쓴다 |
| R5 | **BOM = 앞머리 U+FEFF 하나를 무시 · 쓰지 않는다** | RFC 8259 §8.1(파서는 BOM 을 무시할 수 있고 쓰는 쪽은 붙이지 않는다). 셸 둘 그대로 · agent 는 새로 받는다 |
| R6 | **상한 = 읽기 상한 = 쓰기 상한** — 넘는 원문은 쓰지 않는다. 설정 64 KiB · 상태 4 MiB 그대로 · agents.json 16 MiB · presets.json 4 MiB 새로 | 셸 I4(넘게 쓰면 다음 적재가 통째로 못 쓴다) · 값은 관행이 아니라 실측 비례(이 PC 의 agents.json 1,747 B — 정상이면 안 닿고 쓰레기 파일로부터 메모리를 지키는 선 · 설정 · 상태와 같은 방식) |
| R7 | **손상 = JSON 아님 · 객체 아님 · 상한 초과 · UTF-8 아님 · 버전 꼴 틀림(R4) · 리더 없는 앞 판(R3) · 주인 모양 검사 실패(`Spec::shape` — §3-3 · §3-4).** ★모양 실패는 적재 · 저장 직전 재판정 · 상태 기록기 재판정 어디서나 손상이다(D1 — 메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기)★ · ★읽기 IO 실패는 손상이 아니다 — 거절(R8)이고, 적재 때면 agents · presets 는 읽기 전용(R17)★ | 셸 I3 를 넷으로 — agent 의 「읽기 실패 → 빈 목록 → 덮음」이 함께 닫힌다(§8-1 4). 모양을 판정 밖에 두면 재판정이 모양 깨진 파일을 「쓸 수 있음」으로 덮는다(리뷰 2 라운드 D1 — 지금 agent 는 적재 때 떠 둔다) |
| R8 | **쓰기 판정: Missing · 쓸 수 있음 → 쓴다 · 손상 → 떠 두고(R9) 쓴다 · 새 판 · 읽기 실패 → 거절.** ★판정은 쓰기마다 그 직전에 지금 파일로 한다(R16)★. ★단 적재가 Missing · 손상이었고 아직 한 번도 안 쓴 동안(`Claim::NotYet`)에 쓸 만한 파일이 있으면 떠 두고(R9) 쓴다(D14 — 메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기 · 표 = §3-3)★. 떠 두기 실패 = 설정 · agents · presets 는 그 쓰기 실패(원본 그대로) · 상태만 진행 | ADR-0274 「영향」 대가 2 그대로(상태 = 사용자 결정 D8) · agent 는 설정 쪽(데이터를 사본 없이 덮지 않는다)을 따른다. D14 = 메모리가 그 파일에서 오지 않았는데 실행 중 누가 되살려 둔 파일을 사본 없이 덮지 않는 싼 보호(C1 의 감지 장치는 두지 않는다 — R16) |
| R9 | **손상 사본 = ADR-0274 그대로** — 고정 `<이름>.corrupt` · 복사(원본은 그 자리) · 덮어쓰기 · 첫 쓰기 직전 · 한도 · 중복 생략 · 시각 이름 없음 | 사용자 결정 1c. agent 는 시점이 「적재 때 이동」에서 「첫 저장 직전 복사」로 바뀐다 |
| R10 | **원자 쓰기 = 같은 폴더 임시 → 담고 `sync_all` → rename(R13 · R14) → 부모 폴더 동기화(`OsHooks::sync_dir` — 운영 = platform `fs::sync_dir` · 결과 무시)** · 실패 · 건너뜀이면 임시를 지운다 | POSIX 에서 rename 의 내구성은 부모 폴더 fsync 가 맡는 것이 관행(LevelDB · SQLite 의 유닉스 경로 — 기억 인용 · 미검). Windows 는 명시 무동작이다 — 지금 agent 의 best-effort 도 실제로는 무동작이고(std 가 폴더를 못 연다), SQLite 의 Windows VFS 도 폴더를 동기화하지 않는다(기억 인용 · 미검). 그 OS 가름은 platform 에 둔다(리뷰 1 라운드 — `cfg` 없는 런타임 OS 가름도 platform 몫) |
| R11 | **폴더 = 쓰는 쪽이 쓸 때 만든다(`write_atomic` · `copy_atomic` 안 `create_dir_all`) · 적재 · 판정은 아무것도 만들지 않는다.** ★지금과 달라지는 것★: 부팅 쓰기 · 설정 쓰기 · agent 저장 · mcp 는 이미 만든다(§2-2) — 새로 생기는 것은 ① 상태 기록기가 실행 중 지워진 폴더를 루트까지 다시 만든다 ② 떠 두기(`copy_atomic`)도 그렇다. `state/boot.rs:81` 의 주석 「기록기는 만들지 않는다」와 부딪힌다 → **만든다로 정한다**(메인 판단(위임 — 범위 결정 4 의 「폴더 만들기」) 제안) · 그 주석과 쓰기 앞 `create_dir_all` 들(`boot.rs:82 · 91` · `settings/store.rs:90-91` · `persistence/mod.rs:51` · `presets.rs` 같은 자리 · `mcp_config.rs:112-114 · 159-161`)은 각 채택 단위가 걷는다 | ADR-0265 결정 4 · 설정 저장소 주석(「적재가 만들면 안 되므로 첫 쓰기가 만든다」)을 넷으로. 「기록기는 안 만든다」로 두면 실행 중 폴더가 사라졌을 때 기록기가 디바운스마다 실패해 이번 실행의 화면 상태를 잃을 뿐 지키는 것이 없다 · 쓰기 직전에 폴더를 만드는 것이 흔한 처리다(mkdir -p — 기억 인용 · 미검). 대가 = 실행 중에 데이터 폴더를 통째로 지워 초기화하면 떠 있는 셸이 다시 만든다(데몬 저장소는 지금도 저장마다 다시 만든다) — 초기화는 앱을 끈 뒤에 한다 |
| R12 | **임시 이름 = `<이름>.tmp<pid>.<번호>`**(지금 셸). 남은 임시는 그 파일 주인이 기동 때 · 첫 쓰기 전에 쓴다(산 남의 pid 는 둔다 — `is_alive` = platform `process::pid_alive`). 상태 = 지금 그대로 부팅 · agents · presets = `load` 안 — 데몬은 단일 인스턴스 가드 뒤에 레지스트리를 만든다(`daemon/src/lib.rs:494` → `:659`). **옛 고정 이름 `agents.json.tmp` · `presets.json.tmp` 는 저장소가 같은 자리에서 직접 지운다**(없으면 무시) — `sweep_temps` 는 번호 없는 이름을 임시로 보지 않으므로(`fsutil.rs:212-221` — 「남의 것일 수 있는 파일은 지우지 않는다」) 그 API 는 그대로 둔다. **`write_lock` 은 걷는다** — 대신 저장소가 새로 갖는 상태(R17 의 적재 거절 · 마지막 저장의 재판정 거절(D6) · D14 의 `Claim` · R15 의 보고 표시)를 잎 락 하나 **「저장소 상태 칸」**(가안 `state: Mutex<StoreState>`)에 둔다 — 쥔 채 IO · 로그 · 밖 호출을 하지 않는다 · 락 순서 = `래치 → expected 칸 → profiles(또는 presets) → 저장소 상태 칸`. ★D2 의 dirty 는 이 칸이 아니라 레지스트리 맵 옆(같은 profiles · presets 락)이다★ — 「메모리가 디스크보다 앞섰다」는 레지스트리만 아는 사실이고(저장소는 호출자가 실패 뒤 커밋했는지 모른다) 그 칸을 바꾸는 자리는 전부 이미 그 락을 쥔 `mutate` 안이라 새 락 · 새 간선이 없다(§3-9) | 이름이 겹치지 않으면 동시 쓰기가 서로의 반쪽을 갈아끼우지 않는다(지금 fsutil `temp_path` 머리). 옛 이름은 이름으로 우리 것이 확정되고 옛 코드가 매번 덮던 파일이다 — 그 지식(우리 옛 이름)은 주인의 것이라 `file` 에 넣지 않는다. 레지스트리가 이미 자기 락 안에서 저장하고(`profile.rs:387-407` · `preset.rs:80-87`) 저장소를 부르는 운영 쪽은 레지스트리 하나뿐이라 두 저장이 겹치지 않는다 |
| R13 | **잠김 판정 = §3-2 `is_busy`**(`PermissionDenied` + Windows 32 · 33) | Windows 에서 백신 · 색인기가 잠깐 쥔 파일의 rename · 지우기를 짧게 다시 하는 것이 흔한 처리(graceful-fs 의 win32 rename · Node `fs.rm` 의 `maxRetries` — 기억 인용 · 미검) · 지금 셸 규칙(ADR-0265 결정 4) |
| R14 | **다시 하기 꼴 = 첫 시도 + N 번 · 고정 간격 · 마지막 뒤에 자지 않음 · 잠김 아닌 오류는 바로.** 예산(부르는 쪽) — 데이터 파일(열기 · 읽기 · rename · 복사 원본 열기) 5 × 20 ms = base `file` 의 `BUSY_RETRIES` · `BUSY_PAUSE`(규칙의 집 — 운영 `OsHooks` 가 그 값을 쓴다) · ★agents · presets 의 적재 읽기만 따로 첫 시도 + 19 번 × 50 ms(파일마다 약 1 초 · 두 파일 합쳐 최악 약 2 초 — agent `persistence` 상수 · D7 — 메인 결정(위임) 2026-10-10 「수 초」 · 잠정 — 사용자 확인 대기 · 값은 이 TRD 가 정했다)★ · 임시 폴더 지우기 19 × 10 ms(transport 3-3 이 net 의 열기를 옮길 때는 4 × 100 ms) | 지금 값 그대로 — ADR-0265 결정 4 · scratch 의 200 ms 실측 예산(`scratch.rs:66-71`) · net 의 「5번 시도 · 최악 400 ms(클라이언트 5 s 폴링 안)」(`instance.rs:86-96`). 한 값으로 맞추지 않는다 — 근거가 서로 다르다. **적재 읽기가 긴 까닭** = 거기서 잠김이 예산을 넘으면 그 실행 내내 읽기 전용이다(R17) — 100 ms 짜리 잠깐 쥐기 하나가 실행 전체를 묶지 않게(리뷰 2 라운드 D7). **「수 초」의 아래 끝을 고른 까닭** = 적재는 데몬 기동 6)(`daemon/src/lib.rs:658-659`) 안이고 접속 정보 발행 8)(`:752`) 보다 앞이다 — 셸의 데몬 찾기 마감은 5 초이고(`src-tauri/src/daemon_client/mod.rs:139` · `:457`) 그 실패는 재시도 없이 `Down` 으로 끝난다(같은 파일 `:461-467` 주석). 두 파일 합 2 초면 기동의 나머지 몫이 남는다 |
| R15 | **거절 로그** — 거절 상태에 드는 순간(적재 때 거절 · 저장 직전 재판정의 첫 거절) error 한 번 · 같은 까닭이 이어지는 동안 debug · 쓰기가 한 번 성공하면 다시 처음부터. agents · presets 는 거절이 호출자에게 오류로도 돌아간다(R17) · 셸 설정은 지금 꼴(쓰기 실패가 그 호출의 오류) · 상태 기록기는 거절도 「쓰기 실패 = 로그만」(D8) 길이라 같은 한 번 규칙을 따른다(새 판 거절은 다음 변경까지 기다리고 · 읽기 실패 거절은 디바운스마다 다시 판정한다 — §3-6 · 리뷰 2 라운드 D5) | 같은 줄이 변경 · 디바운스마다 쌓이지 않게 한다 |
| R16 | **저장 직전 재판정 · 프로세스 간 잠금 없음**(메인 결정 2026-10-10). 쓰기 정책을 적재 때 한 번 정해 들고 있지 않는다 — 쓰기마다 그 직전에 지금 파일을 다시 읽어 R8 로 판정한다(설정 = 지금도 그렇다 · 상태 기록기 · agents · presets = 새로). ★프로세스 간 잠금(파일 잠금 · 잠금 파일)은 두지 않는다★ — 한 데이터 폴더의 각 파일을 쓰는 프로세스는 하나뿐이다: `daemon\` 아래(agents · presets · mcp)는 데몬 하나(단일 인스턴스 가드를 레지스트리 만들기 전에 쥔다 — `daemon/src/lib.rs:494` → `:659`) · `shell\` 아래(설정 · 상태)는 셸 하나 — ★근거의 주(主)는 single-instance 플러그인이다★(`src-tauri/src/lib.rs:52` — 맨 먼저 등록 · Windows 뮤텍스 `{identifier}-sim`). ★그 뮤텍스는 데이터 폴더가 아니라 **번들 식별자** 단위다(`src-tauri/src/lib.rs:48-51` — 리뷰 3 라운드 잔손질 · 3 · 4판의 「같은 데이터 폴더를 쓰는 둘째 인스턴스」는 틀린 말이었다)★ — 식별자가 같은 둘째 인스턴스는 거기서 끝나고, 식별자가 다른 빌드(dev `com.engram.dashboard.dev` · 릴리즈)는 데이터 폴더부터 다르다(`src-tauri/tauri.dev.conf.json:11` · ADR-0137). 그래서 운영에서 한 데이터 폴더의 셸 작성자는 하나다 — 둘을 같은 폴더로 보내는 것은 시험 탈출구 `ENGRAM_DATA_DIR`(배포 노브 아님 — 데몬 `data_dir.rs:19-24` · 셸 사본 `src-tauri/src/discovery/layout.rs:15`)뿐이고 그 경우는 아래 실행 잠금이 받친다. 셸 실행 잠금 `state/lock.rs` 는 **보조**다 — 플러그인이 `RunEvent::Exit` 에서 뮤텍스를 앞 인스턴스의 마지막 쓰기(`Final`)보다 먼저 놓는 틈에 새 인스턴스를 그 쓰기 뒤로 세우는 일만 하고(`state/lock.rs:4-6`), best-effort 라 못 잡으면 잠금 없이 진행한다(같은 파일 `:8-10` — 남는 위험은 그 마지막 쓰기와 엇갈리는 것뿐이고 어느 쪽이 이겨도 파일은 온전하다 · storage TRD §6-5 ①). 3판은 이 둘을 같은 무게로 적어 「실행 잠금이 best-effort 라 단일 작성자 전제가 약하다」로 읽혔다(리뷰 2 라운드 C1). 그래서 재판정과 rename 사이에 끼어들 수 있는 것은 사람의 손 편집 · 동기화 도구뿐이고 그 틈(밀리초)은 받아들인다. ★**실행 중 바깥 편집은 지원하지 않는다**(C1 — 메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기)★: 데몬 · 앱이 떠 있는 동안 네 파일을 사람 · 동기화 도구가 바꾸면 다음 저장이 덮는다 — 마지막 작성자가 이긴다(지금 동작과 같아 회귀가 아니다). 고치려면 데몬 · 앱을 끄고 고친다. 재판정이 잡는 것은 그 편집이 새 판 · 못 읽는 파일 · 손상을 만든 경우뿐이다(거절 · 떠 두기) — 「메모리가 지금 파일에서 왔나」는 보지 않는다(D14 의 「적재 때 없음 · 손상 → 첫 저장」 한 경우만 싸게 막는다 — R8). 설정은 쓸 때마다 지금 파일 위에 바꾼 키만 얹으므로(`settings/store.rs:284-286`) 다른 키의 바깥 편집은 지금도 남는다 — 실행 중 메모리에 다시 읽지 않는 것은 같다. ★거부한 대안★ = 파일 신원(파일 id · 수정 시각 · 크기) 또는 내용 지문을 적재 · 쓰기 때 기억해 저장 직전에 대조하고, 다르면 그 저장을 거절하거나 다시 적재한다 — 기각 = 메인 결정: 관행대로 감지 장치를 두지 않는다(앱이 소유한 상태 파일은 실행 중 바깥 편집을 지원하지 않고 마지막 작성자가 이긴다 — Firefox `prefs.js` · Chrome `Preferences` · 기억 인용 · 미검) · 지금 동작과 같아 회귀가 아니다 · 다시 적재 갈래는 §3-9 가 이미 거부한 실행 중 재적재와 같은 정합 설계(명부 · 화면 · 산 세션)가 든다 | 적재 때 정한 값을 들고 있으면 두 가지가 샌다 — ① 실행 중에 파일이 새 판으로 바뀌어도 덮는다 ② 손상이라 「떠 두고 쓴다」로 정한 값을 들고 있으면 첫 저장 뒤 우리 정상 파일을 저장마다 `.corrupt` 로 떠 두어 진짜 손상 사본을 지운다(리뷰 1 라운드 codex 지적의 귀결). 잠금은 위 가드와 겹치는 장치다. 비용 = 저장마다 그 파일 한 번 읽기 + 모양 검사(D1)(agents · presets 는 수명 사건에서만 저장 — ADR-0207 · 상태 기록기는 디바운스 뒤 — 크기 상한 안). ★agents · presets 는 그 일이 profiles · presets 락 안에서 돈다 — 최악 시간은 §3-9(D4)★ |
| R17 | **agents · presets 의 저장 거절 = 읽기 전용 모드**(메인 결정 2026-10-10 — §3-9). 거절 상태에서 레지스트리 변경 요청은 **적용 전에 오류로 돌아간다** · 그 상태가 버스 명부(`agent.list`)에 실린다(적재 거절 · 마지막 저장의 재판정 거절 — 둘 다 · D6) · 적재 때의 거절(새 판 · 읽기 실패 — 적재 읽기의 긴 예산(R14 · D7) 뒤에도 남은 부팅 때 잠김 포함)은 그 실행 내내 풀리지 않는다. ★부르는 쪽 없는 내부 변경은 따로다★ — 저장의 어떤 `Err`(쓰기 IO 실패 · 저장 직전 재판정 거절)에도 메모리에 적용 + dirty(D2 · E1 — 메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기 · §3-9). 디스크는 매 쓰기의 재판정이 지킨다 · 적재 거절(읽기 전용)이면 명부가 비어 내부 변경의 대상이 없다. 2판의 「로그만 · 메모리 변경은 성공처럼 · 재시작하면 사라짐」을 버린다 | SQLite 가 읽기 전용으로 연 DB 에 쓰기를 오류로 돌려주는 모양(기억 인용 · 미검). 호출자(사람 · LLM)가 「바꿨다」고 믿은 변경이 재시작에 사라지는 것을 막는다 |

### 3-6. 부르는 쪽별 적용

- **설정** — `read_document` 가 `classify` · `parse`(설정 `Spec`)를 부른다 · `Document` 에 `Newer` · `differs` 는 `Unusable` 과 같이 `None` · `store::write` 는 `write_policy` 를 따른다(거절 = 지금 「읽기 IO 실패」와 같은 오류 길 — 서비스 `settings/mod.rs` 의 흐름은 그대로 · 쓸 때마다 다시 읽는 것도 그대로 = R16) · 읽기가 잠김 재시도를 얻는다(R14 — 지금 없다) · `FsSettingsFiles::read` 의 계약을 `io::Result<String>`(`read_file_capped` 꼴)로 · ★적재 로그에 새 변형 `LoadNote::Newer { found }`★ — 지금 `LoadNote::Unusable` 의 문구 「(첫 쓰기가 옆에 떠 둔 뒤 새로 쓴다)」(`settings/store.rs:191-195`)는 새 판에선 거짓이다(R2) · 새 문구 가안 = 「설정 파일이 이 앱보다 새 판(found)이 쓴 것이라 기본값으로 둔다 — 덮지 않으며 설정 바꾸기는 오류로 돌아간다」(로그 문구 · 화면 아님) · R11 로 `settings/store.rs:90-91` 의 `create_dir_all` 이 걷힌다. 모양 검사 = `any_shape`(§3-4) · `Claim` 은 늘 `Adopted`(§3-3 — 쓸 때마다 지금 파일 위에 얹는다).
  - **★거절이 LLM 에 `INTERNAL` 로 보인다(리뷰 2 라운드 D13)★** — 지금 쓰기 IO 실패는 `SettingsError::Internal("설정 파일을 못 썼다 …")`(`settings/mod.rs:459`)로 떨어지고, 거절(새 판 · 읽기 실패)이 같은 길을 타면 버스 `settings.set` · `settings.reset`(선언된 오류는 `NOT_FOUND` 뿐 — `layout/commands.rs:431-451`)의 답이 `INTERNAL` 이다 — 「다시 하면 될지도 모른다」로 읽히는 코드다. ★메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기 = 그 오류 코드는 U4 에서 메인이 정한다(§8-3 어휘 항목과 함께 — agent 쪽 가안 `CONFLICT` 와 맞출지 포함)★. 코드가 바뀌면 `settings.*` 선언이 바뀌므로 셸 `catalog_version` 은 U4 의 15 → 16 한 번에 함께 든다.
- **상태** — `codec::decode` = `parse`(상태 `Spec` — 모양 검사 = 머리 해석 `Head`, D1) + 창 해석(관용) · `Unusable::NewerVersion` 은 `Parsed::Newer` 에서 · `unusable_read` 는 `classify` 로 대신. `decide_boot`(`boot.rs:344-349`) — state.json 이 새 판이면 `CopyAsideState` 대신 가드(새 `Guard::StateNewer` 가안 — 읽기 실패 가드 ⅰ 과 같은 칸: 떠 두지도 덮지도 지우지도 않고 이번 실행 저장 없음) · 상태줄 `StateFileStatus::Newer`(wire `newer` — `boot_plugin.rs:167-173` 의 매핑 · `state/restore.rs:59-81` 의 doc · 열거형) · 크래시 사본의 새 판 처리(N6)는 그대로. ★기록기의 저장 직전 재판정(R16)★ — 기록기 IO 이음매(`StateFiles` — `saver.rs:70-81`)에 **state.json 읽기와 떠 두기 두 연산**을 더한다(가안 `read_state` · `copy_aside_state` — 부팅 이음매 `BootFiles` 의 같은 이름 둘 `boot.rs:26` · `:29` 꼴 · 떠 두기는 리뷰 3 라운드 E5 — 메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기: 4판은 `CopyAsideFirst` 를 정해 놓고 기록기가 그것을 할 연산을 이음매에 두지 않았다). 실물 `Fs`(`saver.rs:105-129` — `state` 경로를 이미 쥔다)는 base `file::read_file_capped` · `file::copy_aside` 를 셸 운영 `OsHooks` 로 부르고, 시험 가짜 `FakeFiles`(`saver.rs:825`)는 디스크 칸에 state.json 원문 · 떠 둔 사본 · 떠 두기 실패 주입 · 호출 세기를 더한다(지금 `fail_writes` · `fail_remove` 꼴). 쓰기마다 읽은 결과를 `classify`(상태 `Spec` — 머리 해석까지, D1)에 넣어 판정한다: `Write` = 쓴다 · `CopyAsideFirst` = `copy_aside_state` 로 떠 두고 쓴다 — ★떠 두기가 실패해도 그 쓰기를 한다(R8 · D8 — 상태만 진행 · 로그 R15 · 시험 = 떠 두기 실패 주입 → `write_state` 가 불리고 발행된다)★ · `Refuse` 는 까닭으로 갈린다(D5 — 메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기):
    - `Refuse(Newer)` = **코덱이 거절한 스냅숏과 같은 칸**(`saver.rs:588-599` 의 `Failure::Rejected` 갈래 — 그 변경 번호를 본 것으로 치고 다음 변경까지 기다린다). 새 판 파일은 디바운스로 기다려도 풀리지 않으므로 주기(`POLL_INTERVAL` 0.5 초 · `QUIET` 1 초 — `saver.rs:32-33`)마다 그 파일을 다시 읽지 않는다. 화면이 바뀌면 다시 판정한다.
    - `Refuse(ReadFailed)` = **디스크 오류와 같은 칸**(`Failure::Disk` — 디바운스 `QUIET` 뒤 다시 · 그때 다시 판정). 잠김이 풀리면 이어 쓴다.
    - 로그는 R15(두 갈래 모두 그 실패 종류의 「처음 한 번 error · 이어지면 debug」 — 지금 `write_failures` 접기).
  `Claim`(D14)은 기록기가 쥔다 — 시작 값 = 부팅 판정이 `Usable` 였거나 부팅 ⑤(`write_run_marker` — `boot.rs:729`)가 썼으면 `Adopted`, 그 밖 `NotYet` · 첫 성공 쓰기 뒤 `Adopted`. ⑤ 는 같은 부팅 순서 안에서 방금 읽은 판정을 따르므로 보통 창이 없다 — `NotYet` 이 기록기까지 가는 것은 ⑤ 가 못 썼을 때뿐이다. R11 로 `boot.rs:80-82 · 90-91` 의 주석과 `create_dir_all` 이 걷힌다.
  - **LLM 이 읽는 표면(리뷰 1 라운드 — `restore.status` 의 `state_file` 에 `newer` 를 더하면 함께):** `src-tauri/src/layout/commands.rs:472-479`(명령 설명 — `saves`=false 의 사유 목록 `:472-474` 에 「state_file 이 newer」 · `state_file` 의 값 목록 `:475-479` 에 `newer` · 두 손상 값 설명의 「이 판이 못 읽는 새 판」을 뺀다) · 같은 파일 `catalog_version` **15 → 16**(`:98` — 규칙 `:91-92` 「선언이 바뀌면 올린다(답 모양도 선언이다)」) · `prompts/engram-help.md:80`(`restore.status` 설명 — 같은 셋) · `src-tauri/tests/layout_commands.rs:2072-2075`(`state_file` 값 순회에 `Newer` ↔ `"newer"`) · 같은 시험 파일의 고정 세대 단언 `:619`(`CATALOG_VERSION` 15 → 16 — 리뷰 3 라운드 잔손질) · `state/restore.rs:59-73`(wire 철자 목록과 변형 doc — 손상 두 변형의 「이 판이 못 읽는 새 판」을 뺀다) · 생성물 `src-tauri/bindings/`.
- **agents · presets** — `load`(적재 읽기의 긴 예산 `OsHooks` — R14 · D7) → `Loaded` → `Usable` 이면 그 `doc` 에서 파일 구조체를 역직렬화(모양 검사는 `parse` 가 이미 했다 — `Spec::shape` 가 같은 역직렬화다, D1 · 그래서 여기서 다시 실패하지 않는다) · 적재 판정이 거절이면 저장소는 이 실행 내내 읽기 전용(R17 · §3-9) · `Claim` = 적재가 `Usable` 이면 `Adopted` · Missing · 손상이면 `NotYet`(저장소 상태 칸 — R12) · 저장마다 `judge(…, claim)` 로 다시 판정(R16 · 모양 포함) → `CopyAsideFirst`(손상 · D14)면 `copy_aside` 뒤 쓰기 · 거절이면 쓰지 않고 오류(그 거절을 상태 칸에 남긴다 — D6) · 쓰기 성공이면 `claim = Adopted` 와 그 거절 표시를 지운다 · 쓰기 상한 검사 · 기동 쓸기(R12 — 번호 꼴은 `sweep_temps` · 옛 고정 이름은 직접). ★적재 거절 · `Claim` 은 **첫 `load()` 에서만** 선다(리뷰 3 라운드 잔손질)★ — 뒤의 `load()` 는 읽어 돌려주기만 하고 상태 칸을 건드리지 않는다(시험은 한 저장소에 `save` · `load` 를 여러 번 부른다 — `persistence/mod.rs:180-210` · `presets.rs:147-149`) · 첫 `load()` 전의 저장은 `NotYet` 으로 판정하고 · 성공한 쓰기는 언제든 `Adopted` 로 올리며 첫 `load()` 가 그것을 내리지 않는다. 운영은 레지스트리 생성이 먼저 적재한다(`profile.rs:377-384`). 두 파일의 같은 코드는 한 내부 함수로(ADR-0061 의 「복제」가 「같은 구현 호출」로 바뀐다). 트레이트 · 레지스트리 · 표면의 모양은 §3-9.
- **mcp_config** — `fs::write` 두 곳(`:115` 토큰 · `:162` 세션 설정 조각) → `write_atomic`(폴더 만들기 포함 — 지금의 `create_dir_all` `:112-114` · `:159-161` 이 걷힌다). ★`create_dir_all` 을 걷는 것은 R11 이 `write_atomic` 에 든 뒤(U3)라야 한다★ — 새 데이터 루트에서는 mcp-config 폴더(`data_dir.rs:53`)가 아직 없어, 그 전에 걷으면 첫 에이전트 스폰이 실패한다(리뷰 1 라운드). 조각은 범위 목록(1e) 밖이지만 같은 수명 · 같은 꼴이라 함께 옮긴다(메인 판단(위임)). 임시 파일도 같은 폴더라 기존 부팅 쓸기(폴더 안 파일 전부 — 그 파일 `settings_path` 주석)가 거둔다. ★토큰이 임시 파일에도 잠깐 실린다 — 같은 폴더 · 같은 ACL · 실패면 지운다★. 얻는 것은 크지 않다(파일은 스폰 전에 다 쓴다 — 지키는 것은 쓰는 도중 꺼짐뿐).
- **net instance — A 에서 바꾸지 않는다(메인 결정 2026-10-10).** net 은 transport 3-3 에서 갈린다 — ★후속: 3-3 이 단일 인스턴스 열기를 옮길 때 platform `retry_busy` 를 쓴다★(예산 4 × 100 ms · R14). 그때 넘길 메모: 시도마다 할 일(열기 · 읽기 전용 속성 1회 걷기 · 접근 거부 · 미지원 · 그 밖 오류는 바로 끝)을 `attempt` 로 · 한도 뒤 진단(`live_owner` · 한 번 더 열기)은 그대로 · 지금 속성 걷기가 시도 하나를 쓴다(`instance.rs:209 · 229-231`) — 고리 모양이 바뀌므로 시도 수 셈은 그 자리의 시험과 리뷰가 대조한다. 그때까지 net 의 2a 게이트는 그대로다.
- **usage scratch** — `Drop` 고리 → platform `retry_busy`(base 를 거치지 않는다 — 데이터 파일이 아니다) · ★잠김 아닌 오류는 더는 다시 하지 않는다★ → U6 파일럿이 실제 실패 코드를 먼저 잰다(§8-2 1).

### 3-7. (g) agent 자식 띄우기

- 새 `crates/engram-dashboard-agent/src/transport/spawn.rs`(가안):
  - `ChildGuard<C: Reap>` — codex 의 가드(`codex/transport.rs` 구조체 `:1406` · `Drop` `:1414-1421`)를 일반화한다. std `Child` 와 portable-pty 자식 둘 다 · `Drop` = ★자식 트리 끄기(아래) 뒤★ kill + wait.
  - `spawn_piped(spec, what) -> Result<PipedChild, PtyError>` — Command 조립 · 파이프 셋 · `hide_console_window` · spawn → **곧바로 가드** → 파이프 꺼내기 · `GroupOwner::new()` · `adopt(pid)` → 가드를 푼다. stdio · codex 가 부른다.
  - `drain_stderr(stderr, core, agent_id, thread_name, what)` — 두 벌을 하나로(마스킹 · 「쌓기가 로그보다 먼저」 · 기동 실패 warn — 지금 그대로).
  - pty 는 `spawn_command` 바로 뒤 가드 → 무리 · `try_clone_reader` · `take_writer` 뒤에 푼다.
- **★가드가 끄는 것은 직속 자식뿐이다 — 지금 codex 가드도 같다(리뷰 1 라운드)★.** Windows 의 claude · codex 는 backend 가 platform `console_command` 로 감싼 `cmd.exe /c <CLI> …` 로 뜬다(`stdio.rs:104-105` · claude `backend/claude/mod.rs:344` · codex `backend/codex/mod.rs:980` — codex 는 npm `.cmd` shim 까지 두 겹). 직속 자식은 `cmd.exe` 이고 실제 CLI 는 손자다. 무리 넣기가 실패해 가드가 `cmd.exe` 만 끄면, 그 사이 이미 뜬 손자는 무리 밖에서 부모를 잃은 채 산다. 지금 가드 시험(`codex/transport.rs:6395-6409` — `cmd.exe /c ping …`)도 `cmd.exe` 의 pid 만 본다.
- **물러서기 평가 — 가드 `Drop` 이 먼저 platform `process::kill_tree(pid)`(Windows = `taskkill /PID <pid> /F /T`)를 부른다(메인 판단(위임) 제안 · 권고):**
  - 효과: `cmd.exe` 가 아직 살아 있어 부모 pid 사슬이 서 있으므로 손자 이하까지 끈다.
  - 비용: 실패 경로에서만 `taskkill` 하나를 띄운다(드묾 · 시간은 재지 않았다) · Windows 밖은 `Unsupported` → 지금처럼 직속 kill 로 물러선다 · 결과는 보지 않는다(이어서 직속 kill + wait).
  - 남는 틈: `taskkill` 이 나무를 훑은 뒤 끄기 전에 새로 생긴 손자 — 드물고 닫지 않는다.
  - 다른 안 = 멈춘 채 띄우기(platform `spawn::prepare_tree_root` · `TreeRoot` — 사용량 조회의 선례). 넣기 전에는 손자가 생길 수 없어 직속 kill 로 충분하고, 성공 경로의 같은 틈(넣기 전에 생긴 손자는 무리 밖 — platform `group/mod.rs:69-70` 의 경고)도 닫는다. 그러나 portable-pty(pty 통로)는 멈춘 채 띄우는 길이 없고, stdio · codex 의 성공 경로를 바꾸는 일이라 2g(가드 규칙)의 범위를 넘는다 → A 에서 하지 않는다(§8-3 관측).
- **코더 지시서에 박을 불변식** — 띄운 뒤의 모든 `?` 는 가드 아래 · 무리 칸은 구조체 마지막(TRD 1-3 §3-4 drop 순서) · kill 인과(ADR-0001) 무변경(가드는 통로가 서기 전의 실패 경로만 다룬다).
- **소스 시험** — `the_child_guard_is_armed_before_the_first_fallible_step_after_spawn`(`codex/transport.rs:6613`)은 `open` 본문의 철자를 찾는다 → `spawn.rs` 로 옮기고, 세 통로 `open` 이 무리를 직접 만들지 않음(`GroupOwner::new` 가 `spawn.rs` 와 pty 가드 구간에만)을 함께 잰다. `the_child_guard_reaps_the_child_on_an_early_return`(`:6395` — `#[cfg(windows)]`)도 옮기고 ★손자까지 재게★ 넓힌다(`ping` 이 뜬 뒤 — platform `process::child_pids` · base `testing::wait_until` — 가드를 버리면 `cmd.exe` 와 `ping` 이 둘 다 죽는다). 옮기면 `spawn.rs` 가 4f 명단에 든다.

### 3-8. (h) 셸 락 오염 · 기록기 시계

- **12곳 → `engram_dashboard_base::sync::{lock, wait_timeout}`** — 범위 목록 11(§2-1) + `layout/apply.rs:247`(목록 밖이던 같은 꼴의 운영 자리 — 메인 결정 2026-10-10 으로 넣었다 · §8-1 3). 뜻 그대로(경고 없음 · 독 표시 유지 — ADR-0275 결정 1). 셸은 이미 base 를 부른다(`theme.rs:18`).
- **saver 시계** — `trait SaverClock: engram_dashboard_base::time::Clock { fn wall_ms(&self) -> u64; }`(가안 — agent `LeftoverClock` 선례 · ADR-0275 결정 6). 운영 = base `SystemClock` + `wall_ms` = `now_epoch_ms` 를 u64 로(음수 = 0 — 지금 `map_or(0, …)` 뜻) · 가짜 `FakeClock`(`saver.rs:872` — `Arc<Mutex<…>>`)은 `Sync` 가 그대로 선다(TRD 1-1 D2 의 「`Sync` 를 더해야 한다」).
- ADR-0288 · ADR-0290(패닉 정책)이 먼저 착지해 이 자리들을 바꾸면 U8 은 그 위에 다시 깐다 — 한 곳(base `sync`)으로 모아 두면 그 정책의 손댈 자리가 준다.

### 3-9. agents · presets 읽기 전용 모드 (메인 결정 2026-10-10 · 모양은 제안 · 4판 = 리뷰 2 라운드 D2 · D3 · D4 · D6 · D8 · D9 · 5판 = 리뷰 3 라운드 E1 · E2 · E3 · 잔손질 — 메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기)

**저장소의 상태 셋**(agent `persistence` 공통 내부 — 가안 이름 `FileStore`):

| 상태 | 언제 | 메모리 명부 | 변경 요청 | 풀리나 | `agent.list`(D6 · 낱말 가안) |
|---|---|---|---|---|---|
| 쓸 수 있음 | 적재가 Missing · 쓸 수 있음 · 손상(떠 둘 것) | 파일에서 온 것(손상 · 없음이면 빈) | 저장 직전 재판정(R16 — 모양 · `Claim` 포함) 뒤 쓴다 | — | `writable` |
| **읽기 전용(이 실행 내내)** | 적재 판정이 거절 — 새 판 · 읽기 실패(적재 읽기의 긴 예산 — R14 · D7 — 뒤에도 잠김 포함) | 비었다(그 파일을 읽지 않았다) | 부르는 쪽 있는 변경 = 적용 전에 오류 · 내부 변경 = 대상이 없다(명부가 비어 띄울 화신이 없다 — E1) | ★데몬 재시작으로만★ | `read_only` + 까닭 `newer` · `unreadable` |
| 이번 쓰기 거절 | 적재는 됐는데 마지막 저장의 재판정이 거절(실행 중에 파일이 새 판이 됨 · 못 읽음) | 마지막으로 저장된 디스크 + (dirty 면) 그 뒤의 내부 변경(아래) | 부르는 쪽 있는 변경 = 오류(메모리 그대로) · ★내부 변경 = 메모리 적용 + dirty(E1)★ · 다음 저장은 다시 판정 | 다음 저장이 성공하면 | ★`refusing` + 같은 까닭(D6 — 3판에 없던 셋째 값)★ · 마지막 저장 시도 기준이라 그 사이 파일이 고쳐졌으면 낡았다 — 다음 변경이 다시 판정한다 |

- ★**적재 때의 거절이 실행 내내 풀리지 않아야 하는 까닭 — 재판정만으로는 못 막는 덫**★: 메모리 명부가 그 파일에서 온 것이 아니다. 뒤에 파일이 읽히게 돼도(잠김이 풀림 · 사람이 옛 판 파일로 바꿈) 재판정은 「쓸 수 있음」이라 답하고, 그 위에 빈 명부를 쓰면 명부 전체를 잃는다 — 재판정은 「지금 파일이 쓸 만한가」만 보고 「메모리가 그 파일에서 왔나」를 모른다(D14 의 `Claim::NotYet` 은 그 경우 덮기 전에 떠 두기만 더할 뿐 — 빈 명부가 제자리를 차지하는 것은 못 막는다). 그래서 적재 거절은 저장소 상태 칸에 남겨 둔다(R12). 실행 중에 다시 적재해 푸는 안은 명부가 실행 중에 바뀌어 화면 · 산 세션과의 정합을 따로 설계해야 해서 하지 않는다(§8-3).
- **레지스트리의 변경 입구는 둘이다(가안 — 같은 핵심 `mutate` 에 실패 처리만 다르다 · 이름은 U5b):**
  - **부르는 쪽 있는 변경**(버스 · WS · 그 둘이 부른 띄우기 경로 — 부팅 복원은 내부 입구다, 아래 E3) = 사본에 적용 → 계층 정규화 → `store.save(사본)` → `Ok` 면 사본을 커밋하고 결과를 · `Err`(`ReadOnly` · `Io` 둘 다)면 메모리를 그대로 두고 오류를. 3판 결정 그대로(쓰기 실패도 오류 — §8-3). 「먼저 묻고 적용」(쓸 수 있나만 확인한 뒤 맵에 적용 → 저장)이 아니라 이 꼴인 까닭 = 확인과 저장 사이의 쓰기 실패(디스크 가득 · rename 잠김 예산 초과)가 다시 「메모리만 바뀜」을 만든다.
  - **부르는 쪽 없는 내부 변경**(아래 표 끝 줄 — D2 · E1 · 메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기) = 저장이 **어떤 `Err`** 로 돌아와도(`Io` · 저장 직전 재판정 거절 — `StoreError` 의 어느 변형이든) **사본을 커밋하고 dirty 를 세운다**(로그 R15) → 다음 성공 저장이 싣는다(저장은 늘 맵 전체를 쓰므로 어느 입구의 다음 저장이든). 오늘의 자가 치유(저장이 실패해도 메모리는 남고 다음 저장이 디스크를 맞춘다 — 지금은 모든 변경이 이렇다)를 내부 입구에 그대로 남긴다.
    - ★디스크는 안전하다★ — 매 쓰기가 그 직전에 다시 판정하므로(R16) 파일이 새 판 · 못 읽는 동안 dirty 명부는 디스크에 닿지 않고, 거절이 풀린(파일이 다시 쓸 만해진) 뒤의 첫 성공 저장에만 실린다. 적재 거절(읽기 전용)이면 명부가 비어 내부 변경의 대상이 없다 — 띄우기는 ① 에서 끝나므로(아래 표) 래치 · 감시자 · reaper 가 가리킬 화신도 없다.
    - ★4판의 「`ReadOnly` 면 적용하지 않는다」를 뒤집은 까닭(리뷰 3 라운드 E1)★ — 재판정의 짧은 읽기 잠김 하나(R8 → `Refuse(ReadFailed)`)가 내부 변경을 버렸다: reaper 의 `auto_restore=false`(`reaper.rs:135`)가 안 남으면 메모리의 `true` 가 다음 저장에 실려 죽은 에이전트가 다음 부팅 복원 대상으로 되살아난다(`manager.rs:2003-2010` 의 「플립 true → 펌프 → reaper false」 순서가 막으려던 크래시 루프) · 래치의 세션 id 가 버려진다(ADR-0226) · 부팅 복원이 `Failed` 로 끝난다. 3판의 「전부 적용하지 않는다」(리뷰 2 라운드 D2)와 같은 부류의 손실이 거절 쪽에 남아 있었던 것이다.
    - ★그래서 래치 머리 전제가 다시 참이다★ — 「포트는 반환이 없어 결과가 래치에 돌아오지 않고, 같은 입력이면 답도 같으므로 재시도할 것이 없다」(`session_id_latch.rs:14-16`): 내부 입구가 저장 결과와 무관하게 메모리에 커밋하므로 비교-교체의 답(`manager.rs:543`)은 메모리만으로 정해지고 래치가 다시 부를 까닭이 없다. 바뀌는 것은 디스크 영속 시점뿐이다(dirty 면 다음 성공 저장 — §7 ADR-0226 행).
  - **dirty 의 집 = 레지스트리 맵 옆**(가안 `profiles: Mutex<Inner { map, dirty: bool }>` — presets 같은 꼴). 「메모리가 디스크보다 앞섰다」는 레지스트리만 아는 사실이고(저장소는 호출자가 실패 뒤 커밋했는지 모른다), 세우는 곳 · 지우는 곳이 전부 이미 그 락을 쥔 `mutate` 안이라 새 락 · 새 간선이 없다 — 락 순서 그대로(아래). 세운다 = 내부 입구의 저장 `Err`(어느 변형이든 — E1) · 지운다 = 저장 성공(어느 입구든) · ★종료 때 남은 dirty 는 잃는다★ — 지금 실패한 저장의 메모리 변경이 재시작에 사라지는 것과 같다 · 종료 경로에 마지막 저장(flush)을 더하지 않는다(리뷰 3 라운드 잔손질 — 지금 동작 그대로) · 제안: dirty 동안에는 「변경 없음」으로 답한 `mutate_if` 도 저장한다(지금 `profile.rs:398-407` 은 변경이 없으면 저장하지 않는다 — 밀린 변경을 다음 기회에 싣게). 부르는 쪽 있는 변경이 `Err` 로 「메모리 그대로」 돌아갈 때 그 메모리는 dirty 변경을 담고 있다(사본은 커밋된 맵에서 뜬다) — 거절된 요청 하나가 밀린 내부 변경을 지우지 않는다. 결과 불변식 = 「메모리 명부 = 마지막으로 성공한 디스크 + (dirty 면) 그 뒤의 내부 변경」 — ADR-0071 의 `persisted == observed` 는 dirty 동안만 어긋나고 그 동안은 표시가 선다.
- **락 안에서 드는 시간(D4 — 메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기 = 수락)** — 저장 한 번이 profiles · presets 락 안에서 하는 일이 는다: 지금(직렬화 · 임시 쓰기 · `sync_all` · rename · 폴더 동기화) + 새로(그 파일 다시 읽기 · JSON 파싱 · 모양 검사(D1 — 구조체 역직렬화) · 손상이면 떠 두기 · 각 단계의 잠김 다시 하기) + 변경마다 맵 사본 하나. 보통 = 지금 + 작은 파일 읽기 한 번(이 PC 의 agents.json 1,747 B). **최악 = 잠김 다시 하기의 잠만 약 0.4 초**(R14 의 5 × 20 ms 고리 넷 — 읽기 · 떠 두기 원본 열기 · 떠 두기 rename · 쓰기 rename) + IO + 상한(16 MiB)까지의 파싱 두 번. 그동안 그 락을 기다리는 읽기 — 명부 조회(`manager.rs:823` → `:893` `profiles.list()` — 우편 발송의 임계 경로, ADR-0207 「맥락」) — 가 선다. 수락하는 까닭 = 판정과 쓰기가 같은 락 안이어야 두 저장이 겹치지 않는다(R12 · ADR-0071) · 최악은 그 파일이 잠긴 동안뿐이고 저장은 수명 사건에서만 돈다(ADR-0207 근거 2). ADR-0207 은 「그대로」가 아니라 「개정 확인」이다(§7) — 그 재론 방아쇠 ②(「저장 1회의 락 보유 시간이 ms 단위를 넘는다」)에 이 최악이 설계상 닿는다.
- **트레이트(가안):** `ProfileStore::save(&self, &[AgentProfile]) -> Result<(), StoreError>` · `load(&self) -> Vec<AgentProfile>` 그대로 · 새 `status(&self) -> StoreStatus`(기본 구현 = 쓸 수 있음 — 시험 가짜는 그대로 둔다 · 운영 = 저장소 상태 칸의 적재 거절 · 마지막 재판정 거절 — D6) · `StoreError = ReadOnly(Refusal) | Io(io::Error)`(`ReadOnly` = 이 저장을 판정이 거절했다 — 적재 거절 · 재판정 거절 둘 다 · 어느 쪽인지는 `status()` 가 가른다 · 입구 처리는 변형을 가르지 않는다: 부르는 쪽 있는 입구 = 어느 `Err` 든 오류 · 내부 입구 = 어느 `Err` 든 커밋 + dirty — E1) · `Refusal = Newer { found } | Unreadable` · `StoreStatus = Writable | ReadOnly(Refusal) | Refusing(Refusal)`. `PresetStore` 같은 꼴. ★구현 18곳(D9 — 3판은 16 으로 셌다)★: 운영 2(`persistence/mod.rs:90` · `presets.rs:82`) · 시험 가짜 16 — agent 4(`profile.rs:849` · `preset.rs:147` · `manager.rs:3073` · `:5016`) · daemon 12(`agent_conn.rs:902 · 908` · `connection_core.rs:2772 · 2785` · `control/commands.rs:360 · 370` · `lib.rs:1030 · 1047`(기능 `test-support`) · `tests/control_agent.rs:129 · 140` · `tests/mail_gate.rs:49 · 59`) — 가짜는 `Ok(())` 를 돌려주는 기계적 수정이다(찾는 법 = `rg "impl .*(Profile|Preset)Store for"`).
- **부르는 쪽(가안 — 오류 코드 · 칸 낱말은 U5b 에서 정한다 · §8-3):**

| 길 | 거절(`ReadOnly`)일 때 | 쓰기 실패(`Io`)일 때 |
|---|---|---|
| 버스 `agent.new` · `agent.spawn`(새로 만들기) → `create_agent`(`manager.rs:912-933`) → `upsert` | 오류 답(가안 `CONFLICT` — 이미 선언된 코드라 선언이 안 바뀐다 · 문구에 파일 이름과 까닭) | 오류 답(메모리 그대로 — §8-3) |
| 버스 `agent.rename` · `agent.move` | 같은 오류 | 같음 |
| **띄우기 경로 — 부르는 쪽 있음**(버스 깨우기 `agent.spawn --target` · 만들고 띄우기 · WS `Spawn*` → `activate_profile`(`manager.rs:1578`) · 즉석 by-cwd 생성 → `spawn_agent`(`:1182`) — 정책 인자 `Entry::Caller`, E3 · ★부팅 복원은 이 줄이 아니라 끝 줄이다(E3)★) — 변경 셋(D8), 셋 다 공유 함수 `spawn_agent_watching_link`(`:1191`) 안 · 프로세스를 여는 `open_spawn`(`:1481`)보다 앞: ① `register_for_spawn`(`:1123` · 부르는 자리 `:1234` — 명부 등록 · 갱신, 디스크) ② `epoch_for_spawn`(`:1267-1270` — 화신 표식) ③ `release_session_id`(`:1288-1292` — Fresh 만 · 옛 손잡이를 이력으로, 디스크) | ① · ③ = 띄우지 않고 오류 · ★② 는 메모리만이라 거절 · 실패가 없다(D8)★ · ★버스는 지금 활성화 오류를 전부 `INTERNAL` 로 싣는다(`agent/src/commands.rs:976-977` · 만든 뒤 활성화 `:1015`) — `CONFLICT` 로 가르려면 ① · ③ 의 저장 거절이 `PtyError` 전용 변형(가안 `PtyError::Store(StoreError)` — 전용 변형 선례 `RosterFull` `types.rs:1036`)으로 올라와야 한다 · 변형은 U5b-1, 버스 매핑은 U5b-2(리뷰 3 라운드 잔손질)★ | 같음 |
| WS `CreateProfile` · `DeleteProfile` · `RenameProfile` · `ReparentProfile` · `SetProfileAutoRestore` · `CreatePreset` · `DeletePreset` · `RenamePreset` | 이미 있는 `reply(…, Err(…))`(`connection_core.rs:1259`)로 — 지금 프리셋 셋은 무조건 `Ok` 다(`:1796-1818`) · 화면이 그 실패를 어떻게 보이는지는 U5b 가 잰다 | 같음 |
| **내부 변경(부르는 쪽 없음 — D2 · E1)** — 띄운 뒤 `auto_restore` 올리기(`manager.rs:2011` — 프로세스가 이미 떠 있어 실패로 되돌릴 수 없다 · 입구와 무관하게 늘 내부) · 첫 제출 래치의 세션 id 기록(`:543` · ADR-0226) · claude 파일 감시자의 세션 id 추적(`crates/engram-dashboard-daemon/src/lib.rs:276` · ADR-0008 best-effort) · reaper 내리기(`reaper.rs:135`) · ★부팅 복원의 띄우기 경로 ① · ③(`restore_one` `:2050` → 같은 공유 함수 — 정책 인자 `Entry::Internal`, E3)★ | ★**메모리에 적용 + dirty**(E1 — 4판의 「적용하지 않는다」를 뒤집었다)★ · 로그(R15) — 디스크는 다음 저장의 재판정이 지킨다(거절이 풀린 뒤의 첫 성공 저장이 싣는다) · 적재 거절(읽기 전용)이면 명부가 비어 대상이 없다 · 부팅 복원은 띄우기를 이어 간다 | 같음 — **메모리에 적용 + dirty** → 다음 성공 저장이 싣는다(오늘의 자가 치유 · D2) |

- **`epoch_for_spawn` = 메모리만(D8 — 메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기)** — 그 값은 디스크에 의미 있게 실리지 않는다: 읽기를 건너뛰고(`skip_deserializing`) 쓰기는 자리채움 `0` 이다(`profile.rs:185`). 그래서 지금 `mutate`(`profile.rs:775-785` — 저장 한 번)를 타는 것은 순수 비용이고, 선례 `set_last_failure`(`profile.rs:627` — 「`mutate` 를 타지 않는 유일한 쓰기」 · ADR-0207 거부한 대안 (b) 가 든 성립 조건 = 그 필드가 영속 대상이 아니다)와 같은 꼴로 바꾼다: 락만 잡고 표식을 갈고 저장하지 않는다. 판정과 커밋이 한 락 안이라는 불변식(그 함수 doc)은 그대로다. ★`register_for_spawn` 바로 뒤라는 배치(`manager.rs:1250` 주석 · 소스 시험 `:6285-6286`)도 그대로다★.
- **띄우기 경로의 부분 성공(D8 — 아래 넷째 줄까지는 부르는 쪽 있는 입구 `Entry::Caller` 의 이야기다)** — 각 변경이 자기 저장을 가진 독립 변경이라 「셋 다 아니면 전무」로 묶지 않는다:
  - ① 이 실패하면 ② · ③ 에 닿지 않는다(아무것도 안 바뀐다).
  - ① 이 성공(커밋 · 저장됨)한 뒤 ③ 이 실패하면 → ①은 되돌리지 않는다 — 「명부에 오른 · 갱신된 채 안 뜬 에이전트」이고, 지금 `open_spawn`(`:1481`) 실패 뒤와 같은 끝 모양이다 · ② 의 새 표식은 메모리에 남는다(그 표식을 쥔 화신이 없어 무해 — 다음 띄우기가 다시 뽑는다) · ③ 은 적용되지 않아(사본 → 저장 실패 → 메모리 그대로) 옛 손잡이가 칸에 남는다 — 띄우기가 멈췄으므로 어긋날 화신이 없다.
  - ① · ③ 이 통과한 뒤의 `open_spawn` 실패는 지금 그대로다.
  - 이 어긋남이 생기는 것은 ①과 ③ 사이에 파일이 바뀌거나(재판정 거절) 쓰기가 실패할 때뿐이다 — 적재 거절(읽기 전용)이면 ①에서 끝난다.
  - ★부팅 복원은 「내부 변경」으로 친다 — 메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기(4판 작성자의 「부르는 쪽 있는 변경」 제안을 받지 않음 · 5판 E1 · E3 로 다듬음)★ — `restore_all`(`manager.rs:2022`)이 띄우기 실패를 `RestoreOutcome::Failed` 로 보고하기는 하지만(`:2050-2068`) 그 결과를 받아 되돌릴 사람 · LLM 요청이 없다. 그래서 부팅 때 ① · ③ 의 저장이 **어떤 `Err`** 로 돌아와도(`Io` · 재판정 거절) 메모리에 적용 + dirty 하고 띄우기를 이어 간다(지금처럼 복원은 선다 — 순간 잠김 하나가 에이전트 복원을 막지 않게 · 관행: 복원은 저장 실패에 견딘다). ★4판의 「「이번 쓰기 거절」이면 ① · ③ 을 적용하지 않고 `Failed`」는 뒤집었다(리뷰 3 라운드 E1)★ — 새 판 파일 위에 우리 명부를 쓰지 않는 것은 매 쓰기의 재판정이 이미 지키므로 띄우기를 막을 까닭이 없다. 읽기 전용(적재 거절)이면 명부가 비어 있어 복원할 것이 없다. ★지금 운영 데몬은 `restore_all` 을 부르지 않는다★ — 부팅 자동 복원 기본 OFF(사용자 결정 2026-07-09 · `crates/engram-dashboard-daemon/src/lib.rs:780-786` — 구현 · reaper 처분은 남기고 호출만 껐다 · 시험 `crates/engram-dashboard-agent/tests/activation.rs:622` 이 부른다). 규칙은 그 함수가 다시 불릴 때(그 자리 주석의 명시 복원 명령)를 위해 정해 두고 시험으로 지킨다.
  - **배관 — 한 공유 함수에 정책 인자 하나(E3 — 메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기)** — 부팅 복원은 수동 깨우기와 **같은 동사를 일부러 쓴다**(`restore_one` → `spawn_fresh_settled` `manager.rs:2057-2061` · `resume_no_fallback` 의 「restore_one 과 activate_profile 이 똑같이 재사용한다」 `:2094` — ADR-0082 · ADR-0201 · ADR-0202). 그래서 입구별 사본(복원 전용 띄우기 함수)을 두지 않는다 — 사본은 그 ADR 들이 막은 「입구마다 판정이 갈려 하나가 뒤처진다」를 되살린다. 대신 두 동사가 함께 닿고 ① · ② · ③ 이 모두 그 안에 있는 **한 공유 함수 `spawn_agent_watching_link`(`:1191`)에 정책 인자 하나**(가안 `entry: Entry` — `Entry::Caller` = 부르는 쪽 있는 입구 · `Entry::Internal` = 내부 입구 · 이름은 U5b-1)를 실어 보낸다.
    - 지나는 길: 입구 `restore_one`(`:2050` — `Internal`) · `activate_profile`(`:1578` — `Caller`) · `spawn_agent`(`:1182` — `Caller`) → 두 동사 `spawn_fresh_settled`(`:1770`) · `resume_no_fallback`(`:2100`)는 받은 값을 그대로 넘긴다 → `spawn_agent_watching_link` 가 ① `register_for_spawn`(`:1123` — 같은 인자를 받는다) · ③ `release_session_id` 의 레지스트리 입구를 그 값으로 고른다. ② 는 메모리만이라 인자와 무관하고(D8), `spawn_session` 의 `auto_restore` 올리기(`:2011`)는 입구와 무관하게 늘 내부다.
    - 시험 — ① 소스 시험: 지금 `both_fresh_entrances_settle_the_link_before_reporting_success`(`manager.rs:4336` — `:4356-4378` 이 두 입구가 같은 동사를 부르는지 잰다)를 넓혀, `restore_one` 은 `Entry::Internal` · `activate_profile` 은 `Entry::Caller` 를 같은 동사에 넘기고 `spawn_agent_watching_link` 정의가 하나뿐임을 잰다 ② 동작 시험(U5b-1): 재판정 거절 · `Io` 실패를 내는 저장소로 부팅 복원 → `Started` · 명부에 ① · ③ 이 적용되고 dirty / 같은 저장소로 `activate_profile` → `Err` · 메모리 그대로 · 띄우지 않음.
  - ★**만들고 띄우기(`create_and_start` — 버스 `commands.rs:984-1020`)의 부분 성공(리뷰 4 라운드 codex — 메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기)**★ — 만들기(`register` · 저장)와 띄우기(①)가 저장을 따로 가진 두 변경이라 「만들어졌고 안 떴다」가 생긴다. ★이것은 지금의 계약이고 새 장치를 두지 않는다★: 지금도 등록을 되돌리지 않고(되감기는 둘째 삭제 경로 — ADR-0122) 오류 문구가 회복 길(「다시 만들기가 아니라 그 이름으로 다시 띄우기」)을 나른다(`:1009-1016` 의 주석 · ADR-0172 의 활성화 뒤 알림이 명부에 「마지막 실패」를 싣는다). 이 TRD 가 더하는 갈래만 적는다: ⓐ 저장소가 거절 중(읽기 전용 · `refusing`)이면 만들기가 먼저 `Err` 라 띄우기에 닿지 않는다 — 다시 시도해도 중복이 생기지 않는다(만들기도 거절된다) ⓑ 만들기 저장 성공 뒤 ① 이 `Io` · 재판정 거절로 실패하면 지금의 활성화 실패와 같은 길(같은 문구 · 같은 알림 · 만든 에이전트는 남는다). 오류 코드만 U5b-2 의 저장 거절 매핑(가안 `CONFLICT`)을 탄다. ★U5b-1 정지 지점★: 그때 버스는 아직 옛 `create_agent`(U5a 의미 — `Err` 는 로그 · 메모리는 적용)를 부르므로 만들기는 메모리에 서고, 새 `Caller` 입구의 ① 이 거절하면 위 ⓑ 와 같은 모양(만든 에이전트는 메모리에 남고 띄우기 오류 + 회복 문구)이 된다 — 옛 판 의미 그대로이고 새 위험이 아니다. 시험(U5b-2) = 거절하는 저장소로 `agent.new --start` 류 → 만들기 `Err` · 명부 그대로 · 띄우기 시도 0.
- **변경 뒤 부수효과는 `Ok` 일 때만(D3 — 메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기 · 5판 E2 로 목록을 좁힘)** — 거절 · 실패면 다음이 돌지 않는다: 명부 브로드캐스트(`broadcast_profile_list` · `broadcast_preset_list` — WS 변경 명령의 자리 = 만들기 `connection_core.rs:1603` · 지우기 `:1622` · 자동 복원 토글 `:1718` · 이름 바꾸기 `:1735` · 옮기기 `:1759` · 프리셋 `:1799 · 1808 · 1818`) · 버스의 `notify.roster_changed()`(`agent/src/commands.rs:1002 · 1105 · 1169 · 1177 · 1249` — `agent.rename` 의 `RenameOutcome::Renamed` 갈래 `:1168` 포함) · WS `DeleteProfile` 의 메시징 삭제 정리 훅(`connection_core.rs:1609` · 지우기 `:1620` · 훅 `:1626` — ADR-0116 결정 3 「유일한 프로필 제거 지점」). 지우지 못한 프로필의 파킹 우편 · 계약을 정리하면 산 프로필의 메시징이 깨진다. 지금 그 자리들은 결과를 보지 않고 부른다(`delete_agent` 는 `()` — `manager.rs:935` · 프리셋 셋 `connection_core.rs:1796-1818`) → `Result` 를 받아 `Ok` 갈래 안으로 옮긴다. U5b 시험 = 거절하는 저장소로 `DeleteProfile` → 훅 0회 · 브로드캐스트 0회 · `Ack` 대신 `Error` / 버스 `agent.rename` → `Renamed` 아님 · `roster_changed` 0회.
  - ★**실패 때도 도는 알림은 D3 밖이다(E2 — 메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기)**★ — 버스 깨우기 `wake_existing` 의 `:975` · `create_and_start` 의 활성화 뒤 `:1008`. ADR-0172 가 활성화 **실패** 때도 알리기를 요구한다 — 「마지막 실패」는 명부로만 흐르고(`:1004-1007` — 실패 갈래의 `Err` 는 호출자에게만 간다) 화면에는 아무것도 안 남기 때문이다. 4판은 이 둘을 「`Ok` 일 때만」 목록에 넣어 그 요구를 깼다. ★같은 까닭으로 WS 활성화 뒤 브로드캐스트(`connection_core.rs:1332` · `:1700` — 활성화 결과와 무관하게 명부를 다시 보낸다)도 그대로 둔다(5판을 쓰며 찾은 것 — E2 의 따름)★. 시험 = 거절하는 저장소로 깨우기 → `Err` 이면서 `roster_changed` 1회(지금 동작 유지).
- **상태 노출(LLM 이 보는 자리):** 버스 `agent.list` 의 답 `AgentListOk`(`agent/src/commands.rs:173-175`)에 두 저장소의 상태 칸(가안 `store` — agents · presets 각각 `writable` | `read_only` | `refusing`(D6) + 까닭 `newer` · `unreadable`)을 더한다 → 선언이 바뀌므로 agent `catalog_version` **6 → 7**(`commands.rs:50`) · 고정 세대 단언 `crates/engram-dashboard-agent/tests/command_declarations.rs:75`(6 → 7 — 리뷰 3 라운드 잔손질) · 생성물 `crates/engram-dashboard-agent/bindings/` · 도움말 `prompts/engram-help.md:49-50`(그 칸 — `refusing` 은 「마지막 저장이 거절됐다 · 다음 변경이 다시 본다」) · `:53-64`(바꾸는 명령이 그 상태면 오류로 돌아온다는 한 줄). ★`refusing` 을 두는 까닭(D6)★ = 그 값이 없으면 실행 중 거절은 `writable` 로 보이는데 변경은 오류로 돌아와, LLM 이 그 오류를 설명할 상태를 못 본다. 화면 명부(WS `AgentListUpdated` · `PresetList`)에는 싣지 않는다 — 화면 안내는 열린 것(§8-3).
- **락 순서** — `래치 → expected 칸 → profiles|presets(dirty 포함) → 저장소 상태 칸`(R12). 저장소 상태 칸은 판정 · 디스크 IO 중에 쥐지 않는다(읽고 쓰는 순간만). 사슬이 적힌 자리(U5a 가 고친다): `session_id_latch.rs:23-24` · `manager.rs:516` · `profile.rs:364-365` · `preset.rs:58-59` · ADR-0071:26(ADR 은 개정 도장 — §7).

---

## 4. 사용자가 보는 변화

1. **agents.json · presets.json 손상 사본 이름 · 동작** — `<이름>.corrupt-<ms>`(적재 때 이동 · 쌓임) → `<이름>.corrupt`(첫 저장 직전 복사 · 덮어쓰기). 그 사이 원본은 그 자리에 남고 데몬을 띄울 때마다 같은 오류 로그가 난다. 두 번째 손상은 첫 사본을 덮는다(사용자 수락). **이미 있는 `.corrupt-<ms>` 파일은 건드리지 않는다**(옮기지도 지우지도 않는다). 손상에는 지금처럼 모양이 깨진 파일(항목 하나의 모르는 `kind` 같은 것)도 든다 — 적재와 저장 직전 재판정이 같은 정의를 쓴다(D1).
2. **새 판이 쓴 파일(네 파일 공통)은 덮지 않는다**
   - 설정 — 기본값으로 뜨고 설정 바꾸기가 오류로 돌아온다(지금 「읽기 실패」와 같은 길 — 버스의 오류 코드는 U4 에서 정한다, 지금 길이면 `INTERNAL` · D13) · 파일과 `.corrupt` 그대로. 지금은 첫 쓰기가 떠 두고 덮는다.
   - 상태 — 기본 화면 · 이번 실행 화면 상태 저장 없음 · 새 안내(아래 4) · 파일 그대로. 지금은 `state.json.corrupt` 로 떠 두고 덮는다(TRD S21-storage §6-5 · §12 R3 를 뒤집는다 — §8-1 2).
   - agents · presets — 빈 목록 · ★**읽기 전용**★: 에이전트 · 프리셋 만들기 · 지우기 · 이름 바꾸기 · 옮기기 · 깨우기가 오류로 돌아온다(버스 · 화면 모두 — 화면이 그 오류를 어떻게 보이는지는 U5b 가 잰다) · `agent.list` 가 그 상태를 싣는다 · ★데몬을 다시 띄워야 풀린다★(파일을 고치거나 치운 뒤) · 파일 그대로. 화면 안내는 없다(§8-3). 지금은 첫 저장이 덮는다(결함).
3. **agents · presets 를 읽다 실패하면(없음 말고) 같은 읽기 전용** — 부팅 때 백신 같은 것이 쥐어 파일마다 약 1 초(R14 — 적재 읽기의 긴 예산 · D7) 안에 안 풀린 경우도 든다. 지금은 빈 목록으로 덮어 전부 잃는다.
4. **화면 안내 · 버스** — `restore.status` 의 `state_file` 에 새 값 `newer`(LLM 도 같은 핸들로 본다 — CLAUDE.md 「LLM-우선 제어」 · 도움말 · 명령 설명 · 세대 15 → 16) · 셸 바인딩 · `src/api/restoreClient.ts` 의 값 목록 · `StateFileNotice` 갈래 · `src/i18n/ko.ts` 새 문구 가안 「화면 상태 파일(state.json)이 이 앱보다 새 버전이 쓴 것이라 기본 화면으로 시작했습니다. 이번 실행에서는 화면 상태를 저장하지 않으며 파일은 그대로 둡니다.」. 기존 두 문구(`ko.ts:268-273`)의 「(손상 · 다른 버전의 형식 등)」은 새 판이 더는 그 길로 오지 않으므로 「(손상 · 알 수 없는 형식 등)」으로(가안). ★여기 문구는 가안이다 — 최종안은 U4 에서 메인이 사용자에게 올린다(§8-3)★.
5. **실행 중에 파일이 바뀌면 저장 직전에 다시 판정한다(R16)** — 실행 중에 새 판이 되거나 못 읽히면: 설정 = 지금과 같다(쓸 때마다 읽는다) · 상태 = 그 저장을 건너뛴다(새 판이면 화면이 다시 바뀔 때까지 기다리고 · 못 읽으면 다음 디바운스에 다시 판정 — D5) · agents · presets = 그 변경이 오류로 돌아오고 `agent.list` 가 `refusing`(가안 — D6)을 싣는다(다음 요청은 다시 판정 · 부르는 쪽 없는 내부 변경은 메모리에 남는다 — 아래 10 · E1). 지금은 덮는다.
6. **버전 키 없는 파일** — 상태 · agents · presets 도 1 판으로 읽는다(지금은 손상).
7. **BOM 붙은 agents · presets 를 읽는다**(지금은 손상).
8. **agents · presets 에 상한**(16 MiB · 4 MiB) — 넘으면 손상(지금은 없음). 실측 크기의 약 만 배라 정상이면 안 닿는다.
9. **임시 파일 이름** — `agents.json.tmp` → `agents.json.tmp<pid>.<번호>`(쓰는 동안만 · 죽으면 다음 데몬 기동이 쓸고, 옛 `.tmp` 도 그때 지운다). mcp 폴더에도 쓰는 동안 임시 파일이 잠깐 생긴다.
10. **(열린 것 — §8-3 · 권고대로 가면)** agents · presets 의 쓰기 자체가 실패하면(디스크 가득 · 잠김 예산 초과) 사람 · LLM 이 부른 변경(만들기 · 이름 바꾸기 · 깨우기 등)은 오류로 돌아온다 — 지금은 성공처럼 보이고 재시작에 사라진다. ★부르는 쪽 없는 내부 변경은 다르다(D2 · E1)★ — 쓰기 실패든 이번 쓰기 거절이든 지금처럼 메모리에 남고, 디스크에는 다음 성공 저장(거절이면 그것이 풀린 뒤)이 싣는다 — 그 사이 데몬이 꺼지면 그 변경은 잃는다(예: 세션 id 가 안 남아 다음 활성화가 새 대화 — 지금 실패한 저장과 같은 결말). 읽기 전용(적재 거절)이면 명부가 비어 내부 변경이 생기지 않는다. 부팅 복원도 내부로 쳐서 저장이 실패 · 거절돼도 띄우기를 이어 간다(지금 운영은 부팅 복원을 부르지 않는다 — §3-9).
11. **안 보이는 것** — 설정 · agents · presets 읽기의 잠김 재시도(최악 +100 ms · agents · presets 의 데몬 기동 적재만 파일마다 최악 +1 초 — D7) · 폴더 동기화(Windows 명시 무동작 — 지금과 같은 결과) · 상태 기록기가 실행 중에 지워진 폴더를 다시 만든다(R11) · 무리에 못 넣은 자식은 무리 밖에서 사는 대신 스폰이 실패로 돌아오고 그 손자까지 끈다(g — 드묾) · agents · presets 저장이 파일을 한 번 더 읽는다(보통 무시할 만함 · 파일이 잠긴 동안은 명부 조회가 최악 약 0.4 초 선다 — D4) · (h)는 동작 무변경.
12. **바뀌지 않는 것 — 실행 중 바깥 편집은 지원하지 않는다(C1 — 메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기)** — 데몬 · 앱이 떠 있는 동안 네 파일을 손으로 고치거나 동기화 도구가 바꾸면 다음 저장이 덮는다(마지막 작성자가 이긴다 — 지금과 같다). 고치려면 데몬 · 앱을 끄고 고친다. 그 편집이 새 판 · 못 읽는 파일 · 손상을 만들면 위 2 · 5 · 1 의 길로 간다. 설정은 지금처럼 바꾼 키만 얹으므로 다른 키의 편집은 남는다.
13. **적재 때 없음 · 손상이던 파일이 실행 중에 되살아나 있으면 첫 저장이 덮기 전에 `.corrupt` 로 떠 둔다(D14 — 메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기)** — 예: 데몬이 빈 명부로 뜬 뒤 사람이 백업 agents.json 을 되돌려 놓았으면, 다음 변경이 그것을 `agents.json.corrupt` 로 복사한 뒤 쓴다(내용은 멀쩡한데 이름은 ADR-0274 의 고정 `.corrupt` 다 — 로그가 가른다). 되돌리려면 데몬을 끄고 그 사본을 제자리로. 지금은 사본 없이 덮는다. 상태는 부팅이 판정 바로 뒤에 쓰므로 보통 해당이 없다 · 설정은 해당 없음(덮지 않고 얹는다).

---

## 5. 게이트 · 의존

### 5-1. 직접 의존

| crate | 지금 | 뒤 |
|---|---|---|
| base | `regex` · `tracing` · `tracing-subscriber` | + 서드파티 `serde_json`(U3) — 워크스페이스 의존은 여전히 0 |
| platform | `tracing` · `windows`(Windows) | 그대로(`sync_dir` 는 std 만) |
| 셸 · agent · 데몬 | base · platform 을 이미 직접 의존(`cargo tree --depth 1`, 2026-10-10) | 그대로 — 새 워크스페이스 간선 0 |
| net · messaging · command · transport · protocol · cli | — | 그대로 |

`Cargo.lock` 새 패키지 = 0 — base 를 운영 의존으로 부르는 셋이 이미 `serde_json` 을 끈다(§3-1). CLAUDE.md 「의존성(변경 시 보고)」에 base 의 `serde_json` 을 보고한다(U3).

### 5-2. 바뀌는 게이트 · 바뀌지 않는 게이트

- **base 게이트 ③(입주자 무참조) 정규식에 `file`** — `(logging|text|time|path|sync|testing|writable)` → `+file`. 자리 여섯(정본 하나 + 사본 다섯): `crates/engram-dashboard-base/src/lib.rs:52`(정본) · CLAUDE.md:254 · `.github/workflows/ci.yml:668` · `.claude/skill-bindings/qa.md:161` · `docs/testing-strategy.md:50` · `:179`. U2(모듈을 만드는 단위에서 늘린다 — ADR-0275 「영향」).
- **base 게이트 ①(의존 상한 = 자기 하나)** — 기대값 그대로. ★D 의 이음매가 필요한 까닭을 지키는 것이 이 게이트다★ — base → platform 간선이 생기면 빨개진다.
- **base 게이트 ②(`use tauri` 0) · base 시험 기능 운영 그래프 게이트** — 그대로.
- **platform 4f 명단** — `src-tauri/src/fsutil.rs` 와 그 「시한부 운영 예외(ADR-0275 결정 15)」 줄(`ci.yml:941-942`)을 뺀다(U1 — 판정이 양방향이라 빼지 않으면 빨개진다) · `transport/spawn.rs` 를 더한다(U7 — 손자 시험이 `#[cfg(windows)]`).
- **platform ⑤ · ⑥ · ⑦ · ① · ②** — 그대로(`sync_dir` 는 std 만 · `std::os` 경로 없음).
- **메시징 이름 정규식** — ★바뀌지 않는다★ — 새 워크스페이스 crate 가 없고 `base` 는 이미 알파벳에 있다. 2판의 「알파벳에 `datafile`」과 리뷰 1 라운드가 짚은 다섯째 사본(`docs/reference/architecture-overview.md:498`)은 손댈 일이 없어졌다.
- **net 2a** — 바뀌지 않는다(net 은 A 밖 — §3-6). transport 3-3 이 열기를 platform `retry_busy` 로 옮길 때 그 자리(`ci.yml:1153-1160` · net `src/lib.rs:59-100` · CLAUDE.md:278 · `qa.md:283` · `docs/testing-strategy.md:85`)가 움직인다.
- **회귀 결과 줄** — 증가 0(base 의 lib 단위 · Doc-tests 줄은 이미 있다 · CI 스텝 `ci.yml:277-279` 그대로).
- **생성물 sync** — U4 = `src-tauri/bindings/`(`StateFileStatus`) · U5b = `crates/engram-dashboard-agent/bindings/`(`agent.list` 답) — 생성물을 함께 커밋한다.
- **새 시험(게이트 아님 · 회귀망)** — 운영 `OsHooks` 마다(셸 1 · agent 2(데이터 · 적재 읽기 — D7) · 데몬 1) 「재시도가 잠김 오류를 실제로 다시 한다」 하나(`(OS_HOOKS.retry)(&mut 시도)` — 상태는 시도 클로저가 쥔다 · 가짜 시도 = 첫 번 `PermissionDenied` · 둘째 성공 → `Ok` · 두 번 불림). 아무것도 안 하는 재시도를 넘기는 실수를 잡는다(§3-1 D 의 남는 대가). agent 의 적재 읽기 상수는 「데이터 예산보다 길다」도 함께 잰다(같은 상수를 둘에 넘기는 실수).

---

## 6. 단위

**원칙(TRD 1-3 §5 · 1-1 §6 과 같다):** 단위마다 워크스페이스 빌드 · 회귀 초록으로 끊는다. 단위 안 순서 = 「새 자리 만들기 → 부르는 곳을 파일 단위로 바꾸기 → 옛 자리 지우기」 — ★자료구조를 먼저 갈아엎고 호출부를 나중에 맞추는 순서는 금지★. 단위가 다른 단위의 옛 자리를 남겨 두지 않는다. 착수 전 그 자리 커밋으로 되돌릴 지점을 둔다(이 TRD 를 먼저 로컬 커밋). 주석은 `/code-conventions` 주석 규약을 지시서에 주입한다. **새 ADR(번호 미정)은 U1 바로 앞에 쓴다**(§7).

### 6-1. 단위 목록

| id | 범위 | 건드리는 파일 | 선행 | 크기(어림) | QA |
|---|---|---|---|---|---|
| **U1** | 잠김 규칙 · 폴더 동기화를 platform 으로 · fsutil 이 부른다 — ADR-0275 결정 15 를 닫는다 | platform `src/fs.rs`(+ `is_busy` · `Retry` · `retry_busy(_with)` · `sync_dir` · 옮겨 온 시험 5 + `sync_dir` 시험) · platform `src/lib.rs` 헤더(`:7-10` 시한부 예외 문장 · `:16` `fs` 입주자 설명 · `:42` `cfg!(windows)` 갈래 목록) · 셸 `src/fsutil.rs`(`retry_denied` 가 platform `retry_busy_with` 를 부르고 `is_lock_contention` 을 걷음 · 판정 시험 둘 · 다시 하기 시험 셋 이사) · `ci.yml` 4f 명단(`:941-942`) · CLAUDE.md(「플랫폼 중립」 `:103` · platform 항목 `:159`) | — | 작음(운영 +60/−30 · 시험 5 이사 + 1) | standard |
| **U2** | base `file` 을 fsutil 에서 낳는다 · 셸 호출부 전환 · fsutil.rs 삭제 — ★동작 무변경★ | 새 `crates/engram-dashboard-base/src/file.rs`(또는 `file/` — 하위 모듈은 `super::`)(fsutil 운영 · 시험 이사 — `&str` → `&[u8]` · 공개 함수는 `OsHooks` 인자 · ★시험은 비공개 `_with` 이음매에 상태를 쥔 클로저(D15)★) · base `src/lib.rs`(`pub mod file;` · 헤더 입주자 여덟 · 게이트 ③ 알파벳) · base `Cargo.toml` description · 셸 운영 `OsHooks` 상수(새 작은 모듈 — 자리 · 이름은 이 단위에서 + 그 시험 1) · `settings/store.rs` · `state/{boot.rs, saver.rs, codec.rs}` · `src/lib.rs`(`mod fsutil` 삭제) · `src/fsutil.rs`(삭제 — `fnv1a_64/hex` 는 유일한 소비자 `state/codec.rs` 로) · 게이트 ③ 사본 다섯(CLAUDE.md:254 · `ci.yml:668` · `qa.md:161` · `docs/testing-strategy.md:50 · 179`) · CLAUDE.md base 항목(`:158`) · base 시험 줄(`:233`) · ★입주자 명단 사본 셋(D9)★ — 루트 `Cargo.toml:14`(멤버 주석) · `docs/testing-strategy.md:43`(① 단위 목록에 `file`) · `:160`(base 시험 줄 주석) | U1 · U8 | 중간(대부분 이사 — 약 900줄 · 시험 21) | standard |
| **U3** | 읽기 규칙 · 쓰기 판정 · 폴더 만들기(R11) · 폴더 동기화(R10) — base `file` 만 | base `src/file*`(`Spec`(+ `shape` — D1) · `any_shape` · `Parsed` · `Loaded` · `classify` · `load` · `Claim`(D14) · `judge` · `check_cap` · `WritePolicy` · R10 · R11 · 비공개 `_with` 에 R10 · R11) + 시험(R1~R8 · R10 · R11 · 모양 실패 = `parse` · `classify` · `load` · `judge` 어디서나 손상(D1) · `Newer` 는 모양을 안 본다 · `write_policy` 표 여덟 칸(§3-3 — D14)) · base `Cargo.toml`(+ `serde_json`) · `Cargo.lock`(base 항목의 의존 줄 — 새 패키지 0) · base `src/lib.rs` 헤더(「생성물을 만들지 않는다 — serde 도 …」 문장 `:66-67` — `serde_json` 은 있으나 생성물은 여전히 없다) · CLAUDE.md base 항목(`bindings/` 문장) · 「의존성」 절(보고) | U2 | 작음~중간(+250 · 시험 약 20) | standard |
| **U4** | 셸 채택 — 설정 · 상태 · 새 판 안내 · 기록기 재판정 · LLM 표면 | `settings/store.rs`(+ `LoadNote::Newer` · `:90-91` 걷기 · `any_shape`) · `settings/mod.rs`(거절의 오류 코드 — D13 · `:459`) · `settings/tests.rs` 가짜 · `state/{codec.rs(머리 해석 = 상태 `Spec::shape` — D1), boot.rs(:80-82 · 90-91 걷기 · 새 판 가드 · 기록기에 넘길 `Claim`), saver.rs(재판정 · ★`StateFiles` 에 읽기 · 떠 두기 두 연산 — 실물 `Fs` · 가짜 `FakeFiles` 함께(E5)★ · `Refuse(Newer)` = 다음 변경까지 · `ReadFailed` = 디바운스 — D5 · `Claim` — D14), restore.rs, boot_plugin.rs}` · `src-tauri/src/layout/commands.rs`(`:472-479` · `settings.*` 의 오류 선언(D13 — 코드가 바뀌면) · `catalog_version` `:98` 15 → 16) · `prompts/engram-help.md:80` · `src-tauri/tests/layout_commands.rs:2072-2075` · ★같은 파일의 고정 세대 단언 `:619`(`assert_eq!(CATALOG_VERSION, 15)` → 16 · 세대 사연 주석 `:606-618` 에 16 한 줄 — 리뷰 3 라운드 잔손질)★ · `src-tauri/bindings/`(재생성) · `src/api/restoreClient.ts` · `src/components/layout/StateFileNotice.tsx` · `src/i18n/ko.ts` · 해당 시험(+ 기록기: 새 판 거절 뒤 같은 변경 번호로는 다시 읽지 않음 · 읽기 실패 거절은 디바운스 뒤 다시 · 머리 깨진 state.json 은 재판정에서 손상 → 떠 둔 뒤 씀 · `NotYet` 이면 쓸 만한 파일도 떠 둔 뒤 씀 · ★떠 두기 실패를 주입해도 그 쓰기가 발행됨(E5 · D8)★) | U3 · U8 | 중간 | **full** — GUI: `version: 2` 인 state.json → 기본 화면 + 새 안내 + 파일 바이트 그대로 + `restore.status` 의 `state_file` = `newer` · 실행 중 state.json 을 `version: 2` 로 바꾼 뒤 화면을 바꿔도 파일 그대로(로그에 거절 한 번) · `$version: 2` 인 settings.json → 설정 바꾸기 실패 + 파일 그대로 + 버스 `settings.set` 의 오류 코드 기록(D13) · 깨진 state.json → `.corrupt` 안내(회귀) |
| **U5a** | agent 저장소 채택 — agents · presets(레지스트리는 아직 지금 꼴: `Err` 를 로그로만) | agent `src/persistence/{mod.rs, presets.rs}`(공통 내부 · 적재 판정 · 저장 직전 재판정 · 떠 두기 · 상한 · 쓸기 · 옛 `.tmp` 지우기 · `write_lock` → 저장소 상태 칸(적재 거절 · 마지막 재판정 거절(D6) · `Claim`(D14)) · 모양 검사 = 명부 · 프리셋 칸 역직렬화(D1 — 버전 키는 필수가 아니게 · §3-4) · 운영 `OsHooks` 둘(데이터 · 적재 읽기 — D7) + 그 시험) · 트레이트 `profile.rs:304-309` · `preset.rs:39-44`(`save` → `Result` · `status`) · ★구현 가짜 16(§3-9 목록 — agent 4 · daemon 12 · D9)★ · 락 순서 문장(`session_id_latch.rs:23-24` · `manager.rs:516` · `profile.rs:364-365` · `preset.rs:58-59`) · 주석 `profile.rs:178 · 2013` · agent `commands.rs:69` · 시험(`corrupt_is_preserved_and_empty` ×2 · `version_mismatch_keeps_file` ×2 고침 · 새 판 거절 · 읽기 실패 거절 · 거절이 실행 내내 남음 · 재판정(손상 → 첫 저장 뒤 다시 떠 두지 않음) · ★모양 깨진 파일(항목 하나의 모르는 `kind`) → 적재 빈 목록 · 첫 저장 직전 `.corrupt`(D1)★ · ★적재 Missing 뒤 놓인 쓸 만한 파일 → 첫 저장이 떠 둔 뒤 씀 · 둘째 저장은 안 떠 둠(D14)★ · ★재판정 거절 뒤 `status` = `Refusing` · 성공 저장 뒤 `Writable`(D6)★ · BOM · 버전 없음 · 0 판 · 상한 · 옛 `.tmp` 지움) | U3 | 중간(−150/+200 · 시험 +14) | standard(+ 실 데몬 1회: 새 판 agents.json 이 기동 · 변경 시도 뒤에도 바이트 그대로 · 깨진 agents.json → 첫 저장 뒤 `agents.json.corrupt` · 모르는 `kind` 하나인 agents.json → 같은 결말 · 옛 `agents.json.tmp` 가 기동 뒤 사라짐) |
| **U5b** | 읽기 전용 모드(R17 · §3-9) | agent `src/profile.rs` · `src/preset.rs`(변경 입구 둘 — 부르는 쪽 있음 = 사본 → 저장 → 커밋 · 내부 = 저장의 어떤 `Err` 에도 커밋 + dirty(D2 · E1) · dirty 를 맵 옆에 · `epoch_for_spawn` 메모리만(D8)) · `src/manager.rs`(전파 · ★`Result` 판 다섯 — `create_agent` · `delete_agent` · `rename_agent` · `reparent_agent` · `set_agent_auto_restore`(`:912` · `:935` · `:952` · `:1026` · `:1031` — E4)★ · ★띄우기 경로의 정책 인자 `Entry` — 공유 함수 `spawn_agent_watching_link`(`:1191`) · `register_for_spawn`(`:1123`) · 두 동사(`:1770` · `:2100`) · 세 입구(`:1182` · `:1578` · `:2050`) — E3★ · 부분 성공(D8) · 내부 변경 `:543` · `:2011`) · `src/types.rs`(`PtyError` 저장 거절 변형 — 가안 · 잔손질) · `src/reaper.rs`(내부 변경) · `src/commands.rs`(`agent.new` · `spawn` · `rename` · `move` 오류 · 깨우기 · 만들고 띄우기의 저장 거절을 따로 싣는 오류 코드 매핑(가안 `CONFLICT` · §8-3 · `:976-977` · `:1015` — 잔손질) · `roster_changed` 는 D3 목록만 `Ok` 일 때 — `:975` · `:1008` 은 그대로(E2) · `agent.list` 상태 칸(`refusing` 포함 — D6) · `catalog_version` `:50` 6 → 7) · ★`crates/engram-dashboard-agent/tests/command_declarations.rs:75`(고정 세대 단언 6 → 7 — 잔손질)★ · `crates/engram-dashboard-agent/bindings/`(재생성) · daemon `src/connection_core.rs`(WS 답 `Err` · 변경 명령의 브로드캐스트 · 삭제 정리 훅은 `Ok` 일 때만(D3) · 활성화 뒤 브로드캐스트 `:1332` · `:1700` 은 그대로(E2)) · `prompts/engram-help.md:49-64` · 시험(거절 · 실패하는 가짜 저장소로 레지스트리 · manager · 버스 · WS + D2 · E1: 내부 변경이 `Io` · 재판정 거절 어느 쪽이든 메모리 바뀜 + dirty · 거절 동안 디스크 바이트 그대로 · 거절이 풀린 뒤 다음 저장에 실림 · 부르는 쪽 변경의 `Err` 가 dirty 변경을 안 지움 / D3: `DeleteProfile` 거절 → 훅 0 · 브로드캐스트 0 · `agent.rename` 거절 → `Renamed` 아님 · `roster_changed` 0 / E2: 거절하는 저장소로 깨우기 → `Err` + `roster_changed` 1 / E3: 소스 시험(두 입구가 같은 동사에 다른 `Entry`) + 부팅 복원 → `Started` · dirty 대 `activate_profile` → `Err` · 메모리 그대로(§3-9) / D8: `epoch_for_spawn` 이 저장을 안 부름(`set_last_failure` 시험 꼴) · 부르는 쪽 입구에서 ① 성공 ③ 실패 → 등록은 남고 칸은 옛 손잡이 · 띄우지 않음) | U5a · U4(`engram-help.md` 겹침) | 큼 — ★착수 전 크기를 잰다★. 크면 둘로 순차: **U5b-1** 레지스트리 · manager · reaper · `PtyError` 변형(입구 둘 · dirty · `epoch_for_spawn` · 띄우기 경로의 정책 인자 · 내부 변경 · ★매니저의 `Result` 판 다섯(E4)★ — `Result` 판을 옛 판 옆에 더하고 ★옛 판은 U5a 그대로 돈다(`Err` 는 로그 · 메모리는 적용 — E4: 4판의 「옛 판은 새 판을 불러 로그만」은 버렸다)★ · 띄우기 경로와 내부 변경은 이 단위에서 새 입구로 옮긴다 — 멈춘 트리가 안전한 까닭 = §6-4) → **U5b-2** 표면(버스 · WS 를 `Result` 판으로 옮김 · 부수효과 `Ok` 만(D3 · E2) · 저장 거절의 오류 코드 매핑(가안 `CONFLICT`) · 옛 판 삭제 · `agent.list` · 세대 · 고정 세대 단언 · 바인딩 · 도움말). 접점 = U5b-1 이 착지한 레지스트리 · manager 의 `Result` 판 서명 · `Entry` · `PtyError` 변형 | **full** — 실 데몬: 새 판 agents.json → `agent.new` 가 오류 · `agent.list` 에 읽기 전용 · 파일 바이트 그대로 · 실행 중 agents.json 을 새 판으로 바꾼 뒤 `agent.rename` → 오류 + `agent.list` 에 `refusing` · GUI 에서 에이전트 · 프리셋 만들기 · 지우기가 실패로 돌아옴(무엇이 보이는지 기록 · 명부가 안 바뀜) · 정상 파일로 바꾸고 데몬 재시작 → 풀림 |
| **U6** | 재시도 부르는 쪽(scratch) · 토큰 파일 — ★net 은 뺐다(transport 3-3 후속 · §3-6)★ | agent `src/usage/scratch.rs` · daemon `src/control/mcp_config.rs`(+ 운영 `OsHooks` 와 그 시험) | U1(scratch) · ★U3(mcp_config — R11 이 든 뒤라야 `create_dir_all` 을 걷는다)★ | 작음 | standard + 로컬 실 claude 스폰 1회(`--mcp-config` 경로 · 새 데이터 루트에서도 1회) · ★파일럿 먼저(§8-2 1)★ |
| **U7** | (g) 자식 띄우기 공용화 · 가드 규칙 · 가드는 트리째 | agent `src/transport/{mod.rs, spawn.rs(새), stdio.rs, pty.rs}` · `src/backend/codex/transport.rs`(가드 · 소스 시험 · 손자 시험 이사) · `ci.yml` 4f(`spawn.rs`) · 메모 §11 행(`:336`) 닫음 | U1(4f 명단 — 같은 heredoc 을 U1 이 먼저 고친다 · D11) | 중간 | **full** — claude(JSON · 터미널) · codex 스폰 · kill 뒤 생존 0(GUI) |
| **U8** | (h) 셸 락 오염 12곳 · saver 시계 | 셸 `settings/mod.rs` · `state/{boot_plugin.rs, restore.rs, placement.rs, saver.rs}` · `theme.rs` · `layout/apply.rs` | — | 작음 | standard |
| **U9** | 문서 · ADR 도장 · 낡는 기록 | §7 의 문서 목록 | U1~U8 | 문서 | `/qa` 문서 범위 + `/review doc`(load-bearing) |

### 6-2. 순서 · 물결

```
물결 1:  U1  ∥  U8              (겹침 없음)
물결 2:  U2  ∥  U7              (U2 = U1 의 fsutil · ci.yml · CLAUDE.md, U8 의 saver.rs 뒤 · U7 = U1 이 고친 4f heredoc 뒤.
                                 U2 ↔ U7 은 ci.yml 하나만 겹친다 — 게이트 ③ 스텝(:668) 대 4f heredoc(:941 근처), 다른 스텝)
물결 3:  U3                    (U2 의 base file · lib.rs · CLAUDE.md 와 겹친다)
물결 4:  U4  ∥  U5a  ∥  U6     (셸 대 agent 저장소 대 scratch · mcp_config — 파일이 안 겹친다. 셋 다 U3 의 API 를 소비만 한다)
물결 5:  U5b                   (U5a 의 profile.rs · preset.rs · manager.rs · connection_core.rs, U4 의 engram-help.md 와 겹친다
                                 — 크면 U5b-1 → U5b-2 순차 · §6-1)
물결 6:  U9
```

- ★4판에서 U7 을 물결 1 에서 물결 2 로 옮겼다(리뷰 2 라운드 D11 — 메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기)★ — 3판은 U7 을 U1 과 같은 물결에 두면서 선행 칸에는 「U1 뒤」를 적어 둘이 어긋났다(4f 명단이 같은 heredoc 이다). U7 은 U2 · U3 의 base `file` 을 쓰지 않으므로 물결 2 에서 U2 와 나란히 선다.
- ★U2 ∥ U7 은 워크트리를 갈라 돈다★ — 둘 다 `ci.yml` 에 한 덩어리씩 손대므로 같은 트리에서 동시에 고치지 않는다. 덩어리가 멀어(다른 스텝) 합치기가 깨끗하다. 갈라 돌지 않으면 순차(U2 → U7).

- ★병렬 전 접점을 못 박는다★ — §3-2 의 platform API 와 §3-3 의 base `file` API(이름 · 모양 · 판정 표 · `OsHooks`)가 U2 · U3 에 착지한 모양, §3-9 의 트레이트 모양이 U5a 에 착지한 모양이 뒤 단위의 지시서에 그대로 들어간다. 구현 중 모양이 바뀌면 메인이 이 절을 고친 뒤 다음 물결을 띄운다.
- 사람 한 명이 직렬로 돌면: U8 → U1 → U2 → U3 → U4 → U5a → U5b → U6 → U7 → U9.

### 6-3. 파일 겹침

| | U1 | U2 | U3 | U4 | U5a | U5b | U6 | U7 |
|---|---|---|---|---|---|---|---|---|
| U2 | `fsutil.rs` · `ci.yml`(4f 대 게이트 ③ — 다른 스텝) · CLAUDE.md | | | | | | | |
| U3 | — | base `file*` · base `lib.rs` · CLAUDE.md(base 항목) | | | | | | |
| U4 | — | `settings/store.rs` · `state/{codec,boot,saver}.rs` | — | | | | | |
| U5a | — | — | — | — | | | | |
| U5b | — | — | — | `prompts/engram-help.md` | `profile.rs` · `preset.rs` · `manager.rs` · `connection_core.rs`(5판: `session_id_latch.rs` 는 U5b 에서 안 고친다 — E1 로 머리 전제가 다시 참) | | | |
| U6 | — | — | — | — | — | — | | |
| U7 | `ci.yml` 4f(같은 heredoc — U1 뒤) | `ci.yml`(게이트 ③ 대 4f — 다른 스텝 · 물결 2 에서 나란히) | — | — | — | — | — | |
| U8 | — | `state/saver.rs` | — | `state/{restore,boot_plugin,saver}.rs` · `settings/mod.rs`(D13) | — | — | — | — |

- 4판 대조(D11 뒤 다시 봤다): 같은 물결 안의 쌍은 U1 · U8(—) · U2 · U7(`ci.yml` 다른 스텝) · U4 · U5a · U6(셋 다 —)뿐이다. 나머지 겹침은 전부 물결이 갈린 쌍이다. U5a 가 손대는 시험 가짜(`manager.rs:3073` · `:5016` — D9)는 U5b 와만 겹친다(위 칸의 `manager.rs`).

### 6-4. 단위 안 순서 (빌드가 서게)

- **U1** ① platform 에 API · 시험(아무도 안 부름) ② fsutil 이 platform 을 부르게 · 제 판정과 그 시험을 지운다 ③ 4f 명단 · 문서. ②에서 끊기면 ①만 남은 초록 트리로 되돌린다.
- **U2** ① base `file` 을 fsutil 사본으로 세우고(공개 함수는 `OsHooks` 인자 · 시험은 비공개 `_with` 이음매에 클로저 — D15) 헤더 · 게이트 ③ 사본 여섯 · 입주자 명단 사본 셋(D9)을 함께(아무도 안 부름) ② 셸 운영 `OsHooks` 상수 · 그 시험 ③ 셸 호출부를 파일 단위로(`settings/store.rs` → `state/boot.rs` → `state/saver.rs` → `state/codec.rs`) ④ `fnv1a` 를 codec 으로 ⑤ `fsutil.rs` · `mod fsutil` 삭제. fsutil 과 base `file` 은 이 단위 안에서만 공존한다.
- **U3** ★① base `Cargo.toml` 에 `serde_json` 을 먼저 더한다(D10 — 아무도 안 써도 빌드가 선다 · `unused_crate_dependencies` 는 기본 허용)★ ② 새 타입 · 함수(`Spec` + `shape` · `any_shape` · `Parsed` · `Loaded` · `classify` · `load` · `Claim` · `WritePolicy` · `judge` · `check_cap` — `serde_json::Map` 을 쓰므로 ① 뒤라야 선다 · 아무도 안 부름)와 그 시험 ③ `write_atomic` · `copy_atomic`(과 `_with`)에 R10 · R11 ④ 헤더 · CLAUDE.md · 「의존성」 보고. 3판은 ②를 ①보다 앞에 두어 그 걸음에서 빌드가 깨졌다(리뷰 2 라운드 D10). ★R11 · R10 은 셸 쓰기에 바로 닿는다★(상태 기록기 · 떠 두기가 폴더를 만들게 된다 · 폴더 동기화가 불린다 — Windows 무동작) — 단위가 선언한다. 셸 쪽의 겹친 `create_dir_all` · 주석은 U4 가 걷는다(그 사이에는 중복일 뿐 틀리지 않는다 — 단 `boot.rs:81` 주석이 그동안 낡는다).
- **U4** ① 설정(`read_document` → `classify` · `parse`(`any_shape`) · `write` 의 판정 · `LoadNote::Newer` · 거절의 오류 코드 — D13) ② codec(머리 해석을 상태 `Spec::shape` 로 — D1) ③ `decide_boot` 의 새 판 가드 ④ 기록기 재판정(★`StateFiles` 에 읽기 · 떠 두기 두 연산을 먼저 더하고 실물 · 가짜를 같은 걸음에 — E5★ · `Refuse` 두 갈래 — D5 · 부팅에서 받은 `Claim` — D14 · 떠 두기 실패면 계속 쓴다 — D8) ⑤ `StateFileStatus::Newer` · 매핑 · 바인딩 · LLM 표면 다섯 · 고정 세대 단언(`src-tauri/tests/layout_commands.rs:619` 15 → 16 — 세대를 올리는 같은 걸음에) ⑥ 프론트 셋 ⑦ 겹친 `create_dir_all` · 주석 걷기. ⑤ 뒤 ⑥ 전에 끊겨도 선다 — 프론트는 모르는 값을 `undefined` 로 두고 알림 하나만 잃는다(`restoreClient.ts` 머리 주석).
- **U5a** ① 트레이트에 `status`(기본 구현)를 더하고 `save` 를 `Result` 로 — ★구현 18곳(운영 2 · 가짜 16 — D9)★을 같은 걸음에(가짜는 `Ok(())`) · 레지스트리는 `Err` 를 지금처럼 로그로 ② `FileProfileStore`(공통 내부 · 적재(긴 예산 — D7) · 모양 검사(D1) · 판정 · 저장 · 쓸기 · 저장소 상태 칸(적재 거절 · 재판정 거절 · `Claim`)) ③ `FilePresetStore` ④ 시험 ⑤ 락 순서 문장 · 주석. 걸음마다 빌드가 선다. ★U5a 뒤 U5b 전의 트리는 「거절은 하지만 메모리 변경은 성공처럼」인 중간 모양이다 — 덮어쓰기 결함은 닫혔고 읽기 전용 표면은 아직이다. 그 사이에 머지하지 않는다★.
- **U5b** ① `epoch_for_spawn` 을 메모리만으로(D8 — 혼자 서는 걸음 · 서명 그대로) ② 레지스트리에 변경 입구 둘(부르는 쪽 있음 = `Result` 판 · 내부 = 저장의 어떤 `Err` 에도 커밋 + dirty — D2 · E1)과 dirty 칸을 더한다(옛 판 옆에 — 옛 판은 U5a 그대로 돈다: `Err` 는 로그 · 메모리는 적용) ③ manager · reaper · `PtyError` 를 파일 단위로 — ★manager 의 `Result` 판 다섯(`create_agent` · `delete_agent` · `rename_agent` · `reparent_agent` · `set_agent_auto_restore`)을 옛 판 옆에 더한다 · 옛 판은 U5a 그대로(E4)★ · 띄우기 경로는 정책 인자 `Entry` 를 공유 함수 하나에 실어 새 입구로(E3) · 내부 변경 넷(래치 `:543` · `:2011` · 감시자 · reaper)은 내부 입구로 · 이 걸음의 시험(레지스트리 · manager — D2 · E1 · E3 · D8) ④ 버스 명령 · WS 를 `Result` 판으로 옮기며 부수효과를 `Ok` 갈래 안으로(D3 — 실패 때도 도는 알림은 그대로, E2) · 저장 거절의 오류 코드 매핑(가안 `CONFLICT` — §8-3) ⑤ 옛 판을 지운다 ⑥ `agent.list` 상태 칸(`refusing` 포함) · 세대 · 고정 세대 단언 · 바인딩 · 도움말 ⑦ 표면 시험(D3 · E2 · 버스 · WS). ②까지는 겉으로 보이는 동작이 U5a 와 같다(① 은 디스크에 실리지 않는 값의 저장 하나를 뺄 뿐이다) — 어느 걸음 뒤에 끊겨도 빌드가 선다. U5b-1 · U5b-2 로 가르면 ①~③ · ④~⑦ 이다.
  - ★**U5b-1(③) 뒤에 멈춘 트리는 동작상 안전하다(E4 — 메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기)**★ — 표면(버스 · WS)은 아직 옛 판을 부르고 옛 판은 U5a 그대로라 답과 메모리가 어긋나는 자리가 없다. 예: WS `DeleteProfile` 은 옛 `delete_agent` 로 메모리에서 지운 뒤 `Ok` · 정리 훅을 낸다 — 지운 프로필의 정리라 산 프로필의 메시징을 건드리지 않는다(4판 §6-1 의 「옛 판은 새 판을 불러 로그만」이었다면 거절 때 지워지지 않은 프로필에 `Ok` · 정리 훅이 나갔다 — 리뷰 3 라운드 E4). ③ 에서 새 입구로 옮긴 두 갈래도 일관된다 — 띄우기 경로의 입구(`activate_profile` · `spawn_agent`)는 이미 `Result` 이고 버스 · WS 가 그 `Err` 를 이미 오류로 싣는다(저장 거절은 ④ 의 매핑 전까지 `INTERNAL` · 실패 뒤 알림은 지금 그대로 — E2) · 내부 입구는 메모리에 늘 적용하므로 보이는 것이 U5a 와 같다(dirty 뒤 저장이 하나 느는 것뿐). 남는 것은 U5a 와 같은 중간 모양(표면이 옛 판인 변경은 「거절은 하지만 메모리 변경은 성공처럼」)이라 U5a 처럼 그 사이에 머지하지 않는다.
- **U6** ① 파일럿 — scratch 지우기 실패 코드 실측 ② mcp_config(운영 `OsHooks` · 그 시험) ③ scratch.
- **U7** ① `spawn.rs`(가드 — 트리 끄기 포함 · `spawn_piped` · `drain_stderr` — 아무도 안 부름) ② codex 를 그리로(가드 타입 · 소스 시험 · 손자 시험 이사) ③ stdio ④ pty ⑤ 세 통로를 함께 재는 소스 시험 · 4f.
- **U8** 파일마다 한 걸음 · saver 는 하위 트레이트를 먼저 세우고 `SystemClock` · `FakeClock` 을 옮긴 뒤 옛 트레이트를 지운다.

- **U5b-1 지시서에 넣을 것(리뷰 4 라운드 doc-aware 잔손질)** — ⑴ 소스 시험의 바늘 `only("self.register_for_spawn(profile)?;")`(`manager.rs:6285`)는 `entry` 인자가 붙으면 깨진다 — §3-9 의 「그대로」가 아니라 U5b-1 ③ 에서 그 문자열을 고친다 ⑵ E3 시험은 입구만이 아니라 두 동사와 `spawn_agent_watching_link` 안에 `Entry::` 글자가 없음(통과만)도 잰다 — 그렇지 않으면 핸들 없는 재개의 안쪽 `spawn_fresh_settled` 호출(`:2127`)에 `Caller` 를 박아 부팅 복원 갈래가 조용히 부르는 쪽 의미를 탈 수 있다 ⑶ 「부팅 복원 → ① · ③ 적용」 시험은 `fresh_spawn_release_session_id = true` 인 명령(claude · codex)으로 — 셸이면 ③ 이 안 돈다(`manager.rs:3141`) ⑷ 데몬 `src/lib.rs:276`(감시자 → 내부 입구)을 U5b 파일 목록에 넣거나 내부 메서드가 이름 · 서명을 지킨다고 적는다 ⑸ 부르는 쪽 입구의 저장 거절이 `last_failure = SpawnFailed`(`note_spawn_result` `:1737`)로 적혀 화면 · `agent.list` 에 띄우기 실패로 보인다 — 그 문구가 괜찮은지 U5b-2 에서 메인이 정한다 ⑹ ADR-0019 주석은 `manager.rs:2002` 에서 시작한다.

### 6-5. 단위마다 검증

1. **회귀 수 대조** — `cargo test --workspace -- --test-threads=4` 를 단위 앞뒤로 돌려 `test result:` 줄 수와 통과 총계를 둘 다 견준다(TRD 1-3 §5-2 의 명령 그대로). 기준선 = 마지막 기록 결과 줄 59 · 4233 통과 · 0 실패 · 31 무시(2-4 `/qa standard` · 2026-10-08) — ★U1 착수 직전에 다시 잰다★(이 TRD 는 돌려 보지 않았다).
2. **기대 차이** — 결과 줄: 모든 단위 0(새 crate 없음). 분포: U1 셸 → platform 5 · platform 새 시험 +1(`sync_dir`) · U2 셸 → base **21**(fsutil 27 − U1 의 5 − 셸 `codec` 으로 가는 fnv1a 1) · 셸 새 시험 +1(운영 `OsHooks`) · U3 · U4 · U5a · U5b · U6 · U7 은 단위가 수를 선언한다(옮김 ±N · 새 시험 +k).
3. **게이트** — 건드린 crate 의 게이트 전부 + `cargo fmt --check` + 생성물 sync(U4 = `src-tauri/bindings/` · U5b = `crates/engram-dashboard-agent/bindings/` — 생성물을 함께 커밋) + `cargo test -p engram-dashboard --test lib_unit`(셸이 바뀌는 단위) + 4f(U1 · U7) · base 게이트 ③(U2 뒤 — 알파벳에 `file`).
4. **QA** — 표의 등급(`/qa` 바인딩). full 은 앱을 `scripts/` 런처로 띄운다(셸에서 직접 띄우지 않는다).

---

## 7. ADR · 문서 후속

**새 ADR 하나 — 번호 미정 · U1 바로 앞에 쓴다**(채번 · 링크 · 도장 = `/adr`). ★쓰는 그 자리에서 모든 로컬 · 원격 브랜치의 `docs/decisions/` 를 다시 보고 다음 빈 번호를 선점한다(번호 선점 커밋 — 2-3 · 2-4 꼴)★ — 다른 브랜치가 그 사이 쓸 수 있으므로 이 TRD 는 번호를 적지 않는다.

- **박을 것:** 사용자 결정 1a~2h · 3 · 5(§1 — 1d 의 「writable 은 뺀다 · net 은 transport 3-3 후속」 포함) · 거처 = base `file` + 부르는 쪽이 넘기는 `OsHooks`(§3-1) · ADR-0269 결정 6 「지킬 것」 개정(`serde_json` 을 base 에 — 직렬화는 여전히 주인) · R1~R15(메인 판단(위임 「성숙한 프로그램 관행대로」)) — ★특히 R2: 새 판 파일은 네 파일 모두 덮지 않는다 · TRD S21-storage §6-5 · §12 R3 를 뒤집고 ADR-0265 결정 4 · ADR-0274 를 좁힌다★ · R7 · R8(읽기 IO 실패 = 저장 거절) · R16(저장 직전 재판정 · 프로세스 간 잠금 없음 — 메인 결정) · R17(읽기 전용 모드 — 메인 결정 · 적재 거절은 실행 내내) · (g) 가드 규칙(트리째 끄기 포함) · ★리뷰 2 라운드 메인 결정(위임 · 잠정 — 사용자 확인 대기 · 첨부 r2)★: 실행 중 바깥 편집은 지원하지 않는다 · 셸 단일 작성자 근거 = 단일 인스턴스 플러그인(주) + 실행 잠금(보조)(C1 · R16) · 모양 검사는 `Spec` 에 — 모양 실패는 어디서나 손상(D1 · R7) · 내부 변경의 실패 처리 · dirty(D2) · 부수효과는 `Ok` 일 때만(D3) · 락 안 최악 약 0.4 초 수락(D4) · 기록기의 거절 두 갈래(D5) · `agent.list` 의 셋째 상태(D6) · 적재 읽기의 긴 예산(D7 · R14) · `epoch_for_spawn` 메모리만 · 띄우기 경로 부분 성공(D8) · `Claim` — 적재 때 없음 · 손상이면 첫 저장이 쓸 만한 파일도 떠 둔다(D14 · R8) · `OsHooks` fn 포인터 + 비공개 클로저 이음매(D15) · ★리뷰 3 라운드 메인 결정(위임 · 잠정 — 사용자 확인 대기 · 첨부 r3)★: 내부 입구는 저장의 어떤 `Err` 에도 메모리 적용 + dirty · 부팅 복원도 거절 때 띄우기를 이어 간다(E1 — D2 를 넓힘) · 실패 때도 도는 활성화 알림은 「`Ok` 일 때만」 밖(E2 — ADR-0172) · 부팅 복원 = 수동 깨우기와 같은 공유 함수에 정책 인자 하나(E3 — 사본 금지 · ADR-0082 · 0201 · 0202) · 셸 단일 작성자 근거의 뮤텍스는 번들 식별자 단위(R16 고침).
- **거부한 대안:** §3-1 의 A · B · C · D′ · 「D 를 기각했던 옛 사유와 다시 본 결과」 · R14 의 「한 예산으로 맞추기」 · R2 의 「새 판도 떠 두고 덮기(지금 상태 · 설정의 규칙)」 · R16 의 「적재 때 정한 판정을 들고 있기」 · 「프로세스 간 잠금」 · ★「파일 신원 · 내용 지문을 기억해 저장 직전에 대조하고 다르면 거절 · 다시 적재」(C1 — 근거 = R16 의 메인 결정 문장)★ · R17 의 「로그만 · 메모리 변경은 성공처럼(2판 · 지금 동작)」 · 「실행 중 다시 적재해 읽기 전용을 푼다」 · ★「내부 변경도 전부-아니면-전무」(3판 · D2 — 래치 전제가 깨지고 일시 실패 하나가 세션 id · `auto_restore` 를 영영 잃는다)★ · ★「내부 변경도 재판정 거절이면 적용하지 않는다 · 부팅 복원은 그때 `Failed`」(4판 · E1 — 짧은 읽기 잠김 하나가 reaper 의 `auto_restore=false` 를 버려 죽은 에이전트가 다음 부팅에 되살아나고 · 래치 세션 id 를 잃고 · 복원이 실패한다. 디스크는 재판정이 이미 지킨다)★ · ★「부팅 복원 전용 띄우기 함수(입구별 사본)」(E3 — ADR-0082 · 0201 · 0202 가 막은 입구별 판정 갈림을 되살린다)★ · (g) 의 「멈춘 채 띄우기로 바꾼다」(범위 밖 — §3-7) · 1d 의 「writable 도 platform 정책으로」(§8-1 1). 결정 날조 금지 — 거부 근거는 이 TRD 와 첨부에 적힌 것만 옮긴다.

**개정 대상(`Amends` + 개정당하는 쪽 도장):**

| ADR | 무엇이 바뀌나 |
|---|---|
| ADR-0269 결정 2 표 `file` 행 · 결정 6 「지킬 것」 · 「영향」의 원자적 쓰기 · 손상 사본 목록 | `file` 이 원자 쓰기 · 손상 사본 · 읽기 규칙 · 쓰기 판정 전부를 갖는다(옛 행: `set_aside_corrupt(path, stamp_ms)` · 「원자적 쓰기는 빠졌다 — platform 으로」) · 결정 6 첫째(「base 가 serde 를 안 끌어옴 · `file` 은 바이트만」) → `serde_json` 을 들인다(읽기 판정 — 구조체 직렬화는 주인) · 둘째(「시각은 `time` 을 안 부르고 인자로」) → 고정 이름이라 해당 없음 · 넷째(「원자적 쓰기는 platform 이 생긴 뒤 1-3 으로」) → base `file` · 재시도는 인자 · 「영향」 목록의 `reject_store.rs` |
| ADR-0269 결정 1 · 7 · 9 | ★바뀌지 않음(확인)★ — 1(범용 코드를 받는 자리 · 쪼개기는 나중에 base 에서 — 이번 사용자 문장도 같은 말이다) · 7(입주 판단 = 지금 여러 곳에 쓰이는지 — §3-1 조건 ①) · 9(바닥 기반층 — JSON 을 담는 `//base` 선례) |
| ADR-0266 결정 8 | 원자 쓰기 거처 = platform → base `file` — 근거였던 「함수 안의 OS 분기」가 platform `is_busy` 로 떨어지고 base 는 그 결과를 인자로 받는다 · platform 에는 잠김 규칙 · 다시 하기 · 폴더 동기화만 · 둘째 항목(「`set_aside_corrupt` 는 base `file` — rename 하나로 OS 분기 없음」)은 ADR-0274 뒤 거짓이었고, 거처는 다시 base `file` 이 된다(재시도는 인자 — 사유가 바뀐다) · 사본 목록의 `reject_store.rs` |
| ADR-0275 결정 15 | 시한부 예외를 닫는다 — platform 밖 운영 OS 분기 0 |
| ADR-0265 결정 4 | `write_atomic` 자리(셸 `fsutil.rs` → base `file`) · 「모르는 `$version`」 중 새 판은 떠 두지 않고 쓰기 거절(R2 — 좁힌다) |
| ADR-0274 | 적용 범위가 설정 · 상태에서 네 파일로 · 「통째로 못 쓸 파일」에서 새 판을 뺀다(R2 — 좁힌다) · 「떠 두기는 `fsutil::copy_aside` 한 곳」의 자리 → base `file::copy_aside` |
| ADR-0061 | presets 는 profiles 를 「복제」하지 않고 같은 구현을 부른다 |
| ADR-0071 | 락 순서의 잎 = `write_lock` → 저장소 상태 칸(`:26`) · `mutate` = 부르는 쪽 있는 변경은 사본에 적용 → 저장 → 성공일 때만 커밋(거절 · 실패면 메모리 그대로 — R17) · 부르는 쪽 없는 내부 변경은 저장의 어떤 `Err`(`Io` · 재판정 거절)에도 커밋 + dirty(D2 · E1) — `persisted == observed` 는 dirty 동안만 어긋나고 그 동안은 표시가 선다(디스크는 재판정이 지킨다) |
| ★ADR-0163 결정 7 — 포인터 개정(리뷰 3 라운드 잔손질)★ | 「판정과 커밋은 한 락 안에서 한다」는 그대로다. 그 결정이 가리키는 자리 「`profile.rs:601-609` 의 `mutate` 클로저」(지금 줄로는 `epoch_for_spawn` `profile.rs:775-785`)가 낡는다 — D8 로 `epoch_for_spawn` 은 `mutate` 를 타지 않고 락만 잡아 표식을 간다(저장 없음 · `set_last_failure` `profile.rs:627` 꼴). 개정 표시로 새 자리를 적는다 |
| ADR-0264 결정 2 그림 · 결정 7 | 그림의 `(+ .corrupt-* 사본)`(`:21`)이 `.corrupt` 로 낡는다(개정 표시) · 결정 7 의 꼬리 패턴 `**/data/daemon/state/agents.json*`(`:35`)은 새 이름(`.corrupt` · `.tmp<pid>.<번호>`)도 덮는다(확인) |
| ★ADR-0207 — 개정 확인(D4)★ | 「락을 쥔 채 저장 · 보유 시간을 줄이지 않는다」는 그대로다. 바뀌는 것 = 락 안에서 하는 일이 는다(그 파일 다시 읽기 · 파싱 · 모양 검사 · 손상이면 떠 두기 · 잠김 다시 하기) — 보통은 작은 파일 읽기 하나 더 · 최악은 잠김 다시 하기의 잠만 약 0.4 초(§3-9). 그 ADR 의 재론 방아쇠 ②(「저장 1회의 락 보유 시간이 ms 단위를 넘는다」)에 이 최악이 설계상 닿는다는 것과 수락한 까닭(판정과 쓰기가 같은 락 안이어야 한다 · 최악은 파일이 잠긴 동안뿐 · 저장은 수명 사건에서만)을 그 ADR 에 개정 표시로 남긴다. 방아쇠가 여는 자리((d) · (e))는 바꾸지 않는다. 근거(b)의 `set_last_failure` 선례에 `epoch_for_spawn` 이 더해진다(D8) |
| ★ADR-0226 — 개정 확인(D2 · 5판 E1 로 좁힘)★ | 「세션 id 영속 = 첫 제출 래치 한 지점 · 비교-교체」는 그대로다. 바뀌는 것은 **디스크 영속 시점** 하나다 — 래치의 commit 은 늘 메모리 명부에 적히고(내부 입구 — 저장의 어떤 `Err` 에도 커밋 + dirty, E1), 디스크에는 그 저장이 성공하면 첫 턴 전에, 쓰기 IO 실패 · 재판정 거절이면 다음 성공 저장(거절이면 그것이 풀린 뒤)에 실린다 — 그 사이 데몬이 꺼지면 다음 활성화는 새 대화(codex JSON 의 「되울림 전에 죽으면 영속이 없다」와 같은 결말 · 지금 실패한 저장과도 같다). 읽기 전용(적재 거절)이면 명부가 비어 그 화신이 뜰 수 없다. ★래치 머리(`session_id_latch.rs:14-16`)의 전제 「포트는 반환이 없어 결과가 래치에 돌아오지 않고, 같은 입력이면 답도 같으므로 재시도할 것이 없다」는 E1 로 다시 참이다★ — 비교-교체의 답(`manager.rs:543`)이 메모리만으로 정해진다. 4판이 적었던 「그 전제가 늘 참은 아니다 · 그 문장을 U5b 가 고친다」는 거둔다(래치 파일은 U5b 에서 안 고친다). ADR-0226 에는 영속 시점의 개정 표시만 단다 |
| 바뀌지 않음(확인) | ADR-0135(portfile 제자리 쓰기) · ADR-0282(쓰기 프로브 · `retry_if_vanished` 는 base `writable` 에 그대로 — §8-1 1) · ADR-0270(셸 → agent 없음 — base 는 agent 가 아니다) · ADR-0116 결정 3(프로필 삭제 정리 훅은 유일한 제거 지점에 하나 — 그 지점이 이제 삭제가 성공했을 때만 부른다(D3) · 조항은 그대로) · ADR-0172(실패 때도 명부 알림 — E2 로 그대로 지킨다) · ADR-0082 · ADR-0201 · ADR-0202(부팅 복원과 수동 깨우기는 같은 동사 — E3 이 사본 대신 인자로 지킨다) |

**문서(각 단위 또는 U9):**

1. CLAUDE.md — 「플랫폼 중립」(`:103` — fsutil 예외 문장 · `cfg` 운영 분기 0 — U1) · 「백엔드 모듈 맵」 base 항목(`:158` — 입주자 여덟 · `file` · `serde_json` · `bindings/` 문장 — U2 · U3) · platform 항목(`:159` — `fs` 입주자 설명 · 「이 crate 밖에 남은 `cfg` 운영 OS 분기」 문장 — U1) · 「빌드·검증 명령」 base 시험 줄(`:233`) · 게이트 ③ 줄(`:254`) · 「의존성」 절(base `serde_json` 보고 — U3) · ★「핵심 불변식」 세션 id 영속 항목(`:181` — 「래치는 … commit 하므로 id 가 첫 턴 전에 영속되고」에 「디스크는 저장이 실패 · 거절된 동안 다음 성공 저장까지 늦어진다(dirty — E1)」 한 줄 · U5b — 5판을 쓰며 찾은 것)★ · 워크스페이스 수치 줄(U 들의 실측).
2. base `src/lib.rs` 헤더(입주자 · 게이트 ③ · 생성물 문장) · base `Cargo.toml` description · platform `src/lib.rs` 헤더(`:7-10` · `:16` · `:42`).
3. `.github/workflows/ci.yml` · `.claude/skill-bindings/qa.md` · `docs/testing-strategy.md` — §5-2 의 사본 · ★입주자 명단 사본(D9 — U2)★: 루트 `Cargo.toml:14`(멤버 주석) · `docs/testing-strategy.md:43`(① 단위 목록) · `:160`(base 시험 줄 주석).
4. TRD `docs/process/S21-storage/trd.md` — §6-5 표의 state.json 「버전 초과」 행 · §12 R3 의 「새 판이 쓴 `state.json` 은 버전 초과로 떠 두고 기본 화면」 · N6 — 개정 표시와 이 TRD 를 가리키는 줄.
5. TRD 1-1 §8 D1 · D2 · D3 · TRD 1-3 §5-1 U-W 행 · §9 「storage P3 착지 시점」 — 「→ TRD A」 표시(날짜 박힌 본문은 고치지 않는다).
6. 메모 `docs/refactoring/architecture-discussion-2026-09-26.md` §11(`:336` stdio · pty 가드 행 닫음).
7. `docs/tracking.md` T-47 상태 · T-33 에 「새 판 몫만 닫혔다」 한 줄(§8-2 4) · transport 3-3 의 후속 한 줄 — 「net 단일 인스턴스 열기 재시도는 3-3 에서 platform `retry_busy` 를 쓴다(예산 4 × 100 ms · TRD A §3-6 의 메모)」(3-3 TRD 가 서면 그쪽으로 옮긴다).
8. `docs/reference/structure/session-path-ownership.md:821 · 887-888 · 2000` · `docs/reference/structure/agent-backend.md:237` — 손상 사본 이름 · 새 판 덮어쓰기 서술 · 읽기 전용 모드.
9. `docs/process/step-log.md` — 착지 항목(단위마다 아니라 A 전체 한 항목).
10. 코드 앵커 — base `file` 헤더 · `write_policy` · platform `is_busy` · `sync_dir` · 레지스트리 `mutate` 의 두 입구 · dirty(새 ADR · ADR-0071) · 띄우기 공유 함수의 정책 인자 `Entry`(새 ADR · ADR-0082 — E3) · `epoch_for_spawn`(새 ADR · ADR-0207 · ADR-0163) · `// ADR-0274` 는 `copy_aside` 를 따라간다. 래치 머리는 고치지 않는다(E1 — 전제가 다시 참).

---

## 8. 범위와 어긋난 것 · 미검 · 열린 것

### 8-1. 범위와 어긋났던 것 — 메인 · 사용자 결정(2026-10-10)

1. **base `writable.rs` `retry_if_vanished` 는 1d 에서 뺀다(base 에 그대로).** 잠김 재시도가 아니다 — `NotFound` 일 때 한 번 더 하는 것이고 까닭은 「검사 도중 남이 폴더를 지웠다」는 경합이다(`writable.rs:86-97`). OS 규칙이 없어 platform 헌장(OS 의존 여부)에 안 맞고, ADR-0282 가 그 자리를 base 로 정했다. 넣으려면 platform 에 OS 무관 코드가 들어간다.
2. **「새 판은 덮지 않는다」는 네 파일 모두다** — 메인 판단(위임 「성숙한 프로그램 관행대로」 — 새 판이 쓴 파일을 옛 판이 덮지 않는다). 상태는 TRD S21-storage §6-5 · §12 R3(「새 판 `state.json` 은 떠 두고 기본 화면 · 하향 지원 안 함」)을 뒤집고, 화면 안내 · 버스 값(`newer`)이 새로 생긴다(§4 2 · 4). 설정은 ADR-0265 결정 4 · ADR-0274 의 「모르는 `$version` = 못 쓸 파일」을 좁힌다. 새 ADR 에 든다(§7).
3. **`layout/apply.rs:247` 을 U8 에 넣는다** — 셸 락 오염 복구가 처음 목록보다 하나 많았다(운영 · 같은 꼴 · 1-1 집계 뒤 들어왔다). `discovery/layout.rs:235` 는 시험이라 뺀다.
4. **R7 · R8 유지 — 읽기 IO 실패도 저장을 거절한다.** agent 의 「읽기 IO 실패 → 빈 목록 → 덮음」(`persistence/mod.rs:107-110` · `presets.rs` 같은 자리)은 새 판 결함과 같은 부류라 함께 닫는다 — 사용자가 보는 변화다(§4 3). 3판에서 그 거절은 읽기 전용 모드로 드러난다(R17).
5. **net 몫은 U6 에서 뺀다** — net 은 transport 3-3 에서 갈린다. 후속 = 「transport 3-3 에서 platform retry 를 쓴다」(§3-6 의 메모 · §7 7). 그때까지 net 의 열기 재시도와 2a 게이트는 그대로다.
6. **ADR-0266 결정 8(「`write_atomic` 은 platform 으로」)은 개정한다 — 3판에서 거처는 base `file`** — 그 결정은 원자 쓰기 함수 안의 OS 분기 한 줄 때문에 함수째 OS 층에 두자는 것이었다. 분기를 `is_busy` 로 떼고 그 결과를 인자로 받으면 그 근거가 사라진다(§3-1 · §7).
7. **거처 = base(사용자 결정 2026-10-10 — §1 5)** — 2판의 A(새 crate)를 철회하고, 2판이 기각했던 D(재시도를 부르는 쪽이 넘긴다)를 다시 봐 고쳐 고른다(§3-1). ADR-0269 결정 6 개정은 새 ADR 에 든다.
8. **저장 거절 = 읽기 전용 모드(메인 결정 2026-10-10 — R17 · §3-9)** — 2판의 「agents · presets 거절 = 로그만 · 메모리 변경은 성공처럼」(2판 R15 · §4 2)을 버린다. 부팅 때 일시 잠김이 재시도 예산 안에 안 풀린 경우도 같은 모드다.
9. **저장 직전 재판정 · 프로세스 간 잠금 없음(메인 결정 2026-10-10 — R16)** — 2판의 「판정을 적재 때 기록해 두고 저장이 따른다」(2판 §3-6)를 버린다.
10. **리뷰 2 라운드의 메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기(C1 · D1~D15)** — 항목별 자리는 머리의 4판 개정 목록. 사용자 결정을 뒤집는 것은 없다 — 3판의 메인 결정 둘을 고친다: 「부르는 쪽 없는 내부 변경도 거절이면 적용하지 않는다」를 `Io` 실패에서는 적용 + dirty 로(D2) · 「레지스트리 변경은 모두 사본 → 저장 → 커밋」을 부르는 쪽 있는 변경으로 좁힘(D2). ★앞 것은 5판에서 다시 넓혔다(→ 11)★
11. **리뷰 3 라운드의 메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기(E1~E5 · 잔손질)** — 항목별 자리는 머리의 5판 개정 목록. 사용자 결정을 뒤집는 것은 없다 — 4판의 메인 결정(위임) 셋을 고친다: D2 의 앞 절반(「내부 변경은 `ReadOnly` 면 적용하지 않는다」)을 「저장의 어떤 `Err` 에도 적용 + dirty」로(E1) · 부팅 복원의 「이번 쓰기 거절이면 `Failed`」를 「띄우기를 이어 간다」로(E1) · D3 목록에서 실패 때도 도는 알림 둘을 뺌(E2). 부르는 쪽 있는 입구의 뜻(어느 `Err` 든 오류 · 메모리 그대로)은 바뀌지 않는다.

### 8-2. 미검

1. **scratch 지우기가 실제로 어떤 오류로 실패하나** — 지금 코드는 모든 오류를 다시 하고(`scratch.rs:76-86`), 실측(2026-09-27 40/40)은 시간만 남겼다. `is_busy` 가 그 코드를 못 물면 손자가 쥔 폴더를 다시 안 해 쓸기로 밀린다(기능 손실은 없지만 동작이 바뀐다). U6 ① 파일럿이 잰다 — 145 등이면 platform 판정에 더할지 메인이 정한다.
2. **Windows 폴더 동기화 = 무동작으로 둔 근거** — 2판은 「std `File::open` 이 폴더를 못 연다」를 U3 실측 거리로 두었다. 3판은 platform `sync_dir` 의 Windows 갈래를 명시 무동작으로 정했으므로 그 실측은 결론을 바꾸지 않는다(지금 agent best-effort 의 결과가 무동작이라는 것만 확인 거리로 남는다). 「SQLite 의 Windows VFS 도 폴더를 동기화하지 않는다」는 기억 인용 · 미검.
3. **관행 인용은 원문 대조를 안 했다** — Cargo.lock(R1) · SQLite · Firefox(R2) · LevelDB · SQLite 유닉스 경로 · SQLite Windows VFS(R10) · mkdir -p(R11) · graceful-fs · Node `fs.rm`(R13) · Firefox `prefs.js` · Chrome `Preferences` 의 「실행 중 바깥 편집 미지원 · 마지막 작성자가 이긴다」(R16 · C1) · SQLite 읽기 전용 쓰기 오류(R17). RFC 8259 §8.1(R5)만 확신이 높다. 규칙은 지금 셸 규칙과 실측에도 기대므로 인용이 틀려도 결론은 서지만, ADR 「근거」에 옮기기 전에 확인한다.
4. **T-33(모르는 `kind` 하나가 agents.json 전체를 손상으로 만든다)은 안 닫힌다** — R2 는 새 빌드가 `kind` 를 더할 때 `schema_version` 을 올려야만 그 경우를 막는다. 올리면 R3 의 리더 규칙이 따라붙는다. 안 올린 채면 옛 빌드는 그 파일을 손상으로 떠 두고 빈 목록으로 덮는다(지금과 같은 손실 — 사본 이름만 바뀐다). ★이 문장은 4판에서 다시 참이 됐다★ — 3판은 저장 직전 재판정이 모양을 안 봐 그 파일을 「쓸 수 있음」으로 보고 사본 없이 덮었다(리뷰 2 라운드 D1). 지금은 모양 검사가 `Spec` 에 있어 적재와 재판정이 같은 손상 정의를 쓴다(§3-3).
5. **기준선 회귀 수** — 돌려 보지 않았다(§6-5).
6. **base `file` 시험 수 · 줄 수** — 어림이다(fsutil `#[test]` 27 · 913줄 실측 기준 — 21 이사 + U3 약 25(모양 검사 · `Claim` 표 포함)).
7. **가드의 트리 끄기(U7)** — 실패 경로는 주입할 수 없다(`GroupOwner::new` 를 실패시키는 이음매가 없다 — `codex/transport.rs:6610-6611` 의 소스 시험 사유). 그래서 「가드를 버리면 손자까지 죽는다」만 실프로세스로 재고, 「무리 넣기 실패 → 가드」 길은 소스 시험(배치)만 잰다. `taskkill` 이 그 길에서 드는 시간은 재지 않았다.
8. **U5b 크기** — 레지스트리 API 가 `Result` 로 바뀌면 manager 시험의 부르는 자리가 많다(`manager.rs` 시험 구획의 `profiles.upsert` 등 — 세지 않았다). 4판에서 D2 · D3 · D8 이 더해져 더 커졌고, 5판에서 매니저 `Result` 판 다섯(E4) · 띄우기 경로의 정책 인자(E3 — 세 입구 · 두 동사 · 공유 함수 · `register_for_spawn`)가 U5b-1 에 더 들었다. 착수 전에 재고, 크면 U5b-1 → U5b-2 로 순차(§6-1).
9. **적재 읽기 예산 값(D7 — 파일마다 약 1 초)** — 데몬 기동 시간을 재지 않고 셸 마감(5 초)에서 거꾸로 정했다. U5a 의 실 데몬 QA 에서 보통 기동 시간을 한 번 재 나머지 몫이 넉넉한지 본다. 잠김을 실제로 1 초 넘게 쥐는 도구(백신 · 동기화)가 얼마나 흔한지도 모른다.
10. **dirty 동안 「변경 없는 `mutate_if` 도 저장」(D2 의 제안)** — 저장 횟수를 늘린다(dirty 동안만이라 드묾 · 재지 않았다). 메인이 U5b 지시서에서 채택 여부를 정한다 — 빼도 dirty 변경은 다음 「변경 있는」 저장이 싣는다. ★메인 결정(위임) 2026-10-10 · 잠정 = 채택★ — 밀린 변경이 다음 「변경 있는」 저장까지 기다리지 않게.

### 8-3. 열린 것 (사용자 · 메인 확인 거리)

- **새 판 안내 문구와 기존 두 문구의 고침**(§4 4) — 사용자가 읽는 글이라 이 TRD 에는 가안으로만 둔다. ★메인 결정(위임 「관행대로 알아서」 2026-10-10 · 잠정 — 사용자 확인 대기)★ = U4 에서 메인이 관행대로 확정해 싣고, 사용자 확인은 착지 뒤 보고로 받는다(문구는 나중에 바꾸기 싸다).
- **agents · presets 읽기 전용의 화면 안내가 없다** — LLM 은 `agent.list` 로 보고, 사람은 만들기 · 이름 바꾸기 같은 변경이 실패로 돌아오는 것으로만 안다(그 실패가 화면에 어떻게 보이는지는 U5b 가 잰다). 안내를 두려면 별건(데몬 → 셸 알림 표면 · 문구 — 사용자).
- **(새 · 사용자 체감) 판정을 통과한 뒤의 쓰기 실패도 변경 실패로 하나** — ★메인 결정 = 예(위임 「관행대로」 2026-10-10 · 잠정 — 사용자 확인 대기 · 근거 = SQLite 등 저장 엔진이 디스크 가득 · 잠김에서 변경을 오류로 돌리고 메모리 상태를 커밋하지 않는 관행)★. 권고 = 예(§3-9 의 「사본에 적용 → 저장 → 커밋」). 그러면 디스크 가득 · 잠김 예산 초과 때 이름 바꾸기 · 만들기 같은 변경이 오류로 돌아온다. 아니오면 그 경우만 지금처럼 「메모리는 바뀌고 재시작에 사라짐」이 남는다(거절 상태만 오류). 메인 결정의 문장(「거절 상태면 적용 전에 오류」)은 거절만 말하므로 올린다. ★4판: 이 항목은 부르는 쪽 있는 변경만이다 — 부르는 쪽 없는 내부 변경의 쓰기 실패는 메모리에 적용 + dirty(D2 — 메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기 · §3-9)★ · ★5판: 내부 변경은 재판정 거절도 같다(E1 — 같은 표시)★.
- **(새 · 확인) 읽기 전용은 데몬 재시작으로만 풀린다** — ★메인 확인 = 그대로 간다(2026-10-10 · 잠정 — 사용자 확인 대기)★. 메인 결정(SQLite 읽기 전용 꼴)의 따름이고, 적재 거절 뒤 재판정만으로 풀면 빈 명부가 진짜 명부를 덮는 덫이 있어(§3-9) 이렇게 정했다. 다른 안(파일이 읽히게 되면 실행 중에 다시 적재)은 명부 · 화면 · 산 세션의 정합 설계가 따로 들어 권고하지 않는다.
- **(새 · LLM 이 보는 어휘 — U4 · U5b 에서 메인이 정하고 보고)** 거절의 오류 코드(가안 `CONFLICT` — 이미 선언된 코드 · 다른 안 = 새 코드를 더하는 additive 확장 — 재시도 지시가 갈린다 · ★깨우기 · 만들고 띄우기 경로는 지금 활성화 오류를 전부 `INTERNAL` 로 실으므로(`agent/src/commands.rs:976-977` · `:1015`) 어느 코드든 `PtyError` 저장 거절 변형이 먼저 든다 — §3-9 표 · 리뷰 3 라운드 잔손질★) · `agent.list` 상태 칸의 이름과 낱말(가안 `store` · `writable` | `read_only` | `refusing` · `newer` | `unreadable` — `refusing` = 이번 쓰기 거절, D6 · ★메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기 = 값을 더하고 낱말은 U5b 에서★) · ★설정 거절의 버스 오류 코드(D13 — 지금 길이면 `INTERNAL` · 메인 결정(위임) 2026-10-10 · 잠정 — 사용자 확인 대기 = U4 에서 메인이 정한다 · agent 쪽과 같은 코드로 맞출지 포함)★.
- **(새 · 범위 밖 관측 — 메인 판단 거리)** 띄우기 **성공** 경로에서도 「무리에 넣기 전에 `cmd.exe` 가 이미 띄운 손자」는 무리 밖에 남는다(stdio · codex · pty 셋 다 깨운 채 띄우고 넣는다 — platform `group/mod.rs:69-70` 의 경고 그대로). 그 손자는 통로를 닫아도(`KILL_ON_JOB_CLOSE`) 안 죽는다. 얼마나 자주 그 경합을 지는지는 재지 않았다. 닫는 법 = 멈춘 채 띄우기(stdio · codex 만 — §3-7). A 에 넣지 않고 메모 §11 · `docs/tracking.md` 후보로 적을지 메인이 정한다. ★메인 결정(2026-10-10) = A 에 넣지 않는다 · U8 착지 때 `docs/tracking.md` 에 항목으로 적는다★.
