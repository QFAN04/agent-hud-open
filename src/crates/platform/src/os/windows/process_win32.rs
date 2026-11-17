use super::acl::Local;
use std::{ffi::c_void, mem::size_of, path::PathBuf};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, GetLastError, ERROR_ACCESS_DENIED, FILETIME, HANDLE, INVALID_HANDLE_VALUE,
        STATUS_SUCCESS, STILL_ACTIVE,
    },
    Security::{
        Authorization::ConvertSidToStringSidW, GetTokenInformation, TokenUser, TOKEN_QUERY,
        TOKEN_USER,
    },
    System::{
        Console::GetConsoleProcessList,
        Threading::{
            GetExitCodeProcess, GetProcessTimes, OpenProcess, OpenProcessToken,
            QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        },
    },
};

const PROCESS_CONSOLE_HOST_PROCESS: u32 = 49;
const PROCESS_COMMAND_LINE_INFORMATION: u32 = 60;
const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xC000_0004_u32 as i32;
const STATUS_BUFFER_TOO_SMALL: i32 = 0xC000_0023_u32 as i32;
const UNIX_EPOCH_OFFSET_SECS: u64 = 11_644_473_600;

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtQueryInformationProcess(
        process_handle: HANDLE,
        process_information_class: u32,
        process_information: *mut c_void,
        process_information_length: u32,
        return_length: *mut u32,
    ) -> i32;
}

#[repr(C)]
struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *const u16,
}

pub(super) struct OwnedHandle(pub(super) HANDLE);

impl OwnedHandle {
    pub(super) fn open_limited(pid: u32) -> Result<Self, u32> {
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            Err(unsafe { GetLastError() })
        } else {
            Ok(Self(handle))
        }
    }

    pub(super) fn from_raw(handle: HANDLE) -> Option<Self> {
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            None
        } else {
            Some(Self(handle))
        }
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(self.0) };
        }
    }
}

pub(super) fn process_creation_ticks(handle: HANDLE) -> Option<u64> {
    let mut creation = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut exit = creation;
    let mut kernel = creation;
    let mut user = creation;
    if unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) } == 0 {
        return None;
    }
    Some(((creation.dwHighDateTime as u64) << 32) | (creation.dwLowDateTime as u64))
}

fn ticks_to_unix(ticks: u64) -> Option<i64> {
    let secs = ticks / 10_000_000;
    (secs >= UNIX_EPOCH_OFFSET_SECS).then(|| (secs - UNIX_EPOCH_OFFSET_SECS) as i64)
}

fn process_is_active(handle: HANDLE) -> bool {
    let mut code = 0u32;
    unsafe { GetExitCodeProcess(handle, &mut code) != 0 && code == STILL_ACTIVE as u32 }
}

pub(crate) fn process_start(pid: i32) -> Option<i64> {
    let pid = u32::try_from(pid).ok().filter(|&pid| pid > 0)?;
    let handle = OwnedHandle::open_limited(pid).ok()?;
    process_creation_ticks(handle.0).and_then(ticks_to_unix)
}

pub(crate) fn process_runs(pid: i32, started: Option<i64>) -> bool {
    let Ok(pid) = u32::try_from(pid) else {
        return false;
    };
    if pid == 0 {
        return false;
    }
    let handle = match OwnedHandle::open_limited(pid) {
        Ok(handle) => handle,
        Err(err) => return err == ERROR_ACCESS_DENIED,
    };
    if !process_is_active(handle.0) {
        return false;
    }
    if let (Some(said), Some(began)) = (
        started,
        process_creation_ticks(handle.0).and_then(ticks_to_unix),
    ) {
        return (said - began).abs() <= 2;
    }
    true
}

pub(crate) fn terminal_io(pid: i32) -> bool {
    let Ok(pid) = u32::try_from(pid) else {
        return false;
    };
    if pid == 0 {
        return false;
    }
    let mut attached = [0u32; 64];
    let count = unsafe { GetConsoleProcessList(attached.as_mut_ptr(), attached.len() as u32) };
    if count > 0 && (count as usize) <= attached.len() && attached[..count as usize].contains(&pid)
    {
        return true;
    }
    let Ok(handle) = OwnedHandle::open_limited(pid) else {
        return false;
    };
    if !process_is_active(handle.0) {
        return false;
    }
    let mut host = 0usize;
    let mut returned = 0u32;
    let status = unsafe {
        NtQueryInformationProcess(
            handle.0,
            PROCESS_CONSOLE_HOST_PROCESS,
            (&raw mut host).cast(),
            size_of::<usize>() as u32,
            &mut returned,
        )
    };
    status == STATUS_SUCCESS && (host & 1) == 1
}

pub(super) fn process_sid(handle: HANDLE) -> Option<String> {
    let mut raw_token: HANDLE = std::ptr::null_mut();
    if unsafe { OpenProcessToken(handle, TOKEN_QUERY, &mut raw_token) } == 0 {
        return None;
    }
    let token = OwnedHandle::from_raw(raw_token)?;
    let mut bytes = 0u32;
    unsafe { GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut bytes) };
    if bytes == 0 {
        return None;
    }
    let mut buf = vec![0usize; (bytes as usize).div_ceil(size_of::<usize>())];
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            buf.as_mut_ptr().cast(),
            bytes,
            &mut bytes,
        )
    } == 0
    {
        return None;
    }
    let user = unsafe { &*(buf.as_ptr().cast::<TOKEN_USER>()) };
    let mut raw_sid: *mut u16 = std::ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut raw_sid) } == 0 || raw_sid.is_null() {
        return None;
    }
    let _guard = Local(raw_sid.cast());
    let mut len = 0usize;
    unsafe {
        while *raw_sid.add(len) != 0 {
            len += 1;
        }
        Some(String::from_utf16_lossy(std::slice::from_raw_parts(
            raw_sid, len,
        )))
    }
}

pub(super) fn process_image_path(handle: HANDLE) -> Option<PathBuf> {
    let mut buf = vec![0u16; 1024];
    let mut len = buf.len() as u32;
    if unsafe { QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) }
        == 0
    {
        return None;
    }
    Some(PathBuf::from(String::from_utf16_lossy(
        &buf[..len as usize],
    )))
}

pub(super) fn process_command_line(handle: HANDLE) -> Option<String> {
    let mut words = vec![0usize; 1024];
    let mut needed = 0u32;
    let mut status = unsafe {
        NtQueryInformationProcess(
            handle,
            PROCESS_COMMAND_LINE_INFORMATION,
            words.as_mut_ptr().cast(),
            (words.len() * size_of::<usize>()) as u32,
            &mut needed,
        )
    };
    if (status == STATUS_INFO_LENGTH_MISMATCH || status == STATUS_BUFFER_TOO_SMALL) && needed > 0 {
        words.resize((needed as usize).div_ceil(size_of::<usize>()), 0);
        status = unsafe {
            NtQueryInformationProcess(
                handle,
                PROCESS_COMMAND_LINE_INFORMATION,
                words.as_mut_ptr().cast(),
                (words.len() * size_of::<usize>()) as u32,
                &mut needed,
            )
        };
    }
    if status != STATUS_SUCCESS || words.len() * size_of::<usize>() < size_of::<UnicodeString>() {
        return None;
    }
    let us = unsafe { &*(words.as_ptr().cast::<UnicodeString>()) };
    if us.buffer.is_null() || us.length == 0 || (us.length & 1) != 0 {
        return None;
    }
    let start = words.as_ptr() as usize;
    let end = start + words.len() * size_of::<usize>();
    let ptr = us.buffer as usize;
    let byte_len = us.length as usize;
    if ptr < start || ptr.checked_add(byte_len)? > end {
        return None;
    }
    let slice = unsafe { std::slice::from_raw_parts(us.buffer, byte_len / 2) };
    Some(String::from_utf16_lossy(slice))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn process_start_and_runs_track_current_and_exited_processes() {
        let self_pid = std::process::id() as i32;
        let started = process_start(self_pid).expect("current process has creation time");
        assert!(process_runs(self_pid, None));
        assert!(process_runs(self_pid, Some(started)));
        assert!(!process_runs(self_pid, Some(started - 3600)));
        assert!(!process_runs(0, None));
        assert!(!process_runs(-1, None));
        assert!(process_start(0).is_none());
        assert!(!terminal_io(0));

        let system = std::env::var_os("SystemRoot").unwrap();
        let mut child = Command::new(PathBuf::from(system).join("System32/cmd.exe"))
            .args(["/c", "exit", "0"])
            .spawn()
            .unwrap();
        let child_pid = child.id() as i32;
        let _ = child.wait();
        assert!(!process_runs(child_pid, None));
        assert!(!terminal_io(child_pid));
    }
}
