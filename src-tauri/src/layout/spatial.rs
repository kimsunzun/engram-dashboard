//! 슬롯 공간 타깃 파생(ADR-0068) — 논리 레이아웃 트리에서 방향·이웃·순서를 산출하는 순수 로직.
//!
//! ★Tauri 의존 0 · 픽셀 0★: 이 모듈은 `LayoutNode`(split 방향 + ratio)만 알고 실측 rect·
//! `getBoundingClientRect`·창 크기를 **모른다**. "우하단"·"이 슬롯 오른쪽" 같은 공간 지시를 slot id
//! 로 옮기는 근거를 논리 도면(트리 구조)만으로 계산한다 → 단독 headless 테스트 가능(ADR-0012 격리).
//!
//! ## 계산 뼈대
//! 1. 트리를 재귀 순회하며 각 말단 슬롯에 정규화 rect `[0,1]×[0,1]` 를 부여한다(split 방향·ratio 로 분할).
//!    루트 = 단위정사각형. Horizontal split(좌우) → x 축을 ratio 로 가르고, Vertical split(상하) → y 축.
//! 2. 그 rect 들에서 **모서리 인접(neighbor)** 과 **순서(ordinal)** 를 파생한다.
//!    - neighbor: 두 슬롯이 해당 축에서 맞닿고(경계 좌표 일치) 직교축 구간이 겹치면 인접.
//!    - ordinal: 각 말단 rect 의 **중심점** `(center_y, center_x)` 사전순 GLOBAL 정렬(위→아래,
//!      동률이면 왼쪽→오른쪽) 0-based. ★트리 전위(pre-order)가 아니라 전역 중심 정렬★ — 트리 구조가
//!      아니라 화면상 위치로 매긴다. leaf rect 가 서로 겹치지 않아 중심점 쌍이 유일 → 결정적(deterministic).
//!      단 열/행 응집(cohesion)은 보장하지 않는다: 비대칭 분할에선 전체 높이 한 열(column)이 좌측 열
//!      슬롯들 사이에 끼어들 수 있다(center_y 로만 순서를 매기므로).
//!
//! ★정규화 rect 는 내부 계산 detail★(ADR-0068 — 좌표 노출 보류): 공개 표면은 `neighbors`+`ordinal`
//! 뿐이고 raw 좌표는 스냅샷에 내보내지 않는다. 실측 픽셀·좌표계는 별도 capability 로 후속(보류).
// ADR-0068

use std::collections::HashMap;

use uuid::Uuid;

use super::types::{LayoutNode, SplitDir};

/// 부동소수 경계 비교 허용오차 — ratio 분할로 생기는 좌표는 이진 표현 오차가 누적될 수 있어
/// 정확한 `==` 대신 이 epsilon 안이면 같은 경계로 본다(인접 판정 안정화).
const EPS: f32 = 1e-4;

/// 한 말단 슬롯의 정규화 논리 rect(`[0,1]×[0,1]`). ★내부 계산 detail★ — 공개 표면 아님(ADR-0068).
/// x/y = 좌상단, w/h = 너비/높이. 겹침·경계 인접 판정에만 쓰고 밖으로 내보내지 않는다.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct NormRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl NormRect {
    fn right(&self) -> f32 {
        self.x + self.w
    }
    fn bottom(&self) -> f32 {
        self.y + self.h
    }
    /// 이 rect 의 중심(ordinal 정렬·edge 토큰 선택에 씀 — 경계가 아니라 대표점).
    fn center_x(&self) -> f32 {
        self.x + self.w / 2.0
    }
    fn center_y(&self) -> f32 {
        self.y + self.h / 2.0
    }
}

/// 두 좌표가 (epsilon 안에서) 같은 경계인가.
fn edge_eq(a: f32, b: f32) -> bool {
    (a - b).abs() <= EPS
}

/// 트리를 재귀 순회하며 각 말단 슬롯에 정규화 rect 를 부여한다(전위 순서 — a 먼저). ★순수★.
/// 루트 호출은 `rect = 단위정사각형`으로 시작한다(`leaf_rects`).
fn assign_rects(node: &LayoutNode, rect: NormRect, out: &mut Vec<(Uuid, NormRect)>) {
    match node {
        LayoutNode::Slot { id, .. } => out.push((*id, rect)),
        LayoutNode::Split { dir, ratio, a, b } => {
            // ratio = a 가 차지하는 비율. ★LOAD-BEARING 불변식(ADR-0068 §0 저위험 방어)★:
            // 이 모듈의 공간 계산은 **모든 leaf 가 비퇴화 면적(ratio ∈ (0,1))** 을 가진다고 가정한다 —
            // ratio 0/1 이면 한쪽 leaf 가 zero-width/height 가 되고, 그러면 edge 인접 판정(edge_eq)·
            // overlap·corner 해소가 무너진다(경계가 겹쳐 이웃/코너가 뒤엉킴). 그래서 `[EPS, 1-EPS]` 로
            // 0/1 에서 떼어낸다 → zero-area leaf 는 절대 안 생긴다. clamp 하한(EPS)은 인접 판정 EPS 와
            // 같은 크기라 최소 치수가 판정 임계 이상 → 안전. 정상 ratio(0.2/0.5 등)는 영향 없음.
            // ※ §5 layout-resize command 를 추가할 때 이 불변식(0/1 회피)을 반드시 보존해야 한다.
            // ★★ 이 clamp 만으로는 부족하다(cross-family 리뷰 ①②, ADR-0068 §영향)★★:
            // leaf 절대 크기 = 경로상 ratio 의 곱이라, 분할별로 [EPS,1-EPS] 로 막아도 중첩되면
            // sub-EPS 로 내려간다. 폭/높이 < EPS 인 leaf 는 (①) 재분할 시 하위 overlap 이 정확히 EPS 라
            // `overlap > EPS` 에 탈락해 이웃 소실, (②) 너머 slot 이 edge_eq 로 인접 오판돼 건너뛰어진다.
            // → resize command 는 반드시 UX 최소 칸 크기를 강제(그 이하 = 제거/스냅)해 sub-EPS leaf 를
            //   구조적으로 배제해야 한다. 이 순수 계산층에서 절대 epsilon 만으로 완전 방어는 불가.
            let r = ratio.clamp(EPS, 1.0 - EPS);
            let (ra, rb) = match dir {
                // Horizontal = 좌우 분할: x 축을 ratio 로 가른다(a=왼쪽, b=오른쪽).
                SplitDir::Horizontal => (
                    NormRect {
                        x: rect.x,
                        y: rect.y,
                        w: rect.w * r,
                        h: rect.h,
                    },
                    NormRect {
                        x: rect.x + rect.w * r,
                        y: rect.y,
                        w: rect.w * (1.0 - r),
                        h: rect.h,
                    },
                ),
                // Vertical = 상하 분할: y 축을 ratio 로 가른다(a=위, b=아래).
                SplitDir::Vertical => (
                    NormRect {
                        x: rect.x,
                        y: rect.y,
                        w: rect.w,
                        h: rect.h * r,
                    },
                    NormRect {
                        x: rect.x,
                        y: rect.y + rect.h * r,
                        w: rect.w,
                        h: rect.h * (1.0 - r),
                    },
                ),
            };
            assign_rects(a, ra, out);
            assign_rects(b, rb, out);
        }
    }
}

/// 트리의 모든 말단 슬롯 → 정규화 rect(전위 순서). ★내부용★ — rect 는 밖으로 안 나간다(ADR-0068).
pub(crate) fn leaf_rects(node: &LayoutNode) -> Vec<(Uuid, NormRect)> {
    let mut out = Vec::new();
    assign_rects(
        node,
        NormRect {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        },
        &mut out,
    );
    out
}

/// 한 슬롯의 방향별 이웃(각 = 인접 slot id 또는 None). 논리 도면 파생(픽셀 무관). ADR-0068.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct Neighbors {
    #[ts(type = "string | null")]
    pub up: Option<Uuid>,
    #[ts(type = "string | null")]
    pub down: Option<Uuid>,
    #[ts(type = "string | null")]
    pub left: Option<Uuid>,
    #[ts(type = "string | null")]
    pub right: Option<Uuid>,
}

/// 한 말단 슬롯의 공간 타깃 파생 정보(ViewSnapshot 에 슬롯별로 실린다). ADR-0068.
/// ★좌표 비노출★: neighbors + ordinal 만 — 정규화 rect 는 내부 계산 detail 이라 여기 없다(ADR-0068 결정 3).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct SlotSpatial {
    #[ts(type = "string")]
    pub slot_id: Uuid,
    /// 방향별 인접 슬롯(공유 변 있음). 없으면 각 None.
    pub neighbors: Neighbors,
    /// 순서 인덱스 — 중심점 `(center_y, center_x)` 전역 사전순(위→아래, 동률 왼쪽→오른쪽) 0-based.
    /// ★트리 전위가 아니라 전역 중심 정렬★. 상세·응집 미보장은 모듈 헤더 §계산 뼈대 참조.
    #[ts(type = "number")]
    pub ordinal: u32,
}

/// 슬롯 rect 목록에서 방향별 이웃을 계산한다. `dir` 축에서 두 rect 가 맞닿고(경계 일치) 직교축 구간이
/// 겹치면 인접. 여러 후보가 있으면(예: 오른쪽에 두 슬롯이 세로로 쌓임) 직교축 겹침이 가장 큰 것을 고른다
/// (대표 이웃 하나 — 방향 이동의 자연스러운 타깃).
fn neighbor_in_dir(rects: &[(Uuid, NormRect)], idx: usize, dir: Dir) -> Option<Uuid> {
    let (_, me) = rects[idx];
    let mut best: Option<(Uuid, f32)> = None;
    for (j, (oid, other)) in rects.iter().enumerate() {
        if j == idx {
            continue;
        }
        // 인접 조건: dir 축에서 내 경계 == 상대의 반대 경계 + 직교축 구간 겹침.
        let (adjacent, overlap) = match dir {
            Dir::Right => (
                edge_eq(me.right(), other.x),
                overlap_len(me.y, me.bottom(), other.y, other.bottom()),
            ),
            Dir::Left => (
                edge_eq(me.x, other.right()),
                overlap_len(me.y, me.bottom(), other.y, other.bottom()),
            ),
            Dir::Down => (
                edge_eq(me.bottom(), other.y),
                overlap_len(me.x, me.right(), other.x, other.right()),
            ),
            Dir::Up => (
                edge_eq(me.y, other.bottom()),
                overlap_len(me.x, me.right(), other.x, other.right()),
            ),
        };
        if adjacent && overlap > EPS {
            match best {
                Some((_, bo)) if bo >= overlap => {}
                _ => best = Some((*oid, overlap)),
            }
        }
    }
    best.map(|(id, _)| id)
}

/// 두 구간의 겹치는 길이(음수면 0 처리는 호출측 EPS 비교가 걸러냄).
fn overlap_len(a0: f32, a1: f32, b0: f32, b1: f32) -> f32 {
    a1.min(b1) - a0.max(b0)
}

/// 방향 축(neighbor_in_dir 내부용).
#[derive(Debug, Clone, Copy)]
enum Dir {
    Up,
    Down,
    Left,
    Right,
}

/// 트리에서 각 말단 슬롯의 공간 정보(neighbors + ordinal)를 계산한다. ★순수·픽셀 무관★(ADR-0068).
/// 반환 순서 = ordinal 순(중심점 전역 사전순 — 위→아래·동률 왼쪽→오른쪽). ViewSnapshot 이 그대로 실어
/// 프론트/LLM 에 흘린다.
pub fn compute_spatial(node: &LayoutNode) -> Vec<SlotSpatial> {
    let rects = leaf_rects(node);

    // ordinal: 각 rect 중심점 `(center_y, center_x)` 사전순 GLOBAL 정렬(위→아래, 동률이면 왼쪽→오른쪽).
    // ★트리 전위(pre-order)가 아니라 전역 중심 정렬★ — 화면상 위치로 매긴다. leaf rect 가 disjoint 라
    // 중심점 쌍이 유일 → 결정적. 열/행 응집은 보장 안 함(비대칭 분할에서 전체 높이 열이 좌측 열 사이 끼어듦).
    let mut order: Vec<usize> = (0..rects.len()).collect();
    order.sort_by(|&i, &j| {
        let (_, a) = rects[i];
        let (_, b) = rects[j];
        a.center_y()
            .partial_cmp(&b.center_y())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(
                a.center_x()
                    .partial_cmp(&b.center_x())
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });
    // rects 인덱스 → ordinal.
    let mut ordinal_of: HashMap<usize, u32> = HashMap::new();
    for (ord, &ri) in order.iter().enumerate() {
        ordinal_of.insert(ri, ord as u32);
    }

    // ordinal 순으로 결과를 낸다(안정적 반환 순서).
    order
        .iter()
        .map(|&i| {
            let (id, _) = rects[i];
            SlotSpatial {
                slot_id: id,
                neighbors: Neighbors {
                    up: neighbor_in_dir(&rects, i, Dir::Up),
                    down: neighbor_in_dir(&rects, i, Dir::Down),
                    left: neighbor_in_dir(&rects, i, Dir::Left),
                    right: neighbor_in_dir(&rects, i, Dir::Right),
                },
                ordinal: ordinal_of[&i],
            }
        })
        .collect()
}

/// 공간/방향 토큰 — LLM/사람이 "우하단"·"이 슬롯 오른쪽" 같은 지시를 넘기는 심볼릭 표면(ADR-0068).
/// tmux 결 edge 토큰(모서리 4종) + 포커스 슬롯 상대 방향(4종). resolver 가 이걸 slot id 로 옮긴다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpatialToken {
    /// 모서리(절대) — 트리 전체에서 그 코너에 가장 가까운 말단 슬롯.
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    /// 포커스 슬롯 기준 상대 방향 이웃.
    Left,
    Right,
    Up,
    Down,
}

impl SpatialToken {
    /// wire 문자열(kebab)에서 파싱. 모르는 토큰은 None(호출자 = Err).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "top-left" | "topleft" => Some(Self::TopLeft),
            "top-right" | "topright" => Some(Self::TopRight),
            "bottom-left" | "bottomleft" => Some(Self::BottomLeft),
            "bottom-right" | "bottomright" => Some(Self::BottomRight),
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            "up" | "top" => Some(Self::Up),
            "down" | "bottom" => Some(Self::Down),
            _ => None,
        }
    }
}

/// 공간 토큰 → slot id 해소(ADR-0068). 논리 도면 파생(픽셀 무관)이라 순수·headless 테스트 가능.
/// - 모서리 토큰(top-left 등): 트리 전체에서 그 코너에 가장 가까운(코너까지 거리 최소) 말단 슬롯.
/// - 상대 방향(left/right/up/down): `focused` 슬롯의 그 방향 이웃(없으면 None). focused=None 이면 None.
///
/// ★edge 토큰이 "우하단"을 정확히 집는가(ADR-0068 수용 기준)★: 각 슬롯 rect 의 해당 코너 좌표까지의
/// 거리를 최소화한다 — L-shape(좌측 한 칸 + 우측 상하 2칸)에서 bottom-right 는 우측 아래 슬롯을 집는다.
pub fn resolve_spatial(
    node: &LayoutNode,
    focused: Option<Uuid>,
    token: SpatialToken,
) -> Option<Uuid> {
    let rects = leaf_rects(node);
    if rects.is_empty() {
        return None;
    }
    match token {
        SpatialToken::TopLeft => corner_slot(&rects, 0.0, 0.0),
        SpatialToken::TopRight => corner_slot(&rects, 1.0, 0.0),
        SpatialToken::BottomLeft => corner_slot(&rects, 0.0, 1.0),
        SpatialToken::BottomRight => corner_slot(&rects, 1.0, 1.0),
        SpatialToken::Left => relative_neighbor(&rects, focused, Dir::Left),
        SpatialToken::Right => relative_neighbor(&rects, focused, Dir::Right),
        SpatialToken::Up => relative_neighbor(&rects, focused, Dir::Up),
        SpatialToken::Down => relative_neighbor(&rects, focused, Dir::Down),
    }
}

/// 코너 `(cx,cy)`(단위정사각형 모서리)에 rect 코너가 가장 가까운 슬롯을 고른다. 그 코너 방향 rect 코너를
/// 대표점으로 삼아(예: bottom-right → rect 의 (right,bottom)) 코너까지 유클리드 거리 최소화.
fn corner_slot(rects: &[(Uuid, NormRect)], cx: f32, cy: f32) -> Option<Uuid> {
    let mut best: Option<(Uuid, f32)> = None;
    for (id, r) in rects {
        // 그 코너 쪽 rect 모서리 좌표(cx=1 이면 right, cy=1 이면 bottom).
        let px = if cx >= 0.5 { r.right() } else { r.x };
        let py = if cy >= 0.5 { r.bottom() } else { r.y };
        let d = (px - cx).powi(2) + (py - cy).powi(2);
        match best {
            Some((_, bd)) if bd <= d => {}
            _ => best = Some((*id, d)),
        }
    }
    best.map(|(id, _)| id)
}

/// focused 슬롯의 방향 이웃(공유 변). focused 가 트리에 없거나 None 이면 None.
fn relative_neighbor(rects: &[(Uuid, NormRect)], focused: Option<Uuid>, dir: Dir) -> Option<Uuid> {
    let fid = focused?;
    let idx = rects.iter().position(|(id, _)| *id == fid)?;
    neighbor_in_dir(rects, idx, dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 단일 빈 슬롯 트리 + id.
    fn single() -> (LayoutNode, Uuid) {
        let node = LayoutNode::new_empty_slot();
        let id = super::super::tree::first_slot_id(&node);
        (node, id)
    }

    /// id 로 SlotSpatial 조회.
    fn spatial_of(list: &[SlotSpatial], id: Uuid) -> &SlotSpatial {
        list.iter().find(|s| s.slot_id == id).expect("슬롯 있어야")
    }

    // ── 단일 슬롯 ──────────────────────────────────────────────────────────────

    #[test]
    fn single_slot_has_no_neighbors_ordinal_zero() {
        let (node, id) = single();
        let sp = compute_spatial(&node);
        assert_eq!(sp.len(), 1);
        let s = spatial_of(&sp, id);
        assert_eq!(s.ordinal, 0);
        assert_eq!(s.neighbors, Neighbors::default_none());
    }

    // ── 가로 분할(좌/우 이웃) ───────────────────────────────────────────────────

    #[test]
    fn horizontal_split_left_right_neighbors() {
        // Split{Horizontal, a=left, b=right}. left.right == right / right.left == left.
        let (mut node, left) = single();
        let right =
            super::super::tree::split_in_tree(&mut node, left, SplitDir::Horizontal).unwrap();
        let sp = compute_spatial(&node);

        let l = spatial_of(&sp, left);
        let r = spatial_of(&sp, right);
        assert_eq!(
            l.neighbors.right,
            Some(right),
            "왼쪽 슬롯의 오른쪽 = 오른쪽 슬롯"
        );
        assert_eq!(l.neighbors.left, None);
        assert_eq!(l.neighbors.up, None);
        assert_eq!(l.neighbors.down, None);
        assert_eq!(
            r.neighbors.left,
            Some(left),
            "오른쪽 슬롯의 왼쪽 = 왼쪽 슬롯"
        );
        assert_eq!(r.neighbors.right, None);
        // ordinal: 같은 y(center), 왼쪽 먼저 → left=0, right=1.
        assert_eq!(l.ordinal, 0);
        assert_eq!(r.ordinal, 1);
    }

    // ── 세로 분할(위/아래 이웃) ─────────────────────────────────────────────────

    #[test]
    fn vertical_split_up_down_neighbors() {
        // Split{Vertical, a=top, b=bottom}. top.down == bottom / bottom.up == top.
        let (mut node, top) = single();
        let bottom = super::super::tree::split_in_tree(&mut node, top, SplitDir::Vertical).unwrap();
        let sp = compute_spatial(&node);

        let t = spatial_of(&sp, top);
        let b = spatial_of(&sp, bottom);
        assert_eq!(t.neighbors.down, Some(bottom), "위 슬롯의 아래 = 아래 슬롯");
        assert_eq!(t.neighbors.up, None);
        assert_eq!(t.neighbors.left, None);
        assert_eq!(b.neighbors.up, Some(top), "아래 슬롯의 위 = 위 슬롯");
        assert_eq!(b.neighbors.down, None);
        // ordinal: top(center_y 작음)=0, bottom=1.
        assert_eq!(t.ordinal, 0);
        assert_eq!(b.ordinal, 1);
    }

    // ── L-shape(좌측 한 칸 + 우측 상하 2칸) — bottom-right 수용 기준 ────────────────

    /// 트리: Split{Horizontal, a=Slot(left), b=Split{Vertical, a=Slot(rt), b=Slot(rb)}}.
    /// left = 좌측 전체 높이, rt = 우상단, rb = 우하단.
    fn l_shape() -> (LayoutNode, Uuid, Uuid, Uuid) {
        let (mut node, left) = single();
        // 좌측(left) 오른쪽에 새 슬롯(rroot) — Horizontal split.
        let rroot =
            super::super::tree::split_in_tree(&mut node, left, SplitDir::Horizontal).unwrap();
        // 우측(rroot)을 Vertical 로 상하 분할 → rroot=위(rt), 새 슬롯=아래(rb).
        let rb = super::super::tree::split_in_tree(&mut node, rroot, SplitDir::Vertical).unwrap();
        (node, left, rroot, rb)
    }

    #[test]
    fn l_shape_bottom_right_neighbors() {
        let (node, left, rt, rb) = l_shape();
        let sp = compute_spatial(&node);

        // rb(우하단): 위 = rt, 왼쪽 = left(구간 겹침 — left 는 전체 높이라 하단과도 겹침), 아래·오른쪽 없음.
        let s_rb = spatial_of(&sp, rb);
        assert_eq!(s_rb.neighbors.up, Some(rt), "우하단의 위 = 우상단");
        assert_eq!(
            s_rb.neighbors.left,
            Some(left),
            "우하단의 왼쪽 = 좌측(전체높이라 겹침)"
        );
        assert_eq!(s_rb.neighbors.down, None);
        assert_eq!(s_rb.neighbors.right, None);

        // rt(우상단): 아래 = rb, 왼쪽 = left, 위·오른쪽 없음.
        let s_rt = spatial_of(&sp, rt);
        assert_eq!(s_rt.neighbors.down, Some(rb), "우상단의 아래 = 우하단");
        assert_eq!(s_rt.neighbors.left, Some(left), "우상단의 왼쪽 = 좌측");

        // left(좌측 전체): 오른쪽은 rt/rb 둘 다 후보 → 겹침 큰 쪽(둘 다 0.5 로 동일) 중 하나. 존재만 단언.
        let s_left = spatial_of(&sp, left);
        assert!(
            s_left.neighbors.right == Some(rt) || s_left.neighbors.right == Some(rb),
            "좌측의 오른쪽은 우측 두 슬롯 중 하나"
        );
        assert_eq!(s_left.neighbors.left, None);
    }

    #[test]
    fn l_shape_bottom_right_token_resolves_to_rb() {
        // ★ADR-0068 수용 기준★: "우하단"(bottom-right) 토큰이 우하단 슬롯(rb)으로 해소된다.
        let (node, left, rt, rb) = l_shape();
        assert_eq!(
            resolve_spatial(&node, None, SpatialToken::BottomRight),
            Some(rb),
            "bottom-right → 우하단 슬롯"
        );
        assert_eq!(
            resolve_spatial(&node, None, SpatialToken::TopRight),
            Some(rt),
            "top-right → 우상단 슬롯"
        );
        // top-left / bottom-left → 좌측(전체 높이라 위·아래 코너 모두 좌측이 가장 가까움).
        assert_eq!(
            resolve_spatial(&node, None, SpatialToken::TopLeft),
            Some(left),
            "top-left → 좌측 슬롯"
        );
        assert_eq!(
            resolve_spatial(&node, None, SpatialToken::BottomLeft),
            Some(left),
            "bottom-left → 좌측 슬롯"
        );
    }

    #[test]
    fn l_shape_ordinal_reading_order() {
        // ordinal = 중심점 전역 사전순(center_y 우선): left(center_y=0.5)·rt(0.25)·rb(0.75).
        //   정렬 = rt(0.25) → left(0.5) → rb(0.75). ★전체 높이 left 열이 우측 rt/rb 사이에 끼어든다★
        //   — 열 응집이 보장되지 않는다는 산 증거(모듈 헤더 §계산 뼈대의 cohesion 미보장).
        let (node, left, rt, rb) = l_shape();
        let sp = compute_spatial(&node);
        assert_eq!(spatial_of(&sp, rt).ordinal, 0, "rt(위) 먼저");
        assert_eq!(spatial_of(&sp, left).ordinal, 1);
        assert_eq!(spatial_of(&sp, rb).ordinal, 2, "rb(아래) 마지막");
    }

    // ── 상대 방향(포커스 기준) ──────────────────────────────────────────────────

    #[test]
    fn relative_direction_from_focus() {
        let (mut node, left) = single();
        let right =
            super::super::tree::split_in_tree(&mut node, left, SplitDir::Horizontal).unwrap();
        // 포커스 = 왼쪽. right 토큰 → 오른쪽 슬롯.
        assert_eq!(
            resolve_spatial(&node, Some(left), SpatialToken::Right),
            Some(right)
        );
        // 포커스 = 오른쪽. left 토큰 → 왼쪽 슬롯.
        assert_eq!(
            resolve_spatial(&node, Some(right), SpatialToken::Left),
            Some(left)
        );
        // 오른쪽에 오른쪽 이웃 없음 → None.
        assert_eq!(
            resolve_spatial(&node, Some(right), SpatialToken::Right),
            None
        );
        // focused=None → 상대 방향은 None.
        assert_eq!(resolve_spatial(&node, None, SpatialToken::Right), None);
    }

    #[test]
    fn relative_direction_unknown_focus_is_none() {
        let (node, _id) = single();
        assert_eq!(
            resolve_spatial(&node, Some(Uuid::new_v4()), SpatialToken::Left),
            None,
            "트리에 없는 focused → None"
        );
    }

    // ── 극단 ratio 방어(FIX-4 · ADR-0068 §0) ──────────────────────────────────────

    #[test]
    fn degenerate_ratio_produces_no_zero_area_leaf() {
        // ★FIX-4 불변식 검증★: ratio=0.0(극단) 여도 assign_rects 의 `[EPS,1-EPS]` 클램프가
        // zero-area leaf 를 막는다. 클램프가 없으면(`clamp(0.0,1.0)`) a 쪽 leaf 는 w=0 → area=0 이
        // 되어 이 단언이 깨진다(이 테스트는 가드가 살아있어야만 통과 — load-bearing).
        // 중첩 트리로 검증: 바깥 Horizontal(ratio=0) 안에 좌측 열이 Vertical 로 2분할된 형태.
        let (mut node, left) = single();
        let _rroot =
            super::super::tree::split_in_tree(&mut node, left, SplitDir::Horizontal).unwrap();
        // 바깥 Split 의 ratio 를 0.0 으로 강제(극단값 — 정상 경로엔 없지만 미래 resize command 가정).
        if let LayoutNode::Split { ratio, .. } = &mut node {
            *ratio = 0.0;
        } else {
            panic!("split 후 루트는 Split 이어야");
        }

        // 모든 말단 rect 가 비퇴화 면적(w>0 && h>0). 가드 없으면 좌측 leaf area=0 로 실패.
        for (id, r) in leaf_rects(&node) {
            assert!(
                r.w > 0.0 && r.h > 0.0,
                "leaf {id:?} 는 비퇴화 면적이어야 (w={}, h={}) — ratio 클램프 가드",
                r.w,
                r.h
            );
        }

        // 그리고 파생이 패닉 없이 정상 산출된다(슬롯 2개 다 나옴).
        let sp = compute_spatial(&node);
        assert_eq!(sp.len(), 2, "두 슬롯 다 산출(패닉 없음)");
    }

    #[test]
    fn token_parse_kebab_and_aliases() {
        assert_eq!(
            SpatialToken::parse("bottom-right"),
            Some(SpatialToken::BottomRight)
        );
        assert_eq!(
            SpatialToken::parse("BottomRight"),
            Some(SpatialToken::BottomRight)
        );
        assert_eq!(
            SpatialToken::parse(" top-left "),
            Some(SpatialToken::TopLeft)
        );
        assert_eq!(SpatialToken::parse("up"), Some(SpatialToken::Up));
        assert_eq!(SpatialToken::parse("top"), Some(SpatialToken::Up));
        assert_eq!(SpatialToken::parse("nonsense"), None);
    }
}

impl Neighbors {
    /// 이웃 전부 None(단일 슬롯·테스트 헬퍼).
    #[cfg(test)]
    fn default_none() -> Self {
        Neighbors {
            up: None,
            down: None,
            left: None,
            right: None,
        }
    }
}
