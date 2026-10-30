# 다중 라이브러리 루트 관행 — 여러 출처의 자산 묶음을 어떻게 담고 조합하나 (2026-10-30)

- **상태:** `/research` light — 수집자 1 · 메인 grounding 스팟체크(Claude Code 스킬 문서 · Obsidian 포럼 직접 확인) · 적대 리뷰 없음(light). 결정 대기 — S25 결정 메모 G25(다중 루트) 의 「루트 간 참조 허용?」.
- **쓰임:** S25 asset library 를 1안(다중 루트 + 루트 간 참조 허용 · 깨지면 「찾을 수 없음 · 다시 연결」) 과 2안(루트 하나 + 그 아래에 묶음 clone) 중 어느 쪽으로 갈지.
- **확신도 범례:** 확실 = 1차 문서 직접 확인 · 가능성 높음 = 1차 문서 요약본 · 검색 발췌 · 포럼 · 모름 = 못 찾음.

## 결론

1. **루트 여럿 등록이 관행이다.** 조사한 피어 다섯 중 넷(Claude Code · codex · Blender · Ableton)이 루트를 여러 개 받는다. 한 폴더 아래로 모으라고 강제하는 곳은 없다. 예외 Obsidian 은 보관함마다 루트 하나이고 보관함끼리 일부러 떼어 놓는다.
2. **이름이 겹치는 문제는 이름 앞 접두로 푼다.** Claude Code = 플러그인 스킬은 항상 `/플러그인:스킬` · 추가 폴더 스킬은 겹칠 때만 `/폴더:스킬`. 경로로 찾는 도구(Blender · Ableton)는 「어느 라이브러리의 어느 경로」라 겹침 문제가 없다.
3. **루트를 넘는 참조는 경로 기반 도구(Blender · Ableton)만 한다** — 그리고 둘 다 **깨짐 복구 도구(다시 연결)** 와 **「프로젝트 안으로 복사해 모으기」 탈출구**를 같이 둔다. AI 스킬 시스템(Claude Code · codex)은 이름으로 다른 루트를 가리키는 장치가 문서에 없다(정션 · 심볼릭 링크로 우회).
4. **받은 묶음은 「따로 둔 사본 + 판 단위 갱신」이 흔하다.** Claude Code 플러그인 = 캐시에 사본 · 판이 바뀔 때만 새 사본. 사용자가 그 사본을 고치면 어떻게 되는지는 문서에 없다(모름).

## 피어별

| 피어 | 루트 여럿? | 루트 넘는 참조 | 루트 빠지면 | 받은 묶음 갱신 · 사용자 수정 |
|---|---|---|---|---|
| **Claude Code 스킬 · 플러그인** | 예 — 기업 · 개인 `~/.claude/skills` · 프로젝트 · 하위 폴더 · `--add-dir` 추가 폴더 · 플러그인 · claude.ai 계정 (확실) | 이름으로는 없음(모름). 마켓플레이스 안 플러그인끼리 심볼릭 링크 → 설치 때 내용 복사, 마켓 밖을 가리키면 건너뜀 (확실) | 마켓 제거 = 그 플러그인 제거. 목록에서 빠진 플러그인은 남고 「not found」 표시 (확실) · 스킬 폴더 루트 삭제 = 모름 | 캐시 사본 · 판 바뀔 때만 갱신 · 자동 갱신 기본 꺼짐 · 로컬 경로 마켓은 실파일 직접 (확실) · 캐시 수정 보존 = 모름 |
| **codex CLI 스킬** | 예 — 저장소 `.agents/skills` · 사용자 `~/.agents/skills` · 관리자 · 내장 (가능성 높음) | 모름 · 심볼릭 링크는 따라감 | 모름 | 모름 · 같은 이름은 합치지 않고 둘 다 보임 |
| **Blender 에셋 라이브러리** | 예 — 환경설정에 이름 + 경로 목록 · 라이브러리마다 가져오기 방식(Link = 읽기 전용 참조 · Append = 복사) (확실) | 예 — Link 가 다른 `.blend` 를 참조하고 원본 변경이 따라옴 (확실) | 자리표시 데이터로 참조를 잃지 않고 「Library Relocate」 로 새 경로에 다시 잇는다 (가능성 높음 — 옛 판 문서 발췌) | Link = 원본에서 고침 · Append = 내 사본(원본 안 따라감) (확실) |
| **Ableton Live** | 예 — 브라우저 「Places」: User Library · Packs · 사용자 폴더 (가능성 높음) | 예 — 세트가 바깥 파일을 경로로 참조 · 「Collect All and Save」 가 프로젝트 안으로 복사 (확실) | 상태 줄 경고 + File Manager 에서 손으로 찾기 · 자동 검색 (확실) | 모름 |
| **Obsidian** | 보관함마다 루트 하나 · 여러 보관함은 서로 격리 | 위키링크 · 백링크는 보관함을 못 넘음 — 기능 요청으로만 남음 · 우회 = `obsidian://` URI · 상위 폴더를 보관함으로 열기 (가능성 높음 — 포럼) | 모름 | 해당 없음 |

출처:
- Claude Code 스킬 위치 · 같은 이름 우선순위 · 접두 — https://code.claude.com/docs/en/skills (메인 직접 확인 2026-10-30)
- Claude Code 플러그인 · 마켓 — https://code.claude.com/docs/en/plugin-marketplaces · https://code.claude.com/docs/en/plugins/host-marketplace.md
- codex 스킬 — https://learn.chatgpt.com/docs/build-skills (developers.openai.com/codex/skills 에서 넘어감 · 요약본으로 읽음)
- Blender — https://docs.blender.org/manual/en/latest/editors/preferences/file_paths.html (본문이 얇아 4.x 문서 사본 검색으로 보강)
- Ableton — https://www.ableton.com/en/manual/managing-files-and-sets/
- Obsidian — https://forum.obsidian.md/t/support-for-linking-between-separate-vaults/100628 (메인 직접 확인 — 「Feature archive」 분류의 기능 요청 · 답글이 URI 를 권함)

## 우리에게 — 1안 vs 2안

| | 1안 다중 루트 + 루트 간 참조 허용 | 2안 루트 하나 + 아래에 묶음 clone |
|---|---|---|
| 피어 근거 | 경로 기반 도구(Blender · Ableton)와 같은 꼴 — 둘 다 다시 연결 · 복사 탈출구를 함께 둔다. **우리도 둘 다 이미 있다**(「찾을 수 없음 · 다시 연결」 R19 · 복사 P6) | Obsidian 의 우회(상위 폴더를 하나로 열기)와 같은 꼴 |
| 제자리 폴더 | 됨 — `~/.claude/skills` · 다른 드라이브 · 공유 폴더를 옮기지 않고 등록 | 안 됨 — 루트 안으로 옮기거나 clone 해야 한다 |
| 비용 | 경로 앞 루트 이름(피어 관행 = 접두) · 이동 기록 · 휴지통 루트마다 · 루트 간 옮기기 규칙 | 0 — 지금 설계(R7) 그대로 |
| 깨짐 | 루트를 빼면 다른 루트의 참조가 「찾을 수 없음」 — 사용자 몫 | 루트 안에서만 생긴다 |

## 공백 · 확인 못 한 것

- Blender · Ableton 공식 문서는 요약본 · 검색 발췌로만 읽었다(2차).
- VS Code · npm workspaces 는 다루지 못했다.
- Claude Code: 캐시 플러그인을 고친 것이 갱신에 살아남는지 · 스킬 루트를 지웠을 때 경고가 나는지 — 모름.
- codex: 같은 이름 우선순위 · 갱신 · 제거 동작 — 모름.
- 이름으로 다른 루트를 가리키는 AI 스킬 피어는 못 찾았다(Claude Code 플러그인 접두 · 링크 복사 규칙이 가장 가깝다).
