//! What Corral asks the kernel about processes. Linux answers through /proc; macOS,
//! which has no /proc, through proc_pidinfo, in its C library (libproc). The rest of
//! Corral asks here and never reads /proc itself.

use std::path::PathBuf;

extern "C" {
    fn geteuid() -> u32;
}

/// The user this process runs as.
pub fn euid() -> u32 {
    // SAFETY: geteuid takes nothing and can't fail.
    unsafe { geteuid() }
}

/// A pid the kernel can look up: 0 and negative numbers mean process groups to kill().
fn valid(pid: i64) -> Option<i32> {
    i32::try_from(pid).ok().filter(|&p| p > 0)
}

// ----------------------------------------------------------------------- Linux

/// Running, and not a zombie (ended, waiting for its parent to collect it).
#[cfg(target_os = "linux")]
pub fn pid_alive(pid: i64) -> bool {
    let Some(pid) = valid(pid) else { return false };
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        // "pid (name) S ...": the state comes after the name, which may hold spaces.
        Ok(stat) => stat.rsplit_once(')').and_then(|(_, rest)| rest.trim_start().chars().next()).is_some_and(|s| s != 'Z' && s != 'X'),
        Err(_) => false,
    }
}

/// The folder process `pid` is in right now, as the kernel has it.
#[cfg(target_os = "linux")]
pub fn cwd(pid: i64) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{}/cwd", valid(pid)?)).ok()
}

/// The folder that lists this process's open file descriptors.
#[cfg(target_os = "linux")]
pub const FD_DIR: &str = "/proc/self/fd";

/// The system's out-of-memory kills so far (`oom_kill` in /proc/vmstat).
#[cfg(target_os = "linux")]
pub fn oom_kills() -> Option<u64> {
    std::fs::read_to_string("/proc/vmstat").ok()?.lines()
        .find_map(|l| l.strip_prefix("oom_kill ")).and_then(|n| n.trim().parse().ok())
}

/// The processes whose details this user may read: on Linux every one in /proc.
#[cfg(target_os = "linux")]
pub fn pids() -> Vec<i32> {
    let Ok(dir) = std::fs::read_dir("/proc") else { return Vec::new() };
    dir.flatten().filter_map(|e| e.file_name().to_str()?.parse().ok()).collect()
}

/// The program's name as the kernel keeps it (comm).
#[cfg(target_os = "linux")]
pub fn name(pid: i32) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/comm")).ok().map(|c| c.trim_end().to_string())
}

/// A process's arguments, the program first, when the kernel shows them to this user.
#[cfg(target_os = "linux")]
pub fn args(pid: i32) -> Option<Vec<String>> {
    std::fs::read(format!("/proc/{pid}/cmdline")).ok().map(|b| nul_separated(&b))
}

/// A process's environment (NAME=value), when the kernel shows it to this user: only
/// for this user's own processes.
#[cfg(target_os = "linux")]
pub fn environ(pid: i32) -> Option<Vec<String>> {
    std::fs::read(format!("/proc/{pid}/environ")).ok().map(|b| nul_separated(&b))
}

#[cfg(target_os = "linux")]
fn nul_separated(b: &[u8]) -> Vec<String> {
    b.split(|&c| c == 0).filter(|a| !a.is_empty()).map(|a| String::from_utf8_lossy(a).into_owned()).collect()
}

// ----------------------------------------------------------------------- macOS

#[cfg(target_os = "macos")]
extern "C" {
    fn proc_pidinfo(pid: i32, flavor: i32, arg: u64, buffer: *mut std::ffi::c_void, size: i32) -> i32;
    fn proc_listpids(kind: u32, typeinfo: u32, buffer: *mut std::ffi::c_void, size: i32) -> i32;
    fn proc_name(pid: i32, buffer: *mut std::ffi::c_void, size: u32) -> i32;
    fn sysctl(name: *mut i32, namelen: u32, old: *mut std::ffi::c_void, oldlen: *mut usize, new: *mut std::ffi::c_void, newlen: usize) -> i32;
    fn kill(pid: i32, sig: i32) -> i32;
}

/// proc_pidinfo's answer of `flavor` for `pid` into `buf`, when it filled all of it.
#[cfg(target_os = "macos")]
fn pidinfo(pid: i32, flavor: i32, buf: &mut [u8]) -> bool {
    let size = buf.len() as i32;
    // SAFETY: the kernel writes at most `size` bytes into the buffer.
    unsafe { proc_pidinfo(pid, flavor, 0, buf.as_mut_ptr().cast(), size) == size }
}

/// Running, and not a zombie (ended, waiting for its parent to collect it).
#[cfg(target_os = "macos")]
pub fn pid_alive(pid: i64) -> bool {
    const SZOMB: u32 = 5;
    const ESRCH: i32 = 3;
    let Some(pid) = valid(pid) else { return false };
    match bsd_status(pid) {
        Ok(status) => status != SZOMB,
        Err(Some(ESRCH)) => false,
        // No answer for another user's process (EPERM): kill(pid, 0) says whether it exists.
        Err(_) => {
            // SAFETY: signal 0 only checks that the process exists.
            let exists = unsafe { kill(pid, 0) } == 0;
            exists || std::io::Error::last_os_error().raw_os_error() == Some(1) // EPERM: it exists, and isn't ours
        }
    }
}

/// pbi_status from struct proc_bsdinfo (136 bytes: pbi_flags, then pbi_status), or the
/// error. The argument 1 asks for zombies too: without it the kernel doesn't find them.
#[cfg(target_os = "macos")]
fn bsd_status(pid: i32) -> Result<u32, Option<i32>> {
    const PROC_PIDTBSDINFO: i32 = 3;
    let mut info = [0u8; 136];
    // SAFETY: the kernel writes at most the buffer's size.
    let n = unsafe { proc_pidinfo(pid, PROC_PIDTBSDINFO, 1, info.as_mut_ptr().cast(), info.len() as i32) };
    if n != info.len() as i32 {
        return Err(std::io::Error::last_os_error().raw_os_error());
    }
    Ok(u32::from_ne_bytes([info[4], info[5], info[6], info[7]]))
}

/// The folder process `pid` is in right now, as the kernel has it: struct
/// proc_vnodepathinfo (2352 bytes) starts with the current folder's vnode_info (152
/// bytes) and its path (MAXPATHLEN, 1024 bytes, NUL-terminated).
#[cfg(target_os = "macos")]
pub fn cwd(pid: i64) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    const PROC_PIDVNODEPATHINFO: i32 = 9;
    let mut info = vec![0u8; 2352];
    if !pidinfo(valid(pid)?, PROC_PIDVNODEPATHINFO, &mut info) {
        return None;
    }
    let path = &info[152..152 + 1024];
    let path = &path[..path.iter().position(|&b| b == 0)?];
    (!path.is_empty()).then(|| PathBuf::from(std::ffi::OsStr::from_bytes(path)))
}

/// The folder that lists this process's open file descriptors.
#[cfg(target_os = "macos")]
pub const FD_DIR: &str = "/dev/fd";

/// macOS doesn't count out-of-memory kills where a program can read them.
#[cfg(target_os = "macos")]
pub fn oom_kills() -> Option<u64> {
    None
}

/// The processes whose details this user may read: on macOS this user's own.
#[cfg(target_os = "macos")]
pub fn pids() -> Vec<i32> {
    const PROC_UID_ONLY: u32 = 4;
    let uid = euid();
    // SAFETY: with no buffer, proc_listpids says how many bytes the list takes.
    let need = unsafe { proc_listpids(PROC_UID_ONLY, uid, std::ptr::null_mut(), 0) };
    if need <= 0 {
        return Vec::new();
    }
    // Room for processes that start in between.
    let mut pids = vec![0i32; need as usize / 4 + 64];
    // SAFETY: the kernel writes at most the buffer's size in bytes.
    let got = unsafe { proc_listpids(PROC_UID_ONLY, uid, pids.as_mut_ptr().cast(), (pids.len() * 4) as i32) };
    pids.truncate(got.max(0) as usize / 4);
    pids.retain(|&p| p > 0);
    pids
}

/// The program's name as the kernel keeps it.
#[cfg(target_os = "macos")]
pub fn name(pid: i32) -> Option<String> {
    let mut buf = [0u8; 256];
    // SAFETY: proc_name writes at most the buffer's size and returns the length.
    let n = unsafe { proc_name(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    (n > 0).then(|| String::from_utf8_lossy(&buf[..n as usize]).into_owned())
}

/// A process's arguments and environment, from sysctl's KERN_PROCARGS2: the number of
/// arguments (a C int), the program's path, NULs, the arguments and then the environment,
/// each NUL-terminated. The kernel gives it only for this user's own processes.
#[cfg(target_os = "macos")]
fn procargs(pid: i32) -> Option<(Vec<String>, Vec<String>)> {
    const CTL_KERN: i32 = 1;
    const KERN_ARGMAX: i32 = 8;
    const KERN_PROCARGS2: i32 = 49;
    let mut argmax: i32 = 0;
    let mut len = std::mem::size_of::<i32>();
    let mut mib = [CTL_KERN, KERN_ARGMAX];
    // SAFETY: sysctl writes one int into argmax.
    if unsafe { sysctl(mib.as_mut_ptr(), 2, (&mut argmax as *mut i32).cast(), &mut len, std::ptr::null_mut(), 0) } != 0 || argmax <= 4 {
        return None;
    }
    let mut buf = vec![0u8; argmax as usize];
    let mut len = buf.len();
    let mut mib = [CTL_KERN, KERN_PROCARGS2, pid];
    // SAFETY: sysctl writes at most `len` bytes into the buffer and sets `len` to what it wrote.
    if unsafe { sysctl(mib.as_mut_ptr(), 3, buf.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0) } != 0 || len < 4 {
        return None;
    }
    buf.truncate(len);
    let argc = i32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]).max(0) as usize;
    let rest = &buf[4..];
    let path_end = rest.iter().position(|&b| b == 0)?;
    let start = path_end + rest[path_end..].iter().position(|&b| b != 0)?;
    let mut strings = rest[start..].split(|&b| b == 0).map(|s| String::from_utf8_lossy(s).into_owned());
    let args: Vec<String> = strings.by_ref().take(argc).collect();
    let env = strings.take_while(|s| !s.is_empty()).collect();
    Some((args, env))
}

/// A process's arguments, the program first, when the kernel shows them to this user.
#[cfg(target_os = "macos")]
pub fn args(pid: i32) -> Option<Vec<String>> {
    procargs(pid).map(|(a, _)| a)
}

/// A process's environment (NAME=value), when the kernel shows it to this user: only
/// for this user's own processes.
#[cfg(target_os = "macos")]
pub fn environ(pid: i32) -> Option<Vec<String>> {
    procargs(pid).map(|(_, e)| e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    #[test]
    fn a_running_process_is_alive_and_an_ended_one_is_not() {
        assert!(pid_alive(std::process::id() as i64));
        for pid in [0, -1, i64::MAX] {
            assert!(!pid_alive(pid), "{pid}");
        }
        let mut child = Command::new("sleep").arg("30").stdin(Stdio::null()).spawn().unwrap();
        let pid = child.id() as i64;
        assert!(pid_alive(pid));
        // Ended but not collected yet: a zombie, which counts as gone.
        child.kill().unwrap();
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while pid_alive(pid) {
            assert!(std::time::Instant::now() < end, "a zombie still counts as alive: {}", state(pid));
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        child.wait().unwrap();
        assert!(!pid_alive(pid));
    }

    /// What the kernel says about `pid`, for a failed assertion.
    #[cfg(target_os = "linux")]
    fn state(pid: i64) -> String {
        std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_else(|e| e.to_string())
    }

    #[cfg(target_os = "macos")]
    fn state(pid: i64) -> String {
        format!("pbi_status or errno: {:?}", bsd_status(pid as i32))
    }

    #[test]
    fn the_folder_a_process_is_in() {
        let dir = std::env::temp_dir().join(format!("corral-test-cwd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut child = Command::new("sleep").arg("30").current_dir(&dir).stdin(Stdio::null()).spawn().unwrap();
        let seen = cwd(child.id() as i64);
        child.kill().unwrap();
        child.wait().unwrap();
        // The real path: macOS's temporary folder is behind a link (/var -> /private/var).
        assert_eq!(seen, Some(std::fs::canonicalize(&dir).unwrap()));
        assert_eq!(cwd(std::process::id() as i64), Some(std::env::current_dir().unwrap().canonicalize().unwrap()));
        assert_eq!(cwd(0), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_descriptors_are_listed() {
        let fds: Vec<i32> = std::fs::read_dir(FD_DIR).unwrap().flatten()
            .filter_map(|e| e.file_name().to_str()?.parse().ok()).collect();
        assert!([0, 1, 2].iter().all(|fd| fds.contains(fd)), "{fds:?}");
    }

    #[test]
    fn the_user_is_the_owner_of_a_new_file() {
        use std::os::unix::fs::MetadataExt;
        let file = std::env::temp_dir().join(format!("corral-test-euid-{}", std::process::id()));
        std::fs::write(&file, "").unwrap();
        assert_eq!(std::fs::metadata(&file).unwrap().uid(), euid());
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn a_process_shows_its_name_arguments_and_environment() {
        let mut child = Command::new("sleep").arg("31").env("CORRAL_TEST_MARK", "here is it").stdin(Stdio::null()).spawn().unwrap();
        let pid = child.id() as i32;
        // spawn() can return a moment before the kernel has the new program's name and
        // arguments in place.
        let started = || name(pid).as_deref() == Some("sleep") && args(pid).is_some_and(|a| a.get(1).is_some_and(|a| a == "31"));
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !started() && std::time::Instant::now() < end {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let (name, args, env, listed) = (name(pid), args(pid), environ(pid), pids().contains(&pid));
        child.kill().unwrap();
        child.wait().unwrap();
        assert_eq!(name.as_deref(), Some("sleep"));
        assert_eq!(args.as_deref().and_then(|a| a.get(1)).map(String::as_str), Some("31"), "{args:?}");
        assert!(env.unwrap_or_default().iter().any(|v| v == "CORRAL_TEST_MARK=here is it"));
        assert!(listed);
    }
}
