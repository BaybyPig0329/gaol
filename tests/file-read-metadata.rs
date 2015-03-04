// Any copyright is dedicated to the Public Domain.
// http://creativecommons.org/publicdomain/zero/1.0/

#![allow(deprecated)]
#![feature(collections, env, io, libc, os, path, rand, std_misc)]

extern crate gaol;
extern crate libc;

use gaol::profile::{Operation, PathPattern, Profile};
use gaol::sandbox::{ChildSandbox, ChildSandboxMethods, Command, Sandbox, SandboxMethods};
use libc::c_char;
use std::env;
use std::ffi::{AsOsStr, CString};
use std::old_io::fs::{File, PathExtensions};
use std::rand::{self, Rng};

// A conservative overapproximation of `PATH_MAX` on all platforms.
const PATH_MAX: usize = 4096;

fn allowance_profile(path: &Path) -> Profile {
    Profile::new(vec![
        Operation::FileReadMetadata(PathPattern::Literal(path.clone())),
    ]).unwrap()
}

fn prohibition_profile() -> Profile {
    Profile::new(vec![
        Operation::FileReadMetadata(PathPattern::Subpath(Path::new("/bogus")))
    ]).unwrap()
}

fn allowance_test() {
    let path = Path::new(env::var("GAOL_TEMP_FILE").unwrap().to_str().unwrap());
    if ChildSandbox::new(allowance_profile(&path)).activate().is_ok() {
        drop(path.stat().unwrap())
    }
}

fn prohibition_test() {
    let path = Path::new(env::var("GAOL_TEMP_FILE").unwrap().to_str().unwrap());
    ChildSandbox::new(prohibition_profile()).activate().unwrap();
    drop(path.stat().unwrap())
}

pub fn main() {
    match env::args().skip(1).next() {
        Some(ref arg) if arg == "allowance_test" => return allowance_test(),
        Some(ref arg) if arg == "prohibition_test" => return prohibition_test(),
        _ => {}
    }

    // Need to use `realpath` here for Mac OS X, because the temporary directory is usually a
    // symlink.
    let mut temp_path = env::temp_dir();
    unsafe {
        let c_temp_path = CString::from_slice(temp_path.as_os_str().to_str().unwrap().as_bytes());
        let mut new_temp_path = [0u8; PATH_MAX];
        drop(realpath(c_temp_path.as_ptr(), new_temp_path.as_mut_ptr() as *mut c_char));
        temp_path = Path::new(&new_temp_path[..new_temp_path.position_elem(&0).unwrap()]);
    }

    let suffix: String = rand::thread_rng().gen_ascii_chars().take(6).collect();
    temp_path.push(format!("gaoltest.{}", suffix));
    File::create(&temp_path).unwrap().write_str("super secret\n").unwrap();

    let allowance_status = Sandbox::new(allowance_profile(
            &temp_path)).start(&mut Command::me().unwrap()
                                                 .arg("allowance_test")
                                                 .env("GAOL_TEMP_FILE", temp_path.clone()))
                        .unwrap()
                        .wait()
                        .unwrap();
    assert!(allowance_status.success());

    let prohibition_status = Sandbox::new(prohibition_profile()).start(
        Command::me().unwrap().arg("prohibition_test").env("GAOL_TEMP_FILE", temp_path.clone()))
                                                                .unwrap()
                                                                .wait()
                                                                .unwrap();
    assert!(!prohibition_status.success());
}

extern {
    fn realpath(file_name: *const c_char, resolved_name: *mut c_char) -> *mut c_char;
}

