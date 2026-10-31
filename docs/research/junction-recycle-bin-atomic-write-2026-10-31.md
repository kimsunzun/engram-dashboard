# 정션 · OS 휴지통 · 원자적 교체 — 서버 쪽 라이브러리 선택 (조사 · medium)

- **상태:** medium · 2026-10-31 · 수집자 1 + 메인 grounding + codex 적대 리뷰(effort high) — 판정 FIX. TRD `docs/process/S25-preset-library/trd.md` §5-1 step 0a(구현 부분별 외부 라이브러리 조사)의 서버 쪽 한 갈래.
- **계기:** 프리셋 라이브러리가 ① 폴더 연결(정션) 만들기 · 끄기 ② 지운 것을 `.trash` 에 두고 OS 휴지통으로 보내기(G45) ③ 파일을 원자적으로 교체하기(G38)를 해야 한다. 각각 직접 짤지 crate 를 들일지 가른다.
- **확신도 범례:** 확실 = 1차 출처 + 독립 교차확증(메인 grounding 또는 codex 재확인이 일치) · 가능성 높음 = 단일 1차 출처, 반박 없음 · 불확실 = 추정·미검 · contested = 리뷰가 반증과 함께 반박.

## 결론

**결정 둘(메인 · 2026-10-31).**

- **결정 1 — 라이브러리 파일 쓰기는 `base::file::write_atomic` 을 재사용하고, `sweep_temps` 는 라이브러리에 대해 부르지 않는다.** TRD 에 적힌 `<파일>.engram-tmp` 는 두 번째 규칙이 되므로 쓰지 않는다. `sweep_temps` 를 안 부르는 것은 G40 「자동 정리 없음」을 지키기 위해서다. 라이브러리 폴더를 훑는 쪽은 base 의 임시 이름 꼴(`<이름>.tmp<pid>.<번호>`)을 건너뛴다.
- **결정 2 (M59) — 영구 삭제는 「이 볼륨엔 휴지통이 없다/꺼져 있다」로 확정되는 HRESULT 에서만 한다**(예: `0x8027003A`, `RECYCLE_FORCE_NUKE` 류). 사용 중 · 접근 거부 · 너무 큼 · **알 수 없는 코드는 전부 `.trash` 에 남기고** 항목별 원 HRESULT 를 보고한다. 없는 볼륨 · 꺼진 볼륨 · 이동식 · 네트워크에서의 실제 동작은 구현 파일럿에서 잰다.

**필요별 선택.**

| 필요 | 선택 | 한 줄 이유 | 차선 |
|---|---|---|---|
| 정션 만들기 | `junction` 2.1.0 의 `create` | std 의 `junction_point` 는 불안정(#121709), `symlink_dir` 은 개발자 모드·권한이 필요. 자기 폴더엔 관리자 권한 불필요(가능성 높음) | 원시 `FSCTL_SET_REPARSE_POINT` 직접 호출(crate 재구현) |
| 정션 끄기 | `std::fs::remove_dir` — **단 링크임을 비추종 메타데이터(reparse tag)로 먼저 확인** | `RemoveDirectoryW` 가 링크만 지우고 대상은 그대로(확실). `junction::delete` 는 빈 폴더를 남겨 부적합 | — |
| OS 휴지통 | `IFileOperation` **직접 호출**(`trash` crate 아님) | 항목별 결과 · HRESULT 를 보존해야 G45 가 선다 | `trash` 5.2.9 (아래) |
| 원자적 교체 | `base::file::write_atomic` (std `rename` 위) | 이미 있고 셸 설정 저장소가 쓴다. 새 crate 불필요 | `tempfile` 3.27.0 `persist` (고정 이름엔 부적합) |

**★리뷰가 강제한 정정(아래 본문에 이미 반영).**

1. 「`PreDeleteItem` 에서 `TSF_DELETE_RECYCLE_IF_POSSIBLE` 이 없으면 취소하니 조용한 영구 삭제가 막힌다」는 **과장이었다** — 그 플래그는 「가능하면 휴지통」일 뿐 증명이 아니다. 가드는 두되 **미검증**으로 적는다.
2. `0x8027003A` 는 「휴지통 없음」이 아니라 「휴지통을 못 찾았거나 쓸 수 없음」이다. 세 코드 구분이 파괴적 대체 경로를 허가하게 두지 않는다 → 결정 2.
3. `psiNewlyCreated == NULL` 만으로 「영구 삭제됨」이라 못 한다 → `hrDelete` 와 `GetAnyOperationsAborted` 를 함께 본다.
4. `trash` crate 의 한계 서술(「실패가 전부 Unknown」)은 과했다 — 바로잡은 판은 아래 휴지통 절.
5. `junction::exists` 는 끊긴 정션에 `false` 라 제거 가드로 단독 사용 불가.
6. 기존 writer 의 디렉터리 sync 는 Windows 에서 무동작이다 → 내구성 서술을 정확히.

**TRD 불일치(메인 grounding 발견).** TRD §1-6 의 플랫폼 표 행(G12, 2판)은 OS 휴지통을 되살렸는데 그 아래 불릿은 아직 「(걷음 2026-10-19) OS 휴지통 `fs::move_to_trash`」로 남아 있다 → 정리 필요.

## 1. 정션

| 이름 | 판 · 날짜 | 라이선스 | 이미 잠금 파일에? | 맞음 | 확신도 | 출처 |
|---|---|---|---|---|---|---|
| `junction` (tesuji) | 2.1.0 · 2026-09-24 · 최근 다운로드 약 160만 | MIT | 아니오 (의존 `windows-sys` >=0.59,<=0.61 — 잠금엔 0.59/0.60/0.61 이 있음 · `scopeguard` 는 잠금 상태 미확인) | 맞음 | 가능성 높음 | crates.io API · 로컬 레지스트리 소스 |
| std `junction_point` | 불안정 (#121709) | — | — | 못 씀 | 확실 | `os/windows/fs.rs:720` |
| std `symlink_dir` | 안정 | — | — | 개발자 모드·권한 필요 → 부적합 | 가능성 높음 | rust 1.95.0 std |
| 원시 FSCTL 직접 | — | — | `windows` 계열은 이미 있음 | crate 를 재구현하는 꼴 | 가능성 높음 | 수집자 판단 |

- **만들기:** `junction::create` = `create_dir` + `FSCTL_SET_REPARSE_POINT`(`IO_REPARSE_TAG_MOUNT_POINT`, `src/internals.rs:25-105`). 권한 상승 경로는 `PermissionDenied` 일 때만(`helpers.rs:16-66`). ★**reparse 설정이 실패하면 빈 폴더를 남긴다 — 호출하는 쪽이 치워야 한다.**
- **끄기:** `std::fs::remove_dir`. std 1.95 의 rmdir = `RemoveDirectoryW`(`sys/fs/windows.rs:1382-1383`), MS 문서상 링크만 지우고 대상은 무영향(확실 — 수집자 + codex 재확인 일치). `junction::delete` 는 reparse 데이터만 벗겨 빈 폴더가 남는다.
- ★**정정(리뷰 반영):** `junction::exists` 는 먼저 `Path::exists` 를 불러 링크를 따라가므로 **끊긴 정션엔 `false`** 다. 또 `share_mode(0)` 으로 열어 다른 곳이 폴더를 열고 있으면 실패할 수 있다. 제거 전 「정션인가」 확인은 비추종 메타데이터(`symlink_metadata` 의 reparse tag)로 한다.
- macOS: 심볼릭 링크로 만들고 `remove_file` 로 끈다(수집자 서술 — 미검).

## 2. OS 휴지통

**선택: `IFileOperation` 직접 호출.** 플래그 `FOF_NO_UI | FOFX_RECYCLEONDELETE | FOFX_EARLYFAILURE`(`FOF_ALLOWUNDO` 필요 여부 미정) + `IFileOperationProgressSink`:

- `PreDeleteItem` — `dwFlags` 에 `TSF_DELETE_RECYCLE_IF_POSSIBLE`(0x80)이 없으면 오류를 돌려 취소한다. ★**이 가드가 「못 보내는 것을 미리 드러낸다」는 가정은 미검증이다**(contested → 아래 리뷰). Electron(`platform_util_win.cc#L97`)과 Qt(`qfilesystemengine_win.cpp#L764`)가 같은 가드를 쓰므로 설계를 받치지만 보장은 아니다.
- `PostDeleteItem` — 항목별 `hrDelete` 를 모은다. `psiNewlyCreated` 가 NULL 이면 영구 삭제 후보이나 **그것만으로 단정하지 않는다**.
- 비용: `windows` 0.58 의 `Win32_UI_Shell` + COM implement 매크로 · 전용 STA 스레드.

| 이름 | 판 · 날짜 | 라이선스 | 이미 잠금 파일에? | 맞음 | 확신도 | 출처 |
|---|---|---|---|---|---|---|
| `IFileOperation` 직접 | `windows` 0.58 위 | — | `windows` 있음(feature 추가 필요) | 맞음 — 항목별 HRESULT 가능 | 가능성 높음 (핵심 가정 미검증) | MS docs |
| `trash` | 5.2.9 · 2026-09-13 · ★255(Byron/trash-rs) | MIT | 아니오 (`windows` 0.62 → 세 번째 복사본) | 부분 — 항목별 보고 없음 | 확실(소스 대조 + codex) | 로컬 소스 `src/windows.rs` |
| PowerToys Peek · .NET VB | — | — | — | `SHFileOperation` + `FOF_ALLOWUNDO`(+`WANTNUKEWARNING`, 대화형) — 우리 무창 데몬과 조건이 다름 | 가능성 높음 | codex 재확인 |
| Files 앱 · Go `mono0x/trash` | — | — | — | `IFileOperation` 콜백·플래그 사용 — 직접 호출의 선례 | 가능성 높음 | codex 재확인 |

**`trash` 5.2.9 를 안 쓰는 이유(정정된 판).** 플래그가 `FOF_NO_UI|FOF_ALLOWUNDO|FOF_WANTNUKEWARNING` 이고 `FOFX_RECYCLEONDELETE` 는 없다(`src/windows.rs:97`). `DeleteItem` 을 묶어 `PerformOperations` 한 번으로 돌리고, `FOFX_EARLYFAILURE` 가 없어 일부 삭제가 된 채 실패해도 한 덩어리로 보고된다. ★**리뷰 정정:** Windows 오류는 `Error::Os{code}` 로 매핑되고 `PerformOperations` 실패도 전파된다 — **「성공했는데 aborted」인 경우만** `Error::Unknown`("Some operations were aborted", `:110-115`, TODO "return the reason")로 뭉친다. 따라서 「전부 Unknown」은 틀렸고, 그래도 **항목별 보고가 없다**는 점이 직접 COM 을 정당화한다. 그 밖에: 무창 데몬에서 `FOF_WANTNUKEWARNING` + `FOF_NO_UI` 동작 불명(trash-rs #55) · 스레드마다 STA `CoInitializeEx` 가 `RPC_E_CHANGED_MODE` 에서 패닉(`:318-345`) · `CoCreateInstance` unwrap(`:95`).

**오류 분류 — 결정 2 와 함께 읽을 것.**

- `0x8027003A`(`COPYENGINE_E_RECYCLE_BIN_NOT_FOUND`) = 휴지통을 **못 찾았거나 쓸 수 없음**. 수집자는 이것을 「휴지통 없음」, `0x80070020`(사용 중), `0x80070005`(접근 거부)의 세 갈래로 가르자고 했으나 불완전하다 — 재활용이 꺼진 볼륨은 `RECYCLE_FORCE_NUKE`, 너무 큰 항목 · 과다 경로는 각자 코드, 공유 · 접근은 `COPYENGINE_E_*_SRC/DEST` 변종이 있다(`sherrors.h` L65–95).
- 처리: **원 HRESULT 를 보존하고 「알 수 없음」 범주를 둔다.** 세 코드 구분이 파괴적 대체(영구 삭제)를 허가하지 않는다.
- 사전 판별 수단은 없다: `SHQueryRecycleBinW` 는 내용만 보고, `GetDriveType` 은 거르기용일 뿐(USB 가 fixed 로 보일 수 있고 리디렉션 네트워크 폴더에도 휴지통이 있을 수 있음). 포함 볼륨은 `GetVolumePathName` 으로 풀고, 불확실하면 보수적으로 처리한다.
- `PerformOperations` 는 취소 뒤에도 성공을 돌릴 수 있으니 `GetAnyOperationsAborted` 도 본다. `FOFX_EARLYFAILURE` 는 이미 끝낸 항목을 되돌리지 않는다.

## 3. 원자적 교체

**선택: `base::file::write_atomic` 재사용.** `crates/engram-dashboard-base/src/file.rs` — `write_atomic`(L348) · `write_atomic_unless` · `copy_atomic` · `sweep_temps`(L537, 이 프로세스나 죽은 pid 의 임시만 지움 — agent 영속화와 셸 상태 부팅이 부른다). 임시 이름 `<이름>.tmp<pid>.<번호>`(L501) → sync → `std::fs::rename` → 잠김 시 `OsHooks` 로 재시도 → 디렉터리 sync(ADR-0265 결정 4 · ADR-0291 R10/R11). 셸 `settings/store.rs:87` 이 쓴다. 근거는 결정 1.

| 이름 | 판 · 날짜 | 라이선스 | 이미 잠금 파일에? | 맞음 | 확신도 | 출처 |
|---|---|---|---|---|---|---|
| `base::file::write_atomic` | 저장소 내 | — | 예 | 맞음 | 확실(메인이 소스 확인) | `file.rs:348` |
| std `fs::rename` | 1.95 | — | 예 | 같은 볼륨 원자성 가능성 높음(MS 보장 문구 없음) | 가능성 높음 | `sys/fs/windows.rs:1311-1379` |
| `tempfile` | 3.27.0 | — | 확인 안 함 | `persist` = `SetFileAttributesW` + `MoveFileExW`. 고정 이름은 `Builder` 로 되나 `create_new(true)` 라 꺼진 뒤 남은 파일이 있으면 `AlreadyExists` → 고정 이름엔 부적합 | 가능성 높음 | 로컬 소스 |
| `atomic-write-file` | 0.3.1 | — | 확인 안 함 | 무작위 이름(`rand`) | 가능성 높음 | docs.rs · `generic.rs` |
| `atomicwrites` | 0.4.4 · 2024-09 이후 정체 | — | 확인 안 함 | 낡음 | 가능성 높음 | crates.io |

- std 1.95 의 `rename` = `MoveFileExW(REPLACE_EXISTING)`, `ACCESS_DENIED` 면 `SetFileInformationByHandle(FileRenameInfoEx, REPLACE_IF_EXISTS|POSIX_SEMANTICS)` 로 대체. `WRITE_THROUGH` 는 없다.
- `ReplaceFileW` 는 속성 · ACL 을 보존하고 대상이 없으면 실패한다(가능성 높음 — 수집자가 기억에서 적음, 문서 재확인 안 함).
- ★**내구성 서술 정정(리뷰):** 기존 writer 의 디렉터리 sync 는 Windows 에서 무동작이고(`platform/src/fs.rs:125`) 그 결과를 무시한다(`base/src/file.rs:482`). 「교체 뒤 디렉터리 항목까지 디스크에 내려갔다」고 쓰지 말고, 보장은 임시 파일 sync + 같은 볼륨 rename 까지로 적는다.

## 적대 리뷰 결과 (codex · effort high · FIX)

| 심각도 | 지적 | 처리 |
|---|---|---|
| HIGH | 「`TSF_DELETE_RECYCLE_IF_POSSIBLE` 없으면 거부 → 조용한 영구 삭제 방지」는 과장 | **반영** — 가드는 두되 미검증 표기. 꺼진 휴지통 · 과대 항목 · 용량 · 이동식 · 네트워크를 파일럿에서 잼 |
| HIGH | 오류 분류 불완전 (`0x8027003A` = 없음 또는 사용 불가, `RECYCLE_FORCE_NUKE`, 크기·경로 코드, `*_SRC/DEST` 변종) | **결정으로 정리** — M59: 원 HRESULT 보존 + 알 수 없음 범주, 확정된 「휴지통 없음/꺼짐」에서만 영구 삭제 |
| MED | 「이 항목을 휴지통에 넣을 수 있나」 사전 판별 수단 없음 | **반영** — 사전 판별 안 함 · 불확실하면 보수적 |
| MED | `psiNewlyCreated == NULL` 단독으로 영구 삭제 단정 불가 · `GetAnyOperationsAborted` · EARLYFAILURE 무롤백 | **반영** — `hrDelete` 와 함께 판정 |
| MED | `trash` 한계 과장 (실제론 `Error::Os{code}`, success+aborted 만 Unknown) | **반영** — 서술 정정, 항목별 보고 부재가 직접 COM 의 근거로 남음 |
| MED | `junction::exists` 가 끊긴 정션에 `false` | **반영** — 비추종 메타데이터 확인으로 대체 |
| MED | 기존 writer 디렉터리 sync 가 Windows 에서 무동작 · 결과 무시 | **반영** — 내구성 서술 정확화 |

codex 가 재확인한 것: `RemoveDirectoryW` 가 정션만 지우고 대상 무영향 · PowerToys Peek 와 .NET VB 는 `SHFileOperation` + `FOF_ALLOWUNDO`(대화형) · Files 앱과 Go `mono0x/trash` 는 `IFileOperation` 콜백·플래그 · 더 강한 사전 판별은 못 찾음.

## 공백 · 실측 필요 (구현 파일럿)

- **`IFileOperation` 의 동작 실측** — FAT32 USB · 휴지통 꺼진 볼륨 · 너무 큰 항목 · 이동식 · 네트워크에서 `PreDeleteItem` 플래그 · `hrDelete` · `psiNewlyCreated` · `GetAnyOperationsAborted` 가 실제로 무엇을 내는지. 핵심 가정(가드가 미리 드러냄)은 아직 미검증.
- `FOF_ALLOWUNDO` 필요 여부 · 정확한 `windows` feature 목록.
- `junction` 의 `windows-sys`/`scopeguard` 해석(`cargo tree` 미실행) · 잠금 파일 추가 여부.
- `ReplaceFileW` 문서 재확인(기억에 의존).
- macOS 휴지통은 조사 안 함.
- 라이브러리 폴더 훑기가 base 임시 이름 꼴을 건너뛰는지(결정 1 의 전제) — TRD 의 `.engram-tmp` 문구와 함께 정리.

## 출처

- crates.io API: `junction` 2.1.0 · `trash` 5.2.9 · `tempfile` 3.27.0 · `atomic-write-file` 0.3.1 · `atomicwrites` 0.4.4
- 로컬 cargo 레지스트리 소스: junction-2.1.0 `src/internals.rs:25-105` · `helpers.rs:16-66` · trash-5.2.9 `src/windows.rs:95, 97, 110-115, 318-345` · tempfile-3.27.0
- rust-lang/rust 1.95.0: `sys/fs/windows.rs` 1311-1445 (rename · 1382-1383 rmdir) · `os/windows/fs.rs:720` · 이슈 #121709
- MS docs: `RemoveDirectoryW` · `IFileOperation::SetOperationFlags` · `IFileOperationProgressSink` `PreDeleteItem`/`PostDeleteItem` · `_TRANSFER_SOURCE_FLAGS` · `sherrors.h` L65–95 · docs.rs windows-sys `COPYENGINE_E_RECYCLE_BIN_NOT_FOUND`
- trash-rs 이슈 #55 · #101 · atomic-write-file `generic.rs`
- Electron `platform_util_win.cc#L97` · Qt `qfilesystemengine_win.cpp#L764`
- 저장소: `crates/engram-dashboard-base/src/file.rs` L348 · L482 · L501 · L537 · `platform/src/fs.rs:125` · `src-tauri` `settings/store.rs:87` · ADR-0265 결정 4 · ADR-0291 R10/R11 · TRD `docs/process/S25-preset-library/trd.md` §1-6 · §5-1
