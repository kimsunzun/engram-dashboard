# ★ 에이전트의 라이브러리 파일 조작 — 「경로가 바뀌면 명령, 아니면 직접 + refresh」 선례 — light 조사

| 항목 | 값 |
|---|---|
| 상태 | 조사 완료 · 결정 대기(사용자) |
| 날짜 | 2026-10-17 |
| 방법 | `/research` light — 수집자 1명(검색 · 가져오기 약 10회) · 메인 grounding 스팟체크 2건(Godot 원문 · Obsidian 포럼 WebFetch) · 적대 리뷰 없음 |
| 질문 | 에이전트에게 「만들기 · 내용 고치기 = 파일 직접 쓰고 `library.refresh`(오류를 돌려줌) · 옮기기 · 이름 바꾸기 · 지우기 = `library.move` · `rename` · `delete` 명령(참조를 같은 조작 안에서 고침)」 을 시키는 규칙 — 같은 선례가 있나, 없다면 가장 가까운 꼴은 |

## 결론

- **「경로가 바뀌는 조작은 앱 명령으로」 쪽은 흔하다.** 참조를 고쳐 주는 move/rename 을 도구로 내놓는 곳이 여럿이고(Unity MCP `assets-move` · Unreal MCP `rename_asset` · Obsidian MCP `move_note` · TS 언어 서버를 여는 `ts-refactor-mcp` · Dendron · Foam), 쓰는 이유는 하나같이 「셸 `mv` 는 참조를 끊는다」 다. (가능성 높음 — 대부분 MCP 목록 · README 수준이고 코드는 안 읽었다)
- **두 쪽을 묶은 우리 규칙(내용은 직접 + 명시적 refresh 가 오류를 돌려줌, 경로 조작은 명령 강제) 을 문서로 박은 곳은 못 찾았다.** (불확실 — 검색 약 10회, 전수 아님) 가장 가까운 꼴은 Unity MCP `assets-move` — 명령이 옮기고 항목별 오류를 모은 뒤 `AssetDatabase.Refresh` 를 부른다. 내용은 디스크에 직접 쓰고 refresh/import 가 따라 읽는 것도 Unity 의 평소 꼴이다.
- **어느 곳도 직접 옮기기를 「금지」 하지는 않는다.** 도구를 고르는 것은 권고 수준이고, 밖에서 옮긴 것은 신원 사이드카(Godot `.uid` · Unity `.meta` GUID)나 정리 패스(Unreal redirector)로 되살린다. Obsidian 은 밖에서 옮기면 링크가 깨지는 것을 받아들인다. 내용으로 다시 찾는 곳은 못 봤다.

## 발견

| 도구 | 직접 하는 것 | 명령 · API 로 하는 것 | 밖에서 옮김 · 이름 바꿈 | 출처 | 확신 |
|---|---|---|---|---|---|
| Obsidian | 노트 내용 | 앱 안 옮기기만 링크를 고친다 · 서드파티 MCP `move_note`(링크 갱신 표방) | **링크가 깨진다** — 「The links only update if the file is moved within Obsidian」 · 스크립트용 옮기기 명령은 기능 요청으로 남았다 | https://forum.obsidian.md/t/move-vault-files-by-obsidian-command-line-option-or-url-to-allow-scriptable-link-updates/6776 (메인 확인) · https://glama.ai/mcp/servers/diegorv/another-bloated-obsidian-mcp-server/tools/move_note | 깨짐 = 가능성 높음(메인 확인) · MCP = 불확실(목록 수준) |
| Unity | 내용은 디스크에 직접 | `AssetDatabase.MoveAsset` · Unity MCP `assets-move`(쌍마다 MoveAsset · 항목별 오류 · 끝에 Refresh) | `.meta` GUID 사이드카 — 함께 옮기면 참조 유지 | https://docs.unity3d.com/460/Documentation/ScriptReference/AssetDatabase.MoveAsset.html · https://glama.ai/mcp/servers/butterlatte-zhang/unity-ai-bridge/tools/assets-move | MCP = 가능성 높음 · 사이드카 = 불확실(기억 — 원문 안 읽음) |
| Unreal | 편집기 안 | 이름 바꾸기 · 옮기기 → 옛 자리에 Redirector · Fixup / `-fixupredirects` 로 참조를 다시 저장 · Unreal MCP `rename_asset`(참조 갱신 표방) | 탐색기에서 옮기기는 권하지 않음(포럼 — 답은 안 읽음) | https://dev.epicgames.com/documentation/unreal-engine/asset-redirectors-in-unreal-engine · https://glama.ai/mcp/servers/aadeshrao123/Unreal-MCP/tools/rename_asset | redirector = 가능성 높음 · 밖 옮기기 = 불확실 |
| Godot 4.4+ | 텍스트 파일 직접 | 편집기 FileSystem 독에서 옮기면 참조를 고친다 | **`.uid` 사이드카** — 「when you move a script or shader outside Godot, you should also move the `.uid` file along with it」 · 경로도 함께 저장해 경로가 그대로면 그것으로 푼다(fallback) | https://godotengine.org/article/uid-changes-coming-to-godot-4-4/ (메인 확인) | 확실에 가까움 — 1차 출처 · 메인 원문 확인(단일 출처라 태그는 가능성 높음) |
| VS Code · 에이전트 도구 | 에이전트가 흔히 셸 `mv` | 서드파티 `ts-refactor-mcp`(언어 서버의 「파일 옮기면 import 고치기」 를 에이전트에 연다) · `cclsp`(`rename_symbol`) · Cursor 포럼에 「VS Code 내장 rename 을 열어 달라」 요청 | `mv` 가 import 를 깨는 것이 문제의 출발 | https://cdn.jsdelivr.net/npm/ts-refactor-mcp@0.1.2/README.md · https://forum.cursor.com/t/executing-vscodes-built-in-file-rename-functionality/129697 | MCP = 가능성 높음 · 「1차 도구 없음」 = 불확실(못 찾음 = 없음 아님) |
| Foam · Dendron | 노트 내용 | Foam `renameNote` · Dendron Rename Note / Refactor Hierarchy — 백링크 갱신 | 안 다룸(읽은 범위) | https://wiki.dendron.so/notes/2gHesjl7XP5Q9QNJWRv7E | 가능성 높음 |
| Logseq | — | 앱 안 이름 바꾸기만 | 미확인 | https://discuss.logseq.com/t/is-it-possible-to-change-assets-name-without-breaking-the-existing-notes/6207 | 불확실 |
| JetBrains | — | — | — | — | 기권(안 봄) |

## grounding

- Godot `.uid` — 원문 세 문장 확인(생성 · 함께 옮기기 · 경로 fallback). **지지.**
- Obsidian — 포럼 원문: 밖에서 옮기면 링크가 깨지고 앱 안 옮기기만 고친다 · 스크립트용 옮기기 명령은 요청으로 남았다. **지지.** ★수집자가 이 스레드를 「Obsidian CLI `move` 가 링크를 고친다」 의 출처로 단 것은 **미지지** — 스레드는 그런 명령이 없다고 말한다. 공식 CLI 의 현재 기능은 확인하지 않았다(강등).

## 우리 설계에 주는 것

- 경로 조작을 명령으로 받는 쪽(PRD 의 `library.move` · `rename` · `delete`)은 업계가 같은 이유로 고른 꼴이다.
- 「내용은 직접 + refresh 가 오류를 돌려준다」 는 우리가 더하는 부분이다. Unity 의 MoveAsset → Refresh 흐름이 가장 가깝다.
- 수집자의 추론(출처 없음): Godot 처럼 **경로와 함께 신원을 저장**해 두면 밖에서 옮긴 것도 `library.refresh` 때 되살릴 수 있다. 우리 「다시 잇기」(같은 종류 · 같은 이름 · 정확히 하나)가 그 약한 판이다.

## 공백

- JetBrains 안 봄 · Claude Code / Copilot / Cursor 의 내장 rename 동작은 1차 문서로 확인 안 함 · LSP `workspace/willRenameFiles` 규격은 안 가져옴.
- MCP 서버 능력은 목록(glama.ai) · README 수준이고 코드를 읽지 않았다.
