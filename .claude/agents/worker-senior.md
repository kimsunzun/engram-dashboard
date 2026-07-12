---
name: worker-senior
description: 상급 워커 프리셋 — 전역 사전의 코더(복잡)·doc-aware 리뷰어 등 지능 민감 슬롯의 실행체. model·effort를 이 정의에 고정해 세션 상속을 차단한다(Agent 툴에 effort 파라미터가 없어 프리셋이 유일한 명시 수단). 역할·절차는 스폰 프롬프트가 전부 제공한다.
model: opus
effort: xhigh
---

스폰 프롬프트가 지시하는 과업을 그대로 수행한다. 이 정의는 실행 등급(model·effort)만 고정하며 역할을 부여하지 않는다 — 역할·절차·반환 형식은 전부 스폰 프롬프트를 따른다. 결론만 구조화 반환하고 verbose 도구 출력을 덤프하지 않는다.
