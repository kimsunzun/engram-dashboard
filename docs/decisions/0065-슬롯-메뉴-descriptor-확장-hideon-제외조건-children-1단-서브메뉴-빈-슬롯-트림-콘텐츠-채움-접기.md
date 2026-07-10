# ADR-0065: 슬롯 메뉴 descriptor 확장 — hideOn 제외조건 + children 1단 서브메뉴 (빈-슬롯 트림 + 콘텐츠-채움 접기)

- 상태: 확정 (2026-07-10, 근거: 사용자 결정(옵션 "트림 + 콘텐츠 서브메뉴") + `/research light` OSS 서베이(VS Code·JetBrains·iTerm2·Windows Terminal·GNOME HIG))
- 관련: Amends ADR-0064 (descriptor 스키마 확장: hideOn 제외조건 + children 1단 서브메뉴 (when-DSL 연기를 hideOn으로 부분 실현)) · CLAUDE.md §5(LLM-우선 제어) · ADR-0060(SlotContent 유니온) · `src/commands/slotMenu.ts`(SlotMenuItem/buildSlotMenu) · `src/commands/{slotCommands,slotContentCommands}.ts`(기여) · `src/components/slot/SlotContextMenu.tsx`(렌더)

## 맥락

빈 슬롯(SlotContent=empty) 우클릭 메뉴가 8항목 플랫으로 난잡했다: 콘텐츠-채움 3(에이전트 트리 열기·프리셋 팔레트 열기·에이전트 생성, `target='empty'`·group=content) + 공통 slot-ops 5(가로/세로 분할·팝업 분리·비우기·닫기, `target='*'`·group=slot-ops). 두 문제:

1. **빈 슬롯에서 무의미한 항목 노출** — `slot.empty`(비우기)는 이미 빈 슬롯을 또 비우는 no-op, `slot.popout`(팝업 분리)도 빈 칸을 팝아웃해 실익이 낮다. 둘 다 `target='*'` 보편 등록이라 콘텐츠 타입과 무관하게 무조건 표시된다. ADR-0064는 가시성을 `target` 하나로만 판별하고 "특정 타입에서 제외"할 수단(연기한 `when`)이 없었다.
2. **콘텐츠 vs 레이아웃 평면 혼재** — "이 칸에 뭘 넣나"(콘텐츠 채움)와 "이 칸을 어떻게 하나"(레이아웃 조작)가 한 메뉴에 8줄로 섞여 위계가 약하다(GNOME HIG 3~12 상한의 끝자락).

`/research light` OSS 서베이(grounding = 4앱 공식 docs/이슈 + GNOME HIG): 성숙 멀티패인 앱은 **예외 없이 플랫 구분선 그룹**을 쓰고 레이아웃-ops 서브메뉴화는 **의도적으로 회피**한다(VS Code #247761·Windows Terminal #18137의 split 서브메뉴 제안 모두 미병합). 서브메뉴가 값어치 하는 건 콘텐츠 타입이 동적으로 늘어나는 **"New >" 패턴**뿐. GNOME HIG는 컨텍스트 메뉴 3~12·서브메뉴 3~6, 중첩을 비권장한다.

## 결정

ADR-0064 단일 기여 API(`registerSlotMenu`·command 단일소스·'*' 공통·등록 일원화)는 그대로 두고, **descriptor 스키마만 additive로 2개 확장**한다.

1. **`hideOn?: SlotContentType[]` (제외 조건)** — 특정 콘텐츠 타입 슬롯에서 그 항목을 숨긴다. `slot.empty`·`slot.popout`에 `hideOn: ['empty']`. `target='*'` 보편 등록은 유지하되(공통 ops 단일소스 불변식 준수) 특정 타입만 뺀다. 전체 `when` 문자열 DSL은 도입하지 않고, ADR-0064가 연기한 가시성 조건을 이 좁은 타입-배열로 부분 실현한다.
2. **`children?: SlotMenuItem[]` + 컨테이너 `title` (1단 서브메뉴)** — 콘텐츠-채움 3항목(`slot.fill.agentList`·`slot.fill.presetPalette`·`slot.createAgentHere`)을 "새 콘텐츠 ▶" 컨테이너 항목으로 접는다. 컨테이너는 `commandId` 없이 `title`+`children`을 갖는다(실행 항목은 `commandId`, 컨테이너는 `children`). 향후 백엔드 타입(codex/gemini) 추가 시 여기에 붙는 확장 자리.
3. **렌더** — `SlotContextMenu`에 hover flyout으로 1단 전개. 최근 뷰포트 clamp 로직(ADR 미부여, `aebfa86`)을 재사용해 우측 오버플로 시 좌측으로 전개. **중첩은 1단 한정**(children 안의 children 금지).
4. **§5 LLM 제어 불변** — `hideOn`·`children`은 렌더·가시성 **표현**일 뿐, `slot.fill.*` 등 command는 registry 단일소스로 그대로 직접 호출 가능(`__engramCmd.run`). 서브메뉴로 접어도 LLM 제어 표면에 갭이 생기지 않는다.

빈 슬롯 최종 메뉴: `새 콘텐츠 ▶` · ─── · `가로 분할` · `세로 분할` · `닫기` (5줄, 채움 3은 서브메뉴로).

## 거부한 대안

- **레이아웃-ops(분할/팝업/닫기)를 서브메뉴로 접기** — VS Code(#247761)·Windows Terminal(#18137) 둘 다 split 서브메뉴화 제안을 의도적으로 미병합. 자주 쓰는 동사라 한 단계 클릭 추가는 손해고 GNOME HIG도 중첩 비권장. 사용자가 원한 것도 "채움 접기 + 무의미 트림"이지 레이아웃 중첩이 아님. 거부.
- **전체 `when` 문자열 DSL 지금 도입**(ADR-0064가 연기한 것) — 현재 필요는 "특정 콘텐츠 타입 제외" 하나뿐. 파서/평가기 비용 대비 이득 없음(§0 판단기준: 복합 조건 미등장 = 불확실 → 껍데기도 아닌 미도입 유지). `hideOn` 타입 배열로 충분하고, 복합 가시성 조건이 실제 등장하면 그때 DSL로 승격. 거부(연기 유지).
- **`slot.empty`/`slot.popout`을 '*'에서 빼고 콘텐츠 타입별 재선언** — ADR-0064 "공통 ops = '*' 단일소스, 콘텐츠 재선언 금지"(drift·리뷰 reject) 불변식 위반. hideOn 제외가 '*' 보편성을 지키면서 목표를 달성. 거부.
- **2단+ 깊은 중첩 지원** — GNOME HIG 중첩 비권장 + safety-triangle 마우스 트래킹 문제. 1단으로 제한. 거부.

## 근거

- **OSS 서베이 grounding:** 플랫 구분선 + 콘텐츠/레이아웃 섹션 분리 = VS Code·JetBrains·iTerm2·Windows Terminal 공통. 서브메뉴는 동적 콘텐츠 타입("New >")에만 값어치 — 우리의 향후 다중 백엔드와 정확히 맞물린다.
- **§0 판단기준(위험도×기간):** `hideOn`(좁은 타입필드, 저위험·확장 용이)은 지금, 전체 `when`-DSL(불확실·복합조건 미등장, 되돌리기 비쌈)은 연기. 과도한 추상화(DSL)와 과소(하드코딩 분기) 사이의 중간.
- **하위호환:** `hideOn`·`children` 모두 옵셔널 → 기존 기여 코드 무변경. ADR-0064 핵심(단일 기여·command 단일소스·등록 일원화·row-level 예외)은 전부 유효, 스키마 additive 확장만.

## 영향 / 불변식

- **descriptor 스키마** = `{ commandId?, title?, group, order, hideOn?, children? }`. `commandId` 있으면 실행 항목(title은 registry에서 resolve), `children` 있으면 컨테이너(`commandId` 없이 `title` 필수). 둘 다 없거나 둘 다 있으면 dev 에러(fail-loud).
- **서브메뉴 1단 한정** — `children` 안에 또 `children` 금지(렌더·GNOME HIG). 위반 시 dev 에러.
- **hideOn = 제외(subtraction) 전용** — allowlist 아님. `target='*'` 보편성을 유지하면서 특정 타입만 뺀다. 공통 ops는 여전히 '*' 단일소스(ADR-0064 불변식 유지).
- **§5 불변** — `hideOn`/`children`은 렌더 표현 확장일 뿐. command 직접 호출 경로(팔레트·키바인딩·LLM `__engramCmd.run`)는 불변. 메뉴 나열 여부와 무관하게 command는 항상 호출 가능해야 한다.
- **하위호환** — 기존 `{commandId, group, order}` 기여는 그대로 유효(hideOn/children 미지정 = 기존 동작).
