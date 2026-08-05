//! S14 레이아웃 타입(wire 미러) — 레이아웃 권위 = src-tauri(ADR-0035).
//!
//! ★배치 규약★: 이 타입들은 **src-tauri 안에서만** 정의·export 된다. protocol/daemon crate 에
//! 절대 넣지 않는다 — 데몬은 View 를 일절 모르는 UI 불가지론(ADR-0035). ts-rs 로 프론트
//! (`src/api/layoutTypes.ts`)에 미러하되, 데몬 wire 계약(protocol crate)과는 별개 채널이다.
//!
//! agent_id 는 데몬 에이전트의 "참조 문자열"일 뿐(소유 아님) — close_tab 해도 에이전트는 생존(ADR-0035 디커플링).

use ts_rs::TS;
use uuid::Uuid;

use super::spatial::SlotSpatial;

/// 분할 방향. Horizontal = 좌우(│로 가름), Vertical = 상하(─로 가름).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SplitDir {
    Horizontal,
    Vertical,
}

/// 슬롯 점유자 = 타입드 유니온(ADR-0060).
///
/// 후속 콘텐츠 종류(FileTree/ControlPanel)는 variant 추가로 확장하고, 사용자 커스텀(프리셋 버튼셋)은 새
/// variant 가 아니라 variant 내부 config 데이터로 표현한다(enum 폭발 방지 — ADR-0060 핵심 통찰).
///
/// ★불변식(ADR-0060)★: `Agent` variant 는 데몬 에이전트의 **바인딩(agent_id 참조 문자열)만** 담는다 —
/// 라이브 출력 스트림은 여기 담지 않고 `OutputRouter` 가 agent_id 로 별도 라우팅한다(ADR-0041/0042/0046).
/// epoch(재spawn 재구독 트리거)도 레이아웃 트리 밖(agentStore 소유 — ADR-0007/0046).
// ADR-0060
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum SlotContent {
    /// resolve_spawn_slot 점유 판정에서 "빈"(ADR-0059).
    Empty,
    /// 데몬 에이전트 참조(바인딩만 — 소유 아님, ADR-0035). resolve_spawn_slot 에서 "점유"(ADR-0059).
    Agent { agent_id: String },
    /// MVP=필드 없는 unit — 렌더 대상만 지정하고 데이터는 agentStore 가 쥔다(콘텐츠 종류만 표현, ADR-0060).
    /// 후속 리치화(필터·정렬 기준 등)는 FileTree/ControlPanel 선례처럼 variant 내부 config 필드 추가로
    /// 확장한다(enum 폭발 아님). // ADR-0060
    AgentList,
    /// 프리셋 팔레트(등록된 cwd 프리셋 버튼셋) 뷰. MVP=필드 없는 unit — 프리셋 목록 데이터는
    /// 데몬 소유(presets.json, ADR-0061)라 여기 담지 않고 PresetRegistry wire 로 별도 흐른다.
    /// 후속 config(레이아웃·표시 옵션 등)는 variant 내부 필드로 확장한다(ADR-0060 선례). // ADR-0060
    PresetPalette,
}

impl SlotContent {
    /// 점유 판정 · 첫 빈 슬롯 스캔에서 씀(ADR-0059).
    pub fn is_empty(&self) -> bool {
        matches!(self, SlotContent::Empty)
    }

    pub fn agent_id(&self) -> Option<&str> {
        match self {
            SlotContent::Agent { agent_id } => Some(agent_id),
            // ADR-0060: 비-에이전트 콘텐츠(Empty/AgentList/PresetPalette)는 agent_id 없음.
            SlotContent::Empty | SlotContent::AgentList | SlotContent::PresetPalette => None,
        }
    }
}

/// 레이아웃 트리 노드 — 말단 Slot / 내부 Split 의 재귀 enum. content 는 슬롯 점유자(ADR-0060).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum LayoutNode {
    /// id = 창 간 전역 고유(UUID). // ADR-0060
    Slot {
        #[ts(type = "string")]
        id: Uuid,
        content: SlotContent,
    },
    /// ratio = a 가 차지하는 비율(0.0~1.0 클램프, 기본 0.5).
    Split {
        dir: SplitDir,
        ratio: f32,
        a: Box<LayoutNode>,
        b: Box<LayoutNode>,
    },
}

/// 한 View(탭/팝업 하나) = 레이아웃 트리 + 포커스 슬롯.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct View {
    #[ts(type = "string")]
    pub id: Uuid,
    pub name: String,
    pub layout: LayoutNode,
    /// 가리키던 슬롯이 사라지면 트리 첫 슬롯으로 폴백.
    #[ts(type = "string | null")]
    pub focused_slot_id: Option<Uuid>,
}

/// 탭 바용 View 메타(레이아웃 본체 제외 — `window:tabs-updated` 페이로드).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct ViewMeta {
    #[ts(type = "string")]
    pub id: Uuid,
    pub name: String,
}

/// `get_view` 응답 + `layout:updated` 페이로드.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct ViewSnapshot {
    #[ts(type = "string")]
    pub view_id: Uuid,
    pub layout: LayoutNode,
    #[ts(type = "string | null")]
    pub focused_slot_id: Option<Uuid>,
    /// ★슬롯 공간 타깃 파생(ADR-0068)★: 각 말단 슬롯의 방향 이웃(up/down/left/right) + 순서(ordinal).
    /// 논리 도면(split 방향·ratio)에서 산출한다 — 픽셀·getBoundingClientRect 무관(백엔드 권위 ADR-0035).
    /// ordinal 순으로 담긴다. 좌표 자체는 노출 안 함(ADR-0068 결정 3 — 좌표 보류).
    pub slot_spatial: Vec<SlotSpatial>,
    /// 변경마다 +1(ViewManager.version).
    /// ts-rs u64 기본 매핑=bigint 이나 serde_json 은 number 로 직렬화(런타임=JS number) → 타입도 number 로 고정
    /// (불일치 시 프론트 race 가드 `snap.version > pulled` 에서 bigint↔number 혼용 에러, FIX-1). 카운터라 2^53 비현실적.
    #[ts(type = "number")]
    pub version: u64,
}

impl LayoutNode {
    pub fn new_empty_slot() -> Self {
        LayoutNode::Slot {
            id: Uuid::new_v4(),
            content: SlotContent::Empty, // ADR-0060: 빈 슬롯 = Empty variant.
        }
    }
}
