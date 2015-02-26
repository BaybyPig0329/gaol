// Copyright 2015 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! `seccomp-bpf` support on recent Linux kernels.
//!
//! This works in tandem with `namespace` in order to implement sandbox profiles. It is generally
//! the weaker of the two approaches, because BPF is limited, but it's useful for reducing kernel
//! attack surface area and implementing coarse-grained policies.

#![allow(non_upper_case_globals)]

use profile::{Operation, Profile};

use libc::{AF_INET, AF_INET6, AF_UNIX, O_NONBLOCK, O_RDONLY, c_int, c_ulong, c_ushort};

#[cfg(target_arch="x86")]
const ARCH_NR: u32 = AUDIT_ARCH_X86;
#[cfg(target_arch="x86_64")]
const ARCH_NR: u32 = AUDIT_ARCH_X86_64;
#[cfg(target_arch="arm")]
const ARCH_NR: u32 = AUDIT_ARCH_ARM;

const SECCOMP_RET_KILL: u32 = 0;
const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;

const LD: u16 = 0x00;
const JMP: u16 = 0x05;
const RET: u16 = 0x06;

const W: u16 = 0;
const ABS: u16 = 0x20;

const JEQ: u16 = 0x10;
const JSET: u16 = 0x40;

const K: u16 = 0x00;

const SYSCALL_NR_OFFSET: u32 = 0;
const ARCH_NR_OFFSET: u32 = 4;
const ARG_0_OFFSET: u32 = 16;
const ARG_1_OFFSET: u32 = 24;
const ARG_2_OFFSET: u32 = 32;

const AF_NETLINK: c_int = 16;

const CLONE_VM: c_int = 0x0000_0100;
const CLONE_FS: c_int = 0x0000_0200;
const CLONE_FILES: c_int = 0x0000_0400;
const CLONE_SIGHAND: c_int = 0x0000_0800;
const CLONE_THREAD: c_int = 0x0001_0000;
const CLONE_SYSVSEM: c_int = 0x0004_0000;
const CLONE_SETTLS: c_int = 0x0008_0000;
const CLONE_PARENT_SETTID: c_int = 0x0010_0000;
const CLONE_CHILD_CLEARTID: c_int = 0x0020_0000;

const O_NOCTTY: c_int = 256;
const O_CLOEXEC: c_int = 524288;

const FIONREAD: c_int = 0x541b;

const NETLINK_ROUTE: c_int = 0;

const NR_read: u32 = 0;
const NR_write: u32 = 1;
const NR_open: u32 = 2;
const NR_close: u32 = 3;
const NR_stat: u32 = 4;
const NR_fstat: u32 = 5;
const NR_poll: u32 = 7;
const NR_lseek: u32 = 8;
const NR_mmap: u32 = 9;
const NR_mprotect: u32 = 10;
const NR_munmap: u32 = 11;
const NR_brk: u32 = 12;
const NR_rt_sigreturn: u32 = 15;
const NR_ioctl: u32 = 16;
const NR_access: u32 = 21;
const NR_madvise: u32 = 28;
const NR_socket: u32 = 41;
const NR_connect: u32 = 42;
const NR_sendto: u32 = 44;
const NR_recvfrom: u32 = 45;
const NR_recvmsg: u32 = 47;
const NR_bind: u32 = 49;
const NR_getsockname: u32 = 51;
const NR_clone: u32 = 56;
const NR_exit: u32 = 60;
const NR_readlink: u32 = 89;
const NR_getuid: u32 = 102;
const NR_sigaltstack: u32 = 131;
const NR_futex: u32 = 202;
const NR_sched_getaffinity: u32 = 204;
const NR_exit_group: u32 = 231;
const NR_set_robust_list: u32 = 0;
const NR_sendmmsg: u32 = 307;
const NR_unknown_318: u32 = 318;

const EM_X86_64: u32 = 62;

const __AUDIT_ARCH_64BIT: u32 = 0x8000_0000;
const __AUDIT_ARCH_LE: u32 = 0x4000_0000;
const AUDIT_ARCH_X86_64: u32 = EM_X86_64 | __AUDIT_ARCH_64BIT | __AUDIT_ARCH_LE;

const PR_SET_SECCOMP: c_int = 22;
const PR_SET_NO_NEW_PRIVS: c_int = 38;

const SECCOMP_MODE_FILTER: c_ulong = 2;

static FILTER_PROLOGUE: [sock_filter; 3] = [
    VALIDATE_ARCHITECTURE_0,
    VALIDATE_ARCHITECTURE_1,
    VALIDATE_ARCHITECTURE_2,
];

// A most untimely end...
static FILTER_EPILOGUE: [sock_filter; 1] = [
    KILL_PROCESS,
];

/// Syscalls that are always allowed.
static ALLOWED_SYSCALLS: [u32; 22] = [
    NR_brk,
    NR_close,
    NR_exit,
    NR_exit_group,
    NR_futex,
    NR_getuid,
    NR_madvise,
    NR_mmap,
    NR_mprotect,
    NR_munmap,
    NR_poll,
    NR_read,
    NR_recvfrom,
    NR_recvmsg,
    NR_rt_sigreturn,
    NR_sched_getaffinity,
    NR_sendmmsg,
    NR_sendto,
    NR_set_robust_list,
    NR_sigaltstack,
    NR_unknown_318,
    NR_write,
];

static ALLOWED_SYSCALLS_FOR_FILE_READ_METADATA: [u32; 4] = [
    NR_access,
    NR_fstat,
    NR_readlink,
    NR_stat,
];

static ALLOWED_SYSCALLS_FOR_FILE_READ: [u32; 1] = [
    NR_lseek,
];

static ALLOWED_SYSCALLS_FOR_NETWORK_OUTBOUND: [u32; 2] = [
    NR_bind,
    NR_connect,
];

static ALLOWED_SYSCALLS_FOR_SYSTEM_SOCKET: [u32; 1] = [
    NR_getsockname,
];

const ALLOW_SYSCALL: sock_filter = sock_filter {
    code: RET + K,
    k: SECCOMP_RET_ALLOW,
    jt: 0,
    jf: 0,
};

const KILL_PROCESS: sock_filter = sock_filter {
    code: RET + K,
    k: SECCOMP_RET_KILL,
    jt: 0,
    jf: 0,
};

const EXAMINE_SYSCALL: sock_filter = sock_filter {
    code: LD + W + ABS,
    k: SYSCALL_NR_OFFSET,
    jt: 0,
    jf: 0,
};

const EXAMINE_ARG_0: sock_filter = sock_filter {
    code: LD + W + ABS,
    k: ARG_0_OFFSET,
    jt: 0,
    jf: 0,
};

const EXAMINE_ARG_1: sock_filter = sock_filter {
    code: LD + W + ABS,
    k: ARG_1_OFFSET,
    jt: 0,
    jf: 0,
};

const EXAMINE_ARG_2: sock_filter = sock_filter {
    code: LD + W + ABS,
    k: ARG_2_OFFSET,
    jt: 0,
    jf: 0,
};

const VALIDATE_ARCHITECTURE_0: sock_filter = sock_filter {
    code: LD + W + ABS,
    k: ARCH_NR_OFFSET,
    jt: 0,
    jf: 0,
};

const VALIDATE_ARCHITECTURE_1: sock_filter = sock_filter {
    code: JMP + JEQ + K,
    k: ARCH_NR,
    jt: 1,
    jf: 0,
};

const VALIDATE_ARCHITECTURE_2: sock_filter = KILL_PROCESS;

pub struct Filter {
    program: Vec<sock_filter>,
}

impl Filter {
    pub fn new(profile: &Profile) -> Filter {
        let mut filter = Filter {
            program: FILTER_PROLOGUE.iter().map(|x| *x).collect(),
        };
        filter.allow_syscalls(&ALLOWED_SYSCALLS);

        if profile.allowed_operations().iter().any(|operation| {
            match *operation {
                Operation::FileReadMetadata(_) => true,
                _ => false,
            }
        }) {
            filter.allow_syscalls(&ALLOWED_SYSCALLS_FOR_FILE_READ_METADATA)
        }

        if profile.allowed_operations().iter().any(|operation| {
            match *operation {
                Operation::FileReadAll(_) => true,
                _ => false,
            }
        }) {
            filter.allow_syscalls(&ALLOWED_SYSCALLS_FOR_FILE_READ);

            // Only allow file reading.
            filter.if_syscall_is(NR_open, |filter| {
                filter.if_arg1_hasnt_set(!(O_RDONLY | O_CLOEXEC | O_NOCTTY | O_NONBLOCK) as u32,
                                         |filter| filter.allow_this_syscall())
            });

            // Only allow the `FIONREAD` `ioctl` to be performed.
            filter.if_syscall_is(NR_ioctl, |filter| {
                filter.if_arg1_is(FIONREAD as u32, |filter| filter.allow_this_syscall())
            })
        }

        if profile.allowed_operations().iter().any(|operation| {
            match *operation {
                Operation::NetworkOutbound(_) => true,
                _ => false,
            }
        }) {
            filter.allow_syscalls(&ALLOWED_SYSCALLS_FOR_NETWORK_OUTBOUND)
        }

        if profile.allowed_operations().iter().any(|operation| {
            match *operation {
                Operation::SystemSocket => true,
                _ => false,
            }
        }) {
            filter.allow_syscalls(&ALLOWED_SYSCALLS_FOR_SYSTEM_SOCKET);

            // Only allow Unix, IPv4, IPv6, and netlink route sockets to be created.
            filter.if_syscall_is(NR_socket, |filter| {
                filter.if_arg0_is(AF_UNIX as u32, |filter| filter.allow_this_syscall());
                filter.if_arg0_is(AF_INET as u32, |filter| filter.allow_this_syscall());
                filter.if_arg0_is(AF_INET6 as u32, |filter| filter.allow_this_syscall());
                filter.if_arg0_is(AF_NETLINK as u32, |filter| {
                    filter.if_arg2_is(NETLINK_ROUTE as u32, |filter| filter.allow_this_syscall())
                })
            })
        }

        // Only allow normal threads to be created.
        filter.if_syscall_is(NR_clone, |filter| {
            filter.if_arg0_is((CLONE_VM |
                               CLONE_FS |
                               CLONE_FILES |
                               CLONE_SIGHAND |
                               CLONE_THREAD |
                               CLONE_SYSVSEM |
                               CLONE_SETTLS |
                               CLONE_PARENT_SETTID |
                               CLONE_CHILD_CLEARTID) as u32,
                              |filter| filter.allow_this_syscall())
        });

        filter.program.push_all(&FILTER_EPILOGUE);
        filter
    }

    /// Activates this filter, applying all of its restrictions forevermore. This can only be done
    /// once.
    pub fn activate(&self) -> Result<(),c_int> {
        unsafe {
            let result = prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
            if result != 0 {
                return Err(result)
            }

            let program = sock_fprog {
                len: self.program.len() as c_ushort,
                filter: self.program.as_ptr(),
            };
            let result = prctl(PR_SET_SECCOMP,
                               SECCOMP_MODE_FILTER,
                               &program as *const sock_fprog as usize as c_ulong,
                               -1,
                               0);
            if result == 0 {
                Ok(())
            } else {
                Err(result)
            }
        }
    }

    fn allow_this_syscall(&mut self) {
        self.program.push(ALLOW_SYSCALL)
    }

    fn allow_syscalls(&mut self, syscalls: &[u32]) {
        for &syscall in syscalls.iter() {
            self.if_syscall_is(syscall, |filter| filter.allow_this_syscall())
        }
    }

    fn if_syscall_is<F>(&mut self, number: u32, then: F) where F: FnMut(&mut Filter) {
        self.program.push(EXAMINE_SYSCALL);
        self.if_k_is(number, then)
    }

    fn if_arg0_is<F>(&mut self, value: u32, then: F) where F: FnMut(&mut Filter) {
        self.program.push(EXAMINE_ARG_0);
        self.if_k_is(value, then)
    }

    fn if_arg1_is<F>(&mut self, value: u32, then: F) where F: FnMut(&mut Filter) {
        self.program.push(EXAMINE_ARG_1);
        self.if_k_is(value, then)
    }

    fn if_arg1_hasnt_set<F>(&mut self, value: u32, then: F) where F: FnMut(&mut Filter) {
        self.program.push(EXAMINE_ARG_1);
        self.if_k_hasnt_set(value, then)
    }

    fn if_arg2_is<F>(&mut self, value: u32, then: F) where F: FnMut(&mut Filter) {
        self.program.push(EXAMINE_ARG_2);
        self.if_k_is(value, then)
    }

    fn if_k_is<F>(&mut self, value: u32, mut then: F) where F: FnMut(&mut Filter) {
        let index = self.program.len();
        self.program.push(sock_filter {
            code: JMP + JEQ + K,
            k: value,
            jt: 0,
            jf: 0,
        });
        then(self);
        self.program[index].jf = (self.program.len() - index - 1) as u8;
    }

    fn if_k_hasnt_set<F>(&mut self, value: u32, mut then: F) where F: FnMut(&mut Filter) {
        let index = self.program.len();
        self.program.push(sock_filter {
            code: JMP + JSET + K,
            k: value,
            jt: 0,
            jf: 0,
        });
        then(self);
        self.program[index].jt = (self.program.len() - index - 1) as u8;
    }
}

#[repr(C)]
#[derive(Copy, Clone)]
struct sock_filter {
    code: u16,
    jt: u8,
    jf: u8,
    k: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
#[allow(raw_pointer_derive)]
struct sock_fprog {
    len: c_ushort,
    filter: *const sock_filter,
}

extern {
    fn prctl(option: c_int, arg2: c_ulong, arg3: c_ulong, arg4: c_ulong, arg5: c_ulong) -> c_int;
}

