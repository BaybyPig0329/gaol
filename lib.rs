// Copyright 2015 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

#![allow(deprecated, unused_features)]
#![feature(collections, core, env, io, libc, os, path, rand, rustc_private, std_misc)]

#[macro_use]
extern crate log;

extern crate libc;

pub mod profile;
pub mod sandbox;

pub mod platform {
    #[cfg(target_os="linux")]
    pub use platform::linux::Operation;
    #[cfg(target_os="macos")]
    pub use platform::macos::{ChildSandbox, Operation, Sandbox};

    #[cfg(target_os="linux")]
    pub mod linux;
    #[cfg(target_os="macos")]
    pub mod macos;
}

