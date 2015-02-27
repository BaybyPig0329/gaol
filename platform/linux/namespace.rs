// Copyright 2015 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Sandboxing on Linux via namespaces.

use profile::{Operation, PathPattern, Profile}; 
use libc::{self, c_char, c_int, c_ulong, c_void, gid_t, uid_t};
use std::env;
use std::ffi::{AsOsStr, CString};
use std::old_io::{File, FilePermission, FileStat, FileType, IoError, TempDir};
use std::old_io::fs;
use std::ptr;

/// Creates a namespace and sets up a chroot jail.
pub fn activate(profile: &Profile) -> Result<(),c_int> {
    match try!(Namespace::new(profile)).init() {
        Ok(()) => {}
        Err(_) => return Err(1),
    }

    try!(switch_to_unprivileged_user());
    try!(try!(ChrootJail::new(profile)).enter());
    drop_capabilities()
}

struct Namespace {
    parent_uid: uid_t,
    parent_gid: gid_t,
}

impl Namespace {
    fn new(profile: &Profile) -> Result<Namespace,c_int> {
        let (parent_uid, parent_gid) = unsafe {
            (libc::getuid(), libc::getgid())
        };

        // NB: It would be nice if we could use `CLONE_NEWPID`, but that only works when we spawn a
        // new process, which is contrary to the design of the sandbox right now. I believe that
        // the restrictive `seccomp-bpf` filter prevents us doing anything evil with PIDs anyhow
        // (e.g. sending signals to or ptracing other processes), but we should go over this for
        // sure.
        let mut flags = CLONE_FS | CLONE_NEWUSER | CLONE_NEWIPC | CLONE_NEWNS | CLONE_NEWUTS;

        // If we don't allow network operations, create a network namespace.
        if !profile.allowed_operations().iter().any(|operation| {
            match *operation {
                Operation::NetworkOutbound(_) => true,
                _ => false,
            }
        }) {
            flags |= CLONE_NEWNET
        }

        let result = unsafe {
            unshare(flags)
        };
        if result == 0 {
            Ok(Namespace {
                parent_uid: parent_uid,
                parent_gid: parent_gid,
            })
        } else {
            Err(result)
        }
    }

    fn init(&self) -> Result<(),IoError> {
        // See http://crbug.com/457362 for more information on this.
        try!(try!(File::create(&Path::new("/proc/self/setgroups"))).write_all(b"deny"));

        try!(write!(&mut try!(File::create(&Path::new("/proc/self/gid_map"))),
                    "1 {} 1",
                    self.parent_gid));
        write!(&mut try!(File::create(&Path::new("/proc/self/uid_map"))),
               "1 {} 1",
               self.parent_uid)
    }
}

fn switch_to_unprivileged_user() -> Result<(),c_int> {
    unsafe {
        let result = setresgid(1, 1, 1);
        if result != 0 {
            return Err(result)
        }
        let result = setresuid(1, 1, 1);
        if result == 0 {
            Ok(())
        } else {
            Err(result)
        }
    }
}

struct ChrootJail {
    directory: TempDir,
}

impl ChrootJail {
    fn new(profile: &Profile) -> Result<ChrootJail,c_int> {
        let jail_dir = match TempDir::new("gaol") {
            Ok(jail_dir) => jail_dir,
            Err(_) => return Err(-1),
        };
        let jail = ChrootJail {
            directory: jail_dir,
        };

        let src = CString::from_slice(b"tmpfs");
        let dest = CString::from_slice(jail.directory
                                           .path()
                                           .as_os_str()
                                           .to_str()
                                           .unwrap()
                                           .as_bytes());
        let tmpfs = CString::from_slice(b"tmpfs");
        let result = unsafe {
            mount(src.as_ptr(), dest.as_ptr(), tmpfs.as_ptr(), MS_NOATIME, ptr::null())
        };
        if result != 0 {
            return Err(result)
        }

        for operation in profile.allowed_operations().iter() {
            match *operation {
                Operation::FileReadAll(PathPattern::Literal(ref path)) |
                Operation::FileReadAll(PathPattern::Subpath(ref path)) => {
                    try!(jail.bind_mount(path))
                }
                Operation::FileReadMetadata(PathPattern::Literal(ref path)) |
                Operation::FileReadMetadata(PathPattern::Subpath(ref path)) => {
                    try!(jail.bind_mount(path));
                    try!(jail.disallow_reading(path));
                }
                _ => {}
            }
        }

        Ok(jail)
    }

    fn enter(&self) -> Result<(),c_int> {
        let directory = CString::from_slice(self.directory
                                                .path()
                                                .as_os_str()
                                                .to_str()
                                                .unwrap()
                                                .as_bytes());
        let result = unsafe {
            chroot(directory.as_ptr())
        };
        if result != 0 {
            return Err(result)
        }

        match env::set_current_dir(&Path::new(".")) {
            Ok(_) => Ok(()),
            Err(_) => Err(-1),
        }
    }

    fn bind_mount(&self, source_path: &Path) -> Result<(),c_int> {
        // Create all intermediate directories.
        let mut destination_path = self.directory.path().clone();
        let mut components: Vec<Vec<u8>> =
            destination_path.components()
                            .map(|bytes| bytes.iter().map(|x| *x).collect())
                            .collect();
        let last_component = components.pop();
        for component in components.into_iter() {
            destination_path.push(component);
            if fs::mkdir(&destination_path, FilePermission::all()).is_err() {
                return Err(-1)
            }
        }

        // Create the mount file or directory.
        if let Some(last_component) = last_component {
            destination_path.push(last_component);
            match fs::stat(source_path) {
                Ok(FileStat {
                    kind: FileType::Directory,
                    ..
                }) => {
                    if fs::mkdir(&destination_path, FilePermission::all()).is_err() {
                        return Err(-1)
                    }
                }
                Ok(FileStat {
                    kind: _,
                    ..
                }) => {
                    if File::create(&destination_path).is_err() {
                        return Err(-1)
                    }
                }
                Err(_) => return Err(-1)
            }
        }

        // Create the bind mount.
        destination_path.push(source_path);
        let source_path = CString::from_slice(source_path.as_os_str()
                                                         .to_str()
                                                         .unwrap()
                                                         .as_bytes());
        let destination_path = CString::from_slice(destination_path.as_os_str()
                                                                   .to_str()
                                                                   .unwrap()
                                                                   .as_bytes());
        let bind = CString::from_slice(b"bind");
        let result = unsafe {
            mount(source_path.as_ptr(),
                  destination_path.as_ptr(),
                  bind.as_ptr(),
                  MS_MGC_VAL | MS_BIND | MS_REC,
                  ptr::null_mut())
        };
        if result == 0 {
            Ok(())
        } else {
            Err(result)
        }
    }

    fn disallow_reading(&self, source_path: &Path) -> Result<(),c_int> {
        let mut destination_path = self.directory.path().clone();
        destination_path.push(source_path);
        let destination_path = CString::from_slice(destination_path.as_os_str()
                                                                   .to_str()
                                                                   .unwrap()
                                                                   .as_bytes());
        let result = unsafe {
            libc::chmod(destination_path.as_ptr(), 0)
        };
        if result == 0 {
            Ok(())
        } else {
            Err(result)
        }
    }
}

fn drop_capabilities() -> Result<(),c_int> {
    let result = unsafe {
        capset(&__user_cap_header_struct {
            version: _LINUX_CAPABILITY_VERSION_3,
            pid: 0,
        }, &__user_cap_data_struct {
            effective: 0,
            permitted: 0,
            inheritable: 0,
        })
    };
    if result == 0 {
        Ok(())
    } else {
        Err(result)
    }
}

pub const CLONE_VM: c_int = 0x0000_0100;
pub const CLONE_FS: c_int = 0x0000_0200;
pub const CLONE_FILES: c_int = 0x0000_0400;
pub const CLONE_SIGHAND: c_int = 0x0000_0800;
pub const CLONE_THREAD: c_int = 0x0001_0000;
pub const CLONE_NEWNS: c_int = 0x0002_0000;
pub const CLONE_SYSVSEM: c_int = 0x0004_0000;
pub const CLONE_SETTLS: c_int = 0x0008_0000;
pub const CLONE_PARENT_SETTID: c_int = 0x0010_0000;
pub const CLONE_CHILD_CLEARTID: c_int = 0x0020_0000;
pub const CLONE_NEWUTS: c_int = 0x0400_0000;
pub const CLONE_NEWIPC: c_int = 0x0800_0000;
pub const CLONE_NEWUSER: c_int = 0x1000_0000;
pub const CLONE_NEWNET: c_int = 0x4000_0000;

const MS_NOATIME: c_ulong = 1024;
const MS_BIND: c_ulong = 4096;
const MS_REC: c_ulong = 16384;
const MS_MGC_VAL: c_ulong = 0xc0ed_0000;

#[repr(C)]
#[allow(non_camel_case_types)]
struct __user_cap_header_struct {
    version: u32,
    pid: c_int,
}

#[repr(C)]
#[allow(non_camel_case_types)]
struct __user_cap_data_struct {
    effective: u32,
    permitted: u32,
    inheritable: u32,
}

#[allow(non_camel_case_types)]
type cap_user_header_t = *const __user_cap_header_struct;

#[allow(non_camel_case_types)]
type const_cap_user_data_t = *const __user_cap_data_struct;

const _LINUX_CAPABILITY_VERSION_3: u32 = 0x20080522;

extern {
    fn capset(hdrp: cap_user_header_t, datap: const_cap_user_data_t) -> c_int;
    fn chroot(path: *const c_char) -> c_int;
    fn mount(source: *const c_char,
             target: *const c_char,
             filesystemtype: *const c_char,
             mountflags: c_ulong,
             data: *const c_void)
             -> c_int;
    fn setresgid(rgid: gid_t, egid: gid_t, sgid: gid_t) -> c_int;
    fn setresuid(ruid: uid_t, euid: uid_t, suid: uid_t) -> c_int;
    fn unshare(flags: c_int) -> c_int;
}

