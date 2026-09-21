// Windows manifest 리소스를 **테스트 타깃에도** 링크한다.
//
// 왜 필요한가: 이 셸의 테스트 exe 는 Wry/muda/rfd 사슬을 타고 `comctl32.dll!TaskDialogIndirect` 를
//   정적 import 한다. 그 export 는 comctl32 **v6** 에만 있고, v6 는 바이너리가 `RT_MANIFEST`
//   (`Microsoft.Windows.Common-Controls 6.0.0.0`) 를 품을 때만 side-by-side 로 바인딩된다. manifest 가
//   없으면 로더가 System32 의 v5.82 를 물어 진입점을 못 찾고 **프로세스가 뜨기도 전에** 0xC0000139
//   (STATUS_ENTRYPOINT_NOT_FOUND) 로 죽는다 — 테스트 한 줄도 못 돈다.
//
// 왜 tauri_build 만으로 안 되는가: `tauri_build::build()` 가 그 manifest 를 `$OUT_DIR/resource.lib` 로
//   만들어 주기는 하는데, embed-resource 가 그것을 `cargo:rustc-link-arg-bins=` 로만 흘린다
//   (embed-resource 3.0.9 `src/lib.rs:444`). cargo 는 그 인자를 `[[bin]]` 에만 적용한다.
//
// ★`-bins` 와 `-tests` 로 갈라 emit 하는 것이 핵심이다★ — 접미사 없는 `cargo:rustc-link-arg=` 로
//   한 번에 덮으면 bin 이 같은 리소스를 **두 번** 받아 `CVT1100: duplicate resource type:VERSION` →
//   `LNK1123` 으로 빌드가 깨진다(실측). 여기서는 `-tests` 만 더하므로 bin 쪽 인자는 건드리지 않는다.
//
// 짝이 되는 설정 = `Cargo.toml` 의 `[lib] test = false` + `[[test]] name = "lib_unit"`.
//   cargo 의 `-tests` 는 **kind=test 타깃**(= `[[test]]` 선언분)을 뜻하고 lib 내장 유닛테스트 타깃은
//   거기 들지 않는다. 그래서 같은 `src/lib.rs` 를 명시 `[[test]]` 로 한 번 더 선언해 그 인자를 받게
//   했다. 셋 중 하나라도 되돌리면 0xC0000139 가 그대로 돌아온다.
fn main() {
    tauri_build::build();

    // ★`#[cfg(windows)]` 로 가르지 않는다★ — 아래 함수 머리 주석 참조(호스트 cfg ≠ 타깃 OS).
    emit_resource_link_arg_for_tests();
}

/// `$OUT_DIR` 에서 `tauri_build` 가 남긴 리소스 정적 라이브러리를 찾아 테스트 타깃 링크 인자로 흘린다.
///
/// ★windows **타깃**일 때만 emit 하며, 판정은 `#[cfg(windows)]` 가 아니라 런타임 환경변수다★ — build
/// script 는 **호스트**로 컴파일되는데 이 리소스는 **타깃** 산출물이다. 둘을 같은 것으로 보면
/// 비-Windows 호스트에서 `*-pc-windows-msvc` 로 크로스 빌드할 때 이 코드가 통째로 사라져 emit 이
/// 조용히 빠지고 0xC0000139 가 그대로 돌아온다. 그래서 함수는 무조건 컴파일하고 갈림길만 런타임에 둔다.
///
/// 이름(`resource.lib`)을 박지 않고 확장자로 훑는 이유 = 그 이름은 tauri-build/embed-resource 의 내부
/// 사정이라 업스트림이 갈면 조용히 안 걸린다. 다만 훑기만 하고 **개수를 안 세면** 그 유연함이 그대로
/// 위험이 된다: `$OUT_DIR` 는 빌드 사이에 청소되지 않고 CI 는 `target/` 를 캐시에서 복원하므로,
/// 업스트림 rename 이 한 번이라도 있으면 stale + 신규가 **둘 다** 링크돼 `CVT1100`(duplicate
/// resource type:VERSION) → `LNK1123` 이 난다. 그 인자는 패키지 전역(`-tests`)이라 지금 초록인 통합
/// 타깃 4개까지 함께 깨진다. 그래서 후보가 **정확히 1개일 때만** 통과시키고 0개·2개 이상은 **panic**
/// 으로 가른다.
///
/// ★"이름이 정확히 `resource.lib` 면 그걸 고른다"는 선호는 두지 않는다★ — 그 선호가 있으면 업스트림
/// rename 뒤 남은 **stale `resource.lib`** 를 신규 산출물 옆에서 조용히 집어, 바로 위 개수 검사가
/// 잡으려던 그 상태를 도로 숨긴다.
///
/// ★그런데 확장자 훑기 **하나만** 두면 릴리즈에서 후보가 2개가 된다 — v0.3.0 릴리즈 워크플로를 두 번
/// 죽인 결함이 이것이다(2026-09-22)★. `$OUT_DIR` 에는 리소스가 **아닌** `.lib` 이 정당하게 들어온다:
/// `tauri-build 2.6.3` 의 `src/static_vcruntime.rs` 가 정적 CRT 를 강제하려고 **하드코딩된
/// `msvcrt.lib` 를 덮어쓸 용도의 빈 오브젝트**(82바이트 COFF, 섹션은 `.drectve` 하나뿐인 링커 지시문
/// stub)를 거기 쓰고, 그 디렉터리를 `cargo:rustc-link-search=native=` 로 링커 탐색 경로에 올린다.
/// 즉 `$OUT_DIR` 는 리소스 전용 폴더가 아니라 **네이티브 라이브러리 탐색 폴더**이고, 앞으로도 리소스
/// 아닌 `.lib` 이 더 들어올 수 있다. 게다가 그 stub 은 `create_new` 로 쓰여 한 번 생기면 남는다.
///
/// ★debug 에는 없고 release 에만 있는 것이 이 결함이 여태 안 잡힌 이유다★ — 그 stub 은 `tauri-build`
/// `src/lib.rs:708` 에서 **`STATIC_VCRUNTIME=true` 환경변수가 있을 때만** 쓰인다. 맨 `cargo build` 는
/// 그 값을 주지 않고 tauri CLI 의 패키징 빌드(`npm run tauri -- build`)가 준다(그 CLI 네이티브
/// 바이너리에 그 문자열이 박혀 있다 — 실측). 그래서 이 워크트리에서도 debug OUT_DIR 11곳에는
/// `resource.lib` 하나뿐이고, release OUT_DIR 에만 `msvcrt.lib` 가 함께 있었다(실측 2026-09-22).
///
/// ★그래서 후보는 이름이 아니라 **내용**으로 거른다 — `carries_resource_data`★. 이름으로 거르면 바로
/// 위 "이름 선호를 두지 않는다" 조항이 그대로 무너진다(stale `resource.lib` 를 도로 집는다). 내용은
/// 업스트림이 파일명을 갈아도 따라오지 않는다 — 리소스 산출물은 리소스 데이터를 품고, 링커 지시문
/// stub 은 안 품는다. 거른 뒤에 **살아남은 것에** 기존 "정확히 1개" 규칙을 그대로 적용한다: 0개도
/// 2개 이상도 여전히 panic 이고, 그 두 갈래의 의미는 아래 panic 문구가 갖는다.
///
/// ★"리소스니까 `.rsrc` 섹션 이름이 있겠지"로 **줄이지 말 것** — msvc 산출물에는 그 문자열이 한 번도
/// 안 나온다(실측 2026-09-22)★. `resource.lib` 는 이름만 `.lib` 이지 COFF 도 아카이브도 아니고
/// **Win32 `.res` 파일**이다(link.exe 가 `.res` 를 직접 받는다). `.res` 에는 섹션이라는 개념이 없어
/// `.rsrc` 가 존재할 수 없다 — `.rsrc` 하나로 거르면 **진짜 리소스가 탈락해 "0개" panic** 이 난다.
/// 판정의 실물은 `.res` 널 헤더 쪽이고 `.rsrc` 는 gnu 를 위한 둘째 갈래다.
///
/// ★후보를 읽지 못하면 panic 한다 — 조용히 건너뛰지 않는다★. 건너뛰면 그 파일이 처음부터 없었던 것이
/// 되어 개수 검사의 두 갈래가 함께 무력해진다: 못 읽은 것이 진짜 리소스였다면 "0개"로 서는 대신 **남은
/// stale 하나가 1개로 통과**한다. 읽기 실패는 판정 불능이지 후보 부재가 아니다.
///
/// ★`cargo:warning` 이 아니라 panic 인 것은 의도다★ — 리소스가 아예 없다는 건 tauri_build 가 `[[bin]]`
/// 에도 못 넘겼다는 뜻이라 **릴리즈 바이너리도 manifest 없이** 나간다. 조용히 성공하면 안 되는 빌드다.
/// 게다가 여기서의 조용한 누락은 몇 달짜리 잠복 부채가 된다(이 파일이 고치는 원래 결함이 정확히 그렇게
/// 살아남았다). build script panic 은 시끄럽고 즉시이며, CI 는 push 마다 이 패키지를 빌드한다.
fn emit_resource_link_arg_for_tests() {
    // 타깃 OS 판정. cargo 가 build script 에 넘기는 값이라 크로스 빌드에서도 정확하다.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let out_dir = std::env::var("OUT_DIR")
        .expect("OUT_DIR 부재 — cargo 가 늘 주는 값이라 여기 닿으면 빌드 환경 자체가 깨진 것이다");

    let entries =
        std::fs::read_dir(&out_dir).unwrap_or_else(|e| panic!("OUT_DIR({out_dir}) 열기 실패: {e}"));

    // 1단계 = 이름으로 훑는다.
    // 정규 파일만 후보로 센다 — 이름이 `.lib`/`.a` 로 끝나는 디렉터리를 링커에 넘기면 진단이 엉뚱해진다.
    // ★`.lib` 와 `.a` 를 함께 받는다★ — embed-resource 3.0.9 는 같은 버전에서도 msvc 면 `resource.lib`,
    //   `*-pc-windows-gnu` 면 `libresource.a` 로 이름을 낸다. gnu 도 지원 구성이라(tauri-build 2.6.3 이
    //   `CARGO_CFG_TARGET_ENV == "gnu"` 를 따로 분기한다) `.lib` 만 보면 그 타깃에서 후보가 0개가 되고,
    //   아래 panic 이 "리소스가 아예 안 만들어졌다"고 **틀린 단정**을 하며 빌드를 세운다.
    let mut scanned: Vec<std::path::PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.path())
        .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("lib" | "a")))
        .collect();
    // read_dir 순서는 미규정 — 정렬은 아래 panic 의 **나열 순서**를 고정할 뿐이다(선택과는 무관:
    //   정확히 1개일 때만 통과시키므로 고를 것 자체가 없다).
    scanned.sort();

    // 2단계 = 내용으로 거른다. 여기서 걸러 내는 대표 사례가 `msvcrt.lib` stub 이다(머리 주석 참조).
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for path in scanned {
        let bytes = std::fs::read(&path).unwrap_or_else(|e| {
            panic!(
                "OUT_DIR({out_dir}) 의 후보 파일({p}) 읽기 실패: {e} — 읽지 못한 후보를 조용히 \
                 버리지 않는다. 버리면 그 파일이 처음부터 없었던 것이 되어 아래 개수 검사의 \
                 0개·2개 갈래가 함께 무력해진다(못 읽은 것이 진짜 리소스였다면 남은 stale 하나가 \
                 1개로 통과한다). 읽기 실패는 판정 불능이지 후보 부재가 아니므로 여기서 세운다.",
                p = path.display()
            )
        });
        if carries_resource_data(&bytes) {
            candidates.push(path);
        } else {
            skipped.push(path.display().to_string());
        }
    }

    let chosen = match candidates.len() {
        1 => candidates.remove(0),
        n => {
            let listed = candidates
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            let skipped = skipped.join(", ");
            panic!(
                "OUT_DIR({out_dir}) 에서 manifest 리소스(.lib/.a)를 하나로 특정하지 못했다 — \
                 후보 {n}개: [{listed}] (내용 검사에서 리소스 아님으로 걸러낸 것: [{skipped}]). \
                 0개면 tauri_build 가 리소스를 만들지 못한 것이라 `[[bin]]` 도 manifest 없이 \
                 링크된다(= 릴리즈 바이너리까지 manifest 없음). 2개 이상이면 stale + 신규가 함께 \
                 링크돼 CVT1100(duplicate resource type:VERSION) → LNK1123 으로 이 패키지 테스트 \
                 타깃 5개가 전부 깨진다. 어느 쪽이든 조용히 성공하면 안 되는 빌드다. \
                 ★gnu 타깃에서 0개가 났다면 `carries_resource_data` 의 `.rsrc` 갈래를 넓힐 \
                 자리이지 내용 검사를 걷어낼 자리가 아니다★ — 걷어내면 msvcrt stub 이 도로 \
                 후보가 되어 릴리즈가 다시 선다."
            )
        }
    };

    println!("cargo:rustc-link-arg-tests={}", chosen.display());
}

/// 후보 파일의 **바이트**가 실제 리소스 산출물인지 가른다 — 이름·확장자는 보지 않는다.
///
/// 받는 형태가 둘이고 **검증 상태가 서로 다르다**.
///
/// ① **Win32 `.res` 파일** — msvc 경로의 실물이고 이 워크트리에서 실측했다(2026-09-22:
///    `resource.lib` 13,656바이트가 이 형태, 같은 폴더의 `msvcrt.lib` 82바이트는 아님). `.res` 는
///    선두가 반드시 "널 리소스 항목"으로 시작한다 — DataSize=0, HeaderSize=32, Type=ordinal 0,
///    Name=ordinal 0. link.exe 가 32비트 `.res` 를 알아보는 표식이 그것이라 산출 도구가 임의로 바꿀
///    수 있는 값이 아니다.
/// ② **COFF `.rsrc` 섹션 이름** — 오브젝트나 아카이브가 리소스를 담으면 섹션 이름이 바이트로 박힌다.
///    gnu 의 `libresource.a`(windres → ar)와, 훗날 업스트림이 cvtres 를 거친 진짜 COFF lib 을 내는
///    경우가 여기 든다. ★**미검증**★ — 이 워크트리에 gnu 산출물이 없어 실측하지 못했다(2026-09-22).
///    막는 쪽이 아니라 **넓히는 쪽** 갈래라 틀려도 오검출이 아니라 "0개" panic 으로 시끄럽게 드러난다.
fn carries_resource_data(bytes: &[u8]) -> bool {
    // `.res` 널 헤더의 고정 선두 16바이트.
    const RES_NULL_HEADER: [u8; 16] = [
        0x00, 0x00, 0x00, 0x00, // DataSize = 0
        0x20, 0x00, 0x00, 0x00, // HeaderSize = 32
        0xFF, 0xFF, 0x00, 0x00, // Type = ordinal 0
        0xFF, 0xFF, 0x00, 0x00, // Name = ordinal 0
    ];
    const COFF_RSRC_SECTION: &[u8] = b".rsrc";

    bytes.starts_with(&RES_NULL_HEADER)
        || bytes
            .windows(COFF_RSRC_SECTION.len())
            .any(|w| w == COFF_RSRC_SECTION)
}
