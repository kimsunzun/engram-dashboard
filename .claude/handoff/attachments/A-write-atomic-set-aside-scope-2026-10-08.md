# A 범위 조사 — write_atomic → platform · set_aside_corrupt 통일 (2026-10-08 · 트리 `aef34d4`)

조사 워커(worker-scout) 반환 요지. 줄 번호는 이 트리 기준이다 — 쓰기 전에 다시 본다.

## 결론
- **(1) 원자적 쓰기:** platform 밖에 남은 운영 `cfg` 분기는 **한 줄** — `src-tauri/src/fsutil.rs:320` 의 잠김 판정 `is_lock_contention`(`cfg!(windows) && raw_os_error 32|33`). TRD · ADR 의 `:107` 은 P3 전 줄 번호라 낡았다. 시한부 예외만 걷는 일 = **작다** · 원자 쓰기 묶음을 통째로 platform 으로 = **중간** · agent persistence 두 벌까지 합치기 = **동작이 바뀌어 결정 필요**.
- **(2) 손상 사본 통일:** ★계획의 전제 둘이 P3 착지로 깨졌다★ — ADR-0269 가 정한 모양(base `file::set_aside_corrupt(path, stamp_ms)` · rename → `.corrupt-<ms>`)과 달리 ADR-0274 가 셸 규칙을 **고정 이름 `.corrupt` · 복사 · 덮어쓰기**로 바꿨다. 그 복사(`copy_atomic`)는 잠김 재시도(OS 분기)를 품어 base 에 못 들어간다(base 는 platform 을 의존하지 않는다). ADR-0266 결정 8 둘째 항목 「rename 하나로 OS 분기가 없다」도 이제 거짓. 통일하면 agents.json · presets.json 손상 사본 이름 · 동작이 바뀐다 → **사용자 결정**(디스크에 보인다).
- daemon `usage_service/reject_store.rs` 는 없다(ADR-0284 `9ab15231` 이 걷었다) → TRD 1-3 :382 · ADR-0266 :33 · ADR-0269 :96 의 사본 목록이 낡았다.

## 명세 인용
- ADR-0266 결정 8(`0266-…md:32-34`) — write_atomic 은 platform · 여러 벌 통일은 1-3 · 합치면 ADR-0265 결정 4(「write_atomic 은 셸 공용 fsutil.rs」)가 낡는다 · 「set_aside_corrupt 는 base file 에 남는다 — rename 하나로 OS 분기가 없다」(← 낡음).
- ADR-0275 결정 15(`:46`) 시한부 예외 · 거부한 대안 `:73`(지금 옮기는 안 — P3 가 같은 파일을 바꿔서).
- TRD 1-3 `:50` · `:153`(fs 행 + U-W 때 write_atomic) · `:382`(U-W 단위 — 「하나로 정할 동작」은 그때 **사용자 선택**) · `:545` · `:349`(4f 명단에 fsutil.rs).
- 메모 `:287` · `:304`(구현 갈림길은 사용자에게) · `:306`. §11 에 이 두 건 항목 없음.
- TRD 1-1 `:90` · `:538` D1 · `:453` M5 · `:539` D2(형제 미룸: 셸 `settings/mod.rs` 락 오염 복구 3곳 `:466/:470/:474` · `state/saver.rs:83` `trait Clock` → base `Clock` · `state/boot_plugin.rs:119,242,383,444`).
- ADR-0269 `:31` · `:35` · `:54` · `:95`(손상 사본의 하나로 정한 동작 = 열린 것, 1-1 에서) · `:98`.
- ADR-0274(`0274-…md:16-19`, 사용자 결정 F21) — 고정 이름 `<파일>.corrupt` 덮어쓰기 · 크기 한도 · 중복 생략 · 시각 이름 없음 · 복사 = 임시 + sync_all + rename(`copy_atomic`) · **적용 범위 = 설정 · 상태 파일만**(agent 파일 밖) · `:43` 떠 두기는 `fsutil::copy_aside` 한 곳.

## 현재 코드 인벤토리
| 구현 | 위치 | crate | 운영 호출자 |
|---|---|---|---|
| `write_atomic` · `write_atomic_unless` · `WriteOutcome` · `replace_with` | `src-tauri/src/fsutil.rs:73,82,147` | 셸 | `state/boot.rs:83,92` · `state/saver.rs:30,118` · `settings/store.rs:93` |
| `copy_atomic` · `copy_aside` | `fsutil.rs:101,126` | 셸 | `state/boot.rs:77,102` · `settings/store.rs:97` |
| `sweep_temps` | `fsutil.rs:233` | 셸 | `state/boot.rs:106` |
| `read_capped` · `read_file_capped`(OS 무관 · 잠김 재시도 씀) | `fsutil.rs:25,46` | 셸 | `settings/store.rs:79` · `state/boot.rs:73,87` · `state/saver.rs:122` |
| `fnv1a_64/hex` | `fsutil.rs` | 셸 | `state/codec.rs:183` |
| `is_lock_contention` ★운영 cfg★ | `fsutil.rs:318-321` | 셸 | `retry_denied` 경유 전부 |
| `FileProfileStore::write_atomic` · `preserve_corrupt` | `crates/engram-dashboard-agent/src/persistence/mod.rs:50,79` | agent | save · load(데몬 소유) |
| `FilePresetStore::write_atomic` · `preserve_corrupt` | `…/persistence/presets.rs:46,70` | agent | 같음(ADR-0061 — mod.rs 복제) |
- fsutil.rs 913줄 · 시험 모듈 `:323` 부터 `#[test]` 27 · 시험 안 `cfg!(windows)` `:408` · `:429`. 셸 시험 가짜: `settings/store.rs:387` · `settings/tests.rs:948` · `store.rs:406`.
- 범위 밖: `net/src/portfile.rs:47` `write_in_place`(ADR-0135 의도) · `base/src/writable.rs` · `daemon/src/control/mcp_config.rs:115,162`(평범한 fs::write — 통일 후보인지 별도 판단).

## 동작 차이
| 축 | 셸 fsutil(P3 뒤) | agent persistence ×2 |
|---|---|---|
| 임시 이름 | `<이름>.tmp<pid>.<번호>` | 고정 `agents.json.tmp` · `presets.json.tmp` + write_lock Mutex |
| 파일 sync_all | 함 | 함 |
| 부모 디렉터리 fsync | 안 함 | 함(best-effort `mod.rs:70-74`) |
| rename 잠김 재시도 | 5회 × 20ms · PermissionDenied + Win 32/33 | 없음 |
| 실패 때 임시 파일 | 지움 | 남김(다음 쓰기가 덮음) |
| 폴더 생성 | 안 함 | create_dir_all |
| rename 직전 건너뛰기 | write_atomic_unless | 없음 |
| 남은 임시 쓸기 | sweep_temps(상태 파일만) | 없음 |
| 입력 | `&str` | `Vec<u8>` |
| 손상 사본 | 복사(원본 유지) → 고정 `<이름>.corrupt` 덮어쓰기 · 재시도 · 경로 반환 · 로그는 호출자 | rename(이동) → `<이름>.corrupt-<epoch ms>` 누적 · 재시도 없음 · 로그 함수 안 |
| 손상 판정 | JSON 아님 · 상한 · UTF-8 · 모르는 version | JSON 파싱 실패만(version 불일치는 그대로 둠) |

## 자리와 게이트
- platform `lib.rs:12-22`(입주자 여덟) · `fs.rs`(168줄)에 이미 `is_sharing_violation`(32) · `is_access_denied`(5) — 잠김 판정(32 · 33 · PermissionDenied)의 자연스러운 자리. 입주 규칙 = OS 의존 여부(결정 2) → OS 무관한 `read_capped` · `fnv1a` · `.corrupt` 이름 규칙을 platform 으로 옮기면 헌장과 긴장. 셸 · agent 는 이미 platform 운영 의존(새 간선 없음).
- base `file` 신설 시 입주자 무참조 정규식 4곳(`base/src/lib.rs:52` · `ci.yml:668` · `qa.md:161` · `CLAUDE.md:254`) + CLAUDE.md 「입주자는 일곱」.
- 4f 명단 `ci.yml:941-942`(「시한부 운영 예외(ADR-0275 결정 15) / src-tauri/src/fsutil.rs」) — 운영 분기를 걷어도 시험 `:408` · `:429` 의 `cfg!(windows)` 가 남으면 시험 분기 구획으로 옮기거나, 시험 기대값을 platform 판정으로 바꿔 명단에서 뺀다.
- 문서: `CLAUDE.md:103`(플랫폼 중립) · `:159`(platform 항목) · platform `lib.rs:7-10` · `:108` · ADR-0275 결정 15 · ADR-0266 결정 8 · ADR-0269 :96-98 · ADR-0265 결정 4 · TRD 1-3 `:107` · reject_store 언급.

## 열린 결정
- **사용자:** ① agent 손상 사본을 ADR-0274 규칙으로 맞출지(`agents.json.corrupt-<ms>` 이동 · 누적 → `agents.json.corrupt` 복사 · 덮어쓰기 — 두 번째 손상 때 앞 백업이 사라진다 · F21 은 설정 · 상태에만 적용됐다). 「안 맞춘다」면 D1 은 「통일하지 않음」으로 닫히고 agent 두 벌을 agent 안 공용 함수 하나로 묶는 일만 남는다. ② TRD 1-3 :382 · 메모 :304 가 「하나로 정할 원자 쓰기 동작」을 사용자 선택으로 표시했지만 실제 축은 전부 내부(재시도 · 디렉터리 fsync · 임시 이름 · 실패 정리) — 메인이 정하고 보고할지 TRD 표시대로 올릴지 메인이 정한다. 밖에서 보이는 차이 하나: agent 임시 이름을 pid 꼴로 바꾸면 크래시 뒤 `agents.json.tmp<pid>.<n>` 이 쌓일 수 있다(막으려면 데몬 기동 때 sweep_temps).
- **내부:** 자리 — 잠김 판정만 platform `fs` 로(최소안) vs write_atomic · copy_atomic · sweep_temps 묶음까지. ADR-0266 결정 8 둘째 항목은 전제가 깨져 개정 ADR 로 닫아야 한다.

## 크기 · 단위(권고)
- **U-W1(작음 · 권장 선행)** — 잠김 판정을 platform `fs` 로(33 추가 · 시험 2~3) · fsutil 이 그것을 부른다 · 시험 `:408/:429` 처리 · ci.yml 2줄 · platform lib.rs 2곳 · CLAUDE.md 2곳 · ADR 개정 하나. 약 6파일 · 호출부 0 · 화면/디스크 변화 0. 이것만으로 「시한부 운영 예외」가 닫힌다.
- **U-W2(중간 · 선택)** — 원자 쓰기 묶음을 platform 으로(시험 포함 500~600줄 · 호출부 boot.rs 7 + 주석 3 · saver.rs 3 · store.rs 3) · `.corrupt` · `.tmp` 이름 규칙이 platform 으로 들어가 「도메인 지식 0」과 경계가 애매. U-W1 과 파일 겹침 → 순차 또는 한 묶음.
- **U-W3 + D1(agent · 결정 ① 뒤)** — persistence 두 벌을 공용 원자 쓰기 · 손상 사본으로(시험 9 · `corrupt_is_preserved_and_empty` ×2 가 `.corrupt-` 이름 단언 · 주석 `profile.rs:178,2013` · `commands.rs:69` · `persistence/mod.rs:190`). 셸과 파일이 안 겹쳐 U-W2 와 병렬 가능 — 단 공용 API 모양을 메인이 먼저 못 박는다.
- **D2(TRD 1-1 · 별 단위)** — settings 복구 3곳 · state 복구 사이트 · saver Clock Sync 맞춤. U-W2 와 겹치니 그 뒤.
