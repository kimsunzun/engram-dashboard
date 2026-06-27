//! S14 레이아웃 타입(wire 미러) — 레이아웃 권위 = src-tauri(ADR-0035).
//!
//! ★배치 규약★: 이 타입들은 **src-tauri 안에서만** 정의·export 된다. protocol/daemon crate 에
//! 절대 넣지 않는다 — 데몬은 View 를 일절 모르는 UI 불가지론(ADR-0035). ts-rs 로 프론트
//! (`src/store/layoutTypes.ts`)에 미러하되, 데몬 wire 계약(protocol crate)과는 별개 채널이다.
//!
//! LayoutNode 는 split 트리(에디터 모델) — Slot(말단) / Split(내부 노드)의 재귀 enum. agent_id 는
//! 데몬 에이전트의 "참조 문자열"일 뿐(소유 아님) — close_view 해도 에이전트는 생존(ADR-0035 디커플링).

use ts_rs::TS;
use uuid::Uuid;

/// 분할 방향. Horizontal = 좌우(│로 가름), Vertical = 상하(─로 가름).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SplitDir {
    Horizontal,
    Vertical,
}

/// 레이아웃 트리 노드 — 말단 Slot / 내부 Split 의 재귀 enum.
///
/// `#[serde(tag = "type")]` + snake_case → 프론트 discriminated union(`{type:"slot",...}`/
/// `{type:"split",...}`). agent_id 는 데몬 에이전트 참조 문자열(소유 아님, ADR-0035).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum LayoutNode {
    /// 말단 슬롯. id = 창 간 전역 고유(UUID). agent_id None = 미배정(빈 슬롯).
    Slot {
        #[ts(type = "string")]
        id: Uuid,
        agent_id: Option<String>,
    },
    /// 내부 분할 노드. ratio = a 가 차지하는 비율(0.0~1.0 클램프, 기본 0.5).
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
    /// 현재 포커스된 슬롯. 가리키던 슬롯이 사라지면 트리 첫 슬롯으로 폴백(없으면 None).
    #[ts(type = "string | null")]
    pub focused_slot_id: Option<Uuid>,
}

/// 탭 바용 View 메타(레이아웃 본체 제외 — `view:list-updated` 페이로드).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct ViewMeta {
    #[ts(type = "string")]
    pub id: Uuid,
    pub name: String,
}

/// `get_view` 응답 + `layout:updated` 페이로드. version = 팝업 pull↔listen race 용(get_view race).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct ViewSnapshot {
    #[ts(type = "string")]
    pub view_id: Uuid,
    pub layout: LayoutNode,
    #[ts(type = "string | null")]
    pub focused_slot_id: Option<Uuid>,
    /// 변경마다 +1(ViewManager.version). 팝업이 pull 한 version 이하 emit 은 폐기(초기 유실·중복 방지).
    /// ts-rs u64 기본 매핑=bigint 이나 serde_json 은 number 로 직렬화(런타임=JS number) → 타입도 number 로 고정
    /// (불일치 시 프론트 race 가드 `snap.version > pulled` 에서 bigint↔number 혼용 에러, FIX-1). 카운터라 2^53 비현실적.
    #[ts(type = "number")]
    pub version: u64,
}

impl LayoutNode {
    /// 빈 슬롯(미배정) 새로 생성. root 슬롯 close·새 View 생성 시 시드.
    pub fn new_empty_slot() -> Self {
        LayoutNode::Slot {
            id: Uuid::new_v4(),
            agent_id: None,
        }
    }
}
