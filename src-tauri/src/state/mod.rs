//! 화면 상태 파일(`state.json`)의 영속 모양([`schema`]) · 글 변환([`codec`]) · 메모리 모양과의 변환([`convert`]) ·
//! 트리 창 속성 칸([`tree_attrs`]) · 쓰는 스레드([`saver`]) · 부팅 판정([`boot`]) · 셸 실행 잠금([`lock`]) — TRD
//! S21-storage §6.
// 아직 배선되지 않았다 — 부팅 단계 · 기록기(P3b)가 붙을 때 이 허용을 걷는다.
#![allow(dead_code)]

pub mod boot;
pub mod codec;
pub mod convert;
pub mod lock;
pub mod saver;
pub mod schema;
pub mod tree_attrs;
