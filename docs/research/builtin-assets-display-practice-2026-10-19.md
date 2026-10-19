# 기본 제공(읽기 전용) 에셋을 브라우저에 어떻게 보이나 (2026-10-19)

| 항목 | 값 |
|---|---|
| **상태** | 조사 완료 · 결정 대기(라이브러리 왼쪽 칸에 「Engram 기본 제공」 을 어떻게 둘지) |
| **방법** | `/research` light — 수집자 1(검색 + 공식 문서 읽음) + 메인 grounding(Epic 문서 원문 확인) |
| **질문** | 앱이 싣는 읽기 전용 에셋을 사용자 콘텐츠와 같이 보일 때 ① 기본은 숨김인가 ② 따로 된 뿌리인가 · 위/아래 ③ 읽기 전용 다루기 ④ 몇 개 안 될 때도 트리에 따로 두나 |

## 결론

1. **따로 된 뿌리 · 칸으로 둔다 — 사용자 폴더에 섞지 않는다**(가능성 높음): Unreal = Sources 패널의 `Engine Content` 폴더 · Unity = Packages 뿌리 · Blender = 라이브러리 드롭다운의 「Essentials」 · JetBrains = 설정 목록의 번들 템플릿 묶음.
2. **보임은 갈린다**: Unreal 은 보기 옵션 「Show Engine Content」 를 켜야 보인다(확실 — Epic 문서 「If enabled, shows the `Engine Content` folder in the Sources panel」 · 기본값은 문서에 없다 → 기본 꺼짐은 추정, 가능성 높음). Unity · JetBrains 는 늘 보인다. Blender 는 드롭다운에서 고를 때만.
3. **읽기 전용**: Blender 번들 파일 = 읽기 전용(가능성 높음) · Unity 레지스트리 패키지 = 고칠 수 없음(가능성 높음) · JetBrains = 「Duplicate」 로 고칠 사본을 만든다(불확실).
4. **몇 개 안 되는 내장을 트리 칸으로 두는 피어는 못 찾았다**(불확실) — 찾은 피어는 모두 내장이 크다. 작은 내장은 고르는 자리(피커 · 템플릿 목록)에서 보이는 쪽이 자연스럽다는 것은 추정이다.
5. 위/아래 위치는 확인하지 못했다(모름).

## 피어

| 피어 | 기본 보임 | 자리 | 읽기 전용 | 출처 | 확신도 |
|---|---|---|---|---|---|
| Unreal Content Browser | 옵션 「Show Engine Content」 · 「Show Plugin Content」 켜야 보임(기본값 미기재) | Sources 패널의 `Engine Content` 폴더 | 문서에 없음 | https://dev.epicgames.com/documentation/en-us/unreal-engine/content-browser-settings-in-unreal-engine · https://dev.epicgames.com/documentation/en-us/unreal-engine/sources-panel-reference-in-unreal-engine | 확실(옵션 존재 — 메인 원문 확인) · 기본 꺼짐 가능성 높음 |
| Blender Asset Browser | 드롭다운 「Essentials」 · 끌 수 없음 | 트리가 아니라 라이브러리 고르기 | 번들 파일 읽기 전용 · 링크로 씀 | https://docs.blender.org/manual/en/4.4/editors/asset_browser.html | 가능성 높음(스니펫) |
| Unity Project 창 | 늘 보임 | Packages 뿌리 | 레지스트리 · Git 패키지 고칠 수 없음 | https://docs.unity3d.com/6000.0/Documentation/Manual/upm-inspect.html | 가능성 높음 |
| JetBrains Live Templates | 늘 보임 | 설정 목록의 묶음 | Duplicate = 고칠 사본 · 지운 기본 되살리기 | https://www.jetbrains.com/help/idea/settings-live-templates.html | 불확실(스니펫) |

## 우리에게

선택지: A 왼쪽 칸에 따로 된 칸 · 기본 숨김 · 보기 옵션으로 켬(Unreal 꼴) / B 트리에 두지 않고 고르는 자리(에이전트 슬롯 「기본」 칸 · `+` 검색 맨 위)에서만(지금 설계 · 작은 내장에 맞는 쪽) / C 따로 된 칸 · 늘 보임(접힌 채). 사용자 결정.

## 한계

- light — 적대 리뷰 없음. Godot · VS Code 는 보지 않았다. Unreal 기본값 · 트리 위치는 문서에 없어 엔진에서 직접 봐야 한다.
