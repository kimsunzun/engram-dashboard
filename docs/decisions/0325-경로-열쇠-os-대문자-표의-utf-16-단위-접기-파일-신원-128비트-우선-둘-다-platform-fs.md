# ADR-0325: 경로 열쇠 = OS 대문자 표의 UTF-16 단위 접기 · 파일 신원 = 128비트 우선 — 둘 다 platform `fs`

- 상태: 제안 (2026-10-31, 근거: 메인 M38(재리뷰 B5 · F11) · M60(0a codex 리뷰 BLOCK 유지 · 보고) · 조사 `docs/research/library-scan-file-identity-path-key-crates-2026-10-31.md`)
- 관련: ADR-0315 · ADR-0324 · S25 초안 AH(미채번 — walkdir) · CLAUDE.md 「플랫폼 중립」 · ADR-0266(운영 OS 분기는 platform 안) · TRD `docs/process/S25-preset-library/trd.md` §1-6 · §1-4 `catalog.rs` · 흐름 ① 훑기 꼬리표 · 흐름 ② 「찾을 수 없음」 · §5-2 2 단계 · T-74

## 맥락
Windows 는 대소문자를 가리지 않는다. `Catalog` 의 `items` · `used_by` 는 경로로 찾는 표인데, 짝 비교 `same_path(a,b)` 로는 표의 열쇠를 만들 수 없어 대소문자만 다른 참조가 Windows 에서 「찾을 수 없음」 으로 보였다(M38). 훑기는 바뀐 파일만 다시 파싱하려고 경로 · 수정 시각 · 크기 · 파일 신원을 꼬리표로 견준다(TRD 흐름 ①).

## 결정
1. platform `fs` 에 `path_key(path)` 를 더한다 — **`same_path(a,b)` == 열쇠가 같음**. `Catalog` 의 `items` · `used_by` 는 이 열쇠로 찾고, 경로를 견주는 자리(에이전트 쪽 역조회 · 경로 고침의 맞는 붙임)는 `same_path` 로 견준다(M38).
2. Windows 열쇠 = **UTF-16 코드 단위마다 · BMP 단위만(서로게이트는 그대로) · OS 대문자 매핑**(`RtlUpcaseUnicodeChar` 또는 `LCMapStringEx` invariant UPPERCASE — 파일럿에서 고른다). 그 밖 OS = 그대로(M60).
3. 폴더마다 대소문자를 가리는 폴더(WSL `setCaseSensitiveInfo`)는 지원 밖이라 적는다(M60).
4. 파일 신원(훑기 꼬리표) = **128비트 `FileIdInfo` 우선 · 안 되는 볼륨은 64비트(볼륨 일련번호 + 파일 인덱스)로 물러난다**(M60 — 64비트는 ReFS 에서 유일하지 않을 수 있다). 얻는 길(platform 직접 호출 · `file-id` · `winapi-util`)은 파일럿에서 고른다(조사).
5. 파일 신원은 훑기 꼬리표에만 쓴다 — 다시 연결 후보가 아니다(G38 · T-74 — 그건 R19 「후보 제안 없음」 을 다시 연다).

## 거부한 대안
- **Rust `char::to_uppercase`**(수집자 안 — 한 글자 매핑일 때만) — 보충 평면 이름을 합친다(U+10428 → U+10400 · NTFS 의 `$UpCase` 표는 BMP 만이라 둘을 가른다)(M60 · codex 리뷰 BLOCK 유지).
- **짝 비교 `same_path` 만** — 표의 열쇠를 만들 수 없다(M38).
- **`unicase`** — 완전 접기(`ß` → `ss`)라 NTFS 와 다르다 · **`icu_casemap`** — 무겁다 (기각 근거: 약함) · **git `strcasecmp` 꼴** — ASCII 만 · **`CompareStringOrdinal`** — 비교 함수지 열쇠가 아니다(조사).
- **std Windows `MetadataExt` 의 `volume_serial_number` · `file_index`** — 불안정(rust-lang/rust#63010 · 1.95 에서도 nightly) · **`same-file`** — 저장할 신원 값이 없다(비교만)(조사).

## 근거
- 조사 library-scan(medium · codex 리뷰 BLOCK → M60 으로 정정): 열쇠 방향 = 결정 · NTFS 와의 일치 = 미측 · 정확한 API = 불확실. MS `BY_HANDLE_FILE_INFORMATION` 문서 — 64비트 인덱스는 ReFS 에서 유일하지 않을 수 있다.

## 영향 / 불변식
- 열쇠 규칙을 바꾸면 `same_path` 와 함께 바꾼다 — 둘이 같다는 것이 불변식이다.
- **파일럿 실측**: `RtlUpcaseUnicodeChar` · `LCMapStringEx` 가 NTFS `$UpCase` 와 어디서 갈리나(보충 평면 · 서로게이트 · `ß` · `ı`) · ReFS 에서 128비트 신원이 실제로 얻어지나 · 안 되는 볼륨에서 물러나기가 도나.
- macOS APFS 는 접기 + NFC/NFD 정규화가 필요한데 `unicode-normalization` 이 잠금에 없다 — 문서로 남긴 갭이다(조사 · 「플랫폼 중립」).
- `winapi-util` 을 고르면 platform 게이트 5(OS crate 손 명단)에 더한다(조사).
