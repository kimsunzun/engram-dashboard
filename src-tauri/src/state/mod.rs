//! 화면 상태 파일(`state.json`)의 영속 모양([`schema`]) · 글 변환([`codec`]) · 쓰는 스레드([`saver`]) — TRD
//! S21-storage §6.
// 아직 배선되지 않았다 — 부팅 단계 · 기록기(P3b)가 붙을 때 이 허용을 걷는다.
#![allow(dead_code)]

pub mod codec;
pub mod saver;
pub mod schema;
