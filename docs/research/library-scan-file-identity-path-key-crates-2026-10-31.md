# 라이브러리 훑기 · 파일 신원 · 경로 키 — 무엇을 고르나 (서버 쪽 외부 라이브러리 조사 · medium)

- **상태:** medium · 2026-10-31 · 수집자 1(worker-scout) + 메인 grounding + codex 적대 리뷰(effort high) — 판정 **BLOCK**(경로 키 한 건 — 메인 결정으로 정정해 아래에 반영. 훑기 · 파일 신원은 FIX 급 보완만). TRD `docs/process/S25-preset-library/trd.md` §5-1 step 0a(구현 부분별 외부 라이브러리 조사) 중 서버 쪽 — 폴더 훑기 · 파일 신원 · 경로 키.
- **계기:** 프리셋 라이브러리가 폴더를 훑어 카탈로그를 만들고(TRD §1-4 FsStore), 같은 파일인지 · 같은 경로인지를 가려야 한다(TRD §1-6). 직접 짜기 전에 쓸 만한 crate 가 있는지 본다.
- **확신도 범례:** 확실 = 1차 출처 + 독립 교차확증(메인 grounding 또는 codex 재확인이 일치) · 가능성 높음 = 단일 1차 출처, 반박 없음 · 불확실 = 추정/미검 · contested = 리뷰가 반증과 함께 반박.

## 결론

| 필요 | 고름 | 이유 한 줄 | 차선 |
|---|---|---|---|
| 폴더 훑기 | **walkdir 2.5.0** | 이미 `Cargo.lock` 에 있고(tauri-build 빌드 의존 경유) asset-library 의 직접 의존으로 올려도 잠금 새 패키지 0. `filter_entry` 로 `.trash/` 가지치기 · `*.engram-tmp` · 점 이름 제외. TRD §1-4 가 이미 walkdir 를 가리킨다 | std `read_dir` 재귀 |
| 파일 신원 | **Windows = `winapi-util::file::information(handle)` 의 `volume_serial_number()` + `file_index()` · 그 밖 = std unix `MetadataExt` 의 `dev`/`ino`** (platform crate 안) | winapi-util 0.1.11 이 이미 잠겨 있다. 무crate 대안 = `windows` 0.58 + `Win32_Storage_FileSystem` 으로 `GetFileInformationByHandle`(TRD §1-6 계획) | file-id 0.2.3 |
| 경로 키 | **직접 짠 `platform::fs::path_key`** — ★리뷰로 정정★ 아래 | 그 일을 해 주는 crate 가 NTFS 규칙과 맞지 않는다 | 없음 |

★**리뷰가 강제한 정정 셋** (아래 「적대 리뷰 결과」 상세)★

1. **경로 키 = 메인 결정.** 수집자 안(「문자마다 `to_uppercase()` 가 한 글자일 때만」)은 **반증됐다** — 보충 평면 글자(예 U+10428 → U+10400)를 한 글자 매핑으로 같게 만들어, NTFS 가 구별하는 두 이름이 충돌한다. **결정: UTF-16 코드 단위마다, BMP 단위만(서로게이트는 건드리지 않음), OS 대문자 매핑(`RtlUpcaseUnicodeChar` 또는 `LCMapStringEx` invariant UPPERCASE — 어느 쪽인지는 파일럿에서 고름)으로 대문자화. `same_path` = 키 일치. NTFS 와의 일치는 구현 파일럿에서 실측한다.**
2. **파일 신원 = 지원되는 곳에서는 128비트 `FileIdInfo`(`GetFileInformationByHandleEx`)를 우선**하고 문서화된 대체(볼륨 일련번호 + 64비트 인덱스)를 둔다. 볼륨 일련번호 + 64비트 인덱스는 ReFS 에서 유일하지 않을 수 있다. 위 고름은 「잠금에 이미 있는 것」 기준이고, 128비트까지 얻는 가장 가까운 길은 file-id crate(둘 다 한다) 또는 platform 이 직접 `FileIdInfo` 를 부르는 것 — 파일럿에서 고른다.
3. **훑기 = 루트 심볼릭 링크 정책을 명시한다.** walkdir 2.5.0 은 `follow_links(false)` 여도 `follow_root_links` 기본이 true 라 루트가 링크면 따라간다. 부분 트리 훑기 포함해 정책을 코드로 정한다.

덧붙임 둘: **Windows 폴더별 대소문자 구별**(WSL `fsutil setCaseSensitiveInfo`)이 켜진 폴더에서는 `A.md` 와 `a.md` 가 공존해 무조건 대문자화가 둘을 합친다 → 지원 밖으로 문서화하거나 폴더 플래그를 따른다. **macOS** 는 충실한 키에 대소문자 접기 + NFC/NFD 가 필요한데 `unicode-normalization` 이 잠금에 없다 → 알려진 갭으로 남긴다(TRD §1-6 「그 밖 = 그대로 비교」 는 대소문자 구별 비교다).

## 1. 폴더 훑기

| 이름 | 판 · 날짜 | 라이선스 | 이미 잠금 파일에? | 맞음 | 확신도 | 출처 |
|---|---|---|---|---|---|---|
| **walkdir** | 2.5.0 · 2024-03-01 | Unlicense/MIT | 있음(tauri-build 빌드 의존만 — 운영 직접 의존으로 올려도 새 패키지 0) | 맞음 — `filter_entry` 가지치기 | 확실(`Cargo.lock` 메인 확인 + crates.io) | crates.io API · `Cargo.lock` |
| std `read_dir` 재귀 | — | — | — | 차선 — 가지치기 · 오류 처리를 직접 짠다 | 확실 | — |
| jwalk | 0.9.0 | — | 확인 없음 | 거름 — rayon/crossbeam 의존, 병렬 불필요 | 가능성 높음 | crates.io API |
| ignore | 0.4.34 | — | 확인 없음 | 거름 — gitignore 의미 + globset/regex-automata 를 끌어와 regex-automata 판이 오를 수 있다 | 가능성 높음 | crates.io API |

- 루트 링크: walkdir 2.5.0 은 `follow_root_links` 기본 true(codex · `src/lib.rs`) — 위 정정 3.
- **접합 따라가기**(Windows junction)는 walkdir 가 어떻게 다루는지 **미검증**(기본은 심볼릭 링크를 안 따른다고만 확인) — 공백.

## 2. 파일 신원

| 이름 | 판 · 날짜 | 라이선스 | 이미 잠금 파일에? | 맞음 | 확신도 | 출처 |
|---|---|---|---|---|---|---|
| **winapi-util** `file::information` | 0.1.11 | — | 있음(windows-sys 범위도 충족) | 맞음 — `volume_serial_number()` · `file_index()` 로 얻음. 단 64비트 API 를 감싼 것(ReFS 한계 — 정정 2) | 확실(docs.rs — 수집자는 기억이라 표시, 메인이 확인 · codex 재확인) | docs.rs `winapi_util::file::Information` |
| `windows` 0.58 + `Win32_Storage_FileSystem` | 0.58 | — | 0.58 기반(platform 계획) | 맞음 — 무crate · `FileIdInfo` 도 직접 부를 수 있음 | 가능성 높음 | TRD §1-6 |
| file-id (notify-rs) | 0.2.3 | — | 확인 없음 · windows-sys ^0.60.1 이 잠긴 0.60.2 를 재사용 | 맞음 — 128비트 `FileIdInfo`(ReFS) + 대체 | 가능성 높음(수집자) · 두 API 를 쓴다는 점은 codex 가 일치 | notify-rs `file-id` `src/lib.rs` |
| std Windows `MetadataExt` | — | — | — | **못 씀** — `volume_serial_number`/`file_index` 는 불안정(`windows_by_handle`, rust-lang/rust#63010 열림 · rustc 1.95 안정판) | 확실(메인 · codex 재확인) | rust-lang/rust#63010 · doc.rust-lang.org MetadataExt |
| same-file | 1.0.6 | — | 확인 없음 | 거름 — 저장 가능한 신원 값이 없다(비교만) | 가능성 높음(「아마」) | crates.io |

- 어느 쪽이든 **platform crate 안**에 둔다. winapi-util 을 고르면 OS crate 이므로 **게이트 5 의 손 명단에 더해야 한다.**
- unix 쪽은 std `MetadataExt` 의 `dev`/`ino` 로 충분.

## 3. 경로 키 (대소문자 같은 경로 판정)

- **NTFS 규칙:** 비교는 볼륨 `$UpCase` 표로 하는 서수 대문자화 — UTF-16 단위당 1:1, 완전 접기(`ß`→`ss`) 없음, 정규화 없음(수집자 · 확실도 가능성 높음). 단 그 표는 **BMP 만**이고 보충 평면 글자는 구별된 채 남는다(codex · docs.rs ntfs `upcase_table.rs` — 이 부분이 수집자 안을 반증).
- OS 의 `CompareStringOrdinal(bIgnoreCase)` 는 비교 함수지 키가 아니다(MS docs).

| 이름 | 판 | 맞음 | 확신도 | 출처 |
|---|---|---|---|---|
| **직접 짠 `path_key`** (정정판 — UTF-16 단위 · BMP 만 · OS 대문자 매핑) | — | 맞음. `RtlUpcaseUnicodeChar` vs `LCMapStringEx`(invariant, UPPERCASE)가 `$UpCase` 와 어디서 갈리는지는 **미검** — 틀려도 「못 찾음」이 아니라 충돌이 될 수 있으니 실측 대상 | 정정 방향 = 결정(리뷰 반증에 따른 메인 결정 M60) · NTFS 와 같은지 = 미실측(파일럿에서 잼) · 정확한 API = 불확실 | MS docs · docs.rs ntfs |
| 수집자 안: 문자마다 Rust `to_uppercase()`(한 글자일 때만) | — | **contested — 거름.** U+10428 → U+10400 이 한 글자 매핑이라 NTFS 가 구별하는 두 이름이 같은 키가 된다 | contested(리뷰가 반증) | doc.rust-lang.org char::to_uppercase · unicode.org n_10400(Deseret) |
| unicase | 2.10.0 | 거름 — 완전 접기(`ß`→`ss`)라 NTFS 와 다름 | 확실(소스) | unicase `src/unicode/mod.rs` |
| caseless | 0.2.2 | 거름 | 가능성 높음 | crates.io |
| icu_casemap | 2.4.0 | 거름 — 무겁고 Unicode 3.0 | 가능성 높음 | crates.io |
| git `strcasecmp` | — | 거름 — ASCII 만 | 확실 | git `dir.c:127` |
| dunce | 1.0.5 | 이미 직접 의존 — 정규화가 필요한 **canonicalize 자리에서만** 씀(키는 아님) | 확실 | `Cargo.lock` |

- 위치: `platform::fs::path_key`, **Windows 만**. `same_path` = 키 일치.
- **macOS:** APFS 기본은 대소문자 · 정규화 모두 무시하므로 충실한 키가 접기 + NFC/NFD 를 요구한다(`unicode-normalization` 0.1.26 — 잠금에 없음). 지금은 **문서화된 갭**(APFS 주장은 다시 가져와 확인하지 않음).
- **폴더별 대소문자 구별(Windows):** 정정 덧붙임 참고.

## 적대 리뷰 결과

| # | 심각도 | 지적 | 처리 |
|---|---|---|---|
| 1 | HIGH(BLOCK) | Rust 문자별 `to_uppercase` 키는 NTFS 와 안 맞는다 — 보충 평면 글자가 같은 키가 돼 다른 카탈로그 항목이 충돌(잘못된 참조 해석) | **결정으로 정리·반영** — 위 메인 결정(UTF-16 단위 · BMP 만 · OS 매핑 · 파일럿 실측) |
| 2 | MED | 볼륨 일련번호 + 64비트 인덱스는 ReFS 에서 유일하지 않을 수 있다(MS `BY_HANDLE_FILE_INFORMATION` 문서) | **반영** — 128비트 `FileIdInfo` 우선 + 문서화된 대체. 어느 경로(file-id crate / platform 직접 호출)인지는 파일럿에서 |
| 3 | MED | Windows 폴더별 대소문자 구별로 `A.md` · `a.md` 공존 가능 — 무조건 대문자화는 둘을 합친다 | **남김** — 지원 밖 문서화 또는 폴더 플래그 존중 중 하나를 구현 때 정한다(미결) |
| 4 | MED | walkdir 2.5.0 `follow_root_links` 기본 true | **반영** — 루트 링크 정책을 명시(부분 트리 포함). 정책 내용은 구현 때 정한다 |

codex 재확인(일치): winapi-util 의 `information`/`volume_serial_number`/`file_index` 존재(64비트 API 를 감쌈) · std `windows_by_handle` 여전히 nightly.

## 공백 · 실측 필요

- **경로 키를 NTFS 와 실측 대조**(구현 파일럿 필수): `RtlUpcaseUnicodeChar` vs `LCMapStringEx` 가 실제 볼륨 `$UpCase` 와 어디서 갈리는지, 보충 평면 · 서로게이트 · 특수 글자(예 `ß` · `ı`)에서 같은 이름 쌍 / 다른 이름 쌍이 파일 시스템 동작과 일치하는지.
- ReFS 나 `FileIdInfo` 가 되는 환경에서 128비트 신원이 실제로 얻어지는지, 안 되는 볼륨에서 대체가 동작하는지.
- walkdir 가 Windows junction · 재해석 지점을 어떻게 다루는지(미검증).
- 폴더별 대소문자 구별 폴더를 만났을 때의 정책.
- 미조사: .NET · VS Code 의 경로 키 관행. 확인 못 함: APFS 서술의 재검증. Rust `to_uppercase` 와 `$UpCase` 의 전수 차이는 미측정(정정판은 이 비교를 OS 매핑으로 우회한다).
- platform 게이트 5 손 명단에 winapi-util 추가 여부(고른 길이 winapi-util 일 때).

## 출처

- crates.io API (walkdir 2.5.0 · jwalk 0.9.0 · ignore 0.4.34 · file-id 0.2.3 · same-file 1.0.6 · unicase 2.10.0 · caseless 0.2.2 · icu_casemap 2.4.0 · dunce 1.0.5 · unicode-normalization 0.1.26)
- `Cargo.lock` (walkdir 2.5.0 · winapi-util 0.1.11 — 메인 확인)
- https://github.com/rust-lang/rust/issues/63010 (`windows_by_handle`) · doc.rust-lang.org MetadataExt
- docs.rs `winapi_util::file::Information`
- notify-rs `file-id` `src/lib.rs`
- MS docs `CompareStringOrdinal` · `BY_HANDLE_FILE_INFORMATION`
- unicase `src/unicode/mod.rs` · git `dir.c:127`
- doc.rust-lang.org `char::to_uppercase` · unicode.org n_10400(Deseret) · docs.rs ntfs `upcase_table.rs` (codex 인용)
- walkdir 2.5.0 `src/lib.rs` (`follow_root_links`)
