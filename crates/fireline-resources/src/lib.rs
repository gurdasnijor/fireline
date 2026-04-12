#![forbid(unsafe_code)]

pub mod fs_backend;
pub mod mounter;

pub use fs_backend::*;
pub use mounter::{
    LocalPathMounter, MountedResource, ResourceMounter, ResourceRef, prepare_resources,
};
