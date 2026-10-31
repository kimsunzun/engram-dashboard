# ADR-0336: 서버 부품 선택 — serde-saphyr · jsonschema · ureq · walkdir · junction · pulldown-cmark · sha2

- 상태: 제안 (2026-10-31, 근거: 0a 외부 라이브러리 조사(medium · codex 적대 리뷰) · 사용자 B18(2026-10-19 「구현전에 외부 라이브러리 쓸수있는거 다 조사하고 들어가」) + 메인 M56 · M57 · M60 · M61 · M66(보고))
- 관련: ADR-0318(감싸는 단위) · ADR-0335(원자적 쓰기) · ADR-0332(설정 검사) · ADR-0333(YAML 고쳐 쓰기) · ADR-0322(OS 휴지통) · ADR-0324(잠금) · ADR-0325(경로 열쇠 · 파일 신원) · CLAUDE.md 「의존성 (변경 시 보고)」 · TRD §1-12 · §5 0a · 조사 `docs/research/yaml-schema-https-crates-2026-10-31.md` · `docs/research/library-scan-file-identity-path-key-crates-2026-10-31.md` · `docs/research/markdown-frontmatter-hash-sync-crates-2026-10-31.md` · `docs/research/junction-recycle-bin-atomic-write-2026-10-31.md`

## 맥락
서버 단계(G20)가 쓸 서드파티를 구현 부분마다 다시 조사했다(§5 0a). `Cargo.lock` 에 TLS 스택 · YAML crate · JSON 스키마 검사기가 하나도 없어, 무엇을 고르든 새 패키지가 들어온다(조사 yaml-schema-https).

## 결정
| 쓰임 | 고름 | 자리 · 조건 |
|---|---|---|
| YAML · `.md` 앞머리 **읽기** | `serde-saphyr` 2.0.0 (`default-features = false`, `["deserialize"]`) | asset-library `format` · bool 대상은 `strict_booleans: true`(YAML 1.1 꼴이라 `yes/no/on/off` 가 bool 로 읽힌다) |
| YAML **고쳐 쓰기** | `yaml-edit =0.3.5` | ADR-0333 |
| JSON 스키마 검사 | `jsonschema` **0.60.2** (`default-features = false`, `["resolve-file","idna"]`) — TRD 의 0.59.0 에서 올림 | asset-library `validate` · 포맷 검증 정책은 코드에 밝힌다(두 대상 스키마가 draft-07) |
| 스키마 받기(HTTPS) | `ureq` 3.5.0 블로킹 (`default-features = false`, `["rustls","platform-verifier","gzip","win-system-proxy"]`) | 데몬의 fetch 포트 구현(agent · asset-library 는 네트워크를 모른다 — ADR-0311) · `RootCerts::PlatformVerifier` 를 코드로 밝힌다(feature 만으로는 Mozilla 루트가 기본) · 읽기 API 를 이름으로 정하고 크기 상한을 둔다 · 프록시 = 환경변수 + `win-system-proxy`(HKCU 수동 프록시 하나만 — PAC · WPAD 안 됨 · 간접 `winreg` 를 끈다 — 새 패키지 · T-62 대상 — 메인 2026-10-31 M66) |
| 폴더 훑기 | `walkdir` 2.5.0 (잠금에 이미 있음 — 직접 의존으로 올려도 새 패키지 0) | asset-library `store` · 루트가 링크일 때의 정책을 코드로 밝힌다(`follow_root_links` 기본 true) |
| 디렉터리 정션 | `junction` 2.1.0 `create` · 끄기 = 링크임을 비추종 메타데이터로 먼저 확인한 뒤 `std::fs::remove_dir` | platform `fs` · reparse 설정이 실패하면 남은 빈 폴더를 호출자가 치운다 |
| 지침 `.md` 발췌 | `pulldown-cmark` 0.13.6 (MIT · 신규) | `MetadataBlock` 하위 트리를 건너뛴다 · 표는 `ENABLE_TABLES` |
| `SKILL.md` 앞머리 | 손으로 짠 분리(codex `extract_frontmatter` 규칙 그대로) + serde-saphyr · codex 의 **수선(repair) 규칙 전부를 옮긴다** | 엄격 검사만 하면 codex 가 싣는 스킬을 잘못 막는다 |
| 판 해시(`rev`) | `sha2` 0.10(SHA-256 · 잠금에 있음 — 데몬 바이너리엔 새로) | 원시 바이트만 해시 · 붙임 목록은 길이 접두 정규 인코딩(없으면 `["ab","c"]` · `["a","bc"]` 가 부딪는다) · `std::hash::Hash` 를 거치지 않는다 |
| 원자적 쓰기 · OS 휴지통 · 잠금 · 파일 신원 | 각각 `base::file::write_atomic` · `IFileOperation` 직접 · std `Mutex`+`Condvar` · ADR-0325 | ADR-0335 · ADR-0322 · ADR-0324 · ADR-0325 · OS 휴지통의 `IFileOperation` 직접은 기본값이고 `trash` crate 와의 고름은 구현 때 다시 잰다(ADR-0326 결정 5) |

TOML 이 필요하면 이미 있는 `toml` 1.1.2 · `toml_edit` 0.25 를 다시 쓴다.

## 거부한 대안
- **YAML 읽기**: `serde_yaml` 0.9 — 보관됨(archived) · `serde_yml` — 보관됨 · deprecated 껍데기 · RUSTSEC-2025-0068(unsound) · `noyalib` — 같은 저자 0.x · `serde_norway` — 갱신 정체 · `saphyr-serde` — 자리표시자 · `yaml-rust2` — serde 없음 · `marked-yaml` — 활동 적음 · `saphyr` 0.3.3 — 차점(serde 없음)(조사 yaml-schema-https). `yaml_serde` 0.10.7 은 비교하지 않았다 — 판정 없음.
- **JSON 스키마**: `boon` 0.6.1 — 차점 · `valico` — 낡음(같은 조사). `jsonschema` 기본 feature — reqwest · rustls · aws-lc 를 끌고 온다.
- **HTTPS**: `reqwest` 0.13.5 — 차점: 기본 TLS(rustls + aws-lc-rs)가 Windows 에서 NASM 을 요구한다. ★「async 라서 탈락」 은 과했다(블로킹 · ring 꼴도 있다)★ — 근거를 의존 수와 런타임 불필요로 좁혔고, tokio 위에서 돌거나 데몬 HTTPS 가 늘면 다시 잰다. `native-tls` · `schannel` — 플랫폼마다 갈린다(수집자 판단)(같은 조사 · codex 리뷰 정정).
- **폴더 훑기**: `jwalk` — rayon · crossbeam 의존 · 병렬이 필요 없다 · `ignore` — gitignore 의미 + globset · regex-automata 를 끌고 온다(조사 library-scan).
- **정션**: std `junction_point` — 불안정(#121709) · `symlink_dir` — 개발자 모드 · 권한 필요 · `junction::delete` — reparse 데이터만 벗겨 빈 폴더가 남는다 · `junction::exists` — 링크를 따라가 끊긴 정션에 `false` · 열린 폴더면 실패할 수 있어 제거 가드로 못 쓴다(조사 junction-recycle-bin).
- **마크다운**: `comrak` — AST 기반 · 무겁다 · `markdown-rs` — 느림(불확실 · 수집자 단독)(조사 markdown-frontmatter-hash-sync).
- **앞머리**: `gray_matter` — yaml-rust2 를 끌고 오고 규칙이 codex 와 다르다 · `yaml-front-matter` — 2021 · serde_yaml 의존(같은 조사).
- **판 해시**: `xxhash-rust` — BSL-1.0 · 신규 · `DefaultHasher` — 판 사이 불안정(같은 조사 · `rustc-hash` · `fxhash` 의 서술은 미검).
- **원자적 쓰기 crate**: `tempfile` — `create_new` 라 남은 같은 이름 파일이 있으면 `AlreadyExists` · `atomic-write-file` — 무작위 이름 · `atomicwrites` — 2024-09 이후 정체(조사 junction-recycle-bin). 이미 있는 base 를 쓴다(ADR-0335).

## 근거
- 조사 yaml-schema-https: serde-saphyr = 오류에 줄 · 칸 + 발췌 · 중복 키 기본 거부 · 예산 상한(billion-laughs) · unsafe 없음 · 퍼징 · `Spanned<T>` · MSRV 1.89 / jsonschema = 기본 feature 를 끄면 reqwest · TLS 가 안 들어온다 · 판 자동 감지 · `instance_path()` / ureq = 평범한 데몬 스레드에서 돌아 async 반입 게이트를 지키고 의존 트리가 작다 · TLS = ring(C 컴파일러만 · NASM · CMake 불필요). 셋 다 확실.
- 조사 library-scan: walkdir = 확실(이미 잠금에 있다).
- 조사 junction-recycle-bin: junction 2.1.0(2026-09-24 · MIT)의 `windows-sys` 범위가 잠금에 이미 있다 · `RemoveDirectoryW` 는 링크만 지운다(확실).
- 조사 markdown-frontmatter-hash-sync: codex 앞머리 규칙 = `codex-rs/skills/src/parser.rs`(확실 — 최신 판도 같음) · sha2 = 확실.

## 영향 / 불변식
- 새 패키지: serde-saphyr · jsonschema · ureq + 새 TLS 스택(rustls · ring) · pulldown-cmark · yaml-edit(rowan · countme · text-size) · winreg(`ureq` 의 `win-system-proxy` — M66). CLAUDE.md 「의존성」 에 더할 줄은 구현 때 보고한다.
- `junction` 은 OS crate 의존 게이트(`/qa` 4g · CI `platform gate 5`)의 손 목록에 더한다. async 반입 게이트 · HTTPS 클라이언트는 스키마 받기 단계에서 정한다(M17) — `ureq` 의 `win-system-proxy` 가 그 게이트(직접 의존만 셈)에 안 걸린다는 것은 가능성 높음.
- **확인할 것(조사의 남은 항목)**: 세 crate(serde-saphyr · jsonschema · ureq)의 라이선스는 조사 입력에 없었다 — 들이기 전 `Cargo.toml` · LICENSE 확인 · `jsonschema` 0.60.2 에서 claude 스키마(draft-07 + `$defs`) 통과 시험 · `ureq` 의 304 · 크기 초과 동작 · Windows 시스템 프록시는 HKCU HTTP 프록시 하나만 읽는다(PAC · WPAD 안 됨) · serde-saphyr 의 `Spanned` 는 untagged · flatten 안에서 위치가 사라진다(`kind` 판별을 untagged enum 으로 짜면 오류 위치가 안 나온다) · serde-saphyr 와 codex 의 serde_yaml 의 가장자리 차이 — 두 파서 나란히 골든.
- **구현 판단으로 남긴 것**: 판 해시를 `sha1`(데몬 그래프에 이미 있어 새 패키지 0)로 갈지 — `rev` 가 보안 용도가 아니라는 전제에 달렸다(조사).
- TRD §1-12 표가 이 고름을 받았다(2026-10-31 확인 — `jsonschema` 0.60.2 · HTTPS 칸 = `ureq` + `win-system-proxy` · `trash` 행 = 「거부 — 구현 때 다시 잰다」).
