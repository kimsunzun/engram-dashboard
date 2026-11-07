//! Persistent shape of the screen state file (`state.json`) ([`schema`]) · text conversion ([`codec`]) · conversion to and from the in-memory shape ([`convert`]) ·
//! window placement recording · applying ([`placement`]) · the writing thread ([`saver`]) · boot decision ([`boot`]) · shell run lock ([`lock`]) ·
//! the boot stage · shutdown that connect it to the app lifetime ([`boot_plugin`]) · the 「복원할까요?」 (restore?) state ([`restore`]) — TRD S21-storage §6.

pub mod boot;
pub mod boot_plugin;
pub mod codec;
pub mod convert;
pub mod lock;
pub mod placement;
pub mod restore;
pub mod saver;
pub mod schema;
