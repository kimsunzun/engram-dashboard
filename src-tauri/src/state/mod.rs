//! 화면 상태 파일(`state.json`)의 영속 모양([`schema`]) · 글 변환([`codec`]) · 메모리 모양과의 변환([`convert`]) ·
//! 트리 창 속성 칸([`tree_attrs`]) · 창 자리 기록 · 입히기([`placement`]) · 쓰는 스레드([`saver`]) · 부팅
//! 판정([`boot`]) · 셸 실행 잠금([`lock`]) · 그것을 앱 수명에 잇는 부팅 단계 · 종료([`boot_plugin`]) · 「복원할까요?」
//! 상태([`restore`]) — TRD S21-storage §6.

pub mod boot;
pub mod boot_plugin;
pub mod codec;
pub mod convert;
pub mod lock;
pub mod placement;
pub mod restore;
pub mod saver;
pub mod schema;
pub mod tree_attrs;
