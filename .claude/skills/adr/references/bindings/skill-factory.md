# ADR 바인딩 — skill-factory

골격(`../flow.md`)이 "스크립트"·"ADR 폴더"·"파일명 규약"·"템플릿"·"인덱스"·"상태 범례"·"앵커 명령"이라 부르는 자리에 끼우는 **skill-factory(작업장 skill-lab 자체) 전용 실명령·실체**다. 골격은 어느 프로젝트나 동일한 범용 ADR 엔진이고, 이 파일이 factory로 바인딩한다.

factory ADR은 **경량 형식**이다(제목 / 결합 메타줄 / 결정 / 근거 / 거부 대안) — dashboard보다 슬롯이 적다. 스크립트는 파라미터 플래그로 이 형식을 그대로 섬긴다.

> **정본 = 스킬 내장 `scripts/adr.mjs`(서기 작업 실체) + `doc/decisions/README.md`(상태 범례·인덱스) + `references/formats/adr-light.template.md`(경량 스캐폴드).** 이 파일은 그 **현재 바인딩 스냅샷**일 뿐이다 — 충돌하면 스크립트/README/템플릿을 따르고 이 파일을 고친다(rot 방지). 템플릿·인덱스 표를 통째 복붙해 제2의 출처로 만들지 않는다 — *가리키기만* 한다.

## 스크립트 실명령 (골격 §2 "스크립트 호출"에 주입)

서기 작업은 **스킬 폴더 안** `scripts/adr.mjs`가 결정적으로 한다(dashboard와 동일한 하나의 스크립트 — 자족 스킬). `<skill>` = 이 스킬 폴더 경로. factory 실값을 아래 플래그로 주입한다:

- ADR 폴더 = `I:\Engram\agents\skill-lab\doc\decisions` (`--dir`)
- 인덱스 파일 = `README.md`(기본 — 생략 가능)
- 상태 어휘 = `채택,제안,폐기,거부` (`--status-vocab "채택,제안,폐기,거부"`)
- 기본 상태 = `채택` (`--default-status 채택`)
- 템플릿 = `<skill>/references/formats/adr-light.template.md` (`--template`)
- 코드 앵커 = **없음**(팩토리는 코드 앵커 규약 미사용) → `--anchor-roots ""`(빈 값 = 앵커 스캔 생략)

절대경로 대신 워크스페이스 루트(`I:\Engram\agents\skill-lab`)에서 상대로 돌려도 된다(`--dir doc/decisions`). 아래는 절대경로 예시(복붙 안전):

```bash
DIR="I:/Engram/agents/skill-lab/doc/decisions"
TMPL="<skill>/references/formats/adr-light.template.md"
V="채택,제안,폐기,거부"

# new — 채번(max+1, 재스캔) + 경량 스캐폴드. prose 슬롯 TODO는 스킬이 채움.
node <skill>/scripts/adr.mjs new --dir "$DIR" --template "$TMPL" --status-vocab "$V" --default-status 채택 --anchor-roots "" --title "<한 줄 제목>"

# index — 본문 H1·상태 스캔해 README 인덱스 표 재생성. 기본 --check, --write만 실제 갱신.
node <skill>/scripts/adr.mjs index --check --dir "$DIR" --status-vocab "$V" --anchor-roots ""
node <skill>/scripts/adr.mjs index --write --dir "$DIR" --status-vocab "$V" --anchor-roots ""

# lint — 정합성 점검(read-only, 보고 전용).
node <skill>/scripts/adr.mjs lint --dir "$DIR" --status-vocab "$V" --anchor-roots ""
```

- **supersede 제약** — factory 경량 형식은 **결합 메타 줄**(`- 날짜: … · 상태: … · 결정자: …`)이라 독립 `- 상태:` 줄이 없다. `supersede --mode full`은 상태를 독립 줄로 재기록하므로 이 형식에선 **스크립트가 안전하게 거부**한다(다른 필드 손실 방지 — 수동 처리 필요). `--mode partial`도 `- 관련:` 줄이 없어 양방향 Amends 링크를 못 박는다. factory는 결정 번복이 드물고, 필요 시 수동 폐기 + 새 번호로 처리한다. supersede 자동화가 필요해지면 경량 형식에 관련줄을 추가하거나 dashboard 형식으로 이행하는 걸 별도 결정한다.

## ADR 폴더 + 파일명 규약 (골격 §2에 주입)

- **폴더** — `doc/decisions/`(dashboard는 `docs/`, factory는 `doc/` — 오타 아님, 실제 폴더명이 다름).
- **파일명** — `NNNN-제목-슬러그.md`(4자리 zero-pad + 슬러그). 슬러그는 스크립트가 결정적으로 생성. 기존 예시 `0001-skill-crafting-factory-only.md`.
- **채번** — 스크립트가 `doc/decisions/`의 현재 최대 번호 + 1(쓰기 직전 재스캔). LLM이 지정하지 않는다.

## 템플릿 실체 (골격 §2 prose 채우기에 주입)

`references/formats/adr-light.template.md`가 스캐폴드 정본이다 — 경량 섹션 구조(제목 / 결합 메타줄 `- 날짜: {{DATE}} · 상태: {{STATUS}} · 결정자: 사용자` / 결정 / 근거 / 거부 대안). 스크립트가 이 구조로 빈 슬롯(`TODO`)을 만들고, 스킬이 §1에서 받은 내용으로 메운다. **섹션 구조는 건드리지 않는다.** 기존 3파일(0001~0003)이 이 형식의 산 예시다.

## 인덱스 위치·형식 + 재생성 (골격 §2 "인덱스 재생성"에 주입)

- **위치·형식 정본** — `doc/decisions/README.md`의 "## 인덱스" 표(`| # | 제목 | 상태 |`). `# ` 칸은 `[NNNN](NNNN-제목-슬러그.md)` 링크. dashboard와 동일 형식.
- **재생성** — `index --write`가 본문 H1·상태줄을 스캔해 표를 다시 만든다. 손으로 인덱스 표를 편집하지 않는다.
- **보존 규칙** — 인덱스 셀이 본문보다 정보가 많으면 스크립트가 기존 셀을 보존하고 경고만 낸다(자동 손실 금지) — dashboard와 동일.

## 상태 범례 (골격 §2 lint·§3에 주입)

`doc/decisions/README.md` "상태 범례"가 정본: **채택 / 제안 / 폐기 / 거부**(dashboard는 "확정"이지만 factory는 "채택"을 쓴다 — 기존 3파일이 "채택"). lint은 `--status-vocab`로 주입한 이 어휘만 검사한다(상태줄 단서절 자유서술은 무시). 어휘를 늘리면 README와 호출 플래그를 같이 고친다.

## 코드 앵커 규약 + 고아 검출 (골격 §2 lint에 주입)

factory는 **코드 앵커 규약을 쓰지 않는다**(작업장 = 문서·스킬 골격 중심, load-bearing 코드 앵커 대상 없음). `--anchor-roots ""`로 앵커 스캔을 생략한다 — lint은 번호 정합·상태 어휘·H1만 검사한다.

## 흐름 기록 연동 (골격 §3 "결과 보고"에 주입)

factory는 dashboard의 `step-log` 같은 별도 흐름 기록 체계를 (현재) 두지 않는다. ADR 추가·번복의 흐름 기록·커밋은 **메인(팩토리 세션)이 처리**한다 — 스킬·스크립트는 인덱스·양방향 외 파일을 직접 쓰지 않는다.
