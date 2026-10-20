# 묶음 파일이 같이 든 파일을 가리키는 기준 — 파일 기준 · 루트 기준 · ID

- **상태:** light · 프리셋이 에셋(스킬 · 지침 · 설정)을 가리키는 경로의 기준을 고르는 데 쓴다 · **결정(사용자 2026-10-15) = 절대경로 — 사용자가 라이브러리로 지정한 폴더 기준 · 범위 제한 없음**
- **날짜:** 2026-10-14
- **방법:** research light. 수집자 1 · 메인 grounding(Claude Code 플러그인 · Docker Compose · Cargo 원문 확인) · 적대 리뷰 없음.
- **계기:** 프리셋은 보통 스킬과 한 세트 폴더로 주고받는다(결정 메모). 받은 세트 폴더를 내 라이브러리의 하위 폴더로 넣어도 프리셋이 같이 받은 스킬을 찾아야 한다.

## 결론

1. **같이 실어 보내는 묶음은 「묶음 자리 기준 상대 경로」가 표준이다.**
   - **Claude Code 플러그인** — "Every component path in a manifest is relative to the plugin root and must start with `./`." · "Every component path must resolve inside the plugin root and must exist." 루트 밖으로 나가는 경로(`..`)는 거부(`path escapes plugin directory`). 기준은 매니페스트 파일 자신의 폴더가 아니라 **플러그인 루트**(`.claude-plugin/` 를 품은 폴더)다. **가능성 높음**(공식 문서 원문 — 메인 확인).
   - **Docker Compose `include`** — "Relative paths in Compose files being referred by `include` are resolved relative to their own Compose file path, not based on the local project's directory." **가능성 높음**(공식 문서 원문 — 메인 확인).
   - **Cargo path 의존** — "found in the `hello_utils` folder, relative to the `Cargo.toml` file it's written in." 경로 의존은 상대 경로뿐이라 하위 프로젝트를 옮기기 어렵다는 이유로 RFC 3529(경로 기준 이름)가 나왔다(수집자). **가능성 높음**(공식 문서 원문 — 메인 확인).
   - 공통: 묶음 폴더를 통째로 어디에 두든 안쪽 참조가 이어진다. 묶음 밖의 파일 하나를 따로 옮기면 끊어진다.
2. **프로젝트 전체 에셋 DB 를 가진 엔진은 루트 경로나 ID 를 쓴다.**
   - **Godot** — `res://`(프로젝트 루트 기준) + `uid://`(ID · 경로는 예비). 에디터 안에서 옮기면 참조가 따라가고, 바깥 파일 관리자로 옮겨 `.uid` 를 두고 가면 끊어진다. **불확실**(수집자 · 일부 서드파티 글).
   - **Unity** — `.meta` 의 GUID 로만 가리킨다. `.meta` 를 잃으면 GUID 가 새로 생겨 참조가 전부 끊어진다. 폴더째 다른 프로젝트로 가져가도 `.meta` 가 따라오면 이어진다. **불확실**(수집자 · 옛 매뉴얼).
   - 루트 경로(`res://`)는 묶음 폴더가 다른 자리에 놓이면 맞지 않는다.

## Engram 에 옮기면 (세션 해석 — 결정은 사용자)

- 세트 폴더를 주고받는 흐름은 플러그인 · Compose · Cargo 쪽(묶음 자리 기준)과 같다.
- ★**Engram 고유 사정:** 플러그인은 루트 밖 참조를 금지하지만, Engram 프리셋은 세트 안 스킬과 **라이브러리 공용 스킬을 함께** 가리킬 수 있다. 파일 기준 하나로만 적으면 공용 스킬이 `../../skills/review` 꼴이 되고, 세트를 옮기면 그 줄이 깨진다.
- ID 방식(Unity · Godot)은 파일마다 신원 파일을 붙여야 하고 바깥에서 옮기면 잘 끊어진다 — Engram 은 바깥(탐색기 · 에이전트 셸)에서 옮기는 것이 정해진 길이라 맞지 않는다(해석).

## 한계

- light 라 적대 리뷰 없음. npm `file:` · VS Code 확장 · Unreal 경로 · HTML 의 `/` 루트 경로는 수집하지 못했다.
- Godot · Unity 는 1차 자료를 메인이 확인하지 않았다.

## 추가 (2026-10-15) — 가벼운 도구만 다시 보기 · 엔진 제외

사용자 지적: 엔진(Unity · Godot · Unreal)은 레퍼런스 예외가 무거워 비교 대상이 아니다. 같은 부류(에이전트 · 프롬프트 설정 묶음)만 다시 봤다(수집자 1 · 메인 원문 확인 둘).

- **공용 항목은 경로가 아니라 이름으로 가리키는 것이 주류다.** 경로는 묶음 자기 안의 파일에만 쓴다.
  - **Claude Code 플러그인 의존** — `"dependencies": ["audit-logger", { "name": "secrets-vault", "version": "~2.1.0" }]` · 이름만 쓰면 같은 마켓플레이스에서 찾고 `name@marketplace` 로 다른 곳을 지목한다. ★「Bundle plugins for a team」 = 이름과 의존 목록만 든 플러그인 — 우리 프리셋과 같은 모양★. 없으면: 설치 · `/reload-plugins` · 자동 갱신이 빠진 의존을 설치하고, 못 채우면 그 플러그인이 로드되지 않고 오류를 띄운다(`Requires "secrets-vault@your-marketplace" ~2.1.0, installed 3.0.0`). **가능성 높음**(공식 문서 원문 — 메인 확인).
  - **Continue.dev** — `uses: sanity/sanity-opinionated`(허브 `owner/name`) · 로컬은 `uses: file://…`. 없을 때 동작은 문서에 없다. **가능성 높음**(공식 문서 — 메인 확인).
  - **Goose 레시피** — 하위 레시피는 `path:`(레시피 파일 기준 상대 · 또는 절대) · 공유 = 파일 전달 · 레시피 폴더 · GitHub 저장소 · 딥링크. **불확실**(요약기 경유).
  - **VS Code 프로필** — 확장을 확장 ID 로 · 내보내기/가져오기(파일 · gist). **불확실**(검색 결과만).
- **엔진(참고만):** Unity 패키지 내보내기(의존 포함) · Unreal Migrate(의존 복사 · Content 폴더 필수) · Godot 은 내보내기 없이 복사 + `res://addons/<이름>/` 같은 고정 자리 관례. 셋 다 받는 쪽에서 경로를 고쳐 쓰지 않는다(수집자 · 일부 공식 문서).
- **Engram 에 옮기면(해석):** 경로(절대 · 상대) 말고 **이름 참조**가 셋째 선택지다. 라이브러리 안 어디로 옮겨도 안 끊어지고(옮기기 기능 · 다시 잇기 부담이 줄어든다), 남에게 주면 받는 쪽에 같은 이름이 있으면 이어지고 없으면 「찾을 수 없음」. 대가 = 라이브러리 안에서 이름이 겹치면 안 된다(겹침 규칙 또는 이름 앞 묶음 이름 `owner/name` 같은 구분이 필요).

## 추가 2 (2026-10-15) — 가벼운 브라우저형 도구 · 플러그인 · 엔진 제외

사용자 지적: 플러그인은 이름 범위가 명확해서, 엔진은 무거운 ID 체계라서 비교 대상이 아니다 — 사용자가 관리하는 폴더 안에서 파일끼리 가리키는 가벼운 도구를 보라(수집자 1 · 메인 원문 확인 1 · REAPER 위키는 JS 검증 페이지라 못 읽음).

- **노트 앱** — **Obsidian**: `[[Three laws of motion]]`(이름) · 폴더를 붙이면 "Folder paths start at the vault root and use forward slashes" → **볼트 맨 위 기준 경로**. "Obsidian can automatically update internal links in your vault when you rename a file."(설정에서 끌 수 있다). 바깥에서 옮긴 경우는 도움말에 없다. **가능성 높음**(공식 도움말 원문 — 메인 확인). 「새 링크 형식(가장 짧은 경로 · 상대 · 절대)」 설정은 미확인. **Foam · Dendron** — 이름 링크 · 앱 안 이름 바꾸기가 링크를 고친다(**불확실** — 검색 조각).
- **미디어 도구** — **REAPER**: 프로젝트 폴더 안 파일은 상대, 밖은 전체 경로 · 저장 때 「모든 미디어를 프로젝트 폴더로 옮기기」 · 못 찾으면 불러올 때 묻는다(**불확실** — 검색 요약). **Ableton Live**: 「Collect All and Save」 = 쓰는 샘플을 세트 폴더로 모으기 · 없는 샘플은 검색 · 수동 지정(**불확실** — 포럼). **Lightroom Classic**: 원본 위치를 경로로 저장 · 앱 안 정리는 카탈로그를 고친다 · 바깥에서 옮기면 폴더에 「?」 → 「Find Missing Folder」로 새 자리 지정(**불확실** — 서드파티 블로그). **M3U 재생목록**: 상대 항목은 그 파일 기준 · 절대 항목은 폴더 · 드라이브가 바뀌면 깨진다(**불확실**).
- **공통 모양(해석):** ① 앱 안에서 옮기면 앱이 참조를 고친다 ② 바깥에서 옮기면 「없음」 표시 + 사람이 다시 지정 · 검색(이름 자동 맞춤의 문서는 못 찾음) ③ 남에게 줄 땐 폴더째 복사, 또는 「쓰는 것 모아 담기」(Ableton · REAPER).
- **Engram 에 옮기면(해석):** 라이브러리 맨 위 기준 경로(Obsidian 의 볼트 맨 위 경로 · Lightroom) + 앱 안 옮기기가 고침 + 바깥 옮김은 「찾을 수 없음 · 다시 잇기」 + 줄 때 모아 담기(나중) — 이 부류의 공통 모양과 맞는다. Engram 의 「이름으로 자동 다시 잇기」는 이 부류보다 한 걸음 더 간 것이다.
