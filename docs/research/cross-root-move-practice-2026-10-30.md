# 루트 사이 옮기기 관행 — 옮기기냐 복사냐 · 참조가 따라오나 (2026-10-30)

- **상태:** `/research` light — 수집자 1 · 메인 grounding 스팟체크(Eagle 지원 문서 직접 확인) · 적대 리뷰 없음(light). 결정 대기 — S25 결정 메모 G25 뒤 「루트 사이 파일 옮기기」.
- **쓰임:** S25 asset library 의 다중 루트(G25)에서 루트 A → 루트 B 로 파일을 옮길 때 (가) 복사만 · (나) 옮기기도 허용(참조 따라옴) 중 어느 쪽으로 갈지.
- **확신도 범례:** 확실 = 1차 문서 직접 확인 · 가능성 높음 = 1차 문서 요약 · 커뮤니티 글 · 모름 = 못 찾음.

## 결론

1. **라이브러리를 여럿 두는 자산 관리 도구는 라이브러리 사이를 복사만 한다.** Eagle 은 「다른 라이브러리에 추가」 가 복사만 지원한다고 지원 문서가 직접 말한다(확실 — 이유는 안 적었다). Blender 도 문서에 나온 길은 「Copy Asset Bundle」(복사) 하나다(가능성 높음). 원본은 사용자가 손으로 지운다.
2. **카탈로그 도구(Lightroom Classic)는 드라이브를 넘는 옮기기를 허용하되, 도구 안에서 옮길 때만 참조가 따라온다.** 도구 밖(탐색기)에서 옮기면 「?」 로 잃고 「Find/Update Folder Location」 으로 다시 잇는다(가능성 높음).
3. **Windows 탐색기는 같은 드라이브 = 옮기기, 다른 드라이브 = 복사가 기본이다.** Shift = 강제 옮기기 · Ctrl = 강제 복사(가능성 높음 — Microsoft 블로그 · Q&A 요약으로 확인, 널리 알려진 동작).
4. **드라이브를 넘는 옮기기가 중간에 실패했을 때의 처리를 적어 둔 피어는 없다.** 복사 후 삭제라는 일반 방식만 짐작된다.

## 피어별

| 피어 | 루트 넘는 옮기기 | 참조가 따라오나 | 드라이브 넘는 실패 처리 |
|---|---|---|---|
| **Eagle**(라이브러리 여럿) | 복사만 — 「Add to Other Library」 (확실) · 원본은 손으로 삭제 | 항목 고유 id 가 같으면 같은 파일로 보고 다시 들이지 않음 · 그 밖 모름 | 모름 |
| **EagleFiler** | 끌어다 놓으면 복사 · 옮기기는 계획 중 (가능성 높음 — 포럼) | 모름 | 모름 |
| **Blender 에셋 라이브러리** | 문서상 복사만 — 「Copy Asset Bundle」 (가능성 높음) | 모름 | 모름 |
| **Lightroom Classic** | 허용 — 도구 안 폴더 패널에서 드라이브 넘어도 (가능성 높음) | 도구 안에서 옮기면 따라옴 · 밖에서 옮기면 잃고 다시 잇기 (가능성 높음) | 모름 |
| **Windows 탐색기** | 같은 드라이브 = 옮기기 · 다른 드라이브 = 복사 기본 · Shift/Ctrl 로 바꿈 (가능성 높음) | 모름 | 모름 |
| **VS Code 다중 루트** | 루트 사이 옮기기 가능하다고만 적힘 · 기본이 옮기기인지 복사인지 못 찾음 (불확실) | 언어 확장의 이름 바꾸기 처리에 달림 · 확인 안 함 | 모름 |
| **Obsidian** | 기본 기능 없음 · 커뮤니티 플러그인(Trans Vault = 복사 · 옮기기 둘 다) (가능성 높음) | 보관함 안에서만 자동 갱신 · 플러그인은 양끝을 함께 옮길 때만 고침 (가능성 높음) | 모름 |

출처:
- Eagle — https://eagle.cool/support/article/why-is-there-no-move-option-when-adding-files-to-other-libraries (메인 직접 확인 2026-10-30: 「currently only supports copying files and does not provide a move file option」) · https://en.eagle.cool/article/1304-adding-images-to-other-libraries
- EagleFiler — https://forum.c-command.com/t/moving-not-copying-items-between-libraries/11852
- Blender — https://docs.blender.org/manual/nb/latest/editors/asset_browser.html (검색 요약으로 읽음)
- Lightroom Classic — https://lightroomkillertips.com/?p=14231 · https://www.lightroomqueen.com/find-move-rename-catalog/ (Adobe 문서 아님)
- Windows 탐색기 — https://devblogs.microsoft.com/oldnewthing/?p=37323 · https://learn.microsoft.com/en-us/answers/a/2790390 (요약으로 읽음)
- VS Code — https://code.visualstudio.com/docs/editing/workspaces/multi-root-workspaces
- Obsidian — https://community.obsidian.md/plugins/transvault · https://community.obsidian.md/plugins/vault-transfer · https://forum.obsidian.md/t/moving-notes-from-one-vault-to-another/2040

## 우리에게

| | (가) 루트 사이 = 복사만 | (나) 옮기기 허용 · 참조 따라옴 | (다) 탐색기 꼴 — 같은 드라이브 = 옮기기 · 다른 드라이브 = 복사 · Shift = 옮기기 |
|---|---|---|---|
| 피어 | Eagle · Blender(직접 피어 — 라이브러리 여럿) | Lightroom(카탈로그) | Windows 탐색기 — P5 가 옮기기 · 충돌을 탐색기 규칙으로 정했다 |
| 비용 | 0 — 복사(P6) 그대로 | 드라이브 넘는 옮기기 = 복사 후 삭제를 저널로 · 실패 처리 선례 없음 | (나) 의 비용을 그대로 진다(Shift 로 옮기기가 열린다) |
| 참조 | 원본을 지우면 「찾을 수 없음 · 다시 연결」 — 사용자 몫 | 따라온다 | 옮기기면 따라오고 복사면 원본 그대로 |

## 공백 · 확인 못 한 것

- 드라이브를 넘는 옮기기의 중간 실패 처리는 어느 피어도 문서에 없다.
- Adobe 공식 문서 · Blender 에셋 브라우저 문서 전문 · VS Code 이슈 트래커는 읽지 않았다. VS Code 기본 끌기 동작은 실측하지 않았다.
- Adobe Bridge · Unity/Unreal 콘텐츠 브라우저 · Steam 「설치 폴더 옮기기」(드라이브 넘는 옮기기 피어)는 다루지 못했다.
