# TRD — 경계 리팩터링 A: 데이터 파일 규칙을 하나로 (S21)

> 상태: **2판(2026-10-10) — 메인 결정 반영 · 리뷰 전.** 코드는 한 줄도 바뀌지 않았다.
> **개정(2026-10-10, 메인 결정 반영):** ① 거처 = A(새 crate `engram-dashboard-datafile`) · B · C · D 는 거부로 기록(§3-1) ② base `retry_if_vanished` 를 1d 에서 뺐다(§1 · §8-1) ③ 새 판 규칙은 네 파일 모두 — TRD S21-storage §6-5 · §12 R3 를 뒤집고 ADR-0265 결정 4 · ADR-0274 를 좁힌다 · 새 ADR 에 든다(§3-5 R2 · §7) ④ `layout/apply.rs:247` 을 U8 에(§3-8) ⑤ R7 · R8(읽기 IO 실패 = 저장 거절) 유지 ⑥ net 몫을 U6 에서 뺐다 — net 의 열기 재시도는 transport 3-3 의 후속(§3-6 · §8-1) ⑦ 안내 문구는 가안으로 두고 최종안은 U4 에서 메인이 올린다(§4 · §8-3) ⑧ 새 ADR 은 U1 바로 앞에 쓰고 그때 모든 원격 브랜치를 보고 다음 빈 번호를 선점한다(§7) ⑨ 이 TRD 가 인용한 줄 번호를 실측으로 맞췄다(§2-1). 세부 규칙(§3-5)은 메인 판단(위임) — 메인이 채택했다.
> **범위 = 사용자 결정(2026-10-08~10) — §1.** TRD 1-1 의 미룬 것 D1(손상 사본) · D2(셸 쓸어 담기) · D3(원자 쓰기)와 TRD 1-3 §5-1 의 U-W(원자 쓰기 통일)를 이어받는다. 트리거였던 storage P3 는 이 브랜치에 들어와 있다(`src-tauri/src/fsutil.rs` 가 P3 모양).
> **배치 근거:** `docs/README.md` 「새 기능 **설계 착수** → `process/SN-name/`」 — 같은 단계의 형제 TRD(1-1 · 1-3 · 2-1 · 2-2 · 2-3)가 서는 폴더.
> **표기:** 「실측」 = 기준 트리 `3b9725d`(브랜치 `v0.3.3/refactor/crate-boundaries` — 코드는 범위 조사 트리 `aef34d4` 와 같다, 그 사이 커밋은 인계 문서뿐) · 「사용자 결정(날짜)」 = 문서에 적힌 사용자 결정만 · 「메인 판단(위임)」 = 사용자가 「성숙 프로그램 관행대로 · 메인이 정하고 보고」로 넘긴 세부(2026-10-10) · 「제안」 = 이 TRD 의 안(이름은 전부 가안) · 「기억 인용 · 미검」 = 관행 근거를 원문과 대조하지 않은 것.
> 앵커: ADR-0274(손상 사본 규칙) · ADR-0265 결정 4 · ADR-0266 결정 2 · 3 · 8 · ADR-0275 결정 1 · 6 · 15 · ADR-0269 결정 2 · ADR-0135(portfile 제자리 쓰기 — 바꾸지 않는다) · ADR-0282(쓰기 프로브는 base) · ADR-0061(presets 는 profiles 복제) · ADR-0230 · ADR-0012 · TRD `docs/process/S21-storage/trd.md` §6-2 · §6-5 · §12 R3 · 범위 조사 `.claude/handoff/attachments/A-write-atomic-set-aside-scope-2026-10-08.md`.

---

## 0. 결론 (먼저)

```
① 데이터 파일 넷(설정 · 상태 · agents · presets)의 읽기 규칙 · 원자 쓰기 · 손상 사본 · 「덮어도 되나」 판정을
     구현 하나로 모은다. 셸과 데몬이 같은 함수를 부른다. 파일 이름 · 자리 · 스키마 · 버전 키 이름은 각 주인이 쥔다.

② 「남이 잠깐 쥐었다」 판정과 다시 하기 고리는 platform fs 하나다 — OS 규칙이라서다.
     → 셸 fsutil.rs 의 cfg!(windows) 가 걷혀 platform 밖 운영 OS 분기가 0 이 된다(ADR-0275 결정 15 를 닫는다).

③ 새 판이 쓴 파일과 읽다 실패한 파일은 덮지도 떠 두지도 않는다 — 네 파일 공통.
     → agent 의 「새 판 agents.json 을 빈 목록으로 덮는다」 결함이 닫힌다(같은 부류인 「읽기 실패 → 덮음」도).

④ 거처 = 새 crate engram-dashboard-datafile(platform 위 · serde_json — 메인 판단(사용자 위임) §3-1).

⑤ 덤으로 공용화 둘 — agent 자식 띄우기 세 벌(codex 의 가드를 셋 모두의 규칙으로) ·
     셸 락 오염 복구 12곳과 상태 기록기 시계를 base 로.

⑥ 단위 아홉(U1~U9) — 어느 단위 뒤에 멈춰도 빌드 · 회귀 초록. 병렬 = 첫 물결 U1 ∥ U7 ∥ U8 · 넷째 물결 U4 ∥ U5 ∥ U6.
```

**지금 하지 않는 것:** daemon.json 읽기(net ↔ 셸) · WS 재연결 정책(→ transport 3단계) · net `instance.rs` 의 열기 재시도(net 은 transport 3-3 에서 갈린다 — 그때 platform `retry_busy` 를 쓴다 · §3-6) · base `writable.rs` `retry_if_vanished`(잠김 재시도가 아니다 — §8-1 1) · 락 오염을 패닉으로 둘지 되찾을지(ADR-0288 · ADR-0290 — 별 주제, 브랜치 `v0.3.3/feat/panic-policy`) · net `portfile.rs` 의 제자리 쓰기(ADR-0135) · 셸 실행 잠금 `state/lock.rs` 의 기다림(§2-4) · 항목 하나만 깨진 agents.json 의 관용 파싱(`docs/tracking.md` T-33 — §8-2).

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
| 2g | agent 자식 띄우기(무리에 넣기) 세 벌 공용화 — codex 의 가드(무리 넣기가 실패하면 자식을 죽인다)를 셋 모두의 규칙으로. stdio · pty 의 누수를 고친다(메모 `docs/refactoring/architecture-discussion-2026-09-26.md:334`) | §3-7 · U7 |
| 2h | 셸 손 락 오염 복구 → base `sync`(TRD 1-1 D2 의 목록 + `theme.rs` · `state/restore.rs` · `state/placement.rs`) · `state/saver.rs` 자기 `trait Clock` → base `Clock` | §3-8 · U8 |
| 3 | 뺀다: daemon.json 읽기 · WS 재연결(→ transport 3단계) · 패닉 대 되찾기 정책(ADR-0288/0290 — A 아님) | §0 |
| 4 | 세부(버전 없음 · 새 판 · BOM · 네 파일 상한 · 재시도 수/간격 · 임시 이름 · 폴더 fsync · 폴더 만들기)는 「성숙 프로그램 관행대로 · 메인이 정하고 보고」 | §3-5(메인 판단(위임)) |

**알려진 결함(이 범위가 닫는다):** agent 적재가 자기보다 새 `schema_version` 을 만나면 빈 목록으로 시작하고(`persistence/mod.rs:114-121`), 첫 변경의 저장(`profile.rs:378-393` 의 `ProfileRegistry::new` → `mutate` → `store.save`)이 그 새 파일을 덮는다 — 사본도 없다. 통일 규칙 R2 · R8(§3-5)이 막는다.

---

## 2. 현황 실측 (트리 `3b9725d`)

### 2-1. 사본 목록

| 축 | 자리 | crate | 지금 모양 |
|---|---|---|---|
| 읽기 | `src-tauri/src/settings/store.rs:73-86` · `:131-156` | 셸 | 열기 + `fsutil::read_capped`(★잠김 재시도 없음★) · BOM 떼기 · JSON 객체 · `$version` 없음 = 1 · `1`/`1.0` 만 받음 · 그 밖 = 못 씀 |
| 읽기 | `src-tauri/src/state/codec.rs:97-133` · `:158-170` · `:176-178` | 셸 | `fsutil::read_file_capped`(잠김 재시도) · BOM · 객체 · `version` 없음 = 못 씀 · 정수 값인 수 · 초과 = `NewerVersion` · `InvalidData` = 못 씀 |
| 읽기 | `crates/engram-dashboard-agent/src/persistence/mod.rs:102-129` · `presets.rs:92-120` | agent | `fs::read` — 상한 · BOM · 재시도 없음 · serde 구조체(`schema_version` 없으면 파싱 실패 = 손상) · 불일치 = 빈 목록 · 파일 그대로 · 읽기 IO 실패 = 빈 목록 |
| 쓰기 | `src-tauri/src/fsutil.rs:73-147` · `:166-207` · `:238-280` | 셸 | `write_atomic(_unless)` · `copy_atomic` · `replace_with` · 임시 `<이름>.tmp<pid>.<번호>` · 실패면 임시 지움 · `sweep_temps` |
| 쓰기 | `persistence/mod.rs:50-76` · `presets.rs:46-68` | agent | 고정 `agents.json.tmp` · `presets.json.tmp` + `write_lock` · `create_dir_all` · 부모 폴더 fsync best-effort · 재시도 없음 · 실패면 임시 남김 |
| 쓰기 | `crates/engram-dashboard-daemon/src/control/mcp_config.rs:104-118` · `:157-165` | 데몬 | 토큰을 실은 mcp-config · 세션 설정 조각 — 둘 다 `create_dir_all` + `fs::write` |
| 손상 사본 | `fsutil.rs:126-130` `copy_aside` | 셸 | ADR-0274 — 고정 `.corrupt` · 복사 · 덮어쓰기 · 첫 쓰기 전 |
| 손상 사본 | `persistence/mod.rs:79-87` · `presets.rs:70-79` `preserve_corrupt` | agent | 적재 때 rename(이동) → `<이름>.corrupt-<epoch ms>` · 누적 · 재시도 없음 · 실패 로그만 |
| 잠김 재시도 | `fsutil.rs:294-321` `retry_denied` · `is_lock_contention` | 셸 | 5회 × 20 ms · `PermissionDenied` + Windows 원시 32 · 33 — ★`:320` 이 platform 밖 마지막 운영 `cfg!(windows)`★ |
| 잠김 재시도 | `crates/engram-dashboard-net/src/instance.rs:83-96` · `:206-263` | net | 열기 5번 · 사이 100 ms · 공유 위반(32)만 · 접근 거부(5)는 읽기 전용 속성 걷기 1회 뒤 바로 실패 · 한도 뒤 진단(`live_owner` · 한 번 더 열기) — ★A 에서 바꾸지 않는다(transport 3-3 후속)★ |
| 잠김 재시도 | `crates/engram-dashboard-agent/src/usage/scratch.rs:66-88` | agent | 임시 폴더 지우기 · 예산 200 ms · 간격 10 ms · ★`NotFound` 밖 모든 오류를 다시★ |
| (재시도 아님) | `crates/engram-dashboard-base/src/writable.rs:86-97` `retry_if_vanished` | base | `NotFound` 일 때 한 번 더 — 검사 도중 폴더가 사라지는 경합 · ★범위에서 뺐다(§8-1 1)★ |
| 자식 띄우기 | `crates/engram-dashboard-agent/src/transport/stdio.rs:99-137` · stderr `:232-260` | agent | ★가드 없음★ — `GroupOwner::new()?` · `adopt(pid)?` 실패면 띄운 자식이 무리 밖에서 산다 |
| 자식 띄우기 | `crates/engram-dashboard-agent/src/backend/codex/transport.rs:1401-1475` · stderr `:3993-4015` | agent | `ChildGuard`(`:1401-1421`)가 첫 `?` 앞에 선다 · 소스 시험 `:6613` 이 그 배치를 잰다 |
| 자식 띄우기 | `crates/engram-dashboard-agent/src/transport/pty.rs:85-105` | agent | ★가드 없음★ — 무리 만들기 · 넣기 · `try_clone_reader` · `take_writer` 넷이 띄운 뒤의 `?` |
| 락 오염 | `settings/mod.rs:466 · 470 · 474` · `state/boot_plugin.rs:119 · 242 · 383(wait_timeout) · 444` · `theme.rs:309 · 338` · `state/restore.rs:246` · `state/placement.rs:189` · ★`layout/apply.rs:247`★ | 셸 | 손으로 쓴 `unwrap_or_else(PoisonError::into_inner)` — 마지막 하나는 처음 범위 목록 밖이었고 메인이 U8 에 넣었다(§8-1 3) |
| 시계 | `src-tauri/src/state/saver.rs:83-100` | 셸 | 자기 `trait Clock: Send + 'static { now; wall_ms }` |

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
| 폴더 만들기 · 폴더 동기화 | 만듦(저장소가) · 안 함 | 안 함 · 안 함 | 만듦 · best-effort |

### 2-3. 기록과 어긋난 사실

| 기록 | 실제 |
|---|---|
| ADR-0266 결정 8 · ADR-0275 결정 15 · TRD 1-3 §2-4 · §8 6 의 `fsutil.rs:107` | `:318-321`(`is_lock_contention`) — P3 앞 줄 번호다 |
| ADR-0266 결정 8 둘째 항목 「`set_aside_corrupt` 는 base `file` 에 남는다 — rename 하나로 OS 분기가 없다」 | ADR-0274 가 떠 두기를 복사(`copy_atomic` — 잠김 재시도를 품는다)로 바꿔 거짓이 됐다 |
| TRD 1-3 §5-1 U-W 행 · ADR-0266 결정 8 · ADR-0269 「영향」의 사본 목록에 데몬 `usage_service/reject_store.rs` | 없다(ADR-0284 `9ab15231` 가 걷었다) |
| TRD 1-1 §8 D2 「`settings/store.rs:122` epoch ms(`u128`)」 | 없다(ADR-0274 가 시각 이름을 걷었다) |
| `docs/reference/structure/session-path-ownership.md:821 · 887-888 · 2000` · `docs/reference/structure/agent-backend.md:237` · 주석 `profile.rs:178 · 2013` · `commands.rs:69` 의 `.corrupt-<ms>` · 「밀어낸다」 | 이 TRD 뒤 낡는다(U5 · U9) |

### 2-4. 남길 것

- net `portfile.rs` `write_in_place` — 잡은 핸들로 제자리에 쓴다. rename 은 우리가 쥔 공유 제한에 막힌다(ADR-0135).
- 셸 `state/lock.rs`(3 s 동안 50 ms 간격) — 앞 인스턴스가 **우리 잠금**을 놓기를 기다리는 뜻 있는 기다림이다(근거 = `REPLY_DEADLINE`). 백신 · 색인기의 잠깐 쥐기가 아니라 같은 정책으로 묶지 않는다.
- `src-tauri/src/discovery/layout.rs:235` — 시험 안의 락 오염 복구(운영 아님).
- 셸 `output_channel.rs` — ADR-0231 의 경고 + 독 걷기(ADR-0275 결정 1 의 예외).
- `usage/scratch.rs` `sweep_stale_scratch` — 기동 때 한 번, 재시도 없음.

---

## 3. 설계

### 3-1. 거처 — 결정: A, 새 crate `engram-dashboard-datafile`

> **메인 판단(사용자 위임 — 3단계 전까지 메인이 갈림길을 정하고 보고) · 2026-10-10.** crate 이름은 가안이다(U2 에서 확정).

하나로 모을 것은 넷이다 — (i) 잠김 판정과 다시 하기 고리(OS 규칙) · (ii) 버전 붙은 JSON 읽기(BOM · 상한 · 버전 문 · 판정 — `serde_json` 이 필요하다) · (iii) 원자 쓰기 · 복사 · 떠 두기 · 임시 이름 · 쓸기(OS 무관한 장치지만 (i) 을 부른다) · (iv) 「덮어도 되나」 판정(순수 규칙).

**헌장이 막는 것:** base = 워크스페이스 의존 0(platform 을 못 부른다) · serde 를 끌지 않는다(ADR-0269 결정 2 의 「무거운 의존을 들이지 않는다」) · OS 코드 없음. platform = OS 의존 여부로만 들인다(ADR-0266 결정 2) · 도메인 지식 0(결정 3) · 운영 서드파티는 `windows` · `tracing` 뿐(lib.rs 헤더 「의존」). 셸은 agent 를 의존하지 않고(ADR-0270) 데몬은 dev 의존으로만 닿는다. (i) 은 platform `fs` 다(§3-2). (ii)~(iv) 는 기존 crate 어느 것도 깨끗이 받지 못한다.

**고른 것 — A:** (ii)~(iv) 를 새 crate 하나에 둔다. 의존 = platform · `serde_json`(base · `tracing` · `tokio` 없음).

- **얻는 것:** 두 헌장이 그대로다 · 규칙 전체(특히 R2 · R8 의 「거절」)가 한 crate 한 함수에 선다 — 「구현 하나 · 규칙 하나」(1a · 1b)가 글자 그대로 서는 유일한 안이다 · 개정할 글이 거처 서술뿐이다(ADR-0266 결정 8 · ADR-0269 결정 2 표의 `file` 행).
- **대가:** 워크스페이스 멤버 11 → 12 · 등록 게이트(의존 상한 · `use tauri` 0 · 메시징 이름 정규식 · CI 시험 스텝 · CLAUDE.md 지도) · 회귀 결과 줄 +2. crate 가 따로 서는 까닭이 ADR-0151 의 「독립 사용 · 순환 방지」가 아니라 「두 헌장 사이에 끼었다」다(그래도 Engram 타입을 안 받아 독립으로 쓸 수 있다).

**거부한 대안(새 ADR 「거부한 대안」의 재료):**

- **B — 기존 둘에 나눠 넣기.** (iii) 은 platform `fs`(ADR-0266 결정 8 의 「`write_atomic` 째 옮긴다」를 그 가족으로 넓힘) · (ii) · (iv) 는 base 새 입주자 `file` + base 에 `serde_json`. 새 crate 가 없고 ADR-0266 결정 8 의 문구가 대체로 산다. 그러나 platform 에 OS 무관 코드 약 300줄 + 시험 약 25개가 들고 ADR-0274 의 Engram 정책 이름(`.corrupt`)이 OS 층에 들어간다 → ADR-0266 결정 2 · 3 과 긴장(범위 조사가 짚은 그것). base 에 `serde_json` · 입주자 여덟(입주자 무참조 정규식 4곳 · CLAUDE.md 개수) · ADR-0269 결정 2 개정. 규칙이 두 crate 로 갈려 부르는 쪽 다섯이 「platform 읽기 → base 판정 → platform 쓰기」를 각자 잇는다.
- **C — 전부 platform 에 + `serde_json` 허용.** platform 의 서드파티 규칙이 깨지고 OS 층에 JSON 정책이 든다.
- **D — 전부 base 에, 재시도는 부르는 쪽이 클로저로 넘김.** base 에 어차피 `serde_json` 이 들고, 부를 때마다 platform 재시도를 들고 와야 하는 이음매는 의존을 피하려고만 생긴다.

### 3-2. platform `fs` — 잠김 판정과 다시 하기 (가안)

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
```

- `cfg!(windows)` 는 `is_busy` 한 줄로 platform 안에 들어온다 — 셸 `fsutil.rs:320` 이 걷힌다(1f).
- net `instance.rs` 는 A 에서 이것을 부르지 않는다(transport 3-3 후속 — §3-6). 그때 옮겨도 net 의 「공유 위반만」과 뜻이 갈리지 않는다 — net 은 접근 거부(5)를 고리에 넣기 전에 따로 처리한다(읽기 전용 속성 1회 걷기 · ACL 이면 바로 `AccessDenied`). 새로 다시 하기에 드는 것은 33 하나고, 열기에서는 사실상 나지 않는다.
- `ERROR_DIR_NOT_EMPTY`(145)는 넣지 않는다 — scratch 지우기가 실제로 어떤 코드로 실패하는지 모른다(§8-2 1 · U6 파일럿).
- 시험: fsutil 의 판정 시험 둘(`:392-431` — 원시 코드 · `cfg!(windows)` 기대 — 이 둘이 fsutil 을 4f 명단에 붙드는 시험 분기다)과 다시 하기 시험 셋(`:358-391` · `:433-447`)을 platform 으로 옮긴다.

### 3-3. datafile crate — 데이터 파일 규칙 (가안)

```rust
/// 파일마다 하나 — 주인(저장소)이 상수로 둔다(§3-4).
pub struct Spec {
    pub version_key: &'static str, // "$version" | "version" | "schema_version"
    pub current: u64,              // 이 빌드가 쓰는 판
    pub cap: u64,                  // 읽기 상한 = 쓰기 상한(바이트)
}

/// 원문 하나를 본 결과 — IO 는 모른다.
pub enum Parsed {
    /// 1 ..= current. version < current 면 그 판 리더는 부르는 쪽 몫(지금 넷 다 current = 1 이라 없다 — R3).
    Usable { version: u64, doc: serde_json::Map<String, serde_json::Value> },
    Unusable(String),     // 손상 — 사유 문구(값은 싣지 않는다)
    Newer { found: u64 }, // 이 빌드보다 새 판
}
pub fn parse(text: &str, spec: &Spec) -> Parsed; // BOM 하나 떼기 · JSON 객체 · 버전 문(R1~R5)

/// 파일 하나를 읽은 결과.
pub enum Loaded { Missing, Parsed(Parsed), Failed(io::Error) }
/// 읽기 결과를 판정으로 — NotFound = Missing · InvalidData(상한 · UTF-8) = 손상 · 그 밖 오류 = Failed · Ok = parse.
/// ★셸의 IO 이음매(설정 `SettingsFiles` · 상태 `BootFiles`/`SaverFiles` 의 가짜)는 자기 읽기 결과를 이것에 넣는다★.
pub fn classify(read: io::Result<String>, spec: &Spec) -> Loaded;
pub fn load(path: &Path, spec: &Spec) -> Loaded; // = classify(read_file_capped(path, spec.cap), spec)

/// 그 파일에 써도 되나 — 네 파일 공통 규칙 하나(R8).
pub enum WritePolicy { Write, CopyAsideFirst, Refuse(Refused) }
pub enum Refused { Newer { found: u64 }, ReadFailed }
impl Loaded { pub fn write_policy(&self) -> WritePolicy; }
pub fn check_cap(len: usize, spec: &Spec) -> io::Result<()>; // 쓰기 상한(R6)

// 쓰기 · 읽기 장치 — fsutil 에서 옮긴다(바이트를 받는다 · 셸의 &str 은 as_bytes).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()>;
pub fn write_atomic_unless(path: &Path, bytes: &[u8], skip: impl Fn() -> bool) -> io::Result<WriteOutcome>;
pub enum WriteOutcome { Written, Skipped }
pub fn copy_atomic(from: &Path, to: &Path) -> io::Result<()>;
pub fn copy_aside(path: &Path) -> io::Result<PathBuf>; // `<이름>.corrupt` — ADR-0274
pub fn sweep_temps(dir: &Path, targets: &[&str], is_alive: impl Fn(u32) -> bool) -> io::Result<Vec<(PathBuf, io::Result<()>)>>;
pub fn read_capped(source: impl io::Read, cap: u64) -> io::Result<String>;
pub fn read_file_capped(path: &Path, cap: u64) -> io::Result<String>;
```

- **로그를 내지 않는다** — 값을 돌려주고 부르는 쪽이 자기 문구로 찍는다(지금 fsutil · codec 꼴 그대로).
- **아는 이름은 두 꼴뿐이다** — 임시 `<이름>.tmp<pid>.<번호>` · 사본 `<이름>.corrupt`(지금 fsutil 머리 그대로). 파일 이름 · 자리 · 스키마 · 「항목 하나가 못 쓸 때」 관용은 주인이 쥔다(ADR-0264 의 배치 정본도 그대로 각 crate 에).
- **잠김 재시도는 이 crate 안 한 군데에서 `platform::fs::retry_busy` 를 부른다** — 예산 상수 하나(R14).
- `classify` 와 `load` 를 둘 다 두는 까닭 — 셸은 시험 가짜를 위해 IO 를 트레이트 뒤에서 하고(ADR-0012), agent 는 경로로 바로 읽는다. 판정 규칙은 `classify` 하나다(지금 `codec::unusable_read` 와 설정의 `RawFile` 가 같은 가름을 두 번 한다).

### 3-4. 파일별 `Spec`

| 파일 | 주인 | 버전 키 | `current` | 상한 | 손상 사본 실패 때 |
|---|---|---|---|---|---|
| `shell\config\settings.json` | 셸 `settings/store.rs` | `$version` | 1 | 64 KiB(그대로) | 그 쓰기 실패 · 원본 그대로(그대로) |
| `shell\state\state.json` · `state.crash.json` | 셸 `state/codec.rs` | `version` | `STATE_VERSION` = 1 | 4 MiB(그대로) | 진행(D8 그대로) |
| `daemon\state\agents.json` | agent `persistence/mod.rs` | `schema_version` | 1 | **16 MiB(새)** | 그 저장 실패 · 원본 그대로(새) |
| `daemon\state\presets.json` | agent `persistence/presets.rs` | `schema_version` | 1 | **4 MiB(새)** | 같음(새) |
| 데몬 run 아래 mcp-config · 세션 설정 조각 | 데몬 `control/mcp_config.rs` | — (claude 가 읽는 형식 · 버전 문 없음) | — | — | — (원자 쓰기만 — 1e) |

### 3-5. 하나로 정한 규칙 — 세부 (메인 판단(위임) · 채택 2026-10-10)

> 사용자 위임(2026-10-10 「성숙 프로그램 관행대로 · 메인이 정하고 보고」) → **메인이 아래 권고를 채택했다(2026-10-10)**. R2 · R7 · R8 은 메인이 따로 다시 확인했다(§8-1 2 · 4). 한 줄 근거 = 관행 또는 지금 확정된 셸 규칙. 관행 인용은 기억에 기댄 것이 넷 있다(§8-2 3) — 결론이 그 인용 하나에 기대는 줄은 없다.

| id | 규칙 | 근거 한 줄 |
|---|---|---|
| R1 | **버전 없음 = 1(첫 판)** | 버전 표지가 나중에 붙은 형식은 「표지 없음 = 가장 옛 판」으로 읽는 것이 관행(Cargo.lock 이 `version` 줄 없는 파일을 옛 판으로 읽는다 — 기억 인용 · 미검). 지금 설정 규칙 그대로 · 상태 · agent 는 「손상」이던 것이 읽힌다(§4) |
| R2 | **새 판(`found > current`) = 읽지 않고 덮지 않고 떠 두지 않는다 — 네 파일 모두.** 메모리는 기본값 · 빈 목록, 그 파일 쓰기는 거절 — 설정은 쓸 때마다 다시 읽으므로 파일이 바뀌면 풀리고, 상태 · agent 는 그 프로세스 동안. ★메인 판단(위임 「성숙한 프로그램 관행대로」) — 관행 = 새 판이 쓴 파일을 옛 판이 덮지 않는다★. TRD S21-storage §6-5 · §12 R3(새 판 `state.json` 은 떠 두고 덮는다)를 뒤집고 ADR-0265 결정 4 · ADR-0274(「모르는 `$version` · `version` = 통째로 못 쓸 파일」)를 좁힌다 — 새 ADR 에 든다(§7) | SQLite 는 머리의 「쓰기 판」이 자기보다 크면 그 DB 를 읽기 전용으로 연다 · Firefox 는 새 판이 쓴 프로필을 옛 판이 열지 않는다(둘 다 기억 인용 · 미검). 상태의 크래시 사본은 이미 이렇게 한다(N6) — 그것을 넷으로 넓힌다. **알려진 결함을 닫는다** |
| R3 | **앞 판(`1 ≤ found < current`) = 그 판 리더가 있으면 읽고 없으면 손상** — 지금 넷 다 `current = 1` 이라 해당 없다. `version` 은 앞 판 리더와 함께만 올린다 | TRD S21-storage 「칸 더하기 규칙」(상태 파일)을 네 파일로 |
| R4 | **버전 값 꼴 = 정수 값인 수만**(`1` · `1.0`) — 0 · 음수 · 소수 · 문자열 · `u64` 밖 = 손상 | 지금 상태 `version_number`(`codec.rs:158-170`) 그대로 — JSON 도구가 수를 실수로 다시 쓴다 |
| R5 | **BOM = 앞머리 U+FEFF 하나를 무시 · 쓰지 않는다** | RFC 8259 §8.1(파서는 BOM 을 무시할 수 있고 쓰는 쪽은 붙이지 않는다). 셸 둘 그대로 · agent 는 새로 받는다 |
| R6 | **상한 = 읽기 상한 = 쓰기 상한** — 넘는 원문은 쓰지 않는다. 설정 64 KiB · 상태 4 MiB 그대로 · agents.json 16 MiB · presets.json 4 MiB 새로 | 셸 I4(넘게 쓰면 다음 적재가 통째로 못 쓴다) · 값은 관행이 아니라 실측 비례(이 PC 의 agents.json 1,747 B — 정상이면 안 닿고 쓰레기 파일로부터 메모리를 지키는 선 · 설정 · 상태와 같은 방식) |
| R7 | **손상 = JSON 아님 · 객체 아님 · 상한 초과 · UTF-8 아님 · 버전 꼴 틀림(R4) · 리더 없는 앞 판(R3) · 부르는 쪽 모양 해석 실패.** ★읽기 IO 실패는 손상이 아니다★ | 셸 I3 를 넷으로 — agent 의 「읽기 실패 → 빈 목록 → 덮음」이 함께 닫힌다(§8-1 4) |
| R8 | **쓰기 판정: Missing · 쓸 수 있음 → 쓴다 · 손상 → 떠 두고(R9) 쓴다 · 새 판 · 읽기 실패 → 거절.** 떠 두기 실패 = 설정 · agents · presets 는 그 쓰기 실패(원본 그대로) · 상태만 진행 | ADR-0274 「영향」 대가 2 그대로(상태 = 사용자 결정 D8) · agent 는 설정 쪽(데이터를 사본 없이 덮지 않는다)을 따른다 |
| R9 | **손상 사본 = ADR-0274 그대로** — 고정 `<이름>.corrupt` · 복사(원본은 그 자리) · 덮어쓰기 · 첫 쓰기 직전 · 한도 · 중복 생략 · 시각 이름 없음 | 사용자 결정 1c. agent 는 시점이 「적재 때 이동」에서 「첫 저장 직전 복사」로 바뀐다 |
| R10 | **원자 쓰기 = 같은 폴더 임시 → 담고 `sync_all` → rename(R13 · R14) → 부모 폴더 동기화(best-effort · 결과 무시)** · 실패 · 건너뜀이면 임시를 지운다 | POSIX 에서 rename 의 내구성은 부모 폴더 fsync 가 맡는 것이 관행(LevelDB · SQLite 의 유닉스 경로 — 기억 인용 · 미검). Windows 는 std 로 폴더를 못 열어 무동작으로 보인다(agent 의 지금 best-effort 도 같은 결과일 것 — U3 실측) — 그래서 `cfg` 없이 둔다 |
| R11 | **폴더 = 첫 쓰기가 만든다(`create_dir_all`) · 적재는 아무것도 만들지 않는다** | ADR-0265 결정 4 · 설정 저장소 주석(「적재가 만들면 안 되므로 첫 쓰기가 만든다」)을 넷으로 |
| R12 | **임시 이름 = `<이름>.tmp<pid>.<번호>`**(지금 셸). 남은 임시는 그 파일 주인이 기동 때 · 첫 쓰기 전에 쓴다(산 남의 pid 는 둔다). 상태 = 지금 그대로 부팅 · agents · presets = `load` 안 — 데몬은 단일 인스턴스 가드 뒤에 레지스트리를 만든다(`daemon/src/lib.rs:494` → `:659`). 옛 고정 이름 `agents.json.tmp` · `presets.json.tmp` 도 그 쓸기가 지운다. `write_lock` 은 걷는다 | 이름이 겹치지 않으면 동시 쓰기가 서로의 반쪽을 갈아끼우지 않는다(지금 fsutil `temp_path` 머리). 옛 이름은 이름으로 우리 것이 확정되고 옛 코드가 매번 덮던 파일이다. 레지스트리는 이미 자기 락 안에서 저장한다(`preset.rs:51-54` · `profile.rs` `mutate`) |
| R13 | **잠김 판정 = §3-2 `is_busy`**(`PermissionDenied` + Windows 32 · 33) | Windows 에서 백신 · 색인기가 잠깐 쥔 파일의 rename · 지우기를 짧게 다시 하는 것이 흔한 처리(graceful-fs 의 win32 rename · Node `fs.rm` 의 `maxRetries` — 기억 인용 · 미검) · 지금 셸 규칙(ADR-0265 결정 4) |
| R14 | **다시 하기 꼴 = 첫 시도 + N 번 · 고정 간격 · 마지막 뒤에 자지 않음 · 잠김 아닌 오류는 바로.** 예산(부르는 쪽) — 데이터 파일(열기 · 읽기 · rename · 복사 원본 열기) 5 × 20 ms · 임시 폴더 지우기 19 × 10 ms(transport 3-3 이 net 의 열기를 옮길 때는 4 × 100 ms) | 지금 값 그대로 — ADR-0265 결정 4 · scratch 의 200 ms 실측 예산(`scratch.rs:66-71`) · net 의 「5번 시도 · 최악 400 ms(클라이언트 5 s 폴링 안)」(`instance.rs:83-96`). 한 값으로 맞추지 않는다 — 근거가 서로 다르다 |
| R15 | **거절된 쓰기의 로그** — agent 저장은 처음 한 번 error · 그 뒤 debug(변경마다 저장하므로 같은 줄이 쌓인다) · 셸은 지금 꼴 | agent `save` 는 이미 「로그만 · 호출자를 막지 않는다」(`profile.rs:304-306`) |

### 3-6. 부르는 쪽별 적용

- **설정** — `read_document` 가 `classify` · `parse`(설정 `Spec`)를 부른다 · `Document` 에 `Newer` · `differs` 는 `Unusable` 과 같이 `None` · `store::write` 는 `write_policy` 를 따른다(거절 = 지금 「읽기 IO 실패」와 같은 오류 길 — 서비스 `settings/mod.rs` 의 흐름은 그대로) · 읽기가 잠김 재시도를 얻는다(R14 — 지금 없다) · `FsSettingsFiles::read` 의 계약을 `io::Result<String>`(`read_file_capped` 꼴)로.
- **상태** — `codec::decode` = `parse`(상태 `Spec`) + 머리 해석 · `Unusable::NewerVersion` 은 `Parsed::Newer` 에서 · `unusable_read` 는 `classify` 로 대신. `decide_boot`(`boot.rs:344-349`) — state.json 이 새 판이면 `CopyAsideState` 대신 가드(새 `Guard::StateNewer` 가안 — 읽기 실패 가드 ⅰ 과 같은 칸: 떠 두지도 덮지도 지우지도 않고 이번 실행 저장 없음) · 상태줄 `StateFileStatus::Newer`(wire `newer` — `boot_plugin.rs:167-173` 의 매핑 · `restore.rs:75`) · 크래시 사본의 새 판 처리(N6)는 그대로.
- **agents · presets** — `load` → `Loaded` → 칸(`profiles` · `presets`) 해석(실패 = 손상으로 내림) · 판정을 store 가 쥐고(`load` 때 기록 — 부르기 전 기본값 = `Write`, 지금 동작) `save` 가 따른다 · 손상이면 첫 `save` 가 `copy_aside` 뒤 쓴다 · 쓰기 상한 검사. `ProfileStore` · `PresetStore` 트레이트 모양은 그대로(`save` = 로그만 · `load` = 목록). 두 파일의 같은 코드는 한 내부 함수로(ADR-0061 의 「복제」가 「같은 구현 호출」로 바뀐다).
- **mcp_config** — `fs::write` 두 곳(`:115` 토큰 · `:162` 세션 설정 조각) → `write_atomic`(폴더 만들기 포함 — 지금의 `create_dir_all` 걷힘). 조각은 범위 목록(1e) 밖이지만 같은 수명 · 같은 꼴이라 함께 옮긴다(메인 판단(위임)). 임시 파일도 같은 폴더라 기존 부팅 쓸기(폴더 안 파일 전부 — 그 파일 `settings_path` 주석)가 거둔다. ★토큰이 임시 파일에도 잠깐 실린다 — 같은 폴더 · 같은 ACL · 실패면 지운다★. 얻는 것은 크지 않다(파일은 스폰 전에 다 쓴다 — 지키는 것은 쓰는 도중 꺼짐뿐).
- **net instance — A 에서 바꾸지 않는다(메인 결정 2026-10-10).** net 은 transport 3-3 에서 갈린다 — ★후속: 3-3 이 단일 인스턴스 열기를 옮길 때 platform `retry_busy` 를 쓴다★(예산 4 × 100 ms · R14). 그때 넘길 메모: 시도마다 할 일(열기 · 읽기 전용 속성 1회 걷기 · 접근 거부 · 미지원 · 그 밖 오류는 바로 끝)을 `attempt` 로 · 한도 뒤 진단(`live_owner` · 한 번 더 열기)은 그대로 · 지금 속성 걷기가 시도 하나를 쓴다(`instance.rs:209 · 229-231`) — 고리 모양이 바뀌므로 시도 수 셈은 그 자리의 시험과 리뷰가 대조한다. 그때까지 net 의 2a 게이트는 그대로다.
- **usage scratch** — `Drop` 고리 → `retry_busy` · ★잠김 아닌 오류는 더는 다시 하지 않는다★ → U6 파일럿이 실제 실패 코드를 먼저 잰다(§8-2 1).

### 3-7. (g) agent 자식 띄우기

- 새 `crates/engram-dashboard-agent/src/transport/spawn.rs`(가안):
  - `ChildGuard<C: Reap>` — codex 의 가드(`codex/transport.rs:1402-1424`)를 일반화한다. std `Child` 와 portable-pty 자식 둘 다 · `Drop` = kill + wait.
  - `spawn_piped(spec, what) -> Result<PipedChild, PtyError>` — Command 조립 · 파이프 셋 · `hide_console_window` · spawn → **곧바로 가드** → 파이프 꺼내기 · `GroupOwner::new()` · `adopt(pid)` → 가드를 푼다. stdio · codex 가 부른다.
  - `drain_stderr(stderr, core, agent_id, thread_name, what)` — 두 벌을 하나로(마스킹 · 「쌓기가 로그보다 먼저」 · 기동 실패 warn — 지금 그대로).
  - pty 는 `spawn_command` 바로 뒤 가드 → 무리 · `try_clone_reader` · `take_writer` 뒤에 푼다.
- **코더 지시서에 박을 불변식** — 띄운 뒤의 모든 `?` 는 가드 아래 · 무리 칸은 구조체 마지막(TRD 1-3 §3-4 drop 순서) · kill 인과(ADR-0001) 무변경.
- **소스 시험** — `the_child_guard_is_armed_before_the_first_fallible_step_after_spawn`(`codex/transport.rs:6613`)은 `open` 본문의 철자를 찾는다 → `spawn.rs` 로 옮기고, 세 통로 `open` 이 무리를 직접 만들지 않음(`GroupOwner::new` 가 `spawn.rs` 와 pty 가드 구간에만)을 함께 잰다. `the_child_guard_reaps_the_child_on_an_early_return`(`:6393` — `#[cfg(windows)]`)도 옮기면 `spawn.rs` 가 4f 명단에 든다.

### 3-8. (h) 셸 락 오염 · 기록기 시계

- **12곳 → `engram_dashboard_base::sync::{lock, wait_timeout}`** — 범위 목록 11(§2-1) + `layout/apply.rs:247`(목록 밖이던 같은 꼴의 운영 자리 — 메인 결정 2026-10-10 으로 넣었다 · §8-1 3). 뜻 그대로(경고 없음 · 독 표시 유지 — ADR-0275 결정 1). 셸은 이미 base 를 부른다(`theme.rs:18`).
- **saver 시계** — `trait SaverClock: engram_dashboard_base::time::Clock { fn wall_ms(&self) -> u64; }`(가안 — agent `LeftoverClock` 선례 · ADR-0275 결정 6). 운영 = base `SystemClock` + `wall_ms` = `now_epoch_ms` 를 u64 로(음수 = 0 — 지금 `map_or(0, …)` 뜻) · 가짜 `FakeClock`(`saver.rs:872` — `Arc<Mutex<…>>`)은 `Sync` 가 그대로 선다(TRD 1-1 D2 의 「`Sync` 를 더해야 한다」).
- ADR-0288 · ADR-0290(패닉 정책)이 먼저 착지해 이 자리들을 바꾸면 U8 은 그 위에 다시 깐다 — 한 곳(base `sync`)으로 모아 두면 그 정책의 손댈 자리가 준다.

---

## 4. 사용자가 보는 변화

1. **agents.json · presets.json 손상 사본 이름 · 동작** — `<이름>.corrupt-<ms>`(적재 때 이동 · 쌓임) → `<이름>.corrupt`(첫 저장 직전 복사 · 덮어쓰기). 그 사이 원본은 그 자리에 남고 데몬을 띄울 때마다 같은 오류 로그가 난다. 두 번째 손상은 첫 사본을 덮는다(사용자 수락). **이미 있는 `.corrupt-<ms>` 파일은 건드리지 않는다**(옮기지도 지우지도 않는다).
2. **새 판이 쓴 파일(네 파일 공통)은 덮지 않는다**
   - 설정 — 기본값으로 뜨고 설정 바꾸기가 오류로 돌아온다(지금 「읽기 실패」와 같은 길) · 파일과 `.corrupt` 그대로. 지금은 첫 쓰기가 떠 두고 덮는다.
   - 상태 — 기본 화면 · 이번 실행 화면 상태 저장 없음 · 새 안내(아래 4) · 파일 그대로. 지금은 `state.json.corrupt` 로 떠 두고 덮는다(TRD S21-storage §6-5 · §12 R3 를 뒤집는다 — §8-1 2).
   - agents · presets — 빈 목록 · 이번 실행의 변경은 저장되지 않는다(데몬 로그만 — 화면 안내 없음, §8-3) · 파일 그대로. 지금은 첫 저장이 덮는다(결함).
3. **agents · presets 를 읽다 실패하면(없음 말고) 그 실행은 저장하지 않는다** — 지금은 빈 목록으로 덮어 전부 잃는다.
4. **화면 안내 · 버스** — `restore.status` 의 `state_file` 에 새 값 `newer`(LLM 도 같은 핸들로 본다 — CLAUDE.md 「LLM-우선 제어」) · 셸 바인딩 · `src/api/restoreClient.ts` 의 값 목록 · `StateFileNotice` 갈래 · `src/i18n/ko.ts` 새 문구 가안 「화면 상태 파일(state.json)이 이 앱보다 새 버전이 쓴 것이라 기본 화면으로 시작했습니다. 이번 실행에서는 화면 상태를 저장하지 않으며 파일은 그대로 둡니다.」. 기존 두 문구(`ko.ts:268-272`)의 「(손상 · 다른 버전의 형식 등)」은 새 판이 더는 그 길로 오지 않으므로 「(손상 · 알 수 없는 형식 등)」으로(가안). ★여기 문구는 가안이다 — 최종안은 U4 에서 메인이 사용자에게 올린다(§8-3)★.
5. **버전 키 없는 파일** — 상태 · agents · presets 도 1 판으로 읽는다(지금은 손상).
6. **BOM 붙은 agents · presets 를 읽는다**(지금은 손상).
7. **agents · presets 에 상한**(16 MiB · 4 MiB) — 넘으면 손상(지금은 없음). 실측 크기의 약 만 배라 정상이면 안 닿는다.
8. **임시 파일 이름** — `agents.json.tmp` → `agents.json.tmp<pid>.<번호>`(쓰는 동안만 · 죽으면 다음 데몬 기동이 쓸고, 옛 `.tmp` 도 그때 지운다). mcp 폴더에도 쓰는 동안 임시 파일이 잠깐 생긴다.
9. **안 보이는 것** — 설정 읽기의 잠김 재시도(최악 +100 ms) · 폴더 동기화(Windows 무동작) · 무리에 못 넣은 자식은 무리 밖에서 사는 대신 스폰이 실패로 돌아온다(g — 드묾) · (h)는 동작 무변경.

---

## 5. 게이트 · 의존

### 5-1. 직접 워크스페이스 의존

| crate | 지금 | 뒤 |
|---|---|---|
| datafile | — | platform |
| 셸(`engram-dashboard`) | … · platform | + datafile |
| agent | base · command · platform | + datafile |
| daemon | … · platform | + datafile(mcp_config) |
| net · base · platform | — | 그대로 |

### 5-2. 새로 서는 · 바뀌는 게이트

- **새** — datafile 의존 상한: `cargo tree -p engram-dashboard-datafile --depth 1 --prefix none -e normal,dev,build --target all --all-features | rg "^engram-dashboard" | sort -u` → 첫 칸이 정확히 `engram-dashboard-datafile` · `engram-dashboard-platform`(★이름 집합 일치 — 줄 수로 판정하지 않는다★, cli 상한과 같은 까닭) · `use tauri` 0(경로 확인 먼저) · CI `cargo test --locked -p engram-dashboard-datafile`(프로세스를 띄우지 않으므로 로컬도 `--test-threads` 없음). 정본 = 그 crate `src/lib.rs` 헤더 · 사본 = `ci.yml` · CLAUDE.md 「빌드·검증 명령」 · `.claude/skill-bindings/qa.md` · `docs/testing-strategy.md`.
- **메시징 이름 정규식** — 알파벳에 `datafile`(`ci.yml:542` · CLAUDE.md:257 · `qa.md:254` · `docs/testing-strategy.md:76`).
- **platform 4f 명단** — `src-tauri/src/fsutil.rs` 와 그 「시한부 운영 예외(ADR-0275 결정 15)」 줄을 뺀다(U1 — 판정이 양방향이라 빼지 않으면 빨개진다) · `transport/spawn.rs` 를 더할 수 있다(U7).
- **net 2a** — 바뀌지 않는다(net 은 A 밖 — §3-6). transport 3-3 이 열기를 platform `retry_busy` 로 옮길 때 그 자리(`ci.yml:1153-1160` · net `src/lib.rs:59-100` · CLAUDE.md:278 · `qa.md:283` · `docs/testing-strategy.md:85`)가 움직인다.
- **회귀 결과 줄** — U2 에서 +2(datafile lib 단위 + Doc-tests). 그 밖 단위는 그대로.
- **base 게이트** — 바뀌지 않는다(A 는 base 에 입주자를 더하지 않는다).

---

## 6. 단위

**원칙(TRD 1-3 §5 · 1-1 §6 과 같다):** 단위마다 워크스페이스 빌드 · 회귀 초록으로 끊는다. 단위 안 순서 = 「새 자리 만들기 → 부르는 곳을 파일 단위로 바꾸기 → 옛 자리 지우기」 — ★자료구조를 먼저 갈아엎고 호출부를 나중에 맞추는 순서는 금지★. 단위가 다른 단위의 옛 자리를 남겨 두지 않는다. 착수 전 그 자리 커밋으로 되돌릴 지점을 둔다(이 TRD 를 먼저 로컬 커밋). 주석은 `/code-conventions` 주석 규약을 지시서에 주입한다. **새 ADR 은 U1 바로 앞에 쓴다**(§7).

### 6-1. 단위 목록

| id | 범위 | 건드리는 파일 | 선행 | 크기(어림) | QA |
|---|---|---|---|---|---|
| **U1** | 잠김 규칙을 platform 으로 · fsutil 이 부른다 — ADR-0275 결정 15 를 닫는다 | platform `src/fs.rs`(+ `is_busy` · `Retry` · `retry_busy(_with)` · 옮겨 온 시험 5) · platform `src/lib.rs` 헤더(`:7-10` 시한부 예외 문장 · `fs` 입주자 설명) · 셸 `src/fsutil.rs`(`retry_denied` · `is_lock_contention` 걷고 platform 부름 · 판정 시험 둘 이사) · `ci.yml` 4f 명단 · CLAUDE.md(「플랫폼 중립」 · platform 항목) | — | 작음(운영 +50/−30 · 시험 5 이사) | standard |
| **U2** | datafile crate 를 fsutil 에서 낳는다 · 셸 호출부 전환 · fsutil.rs 삭제 — ★동작 무변경★ | 새 `crates/engram-dashboard-datafile/{Cargo.toml, src/lib.rs, src/write.rs, src/read.rs}`(fsutil 운영 · 시험 이사 — `&str` → `&[u8]`) · 루트 `Cargo.toml`(멤버 · 머리 주석 11 → 12) · `Cargo.lock` · 셸 `Cargo.toml` · `src/lib.rs`(`mod fsutil` 삭제) · `src/fsutil.rs`(삭제 — `fnv1a_64/hex` 는 유일한 소비자 `state/codec.rs` 로) · `settings/store.rs` · `state/{boot.rs, saver.rs, codec.rs}` · `ci.yml`(새 스텝 · 메시징 정규식) · CLAUDE.md(지도 · 「빌드·검증 명령」) · `qa.md` · `docs/testing-strategy.md` | U1 · U8 | 중간(대부분 이사 — 약 900줄 · 시험 22) | standard |
| **U3** | 읽기 규칙 · 쓰기 판정 · 폴더 만들기 · 폴더 동기화 — datafile 만 | datafile `src/{lib.rs, read.rs(Spec · Parsed · Loaded · classify · load · check_cap), policy.rs(WritePolicy), write.rs(R10 · R11)}` + 시험(R1~R8 · R10 · R11) | U2 | 작음~중간(+250 · 시험 약 20) | standard(+ Windows 폴더 동기화 무동작 실측 — §8-2 2) |
| **U4** | 셸 채택 — 설정 · 상태 · 새 판 안내 | `settings/store.rs`(+ `tests.rs` 가짜) · `state/{codec.rs, boot.rs, restore.rs, boot_plugin.rs}` · `src-tauri/bindings/`(재생성) · `src/api/restoreClient.ts` · `src/components/layout/StateFileNotice.tsx` · `src/i18n/ko.ts` · 해당 시험 | U3 · U8 | 중간 | **full** — GUI: `version: 2` 인 state.json → 기본 화면 + 새 안내 + 파일 바이트 그대로 · `$version: 2` 인 settings.json → 설정 바꾸기 실패 + 파일 그대로 · 깨진 state.json → `.corrupt` 안내(회귀) |
| **U5** | agent 채택 — agents · presets | agent `Cargo.toml` · `src/persistence/{mod.rs, presets.rs}`(+ 시험 — `corrupt_is_preserved_and_empty` ×2 · `version_mismatch_keeps_file` ×2 고침 · 새 판 거절 · 읽기 실패 거절 · BOM · 버전 없음 · 0 판 · 상한 · 옛 `.tmp` 쓸기) · 주석 `profile.rs:178 · 2013` · `commands.rs:69` | U3 | 중간(−150/+100 · 시험 +8) | **full** — 실 데몬: 새 판 agents.json 이 에이전트를 만든 뒤에도 바이트 그대로 · 깨진 agents.json → 첫 저장 뒤 `agents.json.corrupt` · 옛 `agents.json.tmp` 가 기동 뒤 사라짐 |
| **U6** | 재시도 부르는 쪽(scratch) · 토큰 파일 — ★net 은 뺐다(transport 3-3 후속 · §3-6)★ | agent `src/usage/scratch.rs` · daemon `{Cargo.toml, src/control/mcp_config.rs}` | U2(mcp_config) · U1(scratch) | 작음 | standard + 로컬 실 claude 스폰 1회(`--mcp-config` 경로) · ★파일럿 먼저(§8-2 1)★ |
| **U7** | (g) 자식 띄우기 공용화 · 가드 규칙 | agent `src/transport/{mod.rs, spawn.rs(새), stdio.rs, pty.rs}` · `src/backend/codex/transport.rs`(가드 · 소스 시험 이사) · `ci.yml` 4f(spawn.rs 를 더하면) · 메모 §11 행 닫음 | — (4f 를 고치면 U1 뒤) | 중간 | **full** — claude(JSON · 터미널) · codex 스폰 · kill 뒤 생존 0(GUI) |
| **U8** | (h) 셸 락 오염 12곳 · saver 시계 | 셸 `settings/mod.rs` · `state/{boot_plugin.rs, restore.rs, placement.rs, saver.rs}` · `theme.rs` · `layout/apply.rs` | — | 작음 | standard |
| **U9** | 문서 · ADR 도장 · 낡는 기록 | §7 의 문서 목록 | U1~U8 | 문서 | `/qa` 문서 범위 + `/review doc`(load-bearing) |

### 6-2. 순서 · 물결

```
물결 1:  U1  ∥  U7  ∥  U8      (겹침 없음 — 단 U7 이 4f 명단을 고치면 같은 heredoc 이라 U1 뒤)
물결 2:  U2                    (U1 의 fsutil · ci.yml · CLAUDE.md, U8 의 saver.rs 와 겹친다)
물결 3:  U3  ∥  U6             (U6 은 datafile 을 소비만 한다 — U3 와 파일이 안 겹친다)
물결 4:  U4  ∥  U5  (∥ U6 계속)  (셸 대 agent — 파일이 안 겹친다. 둘 다 U3 의 API 를 소비만 한다)
물결 5:  U9
```

- ★병렬 전 접점을 못 박는다★ — §3-2 의 platform API 와 §3-3 의 datafile API(이름 · 모양 · 판정 표)가 U2 · U3 에 착지한 모양이 U4 · U5 · U6 의 지시서에 그대로 들어간다. 구현 중 모양이 바뀌면 메인이 이 절을 고친 뒤 다음 물결을 띄운다.
- 사람 한 명이 직렬로 돌면: U8 → U1 → U2 → U3 → U4 → U5 → U6 → U7 → U9.

### 6-3. 파일 겹침

| | U1 | U2 | U3 | U4 | U5 | U6 | U7 |
|---|---|---|---|---|---|---|---|
| U2 | `fsutil.rs` · `ci.yml` · CLAUDE.md | | | | | | |
| U3 | — | datafile `lib.rs` · `read.rs` · `write.rs` | | | | | |
| U4 | — | `settings/store.rs` · `state/{codec,boot}.rs` | — | | | | |
| U5 | — | — | — | — | | | |
| U6 | — | — | — | — | — | | |
| U7 | `ci.yml` 4f(spawn.rs 를 더할 때만) | — | — | — | — | — | |
| U8 | — | `state/saver.rs` | — | `state/{restore,boot_plugin}.rs` | — | — | — |

### 6-4. 단위 안 순서 (빌드가 서게)

- **U1** ① platform 에 API · 시험(아무도 안 부름) ② fsutil 이 platform 을 부르게 · 제 판정과 그 시험 둘을 지운다 ③ 4f 명단 · 문서. ②에서 끊기면 ①만 남은 초록 트리로 되돌린다.
- **U2** ① 새 crate 를 fsutil 사본으로 세우고 워크스페이스 · 게이트에 등록(아무도 안 부름) ② 셸 호출부를 파일 단위로(`settings/store.rs` → `state/boot.rs` → `state/saver.rs` → `state/codec.rs`) ③ `fnv1a` 를 codec 으로 ④ `fsutil.rs` · `mod fsutil` 삭제. fsutil 사본과 새 crate 는 이 단위 안에서만 공존한다.
- **U3** 새 타입 · 함수를 더하고(아무도 안 부름) 시험 → `write_atomic` 에 R10 · R11. ★R11 은 셸 쓰기에 바로 닿는다★(설정은 이미 폴더를 만든다 — 상태 · 크래시 사본에 새로 생기는 동작) — 단위가 선언한다.
- **U4** ① 설정(`read_document` → `classify` · `parse` · `write` 의 판정) ② codec ③ `decide_boot` 의 새 판 가드 ④ `StateFileStatus::Newer` · 매핑 · 바인딩 ⑤ 프론트 셋. ④ 뒤 ⑤ 전에 끊겨도 선다 — 프론트는 모르는 값을 `undefined` 로 두고 알림 하나만 잃는다(`restoreClient.ts` 머리 주석).
- **U5** ① `FileProfileStore`(적재 · 판정 기록 · 저장 · 쓸기) ② `FilePresetStore` ③ 시험 ④ 주석. 파일마다 빌드가 선다.
- **U6** ① 파일럿 — scratch 지우기 실패 코드 실측 ② mcp_config ③ scratch.
- **U7** ① `spawn.rs`(가드 · `spawn_piped` · `drain_stderr` — 아무도 안 부름) ② codex 를 그리로(가드 타입 · 소스 시험 이사) ③ stdio ④ pty ⑤ 세 통로를 함께 재는 소스 시험 · 4f.
- **U8** 파일마다 한 걸음 · saver 는 하위 트레이트를 먼저 세우고 `SystemClock` · `FakeClock` 을 옮긴 뒤 옛 트레이트를 지운다.

### 6-5. 단위마다 검증

1. **회귀 수 대조** — `cargo test --workspace -- --test-threads=4` 를 단위 앞뒤로 돌려 `test result:` 줄 수와 통과 총계를 둘 다 견준다(TRD 1-3 §5-2 의 명령 그대로). 기준선 = 마지막 기록 결과 줄 59 · 4233 통과 · 0 실패 · 31 무시(2-4 `/qa standard` · 2026-10-08) — ★U1 착수 직전에 다시 잰다★(이 TRD 는 돌려 보지 않았다).
2. **기대 차이** — 결과 줄: U2 +2 · 그 밖 0. 분포: U1 셸 → platform 5 · U2 셸 → datafile 22(fsutil 27 − U1 의 5) · U4 · U5 · U6 · U7 은 단위가 수를 선언한다(옮김 ±N · 새 시험 +k).
3. **게이트** — 건드린 crate 의 게이트 전부 + `cargo fmt --check` + 생성물 sync(U4 는 `src-tauri/bindings/` 가 바뀐다 — 생성물을 함께 커밋) + `cargo test -p engram-dashboard --test lib_unit`(셸이 바뀌는 단위) + 4f(U1 · U7) · 새 datafile 게이트(U2 뒤 늘).
4. **QA** — 표의 등급(`/qa` 바인딩). full 은 앱을 `scripts/` 런처로 띄운다(셸에서 직접 띄우지 않는다).

---

## 7. ADR · 문서 후속

**새 ADR 하나 — U1 바로 앞에 쓴다**(채번 · 링크 · 도장 = `/adr`). ★쓰는 그 자리에서 모든 원격 브랜치를 다시 보고 다음 빈 번호를 선점한다(번호 선점 커밋 — 2-3 · 2-4 꼴)★ — 2026-10-10 에 모든 로컬 · 원격 ref 의 `docs/decisions/` 를 훑은 최댓값은 0290 이었다(0287 = master · 0288~0290 = `v0.3.3/feat/panic-policy`) → 그날이면 0291 이다. 그 사이 다른 브랜치가 쓸 수 있으므로 이 값을 베끼지 않는다.

- **박을 것:** 사용자 결정 1a~2h · 3(§1 — 1d 의 「writable 은 뺀다 · net 은 transport 3-3 후속」 포함) · 거처 A(§3-1 — 메인 판단(사용자 위임 — 3단계 전까지 메인이 갈림길을 정하고 보고)) · R1~R15(메인 판단(위임 「성숙한 프로그램 관행대로」)) — ★특히 R2: 새 판 파일은 네 파일 모두 덮지 않는다 · TRD S21-storage §6-5 · §12 R3 를 뒤집고 ADR-0265 결정 4 · ADR-0274 를 좁힌다★ · R7 · R8(읽기 IO 실패 = 저장 거절) · (g) 가드 규칙.
- **거부한 대안:** §3-1 의 B · C · D · R14 의 「한 예산으로 맞추기」 · R2 의 「새 판도 떠 두고 덮기(지금 상태 · 설정의 규칙)」 · 1d 의 「writable 도 platform 정책으로」(§8-1 1). 결정 날조 금지 — 거부 근거는 이 TRD 에 적힌 것만 옮긴다.

**개정 대상(`Amends` + 개정당하는 쪽 도장):**

| ADR | 무엇이 바뀌나 |
|---|---|
| ADR-0266 결정 8 | 원자 쓰기는 platform 이 아니라 datafile(A) — platform 에는 잠김 규칙만 · 둘째 항목(「`set_aside_corrupt` 는 base `file` — rename 하나로 OS 분기 없음」)은 이미 거짓(§2-3) · 사본 목록의 `reject_store.rs` |
| ADR-0275 결정 15 | 시한부 예외를 닫는다 — platform 밖 운영 OS 분기 0 |
| ADR-0269 결정 2 표 `file` 행 · 「영향」 손상 사본 목록 | base `file` 을 만들지 않는다 — 손상 사본은 datafile 의 `copy_aside` |
| ADR-0265 결정 4 | `write_atomic` 자리(셸 `fsutil.rs` → datafile) · 「모르는 `$version`」 중 새 판은 떠 두지 않고 쓰기 거절(R2 — 좁힌다) |
| ADR-0274 | 적용 범위가 설정 · 상태에서 네 파일로 · 「통째로 못 쓸 파일」에서 새 판을 뺀다(R2 — 좁힌다) · 「떠 두기는 `fsutil::copy_aside` 한 곳」의 자리 |
| ADR-0061 | presets 는 profiles 를 「복제」하지 않고 같은 구현을 부른다 |
| 바뀌지 않음(확인) | ADR-0135(portfile 제자리 쓰기) · ADR-0282(쓰기 프로브 · `retry_if_vanished` 는 base 에 그대로 — §8-1 1) · ADR-0270(셸 → agent 없음 — datafile 은 agent 가 아니다) |

**문서(각 단위 또는 U9):**

1. CLAUDE.md — 「플랫폼 중립」(fsutil 예외 문장 · `cfg` 운영 분기 0) · 「백엔드 모듈 맵」(datafile 항목 신설 · platform 항목의 「이 crate 밖에 남은 cfg 운영 OS 분기」 문장) · 「빌드·검증 명령」(datafile 시험 줄 · 게이트 줄 · 메시징 정규식) · 워크스페이스 수치 줄(U 들의 실측).
2. 루트 `Cargo.toml` 머리 주석(멤버 12 · datafile 한 줄) · platform `src/lib.rs` 헤더(`:7-10` · `fs` 입주자).
3. `.github/workflows/ci.yml` · `.claude/skill-bindings/qa.md` · `docs/testing-strategy.md` — §5-2 의 사본.
4. TRD `docs/process/S21-storage/trd.md` — §6-5 표의 state.json 「버전 초과」 행 · §12 R3 의 「새 판이 쓴 `state.json` 은 버전 초과로 떠 두고 기본 화면」 · N6 — 개정 표시와 이 TRD 를 가리키는 줄.
5. TRD 1-1 §8 D1 · D2 · D3 · TRD 1-3 §5-1 U-W 행 · §9 「storage P3 착지 시점」 — 「→ TRD A」 표시(날짜 박힌 본문은 고치지 않는다).
6. 메모 `docs/refactoring/architecture-discussion-2026-09-26.md` §11(`:334` stdio · pty 가드 행 닫음).
7. `docs/tracking.md` T-47 상태 · T-33 에 「새 판 몫만 닫혔다」 한 줄(§8-2 4) · transport 3-3 의 후속 한 줄 — 「net 단일 인스턴스 열기 재시도는 3-3 에서 platform `retry_busy` 를 쓴다(예산 4 × 100 ms · TRD A §3-6 의 메모)」(3-3 TRD 가 서면 그쪽으로 옮긴다).
8. `docs/reference/structure/session-path-ownership.md:821 · 887-888 · 2000` · `docs/reference/structure/agent-backend.md:237` — 손상 사본 이름 · 새 판 덮어쓰기 서술.
9. `docs/process/step-log.md` — 착지 항목(단위마다 아니라 A 전체 한 항목).
10. 코드 앵커 — datafile 헤더 · `write_policy` · platform `is_busy` 에 새 ADR · `// ADR-0274` 는 `copy_aside` 를 따라간다.

---

## 8. 범위와 어긋난 것 · 미검 · 열린 것

### 8-1. 범위와 어긋났던 것 — 메인 결정(2026-10-10)

1. **base `writable.rs` `retry_if_vanished` 는 1d 에서 뺀다(base 에 그대로).** 잠김 재시도가 아니다 — `NotFound` 일 때 한 번 더 하는 것이고 까닭은 「검사 도중 남이 폴더를 지웠다」는 경합이다(`writable.rs:86-97`). OS 규칙이 없어 platform 헌장(OS 의존 여부)에 안 맞고, ADR-0282 가 그 자리를 base 로 정했다. 넣으려면 platform 에 OS 무관 코드가 들어간다.
2. **「새 판은 덮지 않는다」는 네 파일 모두다** — 메인 판단(위임 「성숙한 프로그램 관행대로」 — 새 판이 쓴 파일을 옛 판이 덮지 않는다). 상태는 TRD S21-storage §6-5 · §12 R3(「새 판 `state.json` 은 떠 두고 기본 화면 · 하향 지원 안 함」)을 뒤집고, 화면 안내 · 버스 값(`newer`)이 새로 생긴다(§4 2 · 4). 설정은 ADR-0265 결정 4 · ADR-0274 의 「모르는 `$version` = 못 쓸 파일」을 좁힌다. 새 ADR 에 든다(§7).
3. **`layout/apply.rs:247` 을 U8 에 넣는다** — 셸 락 오염 복구가 처음 목록보다 하나 많았다(운영 · 같은 꼴 · 1-1 집계 뒤 들어왔다). `discovery/layout.rs:235` 는 시험이라 뺀다.
4. **R7 · R8 유지 — 읽기 IO 실패도 저장을 거절한다.** agent 의 「읽기 IO 실패 → 빈 목록 → 덮음」(`persistence/mod.rs:106-110` · `presets.rs` 같은 자리)은 새 판 결함과 같은 부류라 함께 닫는다 — 사용자가 보는 변화다(§4 3).
5. **net 몫은 U6 에서 뺀다** — net 은 transport 3-3 에서 갈린다. 후속 = 「transport 3-3 에서 platform retry 를 쓴다」(§3-6 의 메모 · §7 7). 그때까지 net 의 열기 재시도와 2a 게이트는 그대로다.
6. **ADR-0266 결정 8(「`write_atomic` 은 platform 으로」)은 A 로 개정한다** — 그 결정은 원자 쓰기 함수 안의 OS 분기 한 줄 때문에 함수째 OS 층에 두자는 것이었다. 분기를 `is_busy` 로 떼면 그 근거가 사라진다(§7).

### 8-2. 미검

1. **scratch 지우기가 실제로 어떤 오류로 실패하나** — 지금 코드는 모든 오류를 다시 하고(`scratch.rs:76-86`), 실측(2026-09-27 40/40)은 시간만 남겼다. `is_busy` 가 그 코드를 못 물면 손자가 쥔 폴더를 다시 안 해 쓸기로 밀린다(기능 손실은 없지만 동작이 바뀐다). U6 ① 파일럿이 잰다 — 145 등이면 platform 판정에 더할지 메인이 정한다.
2. **Windows 폴더 동기화가 무동작인가** — std `File::open` 이 폴더를 못 연다고 보고 R10 을 `cfg` 없이 썼다. U3 이 잰다(열리면 `sync_all` 이 무엇을 하는지도).
3. **관행 인용 넷은 원문 대조를 안 했다** — Cargo.lock(R1) · SQLite · Firefox(R2) · LevelDB · SQLite 유닉스 경로(R10) · graceful-fs · Node `fs.rm`(R13). RFC 8259 §8.1(R5)만 확신이 높다. 규칙은 지금 셸 규칙과 실측에도 기대므로 인용이 틀려도 결론은 서지만, ADR 「근거」에 옮기기 전에 확인한다.
4. **T-33(모르는 `kind` 하나가 agents.json 전체를 손상으로 만든다)은 안 닫힌다** — R2 는 새 빌드가 `kind` 를 더할 때 `schema_version` 을 올려야만 그 경우를 막는다. 올리면 R3 의 리더 규칙이 따라붙는다.
5. **기준선 회귀 수** — 돌려 보지 않았다(§6-5).
6. **datafile 시험 수 · 줄 수** — 어림이다(fsutil `#[test]` 27 · 913줄 실측 기준).

### 8-3. 열린 것 (사용자 확인 거리)

- **새 판 안내 문구와 기존 두 문구의 고침**(§4 4) — 사용자가 읽는 글이라 이 TRD 에는 가안으로만 둔다. 최종안은 U4 에서 메인이 사용자에게 올린다.
- **agents · presets 가 새 판 · 읽기 실패로 저장을 멈출 때 화면 안내가 없다** — 데몬 로그만 남는다. 사용자는 에이전트 목록이 비어 있고 만든 에이전트가 재시작 뒤 사라지는 것으로만 안다. 안내를 두려면 별건(데몬 → 셸 알림 표면)이다.
