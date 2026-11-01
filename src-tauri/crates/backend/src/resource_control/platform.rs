//! Native process accounting without invoking AI-controlled shell utilities.

use super::types::ProcessSample;
use std::io;

#[cfg(target_os = "linux")]
pub(super) fn snapshot() -> io::Result<Vec<ProcessSample>> {
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page_size <= 0 {
        return Err(io::Error::last_os_error());
    }
    let mut samples = Vec::new();
    for entry in std::fs::read_dir("/proc")?.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse().ok())
        else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        let swap_kb = std::fs::read_to_string(entry.path().join("status"))
            .ok()
            .and_then(|status| {
                status.lines().find_map(|line| {
                    line.strip_prefix("VmSwap:")?
                        .split_whitespace()
                        .next()?
                        .parse::<u64>()
                        .ok()
                })
            })
            .unwrap_or(0);
        if let Some(sample) = parse_linux_stat(pid, &stat, page_size as u64, swap_kb) {
            samples.push(sample);
        }
    }
    Ok(samples)
}

#[cfg(target_os = "linux")]
fn parse_linux_stat(pid: u32, stat: &str, page_size: u64, swap_kb: u64) -> Option<ProcessSample> {
    let fields: Vec<_> = stat.rsplit_once(") ")?.1.split_whitespace().collect();
    Some(ProcessSample {
        pid,
        parent: fields.get(1)?.parse().ok()?,
        group: fields.get(2)?.parse().ok()?,
        started: fields.get(19)?.parse().ok()?,
        memory_bytes: fields
            .get(21)?
            .parse::<u64>()
            .ok()?
            .saturating_mul(page_size)
            + swap_kb.saturating_mul(1024),
    })
}

#[cfg(target_os = "macos")]
pub(super) fn snapshot() -> io::Result<Vec<ProcessSample>> {
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 {
        return Err(io::Error::last_os_error());
    }
    let mut pids = vec![0i32; count as usize + 256];
    let count = unsafe {
        libc::proc_listallpids(
            pids.as_mut_ptr().cast(),
            std::mem::size_of_val(pids.as_slice()) as i32,
        )
    };
    if count <= 0 {
        return Err(io::Error::last_os_error());
    }
    let mut samples = Vec::new();
    for &pid in pids.iter().take(count as usize).filter(|&&pid| pid > 0) {
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let mut task: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
        let info_size = std::mem::size_of_val(&info) as i32;
        let task_size = std::mem::size_of_val(&task) as i32;
        let info_read = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                info_size,
            )
        };
        let task_read = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTASKINFO,
                0,
                (&mut task as *mut libc::proc_taskinfo).cast(),
                task_size,
            )
        };
        if info_read == info_size && task_read == task_size {
            samples.push(ProcessSample {
                pid: pid as u32,
                parent: info.pbi_ppid,
                group: info.pbi_pgid,
                started: info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec,
                memory_bytes: task.pti_resident_size,
            });
        }
    }
    Ok(samples)
}

#[cfg(windows)]
pub(super) fn snapshot() -> io::Result<Vec<ProcessSample>> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
    };

    let handle = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let mut samples = Vec::new();
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of_val(&entry) as u32;
    let mut found = unsafe { Process32FirstW(handle, &mut entry) };
    while found != 0 {
        let process = unsafe {
            OpenProcess(
                PROCESS_QUERY_INFORMATION | PROCESS_VM_READ,
                0,
                entry.th32ProcessID,
            )
        };
        if !process.is_null() {
            let mut memory: PROCESS_MEMORY_COUNTERS_EX = unsafe { std::mem::zeroed() };
            let size = std::mem::size_of_val(&memory) as u32;
            let measured = unsafe {
                K32GetProcessMemoryInfo(
                    process,
                    (&mut memory as *mut PROCESS_MEMORY_COUNTERS_EX)
                        .cast::<PROCESS_MEMORY_COUNTERS>(),
                    size,
                )
            };
            if measured != 0 {
                if let Some(started) = windows_creation_time(process) {
                    samples.push(ProcessSample {
                        pid: entry.th32ProcessID,
                        parent: entry.th32ParentProcessID,
                        group: 0,
                        started,
                        memory_bytes: memory.PrivateUsage as u64,
                    });
                }
            }
            unsafe { CloseHandle(process) };
        }
        found = unsafe { Process32NextW(handle, &mut entry) };
    }
    unsafe { CloseHandle(handle) };
    Ok(samples)
}

#[cfg(windows)]
fn windows_creation_time(handle: windows_sys::Win32::Foundation::HANDLE) -> Option<u64> {
    use windows_sys::Win32::Foundation::FILETIME;
    let mut created: FILETIME = unsafe { std::mem::zeroed() };
    let mut exited = created;
    let mut kernel = created;
    let mut user = created;
    let success = unsafe {
        windows_sys::Win32::System::Threading::GetProcessTimes(
            handle,
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    };
    (success != 0)
        .then_some((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub(super) fn snapshot() -> io::Result<Vec<ProcessSample>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "process accounting is unavailable",
    ))
}

pub(super) fn terminate(samples: &[ProcessSample]) {
    let Ok(current) = snapshot() else { return };
    for sample in samples.iter().rev() {
        if !current
            .iter()
            .any(|process| process.pid == sample.pid && process.started == sample.started)
        {
            continue;
        }
        #[cfg(unix)]
        unsafe {
            libc::kill(sample.pid as libc::pid_t, libc::SIGKILL);
        }
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::Foundation::CloseHandle;
            use windows_sys::Win32::System::Threading::{
                OpenProcess, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
            };
            let handle = OpenProcess(
                PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                sample.pid,
            );
            if !handle.is_null() {
                if windows_creation_time(handle) == Some(sample.started) {
                    TerminateProcess(handle, 1);
                }
                CloseHandle(handle);
            }
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn resource_snapshot_includes_the_current_process() {
        let samples = snapshot().unwrap();
        let process = samples
            .iter()
            .find(|process| process.pid == std::process::id())
            .unwrap();
        assert!(process.started > 0);
        assert!(process.memory_bytes > 0);
    }

    #[test]
    fn resource_linux_stat_handles_spaces_and_parentheses_in_names() {
        let mut fields = vec!["0"; 22];
        fields[0] = "S";
        fields[1] = "10";
        fields[2] = "42";
        fields[19] = "1234";
        fields[21] = "12";
        let stat = format!("42 (name ) with spaces) {}", fields.join(" "));
        let process = parse_linux_stat(42, &stat, 4096, 100).unwrap();
        assert_eq!(process.parent, 10);
        assert_eq!(process.group, 42);
        assert_eq!(process.started, 1234);
        assert_eq!(process.memory_bytes, 12 * 4096 + 100 * 1024);
    }
}
