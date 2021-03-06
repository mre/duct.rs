extern crate tempdir;
use self::tempdir::TempDir;

use os_pipe::FromFile;

use super::*;
use std::collections::HashMap;
use std::env;
use std::env::consts::EXE_EXTENSION;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io;
use std::io::prelude::*;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str;
use std::sync::{Once, ONCE_INIT};

fn path_to_exe(name: &str) -> PathBuf {
    // This project defines some associated binaries for testing, and we shell out to them in
    // these tests. `cargo test` doesn't automatically build associated binaries, so this
    // function takes care of building them explicitly.
    static CARGO_BUILD_ONCE: Once = ONCE_INIT;
    CARGO_BUILD_ONCE.call_once(|| {
        let build_status = Command::new("cargo")
            .arg("build")
            .arg("--quiet")
            .status()
            .unwrap();
        assert!(build_status.success(),
                "Cargo failed to build associated binaries.");
    });

    Path::new("target").join("debug").join(name).with_extension(EXE_EXTENSION)
}

fn true_cmd() -> Expression {
    cmd!(path_to_exe("status"), "0")
}

fn false_cmd() -> Expression {
    cmd!(path_to_exe("status"), "1")
}

#[test]
fn test_cmd() {
    let output = cmd!(path_to_exe("echo"), "hi").read().unwrap();
    assert_eq!("hi", output);
}

#[test]
fn test_sh() {
    // Windows compatible.
    let output = sh("echo hi").read().unwrap();
    assert_eq!("hi", output);
}

#[test]
fn test_start() {
    let handle1 = cmd!(path_to_exe("echo"), "hi").stdout_capture().start();
    let handle2 = cmd!(path_to_exe("echo"), "lo").stdout_capture().start();
    let output1 = handle1.wait().unwrap();
    let output2 = handle2.wait().unwrap();
    assert_eq!("hi", str::from_utf8(&output1.stdout).unwrap().trim());
    assert_eq!("lo", str::from_utf8(&output2.stdout).unwrap().trim());
}

#[test]
fn test_error() {
    let result = false_cmd().run();
    if let Err(Error(ErrorKind::Status(output), _)) = result {
        // Check that the status is non-zero.
        assert!(!output.status.success());
    } else {
        panic!("Expected a status error.");
    }
}

#[test]
fn test_unchecked() {
    let unchecked_false = false_cmd().unchecked();
    // Unchecked errors shouldn't prevent the right side of `then` from
    // running, and they shouldn't cause `run` to return an error.
    let output = unchecked_false.then(cmd!(path_to_exe("echo"), "waa"))
        .then(unchecked_false)
        .stdout_capture()
        .run()
        .unwrap();
    // The value of the exit code is preserved.
    assert_eq!(1, output.status.code().unwrap());
    assert_eq!("waa", String::from_utf8_lossy(&output.stdout).trim());
}

#[test]
fn test_unchecked_in_pipe() {
    let zero = cmd!(path_to_exe("status"), "0");
    let one = cmd!(path_to_exe("status"), "1");
    let two = cmd!(path_to_exe("status"), "2");

    // Right takes precedence over left.
    let output = one.pipe(two.clone()).unchecked().run().unwrap();
    assert_eq!(2, output.status.code().unwrap());

    // Except that checked on the left takes precedence over unchecked on
    // the right.
    let output = one.pipe(two.unchecked()).unchecked().run().unwrap();
    assert_eq!(1, output.status.code().unwrap());

    // Right takes precedence over the left again if they're both unchecked.
    let output = one.unchecked().pipe(two.unchecked()).unchecked().run().unwrap();
    assert_eq!(2, output.status.code().unwrap());

    // Except that if the right is a success, the left takes precedence.
    let output = one.unchecked().pipe(zero.unchecked()).unchecked().run().unwrap();
    assert_eq!(1, output.status.code().unwrap());

    // Even if the right is checked.
    let output = one.unchecked().pipe(zero).unchecked().run().unwrap();
    assert_eq!(1, output.status.code().unwrap());
}

#[test]
fn test_pipe() {
    let output = sh("echo xxx").pipe(cmd!(path_to_exe("x_to_y"))).read().unwrap();
    assert_eq!("yyy", output);

    // Check that errors on either side are propagated.
    let result = true_cmd().pipe(false_cmd()).run();
    match result {
        Err(Error(ErrorKind::Status(output), _)) => {
            assert!(output.status.code().unwrap() == 1);
        }
        _ => panic!("should never get here"),
    }

    let result = false_cmd().pipe(true_cmd()).run();
    match result {
        Err(Error(ErrorKind::Status(output), _)) => {
            assert!(output.status.code().unwrap() == 1);
        }
        _ => panic!("should never get here"),
    }
}

#[test]
fn test_then() {
    let output = true_cmd().then(sh("echo lo")).read().unwrap();
    assert_eq!("lo", output);

    // Check that errors on either side are propagated.
    let result = true_cmd().then(false_cmd()).run();
    match result {
        Err(Error(ErrorKind::Status(output), _)) => {
            assert!(output.status.code().unwrap() == 1);
        }
        _ => panic!("should never get here"),
    }

    let result = false_cmd().then(true_cmd()).run();
    match result {
        Err(Error(ErrorKind::Status(output), _)) => {
            assert!(output.status.code().unwrap() == 1);
        }
        _ => panic!("should never get here"),
    }
}

#[test]
fn test_input() {
    let expr = cmd!(path_to_exe("x_to_y")).input("xxx");
    let output = expr.read().unwrap();
    assert_eq!("yyy", output);
}

#[test]
fn test_stderr() {
    let (mut reader, writer) = ::os_pipe::pipe().unwrap();
    sh("echo hi>&2").stderr_file(File::from_file(writer)).run().unwrap();
    let mut s = String::new();
    reader.read_to_string(&mut s).unwrap();
    assert_eq!(s.trim(), "hi");
}

#[test]
fn test_null() {
    let expr = cmd!(path_to_exe("cat"))
        .stdin_null()
        .stdout_null()
        .stderr_null();
    let output = expr.read().unwrap();
    assert_eq!("", output);
}

#[test]
fn test_path() {
    let dir = TempDir::new("test_path").unwrap();
    let input_file = dir.path().join("input_file");
    let output_file = dir.path().join("output_file");
    File::create(&input_file).unwrap().write_all(b"xxx").unwrap();
    let expr = cmd!(path_to_exe("x_to_y"))
        .stdin(&input_file)
        .stdout(&output_file);
    let output = expr.read().unwrap();
    assert_eq!("", output);
    let mut file_output = String::new();
    File::open(&output_file).unwrap().read_to_string(&mut file_output).unwrap();
    assert_eq!("yyy", file_output);
}

#[test]
fn test_swapping() {
    let output = sh("echo hi")
        .stdout_to_stderr()
        .stderr_capture()
        .run()
        .unwrap();
    let stderr = str::from_utf8(&output.stderr).unwrap().trim();
    assert_eq!("hi", stderr);

    // Windows compatible. (Requires no space before the ">".)
    let output = sh("echo hi>&2").stderr_to_stdout().read().unwrap();
    assert_eq!("hi", output);
}

#[test]
fn test_file() {
    let dir = TempDir::new("test_file").unwrap();
    let file = dir.path().join("file");
    File::create(&file).unwrap().write_all(b"example").unwrap();
    let expr = cmd!(path_to_exe("cat")).stdin_file(File::open(&file).unwrap());
    let output = expr.read().unwrap();
    assert_eq!(output, "example");
}

#[test]
fn test_ergonomics() {
    let mystr = "owned string".to_owned();
    let mypathbuf = Path::new("a/b/c").to_owned();
    let myvec = vec![1, 2, 3];
    // These are nonsense expressions. We just want to make sure they compile.
    let _ = sh("true").stdin(&*mystr).input(&*myvec).stdout(&*mypathbuf);
    let _ = sh("true").stdin(mystr).input(myvec).stdout(mypathbuf);

    // Unfortunately, this one doesn't work with our Into<Vec<u8>> bound on input().
    // TODO: Is it worth having these impls for &Vec in other cases?
    // let _ = sh("true").stdin(&mystr).input(&myvec).stdout(&mypathbuf);
}

#[test]
fn test_capture_both() {
    // Windows compatible, no space before ">", and we trim newlines at the end to avoid
    // dealing with the different kinds.
    let output = sh("echo hi")
        .then(sh("echo lo>&2"))
        .stdout_capture()
        .stderr_capture()
        .run()
        .unwrap();
    assert_eq!("hi", str::from_utf8(&output.stdout).unwrap().trim());
    assert_eq!("lo", str::from_utf8(&output.stderr).unwrap().trim());
}

#[test]
fn test_dir() {
    // This test checks the interaction of `dir` and relative exe paths.
    // Make sure that's actually what we're testing.
    let pwd_path = path_to_exe("pwd");
    assert!(pwd_path.is_relative());

    let pwd = cmd!(pwd_path);

    // First assert that ordinary commands happen in the parent's dir.
    let pwd_output = pwd.read().unwrap();
    let pwd_path = Path::new(&pwd_output);
    assert_eq!(pwd_path, env::current_dir().unwrap());

    // Now create a temp dir and make sure we can set dir to it. This
    // also tests the interaction of `dir` and relative exe paths.
    let dir = TempDir::new("duct_test").unwrap();
    let pwd_output = pwd.dir(dir.path()).read().unwrap();
    let pwd_path = Path::new(&pwd_output);
    // pwd_path isn't totally canonical on Windows, because it
    // doesn't have a prefix. Thus we have to canonicalize both
    // sides. (This also handles symlinks in TMP_DIR.)
    assert_eq!(pwd_path.canonicalize().unwrap(),
               dir.path().canonicalize().unwrap());
}

#[test]
fn test_env() {
    let output = cmd!(path_to_exe("print_env"), "foo")
        .env("foo", "bar")
        .read()
        .unwrap();
    assert_eq!("bar", output);
}

#[test]
fn test_full_env() {
    let var_name = "test_env_remove_var";

    // Capture the parent env, and make sure it does *not* contain our variable.
    let mut clean_env: HashMap<OsString, OsString> = env::vars_os().collect();
    clean_env.remove(AsRef::<OsStr>::as_ref(var_name));

    // Run a child process with that map passed to full_env(). It should be guaranteed not to
    // see our variable, regardless of any outer env() calls or changes in the parent.
    let clean_child = cmd!(path_to_exe("print_env"), var_name).full_env(clean_env);

    // Dirty the parent env. Should be suppressed.
    env::set_var(var_name, "junk1");
    // And make an outer env() call. Should also be suppressed.
    let dirty_child = clean_child.env(var_name, "junk2");

    // Check that neither of those have any effect.
    let output = dirty_child.read().unwrap();
    assert_eq!("", output);
}

#[test]
fn test_broken_pipe() {
    // If the input writing thread fills up its pipe buffer, writing will block. If the process
    // on the other end of the pipe exits while writer is waiting, the write will return an
    // error. We need to swallow that error, rather than returning it.
    let myvec = vec![0; 1_000_000];
    true_cmd().input(myvec).run().unwrap();
}

#[test]
fn test_suppress_broken_pipe() {
    let broken_pipe_error = Err(io::Error::new(io::ErrorKind::BrokenPipe, ""));
    assert!(::suppress_broken_pipe_errors(broken_pipe_error).is_ok());

    let other_error = Err(io::Error::new(io::ErrorKind::Other, ""));
    assert!(::suppress_broken_pipe_errors(other_error).is_err());
}

#[test]
fn test_silly() {
    // A silly test, purely for coverage.
    ::IoValue::Null.try_clone().unwrap();
}

#[test]
fn test_path_sanitization() {
    // We don't do any chdir'ing in this process, because the tests runner is multithreaded,
    // and we don't want to screw up anyone else's relative paths. Instead, we shell out to a
    // small test process that does that for us.
    cmd!(path_to_exe("exe_in_dir"), path_to_exe("status"), "0")
        .run()
        .unwrap();
}
