//! 슬롯 공간 타깃 파생(ADR-0068) — 레이아웃 트리에서 방향·이웃·순서를 산출하는 순수 로직.
//!
//! ★Tauri 의존 0 · 픽셀 0★: 실측 rect·`getBoundingClientRect`·창 크기를 **모른다**. "우하단"·"이 슬롯
//! 오른쪽" 같은 공간 지시를 slot id 로 옮기는 근거를 논리 도면(트리 구조)만으로 계산한다 → 단독 headless
//! 테스트 가능(ADR-0012 격리).
//!
//! ## 계산 뼈대
//! 1. 말단 슬롯의 정규화 사각형은 `geometry::compute` 의 경계 꼴을 그대로 받는다 — 셸 안의 기하 출처는
//!    그 하나다(ADR-0227).
//! 2. 그 사각형들에서 **모서리 인접(neighbor)** 과 **순서(ordinal)** 를 파생한다.
//!    - neighbor: 두 슬롯이 해당 축에서 맞닿고(경계 좌표가 정확히 같다) 직교축 구간 겹침이 양수면 인접.
//!    - ordinal: 각 말단 rect 의 **중심점** `(center_y, center_x)` 사전순 GLOBAL 정렬(위→아래,
//!      동률이면 왼쪽→오른쪽) 0-based. ★트리 전위(pre-order)가 아니라 전역 중심 정렬★ — 트리 구조가
//!      아니라 화면상 위치로 매긴다. 비교는 `f64::total_cmp` 전순서이고, 중심점까지 같으면 트리 전위 순을
//!      유지한다(안정 정렬) → 결정적(deterministic).
//!      단 열/행 응집(cohesion)은 보장하지 않는다: 비대칭 분할에선 전체 높이 한 열(column)이 좌측 열
//!      슬롯들 사이에 끼어들 수 있다(center_y 로만 순서를 매기므로).
//!    - 면적 0 잎(`x0 >= x1` 또는 `y0 >= y1` — f64 경계가 무너질 만큼 깊은 트리에서만 생긴다)은 이웃을
//!      갖지도 되지도 않고 모서리 토큰 후보에서도 빠진다. 순서는 같은 규칙으로 매겨 명단에서 빠지지 않는다.
//!
//! ★공개 표면은 `neighbors`+`ordinal` 뿐이다★(ADR-0068): 사각형은 이 계산의 입력일 뿐 이 모듈이
//! 내보내지 않는다.

use uuid::Uuid;

use super::geometry::{self, SlotRect};
use super::types::LayoutNode;

// ADR-0227
fn has_area(r: &SlotRect) -> bool {
    r.x0 < r.x1 && r.y0 < r.y1
}

fn center_x(r: &SlotRect) -> f64 {
    (r.x0 + r.x1) / 2.0
}

fn center_y(r: &SlotRect) -> f64 {
    (r.y0 + r.y1) / 2.0
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
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct SlotSpatial {
    #[ts(type = "string")]
    pub slot_id: Uuid,
    pub neighbors: Neighbors,
    /// 순서 인덱스 — 중심점 `(center_y, center_x)` 전역 사전순(위→아래, 동률 왼쪽→오른쪽) 0-based.
    /// 상세·응집 미보장은 모듈 헤더 §계산 뼈대 참조.
    #[ts(type = "number")]
    pub ordinal: u32,
}

/// 여러 후보가 있으면(예: 오른쪽에 두 슬롯이 세로로 쌓임) 직교축 겹침이 가장 큰 것을 고른다
/// (대표 이웃 하나 — 방향 이동의 자연스러운 타깃).
fn neighbor_in_dir(rects: &[SlotRect], idx: usize, dir: Dir) -> Option<Uuid> {
    let me = &rects[idx];
    if !has_area(me) {
        return None;
    }
    let mut best: Option<(Uuid, f64)> = None;
    for (j, other) in rects.iter().enumerate() {
        if j == idx || !has_area(other) {
            continue;
        }
        // ADR-0227: 경계는 정확히(`==`) 비교한다 — 맞닿는 두 칸의 경계는 언제나 어느 한 분할의 `at`
        // 하나이고 geometry 가 그 값을 양쪽 자손에 복사하므로, 참 인접은 칸 크기와 무관하게 비트 단위로 같다.
        // 절대 허용오차로 되돌리지 말 것 — 폭이 허용오차보다 얇은 칸이 생기면 그 너머 칸이 인접으로
        // 오판돼 얇은 칸을 건너뛰고, 얇은 칸과의 직교축 겹침이 임계에 걸려 이웃이 사라진다(패닉 없이
        // 조용히 틀린다).
        // 모서리만 닿는 칸도 두 끝점이 서로 다른 분할 경로에서 따로 계산되면 몇 ulp(경로가 깊을수록
        // 커진다) 겹침으로 후보에 들 수 있다. 내 변의 반대편은 잎들이 빈틈없이 덮으므로 겹침이 그보다 훨씬
        // 큰 참 이웃이 있어 아래 겹침 최대 규칙에서 밀린다 — 단 내 칸 자체가 몇 ulp 두께로 무너지기
        // 직전이면 이 여유가 사라져 동률·역전이 날 수 있다.
        let (adjacent, overlap) = match dir {
            Dir::Right => (
                me.x1 == other.x0,
                overlap_len(me.y0, me.y1, other.y0, other.y1),
            ),
            Dir::Left => (
                me.x0 == other.x1,
                overlap_len(me.y0, me.y1, other.y0, other.y1),
            ),
            Dir::Down => (
                me.y1 == other.y0,
                overlap_len(me.x0, me.x1, other.x0, other.x1),
            ),
            Dir::Up => (
                me.y0 == other.y1,
                overlap_len(me.x0, me.x1, other.x0, other.x1),
            ),
        };
        if adjacent && overlap > 0.0 {
            match best {
                Some((_, bo)) if bo >= overlap => {}
                _ => best = Some((other.slot_id, overlap)),
            }
        }
    }
    best.map(|(id, _)| id)
}

/// 두 구간의 겹치는 길이(떨어져 있거나 끝점만 닿으면 0 이하 — 호출측 `> 0.0` 이 걸러낸다).
fn overlap_len(a0: f64, a1: f64, b0: f64, b1: f64) -> f64 {
    a1.min(b1) - a0.max(b0)
}

#[derive(Debug, Clone, Copy)]
enum Dir {
    Up,
    Down,
    Left,
    Right,
}

/// 반환 순서 = ordinal 순.
pub fn compute_spatial(node: &LayoutNode) -> Vec<SlotSpatial> {
    let rects = geometry::compute(node).slots;

    let mut order: Vec<usize> = (0..rects.len()).collect();
    order.sort_by(|&i, &j| {
        let (a, b) = (&rects[i], &rects[j]);
        center_y(a)
            .total_cmp(&center_y(b))
            .then_with(|| center_x(a).total_cmp(&center_x(b)))
    });

    order
        .iter()
        .enumerate()
        .map(|(ord, &i)| SlotSpatial {
            slot_id: rects[i].slot_id,
            neighbors: Neighbors {
                up: neighbor_in_dir(&rects, i, Dir::Up),
                down: neighbor_in_dir(&rects, i, Dir::Down),
                left: neighbor_in_dir(&rects, i, Dir::Left),
                right: neighbor_in_dir(&rects, i, Dir::Right),
            },
            ordinal: ord as u32,
        })
        .collect()
}

/// 공간/방향 토큰 — LLM/사람이 "우하단"·"이 슬롯 오른쪽" 같은 지시를 넘기는 심볼릭 표면(ADR-0068).
/// tmux 결 edge 토큰(모서리 4종) + 포커스 슬롯 상대 방향(4종).
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

/// ★edge 토큰이 "우하단"을 정확히 집는가(ADR-0068 수용 기준)★: 각 슬롯 rect 의 해당 코너 좌표까지의
/// 거리를 최소화한다 — L-shape(좌측 한 칸 + 우측 상하 2칸)에서 bottom-right 는 우측 아래 슬롯을 집는다.
pub fn resolve_spatial(
    node: &LayoutNode,
    focused: Option<Uuid>,
    token: SpatialToken,
) -> Option<Uuid> {
    let rects = geometry::compute(node).slots;
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
/// 대표점으로 삼아(예: bottom-right → rect 의 (right,bottom)) 코너까지 유클리드 거리 최소화. 거리가 같으면
/// 먼저 본(트리 전위 순) 슬롯. 면적 0 잎은 후보가 아니다(이웃 제외와 같은 규칙).
fn corner_slot(rects: &[SlotRect], cx: f64, cy: f64) -> Option<Uuid> {
    let mut best: Option<(Uuid, f64)> = None;
    for r in rects.iter().filter(|r| has_area(r)) {
        let px = if cx >= 0.5 { r.x1 } else { r.x0 };
        let py = if cy >= 0.5 { r.y1 } else { r.y0 };
        let d = (px - cx).powi(2) + (py - cy).powi(2);
        match best {
            Some((_, bd)) if bd <= d => {}
            _ => best = Some((r.slot_id, d)),
        }
    }
    best.map(|(id, _)| id)
}

/// focused 슬롯의 방향 이웃(공유 변). focused 가 트리에 없거나 None 이면 None.
fn relative_neighbor(rects: &[SlotRect], focused: Option<Uuid>, dir: Dir) -> Option<Uuid> {
    let fid = focused?;
    let idx = rects.iter().position(|r| r.slot_id == fid)?;
    neighbor_in_dir(rects, idx, dir)
}

#[cfg(test)]
mod tests {
    use super::super::types::{SlotContent, SplitDir};
    use super::*;

    fn single() -> (LayoutNode, Uuid) {
        let node = LayoutNode::new_empty_slot();
        let id = super::super::tree::first_slot_id(&node);
        (node, id)
    }

    fn spatial_of(list: &[SlotSpatial], id: Uuid) -> &SlotSpatial {
        list.iter().find(|s| s.slot_id == id).expect("슬롯 있어야")
    }

    // `split_in_tree` 는 늘 0.5 로 나누므로, 비율·모양을 직접 정한 트리는 이 도우미로 짓는다.
    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn slot(n: u128) -> LayoutNode {
        LayoutNode::Slot {
            id: id(n),
            content: SlotContent::Empty,
        }
    }

    fn split(n: u128, dir: SplitDir, ratio: f64, a: LayoutNode, b: LayoutNode) -> LayoutNode {
        LayoutNode::Split {
            id: id(n),
            dir,
            ratio,
            a: Box::new(a),
            b: Box::new(b),
        }
    }

    fn rect_of(node: &LayoutNode, n: u128) -> SlotRect {
        geometry::compute(node)
            .slots
            .into_iter()
            .find(|s| s.slot_id == id(n))
            .expect("슬롯 있어야")
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

    // ── LeftRight 분할(좌/우 이웃) ───────────────────────────────────────────────

    #[test]
    fn left_right_split_has_left_right_neighbors() {
        let (mut node, left) = single();
        let right =
            super::super::tree::split_in_tree(&mut node, left, SplitDir::LeftRight).unwrap();
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
        assert_eq!(l.ordinal, 0);
        assert_eq!(r.ordinal, 1);
    }

    // ── TopBottom 분할(위/아래 이웃) ─────────────────────────────────────────────

    #[test]
    fn top_bottom_split_has_up_down_neighbors() {
        let (mut node, top) = single();
        let bottom =
            super::super::tree::split_in_tree(&mut node, top, SplitDir::TopBottom).unwrap();
        let sp = compute_spatial(&node);

        let t = spatial_of(&sp, top);
        let b = spatial_of(&sp, bottom);
        assert_eq!(t.neighbors.down, Some(bottom), "위 슬롯의 아래 = 아래 슬롯");
        assert_eq!(t.neighbors.up, None);
        assert_eq!(t.neighbors.left, None);
        assert_eq!(b.neighbors.up, Some(top), "아래 슬롯의 위 = 위 슬롯");
        assert_eq!(b.neighbors.down, None);
        assert_eq!(t.ordinal, 0);
        assert_eq!(b.ordinal, 1);
    }

    // ── L-shape(좌측 한 칸 + 우측 상하 2칸) — bottom-right 수용 기준 ────────────────

    /// 트리: Split{LeftRight, a=Slot(left), b=Split{TopBottom, a=Slot(rt), b=Slot(rb)}}.
    /// left = 좌측 전체 높이, rt = 우상단, rb = 우하단.
    fn l_shape() -> (LayoutNode, Uuid, Uuid, Uuid) {
        let (mut node, left) = single();
        let rroot =
            super::super::tree::split_in_tree(&mut node, left, SplitDir::LeftRight).unwrap();
        let rb = super::super::tree::split_in_tree(&mut node, rroot, SplitDir::TopBottom).unwrap();
        (node, left, rroot, rb)
    }

    #[test]
    fn l_shape_bottom_right_neighbors() {
        let (node, left, rt, rb) = l_shape();
        let sp = compute_spatial(&node);

        let s_rb = spatial_of(&sp, rb);
        assert_eq!(s_rb.neighbors.up, Some(rt), "우하단의 위 = 우상단");
        assert_eq!(
            s_rb.neighbors.left,
            Some(left),
            "우하단의 왼쪽 = 좌측(전체높이라 겹침)"
        );
        assert_eq!(s_rb.neighbors.down, None);
        assert_eq!(s_rb.neighbors.right, None);

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
            super::super::tree::split_in_tree(&mut node, left, SplitDir::LeftRight).unwrap();
        assert_eq!(
            resolve_spatial(&node, Some(left), SpatialToken::Right),
            Some(right)
        );
        assert_eq!(
            resolve_spatial(&node, Some(right), SpatialToken::Left),
            Some(left)
        );
        assert_eq!(
            resolve_spatial(&node, Some(right), SpatialToken::Right),
            None
        );
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
        // ★FIX-4 불변식 검증★: ratio=0.0(극단) 여도 geometry 의 비율 클램프(`[RATIO_MIN, RATIO_MAX]`)가
        // zero-area leaf 를 막는다. 클램프가 없으면 a 쪽 leaf 는 폭 0 이 되어 이 단언이 깨지고, 면적 0 잎은
        // 이웃에서 빠지므로 아래 이웃 단언도 깨진다(이 테스트는 가드가 살아있어야만 통과 — load-bearing).
        let (mut node, left) = single();
        let right =
            super::super::tree::split_in_tree(&mut node, left, SplitDir::LeftRight).unwrap();
        // 극단값 — 쓰기 경로로는 안 들어오는 값을 트리에 직접 심는다.
        if let LayoutNode::Split { ratio, .. } = &mut node {
            *ratio = 0.0;
        } else {
            panic!("split 후 루트는 Split 이어야");
        }

        for r in geometry::compute(&node).slots {
            assert!(
                has_area(&r),
                "leaf {:?} 는 비퇴화 면적이어야 ({r:?}) — ratio 클램프 가드",
                r.slot_id
            );
        }

        let sp = compute_spatial(&node);
        assert_eq!(sp.len(), 2, "두 슬롯 다 산출(패닉 없음)");
        assert_eq!(spatial_of(&sp, left).neighbors.right, Some(right));
        assert_eq!(spatial_of(&sp, right).neighbors.left, Some(left));
    }

    // ── 아주 얇은 잎(정확 인접 — 절대 허용오차 없음) ─────────────────────────────────

    /// 위 절반 = 0.1 LR 사슬 다섯 단(조상 100 + 그 안 네 단)이라 맨 안쪽 칸 폭이 1e-5 이고, 그 칸을 다시
    /// 반으로 나눈다(1·2). 3 = 폭 9e-5, 4·5·6·7 = 그 오른쪽으로 점점 넓은 칸. 아래 절반 = 전체 폭 한 칸(8).
    fn thin_tree() -> LayoutNode {
        let lr = SplitDir::LeftRight;
        let chain = split(
            100,
            lr,
            0.1,
            split(
                101,
                lr,
                0.1,
                split(
                    102,
                    lr,
                    0.1,
                    split(
                        103,
                        lr,
                        0.1,
                        split(104, lr, 0.1, split(105, lr, 0.5, slot(1), slot(2)), slot(3)),
                        slot(4),
                    ),
                    slot(5),
                ),
                slot(6),
            ),
            slot(7),
        );
        split(106, SplitDir::TopBottom, 0.5, chain, slot(8))
    }

    #[test]
    fn thin_leaves_keep_exact_neighbors() {
        let tree = thin_tree();
        for n in [1, 2, 3] {
            let r = rect_of(&tree, n);
            assert!(r.x1 - r.x0 < 1e-4, "잎 {n} 폭이 1e-4 아래여야 — {r:?}");
        }

        let sp = compute_spatial(&tree);
        let nb = |n| spatial_of(&sp, id(n)).neighbors;
        let some = |n| Some(id(n));
        assert_eq!(
            nb(1),
            Neighbors {
                up: None,
                down: some(8),
                left: None,
                right: some(2),
            }
        );
        assert_eq!(
            nb(2),
            Neighbors {
                up: None,
                down: some(8),
                left: some(1),
                right: some(3),
            },
            "얇은 칸 너머(3)로 건너뛰지 않고, 아래 칸과의 겹침(5e-6)도 잃지 않는다"
        );
        assert_eq!(
            nb(3),
            Neighbors {
                up: None,
                down: some(8),
                left: some(2),
                right: some(4),
            }
        );
        assert_eq!(nb(4).left, some(3));
        assert_eq!(nb(8).up, some(7), "겹침 최대 = 가장 넓은 칸");

        // 위 줄은 center_y 가 같아 center_x 로만 갈린다 — 1e-6 단위 차이에서도 왼쪽→오른쪽.
        let order: Vec<Uuid> = sp.iter().map(|s| s.slot_id).collect();
        assert_eq!(order, (1..=8).map(id).collect::<Vec<_>>());
    }

    // ── 모서리 접촉 ─────────────────────────────────────────────────────────────

    #[test]
    fn corner_only_touch_is_not_a_neighbor() {
        // 2×2 격자 — 대각선 두 칸은 모서리 한 점만 닿는다(직교축 겹침 정확히 0). 두 루트 방향 모두.
        for (root, row) in [
            (SplitDir::TopBottom, SplitDir::LeftRight),
            (SplitDir::LeftRight, SplitDir::TopBottom),
        ] {
            // 1 = 원점 칸, 4 = 그 대각선. 2 는 1 의 옆(같은 루트 반쪽), 3 은 1 의 루트 반대편.
            let grid = split(
                100,
                root,
                0.5,
                split(101, row, 0.5, slot(1), slot(2)),
                split(102, row, 0.5, slot(3), slot(4)),
            );
            let sp = compute_spatial(&grid);
            for s in &sp {
                let n = s.neighbors;
                let all = [n.up, n.down, n.left, n.right];
                let diagonal = match s.slot_id {
                    x if x == id(1) => id(4),
                    x if x == id(4) => id(1),
                    x if x == id(2) => id(3),
                    _ => id(2),
                };
                assert!(
                    !all.contains(&Some(diagonal)),
                    "{root:?}: {:?} 의 대각선 {diagonal:?} 은 이웃이 아니다",
                    s.slot_id
                );
                assert_eq!(
                    all.iter().filter(|x| x.is_some()).count(),
                    2,
                    "{root:?}: 격자 칸마다 변을 맞댄 이웃은 정확히 둘"
                );
            }
        }
    }

    /// 왼쪽 열은 0.65 에서, 오른쪽 열은 0.3 + (1 − 0.3)·0.5 에서 가로로 나뉜다 — 십진으로는 같은 높이지만
    /// 따로 계산돼 오른쪽 경계가 1 ulp 낮다. 1 = 왼쪽 위 · 2 = 왼쪽 아래 · 3·4·5 = 오른쪽 위·가운데·아래.
    fn ulp_tree() -> LayoutNode {
        split(
            100,
            SplitDir::LeftRight,
            0.5,
            split(101, SplitDir::TopBottom, 0.65, slot(1), slot(2)),
            split(
                102,
                SplitDir::TopBottom,
                0.3,
                slot(3),
                split(103, SplitDir::TopBottom, 0.5, slot(4), slot(5)),
            ),
        )
    }

    #[test]
    fn one_ulp_corner_candidate_loses_to_true_neighbor() {
        let tree = ulp_tree();
        let (lt, lb, rb) = (rect_of(&tree, 1), rect_of(&tree, 2), rect_of(&tree, 5));
        // 전제를 실측한다 — 1 ulp 어긋남이 없으면 이 테스트는 아무것도 재지 않는다.
        assert_eq!(lt.x1, rb.x0, "같은 루트 경계 — 인접 판정을 통과한다");
        assert_eq!(
            lt.y1.to_bits() - rb.y0.to_bits(),
            1,
            "5 의 위 경계가 1 의 아래 경계보다 정확히 1 ulp 낮다 — 모서리 접촉이 겹침 양수로 보인다"
        );
        assert!(rb.y0 < lt.y1 && lb.y1 > rb.y0);

        let sp = compute_spatial(&tree);
        // 5 의 왼쪽 후보는 전위 순으로 1(겹침 1 ulp, 거짓)이 먼저, 2(참)가 나중이다 — 먼저 본 후보를
        // 고르는 규칙이었다면 1 이 뽑힌다.
        assert_eq!(
            spatial_of(&sp, id(5)).neighbors.left,
            Some(id(2)),
            "겹침 최대 규칙 — 1 ulp 후보가 아니라 참 이웃"
        );
        // 1 쪽에서도 5 는 1 ulp 후보일 뿐 — 오른쪽 이웃은 겹침이 가장 큰 4.
        assert_eq!(spatial_of(&sp, id(1)).neighbors.right, Some(id(4)));
        assert_eq!(spatial_of(&sp, id(2)).neighbors.right, Some(id(5)));
    }

    // ── 면적 0 잎(트리에 직접 심은 값 — 방어) ───────────────────────────────────────

    /// 0.9 로 늘 b 쪽을 나눈 LR 사슬 — 17 단째에서 경계가 반올림으로 1.0 에 붙어 마지막 b 잎(18)의 폭이
    /// 정확히 0 이 된다. 잎 = 1..=17(각 단의 a) + 18.
    fn collapsed_chain() -> LayoutNode {
        const DEPTH: u128 = 17;
        let mut node = slot(DEPTH + 1);
        for k in (1..=DEPTH).rev() {
            node = split(100 + k, SplitDir::LeftRight, 0.9, slot(k), node);
        }
        node
    }

    #[test]
    fn zero_area_leaf_is_isolated_but_ordered() {
        let tree = collapsed_chain();
        let g = geometry::compute(&tree);
        let z = rect_of(&tree, 18);
        assert_eq!(z.x0, z.x1, "18 은 폭 0 이어야 — {z:?}");
        assert_eq!(
            g.slots.iter().filter(|r| !has_area(r)).count(),
            1,
            "면적 0 잎은 18 하나뿐"
        );
        // 17 과 18 은 경계 1.0 을 정확히 공유한다 — 건너뛰기가 없으면 서로 이웃이 된다.
        assert_eq!(rect_of(&tree, 17).x1, z.x0);

        let sp = compute_spatial(&tree);
        assert_eq!(sp.len(), 18, "면적 0 잎도 명단에 있다(패닉 없음)");
        assert_eq!(spatial_of(&sp, id(18)).neighbors, Neighbors::default_none());
        for s in &sp {
            let n = s.neighbors;
            assert!(
                ![n.up, n.down, n.left, n.right].contains(&Some(id(18))),
                "{:?} 의 이웃에 면적 0 잎이 들면 안 된다",
                s.slot_id
            );
        }
        assert_eq!(spatial_of(&sp, id(17)).neighbors.right, None);
        assert_eq!(spatial_of(&sp, id(17)).neighbors.left, Some(id(16)));

        // 17 과 18 은 중심점까지 같다 — 동률은 트리 전위 순(17 먼저)이라 순서가 전위 순과 같다.
        let r17 = rect_of(&tree, 17);
        assert_eq!(
            (center_y(&r17), center_x(&r17)),
            (center_y(&z), center_x(&z))
        );
        let order: Vec<Uuid> = sp.iter().map(|s| s.slot_id).collect();
        assert_eq!(order, (1..=18).map(id).collect::<Vec<_>>());

        assert_eq!(compute_spatial(&tree), sp, "두 번 계산해 같은 결과");
    }

    #[test]
    fn corner_token_skips_zero_area_leaf() {
        // 모서리 토큰은 거리 동률이면 전위 순으로 먼저 본 칸을 고르므로, 면적 0 잎이 모서리를 가져가려면
        // 전위 순 맨 앞 — 원점 쪽 a 칸 — 이어야 한다. 원점 쪽 경계는 반올림으로 안 붙고 언더플로로만 0 이
        // 되므로 0.1 LR 사슬을 324 단 쌓는다. z(1000) = 맨 안쪽 a 잎 · k = k 단째 b 잎.
        const DEPTH: u128 = 324;
        let lr = SplitDir::LeftRight;
        let mut node = split(10_000 + DEPTH, lr, 0.1, slot(1000), slot(DEPTH));
        for k in (1..DEPTH).rev() {
            node = split(10_000 + k, lr, 0.1, node, slot(k));
        }

        let g = geometry::compute(&node);
        let z = rect_of(&node, 1000);
        assert_eq!((z.x0, z.x1), (0.0, 0.0), "z 는 원점에서 폭 0 이어야");
        assert_eq!(g.slots[0].slot_id, id(1000), "z 가 전위 순 맨 앞");
        assert_eq!(
            g.slots.iter().filter(|r| !has_area(r)).count(),
            1,
            "면적 0 잎은 z 하나뿐"
        );

        for token in [SpatialToken::TopLeft, SpatialToken::BottomLeft] {
            assert_eq!(
                resolve_spatial(&node, None, token),
                Some(id(DEPTH)),
                "{token:?} → z 를 건너뛴 다음 칸"
            );
        }
        assert_eq!(
            resolve_spatial(&node, None, SpatialToken::TopRight),
            Some(id(1))
        );

        let sp = compute_spatial(&node);
        assert_eq!(sp.len(), DEPTH as usize + 1);
        assert_eq!(
            spatial_of(&sp, id(1000)).neighbors,
            Neighbors::default_none()
        );
        let inner = spatial_of(&sp, id(DEPTH)).neighbors;
        assert_eq!(inner.left, None, "z 는 누구의 이웃도 아니다");
        assert_eq!(inner.right, Some(id(DEPTH - 1)));
        assert_eq!(compute_spatial(&node), sp, "두 번 계산해 같은 결과");
    }

    // ── 결정성 ─────────────────────────────────────────────────────────────────

    #[test]
    fn ordering_is_deterministic_and_complete() {
        let nan = split(
            100,
            SplitDir::TopBottom,
            f64::NAN,
            slot(1),
            split(101, SplitDir::LeftRight, f64::NAN, slot(2), slot(3)),
        );
        for tree in [thin_tree(), ulp_tree(), collapsed_chain(), nan] {
            let first = compute_spatial(&tree);
            assert_eq!(compute_spatial(&tree), first, "같은 트리 = 같은 결과");
            let ordinals: Vec<u32> = first.iter().map(|s| s.ordinal).collect();
            assert_eq!(ordinals, (0..first.len() as u32).collect::<Vec<_>>());
            assert_eq!(
                first.len(),
                geometry::compute(&tree).slots.len(),
                "모든 잎이 명단에"
            );
        }
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
