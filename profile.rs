// Copyright 2015 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use platform;

/// A sandbox profile, which specifies the set of operations that this process is allowed to
/// perform. Operations not in the list are implicitly denied.
///
/// If the process attempts to perform an operation in the list that this platform can prohibit
/// after the sandbox is entered via `enter()`, the operation will either fail or the process will
/// be immediately terminated. You can check whether an operation can be prohibited on this
/// platform with `Operation::prohibition_supported()`.
///
/// Because of platform limitiations, patterns within one profile are not permitted to overlap; the
/// behavior is undefined if they do. For example, you may not allow metadata reads of the subpath
/// rooted at `/dev` while allowing full reads of `/dev/null`; you must instead allow full reads of
/// `/dev` or make the profile more restrictive.
pub struct Profile {
    allowed_operations: Vec<Operation>,
}

/// An operation that this process is allowed to perform.
pub enum Operation {
    /// All file-related reading operations may be performed on this file.
    FileReadAll(PathPattern),
    /// Metadata (for example, `stat` or `readlink`) of this file may be read.
    FileReadMetadata(PathPattern),
    /// Outbound network connections to the given address may be initiated.
    NetworkOutbound(AddressPattern),
    /// System information may be read (via `sysctl` on Unix).
    SystemInfoRead,
    /// Sockets may be created.
    SystemSocket,
    /// Platform-specific operations.
    PlatformSpecific(platform::Operation),
}

/// Describes a path or paths on the filesystem.
pub enum PathPattern {
    /// One specific path, which must not represent a directory.
    Literal(Path),
    /// A directory and all of its contents, recursively.
    Subpath(Path),
}

/// Describes a network address.
pub enum AddressPattern {
    /// TCP connections on the given port.
    Tcp(u16),
    /// A local socket at the given path (for example, a Unix socket).
    LocalSocket(Path),
}

impl Profile {
    /// Creates a new profile with the given set of allowed operations.
    pub fn new(allowed_operations: Vec<Operation>) -> Profile {
        Profile {
            allowed_operations: allowed_operations,
        }
    }

    /// Returns the list of allowed operations.
    pub fn allowed_operations(&self) -> &[Operation] {
        self.allowed_operations.as_slice()
    }
}

