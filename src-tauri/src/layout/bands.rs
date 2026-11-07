//! The source of truth for docking band geometry (TRD S21-slot-docking §4-5) — the tab bar zone over one window's canvas · outer bands · area bands · their effective regions ·
//! the attach share · the attach rectangle (preview) · picking a band for a point · area zone lines. Coordinates = CSS px of that window's canvas (origin at the canvas top-left) ·
//! bands · regions · attach rectangles are half-open integer rectangles.
//!
//! ★It is one set with the web mirror★ — during a drag, the drag does not match its decision with the shell (ADR-0296 · ADR-0297), so the web (`dock/bands.ts` ·
//! `dockedTree.ts` · `judge.ts`) counts the same rule alone. The two consume the golden `bands_golden.json` together — if the rule
//! is changed, recapture the golden and fix the TS version in the same change (if only one side is fixed, the other turns red). The numbers are the one set of
//! `pub const` at the head of this file and the web reads them from the generated `src-tauri/bindings/DockConstants.ts` — the same numbers are not
//! written again inside this file's body.
//!
//! [`choose`] points only at the effective regions (`BandHit::areas`). The preview is the chosen band's `landing`, and the diagonals of the area zone lines
//! ([`slot_zones`]) are the same lines as the [`choose`] boundary between two neighboring area bands. The attach rectangle and the drop result pass through the same
//! `docked_tree` with the same share (`BandHit::share`) (PRD R4 · R7) — the drop carries that `share` on `Seam` as it is and
//! does not compute it again.
//!
//! Precedence order (R6a) = tab bar zone > outer band > area band. A blocked band (R8 — dropping would create a new zero-area area) is treated as absent —
//! it is not chosen, and a blocked outer band does not cover the area band beneath (AC36). An area band stands not on the area frame but on that area's band frame ([`band_frame`] —
//! the area frame minus the share of valid outer bands) — so even an area touching a canvas edge keeps the area band of that edge.
//!
//! Entry points: [`strip_zone`] → [`zone_in_canvas`] (tab bar zone) · [`effective_bands`] (all effective bands of one window) ·
//! [`bands_at`] (outer bands + the bands of the area under the point) → [`choose`] · [`slot_zones`] (the area zone lines to draw from a point).

use ts_rs::TS;
use uuid::Uuid;

use super::dock::{docked_tree, incoming_rect, DockAnchor, DockSide};
use super::geometry::{self as layout_geometry, px_edge, PxRect};
use super::{tree, CanvasPx, LayoutNode};

/// Area band thickness = 10% of the area length (ADR-0295 decision 3 · PRD R3 — the 0.1 form of VS Code `editorDropTarget.ts`).
// ADR-0295
pub const BAND_SHARE: f64 = 0.10;

/// Minimum thickness of an area band (CSS px) — the thinnest band that can be pointed at with a pointer (G5 — the top edge of the peer Lumino root). Even in an area of width 50
/// (AC39), the left and right bands do not exceed half the area.
// ADR-0295
pub const BAND_MIN_PX: u32 = 12;

/// Outer band thickness (CSS px — PRD R5). It must be larger than [`STRIP_ZONE_PX`] so that the effective part of the outer top band remains right below the tab bar zone
/// (AC44 — assertion below). 20 = user decision 2026-10-19 (「바깥은 트리거 되는 영역을 좀 작게」) — if the tab bar is directly
/// above the canvas, the outer top band left under the tab bar is 5 px.
pub const OUTER_BAND_PX: u32 = 20;

/// Tab bar zone = the width (CSS px) by which the base rectangle ([`strip_zone`]'s `area` — decided by the webview) is widened up and down — Chrome
/// `kVerticalDetachMagnetism` (TDC#401 · survey A5).
/// The touch value (50) is not used — only the mouse is handled.
// ADR-0295
pub const STRIP_ZONE_PX: f64 = 15.0;

/// Upper bound of the attach share — even if the original size is large, up to half of the area attached to (the canvas for an outer band) (ADR-0295 decision 4 · PRD R7).
// ADR-0295
pub const ATTACH_SHARE_MAX: f64 = 0.5;

/// Chrome `kMaximizedWindowInset` (TDC#129) — a new window is made smaller than the work area by this much (DIP) on each side so that it is not
/// maximized as soon as it is made (`drag_size::dragged_size`).
pub const MAXIMIZED_INSET_DIP: f64 = 10.0;

/// Tie-break order — for both outer band and area band corners (R6a ③).
pub const SIDE_ORDER: [DockSide; 4] = [
    DockSide::Left,
    DockSide::Right,
    DockSide::Top,
    DockSide::Bottom,
];

const _: () = assert!(OUTER_BAND_PX as f64 > STRIP_ZONE_PX);

// If the upper bound is raised above the model's upper bound, the preview is drawn but the drop is rejected.
const _: () = assert!(ATTACH_SHARE_MAX <= crate::layout::dock::SHARE_MAX);

/// Size — logical (DIP) px (fractions allowed) · the same as CSS px at webview zoom 1.
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct SizeCss {
    pub w: f64,
    pub h: f64,
}

/// Point — CSS px (fractions allowed). The origin is decided by the place of use: the webview wire (`TabDrop`, etc.) is that window's viewport top-left · band decisions
/// are that window's canvas top-left.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct PointCss {
    pub x: f64,
    pub y: f64,
}

/// Rectangle — in CSS px bounds form (left · top · right · bottom of `getBoundingClientRect` · like [`PointCss`], the origin is decided by the
/// place of use). It must satisfy `x0 <= x1` · `y0 <= y1`. It is read as half-open (left · top included, right · bottom excluded)
/// ([`contains`]).
// No ts export is attached — the web uses the TS-native type in `dock/overlayTypes.ts` and the two versions are bound by the golden (TRD §5-4).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RectCss {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

/// One band — the chosen band's decision · drawing · drop material are all here.
#[derive(Debug, Clone, PartialEq)]
pub struct BandHit {
    pub anchor: DockAnchor,
    pub side: DockSide,
    /// The whole band rectangle (before subtracting the preceding regions) — its `side` edge is the same as the area band frame ([`band_frame`] · the canvas for an outer band) edge.
    /// Material for the normalized distance of [`choose`] (distance to the edge ÷ thickness).
    pub band: PxRect,
    /// Attach share ([`attach_share`]) — `landing` was computed with this value and the drop's `Seam.share` is also this value.
    pub share: f64,
    /// The rectangle the inserted tab will occupy if dropped (preview — R4).
    pub landing: PxRect,
    /// Effective region — the list of rectangles that remain after subtracting preceding regions from the band (never empty · never overlapping each other). Where [`choose`] points.
    pub areas: Vec<PxRect>,
}

/// Input for one window.
pub struct WindowTargets<'a> {
    pub canvas: CanvasPx,
    /// The layout of the tab visible in that window.
    pub layout: &'a LayoutNode,
    /// The layout of the dragged tab.
    pub incoming: &'a LayoutNode,
    /// The original size of the dragged tab (R7) — if unknown (the size field is absent or broken), put NaN: the share becomes the upper bound ([`ATTACH_SHARE_MAX`]).
    pub original: SizeCss,
    /// That window's tab bar zone — the value of [`zone_in_canvas`] (canvas coordinates · integer bounds · may be negative upward). It must equal the
    /// bounds the tab bar decision used. `None` = the tab bar is unknown (it is not subtracted from the bands).
    pub zone: Option<RectCss>,
}

/// The zone lines of one area (canvas CSS px) — what to draw = the `centre` border + four lines joining the same-side corners of `slot` and `centre`
/// (the boundaries of two neighboring area bands). `centre` is inside `slot` and its width or height may be 0 (a narrow area — a line segment).
// No ts export is attached — the web uses the TS-native type in `dock/overlayTypes.ts` and the two versions are bound by the golden (TRD §5-4 —
// the number of generated bindings is not increased).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SlotZones {
    /// The frame on which the area bands stand ([`band_frame`]) — for an area touching a canvas edge it is smaller than the area frame (that share is an outer band, so no line is drawn).
    pub slot: PxRect,
    /// The center that falls in none of the area bands ([`slot_centre`]).
    pub centre: RectCss,
}

/// Tab bar zone — `area` (base rectangle · viewport CSS px) as it is horizontally · vertically up and down by [`STRIP_ZONE_PX`]. The base is decided by the webview
/// (`tabStrip/measure.ts` `stripZoneArea`) — horizontal = from the left of the first head ~ the right end of `+` (user decision 2026-10-26 · T-66 — only where tabs are is
/// the tab bar zone · TRD S26-window-chrome §7-9) × vertical = the title row if the tab bar is inside the title row · otherwise the tab bar row. The vertical
/// decision is exactly Chrome's `DoesTabStripContain` (TDC#1461-1477 · `DoesRectContainVerticalPointExpanded` TDC#234).
// ADR-0295
pub fn strip_zone(area: RectCss) -> RectCss {
    RectCss {
        x0: area.x0,
        y0: area.y0 - STRIP_ZONE_PX,
        x1: area.x1,
        y1: area.y1 + STRIP_ZONE_PX,
    }
}

/// Half-open decision (left · top included) — even if neighboring rectangles share the same boundary, a point falls on only one side. A NaN point is nowhere.
pub fn contains(r: &RectCss, p: PointCss) -> bool {
    (r.x0..r.x1).contains(&p.x) && (r.y0..r.y1).contains(&p.y)
}

/// The tab bar zone (that window's viewport CSS — [`strip_zone`]) moved to canvas coordinates and widened outward to integer bounds. The tab bar decision ·
/// subtracting from the bands both use this one boundary. The widened sub-1 px belongs to the tab bar side.
// If fractional bounds (a tab bar at 125% scale, etc.) are used as they are for the tab bar decision and as integers for the bands, the line between them becomes neither tab bar nor band
// (e.g. — zone end 43.125 · cursor 43.2 is over the outer top band but nothing).
// ADR-0295
pub fn zone_in_canvas(zone: RectCss, canvas_origin: PointCss) -> RectCss {
    RectCss {
        x0: (zone.x0 - canvas_origin.x).floor(),
        y0: (zone.y0 - canvas_origin.y).floor(),
        x1: (zone.x1 - canvas_origin.x).ceil(),
        y1: (zone.y1 - canvas_origin.y).ceil(),
    }
}

/// Attach share — the share that the new one occupies of the space it wraps when attached to the `side` of area (canvas for an outer band) `target`. With the
/// original length `L` in the splitting direction and area length `T`, `min(L, T ÷ 2) ÷ T` clamped to `[RATIO_MIN, ATTACH_SHARE_MAX]` (ADR-0295 decision 4 ·
/// PRD R7).
///
/// ★For any input it is a finite value in `[tree::RATIO_MIN, ATTACH_SHARE_MAX]`★ — the preview ([`landing_rect`]) and the drop
/// (`Seam.share`) pass this same value to `docked_tree`, and the drop side rejects out-of-range with `DockError::InvalidShare`.
/// So if the area length is 0, the upper bound · if `L` is NaN or +∞, the upper bound · if 0 or less, the lower bound.
// ADR-0295
pub fn attach_share(original: SizeCss, side: DockSide, target: PxRect) -> f64 {
    let (l, t) = match side {
        DockSide::Left | DockSide::Right => (original.w, target.x1.saturating_sub(target.x0)),
        DockSide::Top | DockSide::Bottom => (original.h, target.y1.saturating_sub(target.y0)),
    };
    if t == 0 {
        return ATTACH_SHARE_MAX;
    }
    let t = f64::from(t);
    // `f64::min` returns the non-NaN side — a NaN `L` goes to the upper bound.
    let share = l.min(t * ATTACH_SHARE_MAX) / t;
    share.clamp(tree::RATIO_MIN, ATTACH_SHARE_MAX)
}

/// The band frame of area `frame` — the rectangle on which the area bands ([`slot_band`]) · area zone lines ([`slot_centre`]) stand. From the area frame, subtract [`OUTER_BAND_PX`]
/// inside the canvas edge for the edges where an outer band stands (`outer` — edges of valid outer bands). Edges where the outer band is blocked are not subtracted —
/// that share is taken by the area band (AC36). If the area falls entirely inside that share, it is an empty rectangle.
// Do not attach it to the area frame edge — if the area band (10% · minimum 12) is thinner than the outer band, it is covered entirely and there is no length left to split along that edge, and
// the outer band preview shown at a cursor aimed at that area edge is the same rectangle as that area, so it looks like an area band (canvas 882 × 544 · the
// bottom area of a top-bottom halving = band 27 was covered by outer band 32 · 2026-10-19).
// ADR-0295
pub fn band_frame(frame: PxRect, canvas: CanvasPx, outer: &[DockSide]) -> PxRect {
    let cut = |side| {
        if outer.contains(&side) {
            OUTER_BAND_PX
        } else {
            0
        }
    };
    let x0 = frame.x0.max(cut(DockSide::Left)).min(frame.x1);
    let y0 = frame.y0.max(cut(DockSide::Top)).min(frame.y1);
    PxRect {
        x0,
        y0,
        x1: frame
            .x1
            .min(canvas.w.saturating_sub(cut(DockSide::Right)))
            .max(x0),
        y1: frame
            .y1
            .min(canvas.h.saturating_sub(cut(DockSide::Bottom)))
            .max(y0),
    }
}

/// The `side` band of area `frame` — thickness = `max(round(BAND_SHARE × 칸 길이), BAND_MIN_PX)` (left-right = width · top-bottom = height · counted
/// by the area frame length) · attached to the `side` edge of band frame `room` ([`band_frame`]) and cut to within `room` (if the thickness exceeds `room`, all of `room`).
/// `round` rounds .5 away from 0 (G19) — the drawn band and the decision are the same integer.
// ADR-0295
pub fn slot_band(frame: PxRect, room: PxRect, side: DockSide) -> PxRect {
    let (tw, th) = room_thickness(frame, room);
    match side {
        DockSide::Left => PxRect {
            x1: room.x0 + tw,
            ..room
        },
        DockSide::Right => PxRect {
            x0: room.x1 - tw,
            ..room
        },
        DockSide::Top => PxRect {
            y1: room.y0 + th,
            ..room
        },
        DockSide::Bottom => PxRect {
            y0: room.y1 - th,
            ..room
        },
    }
}

fn band_thickness(len: u32) -> u32 {
    let share = (f64::from(len) * BAND_SHARE).round() as u32;
    share.max(BAND_MIN_PX).min(len)
}

// (left-right band thickness, top-bottom band thickness) — counted by the area frame length and cut to within the band frame.
fn room_thickness(frame: PxRect, room: PxRect) -> (u32, u32) {
    let len = |a: u32, b: u32| b.saturating_sub(a);
    (
        band_thickness(len(frame.x0, frame.x1)).min(len(room.x0, room.x1)),
        band_thickness(len(frame.y0, frame.y1)).min(len(room.y0, room.y1)),
    )
}

/// The center rectangle of area `frame` (band frame `room` — [`band_frame`]) (canvas CSS px · fractional) — the place that falls in none of the four area bands ([`slot_band`]).
/// The lines joining a `room` corner and the same-side corner of this rectangle are the [`choose`] boundary of two neighboring area bands (where the normalized
/// distances are equal). For a narrow area where the left-right (top-bottom) bands exceed half of `room` and overlap, it shrinks to a line segment with width 0 on that axis —
/// then the boundary of the two overlapping bands is that segment. Places cut by blocked area bands · the tab bar zone are not reflected (the shape counted by band thickness alone).
// Point on the diagonal = corner + s·(left-right thickness, top-bottom thickness) — s of 1 is the center corner, and if it first reaches the center line of the band frame
// it stops there (beyond that the overlapping opposite band is closer).
pub fn slot_centre(frame: PxRect, room: PxRect) -> RectCss {
    let (tw, th) = room_thickness(frame, room);
    let (tw, th) = (f64::from(tw), f64::from(th));
    let reach = |a: u32, b: u32, t: f64| {
        if t > 0.0 {
            f64::from(b.saturating_sub(a)) / (2.0 * t)
        } else {
            1.0
        }
    };
    let s = reach(room.x0, room.x1, tw)
        .min(reach(room.y0, room.y1, th))
        .min(1.0);
    RectCss {
        x0: f64::from(room.x0) + s * tw,
        y0: f64::from(room.y0) + s * th,
        x1: f64::from(room.x1) - s * tw,
        y1: f64::from(room.y1) - s * th,
    }
}

/// The outer bands of the four canvas edges (in [`SIDE_ORDER`]) — thickness [`OUTER_BAND_PX`] · not exceeding the canvas.
pub fn outer_strips(canvas: CanvasPx) -> [(DockSide, PxRect); 4] {
    let (w, h) = (canvas.w, canvas.h);
    let (tw, th) = (OUTER_BAND_PX.min(w), OUTER_BAND_PX.min(h));
    [
        (
            DockSide::Left,
            PxRect {
                x0: 0,
                y0: 0,
                x1: tw,
                y1: h,
            },
        ),
        (
            DockSide::Right,
            PxRect {
                x0: w - tw,
                y0: 0,
                x1: w,
                y1: h,
            },
        ),
        (
            DockSide::Top,
            PxRect {
                x0: 0,
                y0: 0,
                x1: w,
                y1: th,
            },
        ),
        (
            DockSide::Bottom,
            PxRect {
                x0: 0,
                y0: h - th,
                x1: w,
                y1: h,
            },
        ),
    ]
}

/// The area frames of the visible tab layout `layout` (canvas CSS px · tree preorder) — neighboring areas share the same integer boundary.
pub fn frames(layout: &LayoutNode, canvas: CanvasPx) -> Vec<(Uuid, PxRect)> {
    layout_geometry::compute(layout)
        .slots
        .iter()
        .map(|r| (r.slot_id, layout_geometry::frame_px(canvas.w, canvas.h, r)))
        .collect()
}

/// The area containing point `p` (canvas CSS px) — `None` = outside the canvas. Area frames are half-open, so a point on a neighboring-area boundary belongs to the right · lower area.
pub fn frame_at(layout: &LayoutNode, canvas: CanvasPx, p: PointCss) -> Option<(Uuid, PxRect)> {
    frames(layout, canvas)
        .into_iter()
        .find(|(_, frame)| contains_px(frame, (p.x, p.y)))
}

/// The canvas px rectangle the inserted one will occupy when `incoming` is inserted at `side` of `anchor` in target tree `target` with share `share`
/// — the circumscribed rectangle of the inserted areas that serves as the preview and that the screen draws after the drop (PRD R4). `None` if the result creates a new zero-area area
/// (R8) or there is no reference area. `share` must be in the model share range
/// `tree::RATIO_MIN ..=` [`crate::layout::dock::SHARE_MAX`] (the `docked_tree` contract — the value of [`attach_share`] is always inside
/// it).
// Move each of the four bounds of the normalized circumscribed rectangle with `px_edge` — `px_edge` is monotonic, so the same integers come out as the circumscribed rectangle
// of the area frames (`frame_px`) that will be drawn after the drop.
// ADR-0227
pub fn landing_rect(
    target: &LayoutNode,
    anchor: DockAnchor,
    side: DockSide,
    incoming: &LayoutNode,
    canvas: CanvasPx,
    share: f64,
) -> Option<PxRect> {
    let result = docked_tree(target, anchor, side, incoming, share).ok()?;
    let r = incoming_rect(
        &layout_geometry::compute(&result),
        &tree::slot_ids(incoming),
    )?;
    Some(PxRect {
        x0: px_edge(canvas.w, r.x0),
        y0: px_edge(canvas.h, r.y0),
        x1: px_edge(canvas.w, r.x1),
        y1: px_edge(canvas.h, r.y1),
    })
}

/// What remains of rectangle `r` after subtracting `cut` — up to four non-overlapping rectangles (top · bottom whole · the left · right in between —
/// carried in this order). Empty pieces are not carried.
pub fn subtract(r: PxRect, cut: PxRect) -> Vec<PxRect> {
    if is_empty(&r) {
        return Vec::new();
    }
    if is_empty(&cut) || !overlaps(&r, &cut) {
        return vec![r];
    }
    let (y0, y1) = (r.y0.max(cut.y0), r.y1.min(cut.y1));
    [
        PxRect { y1: cut.y0, ..r },
        PxRect { y0: cut.y1, ..r },
        PxRect {
            x1: cut.x0,
            y0,
            y1,
            ..r
        },
        PxRect {
            x0: cut.x1,
            y0,
            y1,
            ..r
        },
    ]
    .into_iter()
    .filter(|p| !is_empty(p))
    .collect()
}

fn subtract_all(r: PxRect, cuts: &[PxRect]) -> Vec<PxRect> {
    cuts.iter().fold(vec![r], |pieces, cut| {
        pieces.into_iter().flat_map(|p| subtract(p, *cut)).collect()
    })
}

/// All effective bands of one window — after the outer bands ([`SIDE_ORDER`]), per area (in [`frames`] order) that area's bands ([`SIDE_ORDER`]). Blocked
/// bands and bands entirely covered by preceding regions are absent.
// ADR-0295
pub fn effective_bands(w: &WindowTargets) -> Vec<BandHit> {
    collect(w, |_| true)
}

/// Outer bands + the bands of the area containing point `p` (canvas CSS px) — all that [`choose`] will see at that point. The order · values are
/// the same as [`effective_bands`] minus the area bands outside that area.
pub fn bands_at(w: &WindowTargets, p: PointCss) -> Vec<BandHit> {
    collect(w, |frame| contains_px(frame, (p.x, p.y)))
}

/// The edges of the outer bands in `bands` (the values of [`effective_bands`] · [`bands_at`]) — `outer` of [`band_frame`].
pub fn outer_sides(bands: &[BandHit]) -> Vec<DockSide> {
    bands
        .iter()
        .filter(|b| b.anchor == DockAnchor::Outer)
        .map(|b| b.side)
        .collect()
}

fn collect(w: &WindowTargets, keep: impl Fn(&PxRect) -> bool) -> Vec<BandHit> {
    let zone = zone_cut(w.zone);
    let whole = PxRect {
        x0: 0,
        y0: 0,
        x1: w.canvas.w,
        y1: w.canvas.h,
    };
    let mut out: Vec<BandHit> = outer_strips(w.canvas)
        .into_iter()
        .filter_map(|(side, strip)| {
            band_hit(w, DockAnchor::Outer, side, strip, whole, zone.as_slice())
        })
        .collect();
    // What is subtracted from the area bands = the tab bar zone + all valid outer band regions (R6a ① · blocked outer bands are already out — AC36).
    let cuts: Vec<PxRect> = zone
        .into_iter()
        .chain(out.iter().flat_map(|b| b.areas.iter().copied()))
        .collect();
    let outer = outer_sides(&out);
    for (slot, frame) in frames(w.layout, w.canvas) {
        if !keep(&frame) {
            continue;
        }
        let room = band_frame(frame, w.canvas, &outer);
        for side in SIDE_ORDER {
            let band = slot_band(frame, room, side);
            out.extend(band_hit(
                w,
                DockAnchor::Slot(slot),
                side,
                band,
                frame,
                &cuts,
            ));
        }
    }
    out
}

fn band_hit(
    w: &WindowTargets,
    anchor: DockAnchor,
    side: DockSide,
    band: PxRect,
    target: PxRect,
    cuts: &[PxRect],
) -> Option<BandHit> {
    let areas = subtract_all(band, cuts);
    if areas.is_empty() {
        return None;
    }
    let share = attach_share(w.original, side, target);
    let landing = landing_rect(w.layout, anchor, side, w.incoming, w.canvas, share)?;
    Some(BandHit {
        anchor,
        side,
        band,
        share,
        landing,
        areas,
    })
}

// Cut off what is outside the canvas (negative) to an integer rectangle — even if a non-integer value comes, widen outward so no region containing a point inside the zone is left.
fn zone_cut(zone: Option<RectCss>) -> Option<PxRect> {
    let z = zone?;
    let lo = |v: f64| v.floor().max(0.0) as u32;
    let hi = |v: f64| v.ceil().max(0.0) as u32;
    let r = PxRect {
        x0: lo(z.x0),
        y0: lo(z.y0),
        x1: hi(z.x1),
        y1: hi(z.y1),
    };
    (!is_empty(&r)).then_some(r)
}

/// The place of the band chosen at point `p` (canvas CSS px) — ① if there are outer band regions containing the point, the one among them with the smallest normalized distance (distance to the band edge
/// ÷ thickness) ② otherwise the same criterion among area band regions containing the point ③ if equal, the earlier one in `bands` (so
/// [`SIDE_ORDER`]) ④ if none, `None` (area center · outside the canvas — R6). The tab bar zone is filtered out before this function.
// ADR-0295
pub fn choose(bands: &[BandHit], p: PointCss) -> Option<usize> {
    let p = (p.x, p.y);
    let nearest = |outer: bool| {
        let mut best: Option<(usize, f64)> = None;
        for (i, b) in bands.iter().enumerate() {
            if (b.anchor == DockAnchor::Outer) != outer
                || !b.areas.iter().any(|a| contains_px(a, p))
            {
                continue;
            }
            let d = edge_gap(p, &b.band, b.side) / thickness(&b.band, b.side);
            if best.is_none_or(|(_, x)| d < x) {
                best = Some((i, d));
            }
        }
        best.map(|(i, _)| i)
    };
    nearest(true).or_else(|| nearest(false))
}

/// The area zone lines to draw at point `p` (canvas CSS px) — `bands` = [`effective_bands`] of the same window or [`bands_at`] of that point.
/// `None` = the chosen band is an outer band (then the guide is the layout frame) · `p` is outside the canvas · the area under the point has no valid area band.
/// The tab bar zone is filtered out before this function (R6a — no lines are drawn on the tab bar).
// User decision 2026-10-19 — guide = thin lines of the cursor's area · the outer frame only when touching an outer band.
pub fn slot_zones(w: &WindowTargets, bands: &[BandHit], p: PointCss) -> Option<SlotZones> {
    if choose(bands, p).is_some_and(|i| bands[i].anchor == DockAnchor::Outer) {
        return None;
    }
    let (slot, frame) = frame_at(w.layout, w.canvas, p)?;
    if !bands.iter().any(|b| b.anchor == DockAnchor::Slot(slot)) {
        return None;
    }
    let room = band_frame(frame, w.canvas, &outer_sides(bands));
    Some(SlotZones {
        slot: room,
        centre: slot_centre(frame, room),
    })
}

fn is_empty(r: &PxRect) -> bool {
    r.x0 >= r.x1 || r.y0 >= r.y1
}

fn overlaps(a: &PxRect, b: &PxRect) -> bool {
    a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1
}

fn contains_px(r: &PxRect, p: (f64, f64)) -> bool {
    (f64::from(r.x0)..f64::from(r.x1)).contains(&p.0)
        && (f64::from(r.y0)..f64::from(r.y1)).contains(&p.1)
}

/// From a point to the `side` edge of `frame` — 0 or more if inside.
fn edge_gap(p: (f64, f64), frame: &PxRect, side: DockSide) -> f64 {
    match side {
        DockSide::Left => p.0 - f64::from(frame.x0),
        DockSide::Right => f64::from(frame.x1) - p.0,
        DockSide::Top => p.1 - f64::from(frame.y0),
        DockSide::Bottom => f64::from(frame.y1) - p.1,
    }
}

// Called only for bands that contain the point — then the thickness is positive and the division holds.
fn thickness(r: &PxRect, side: DockSide) -> f64 {
    match side {
        DockSide::Left | DockSide::Right => f64::from(r.x1 - r.x0),
        DockSide::Top | DockSide::Bottom => f64::from(r.y1 - r.y0),
    }
}

#[cfg(test)]
mod golden;

#[cfg(test)]
mod tests;
