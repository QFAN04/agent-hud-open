use super::super::ProcessAncestor;
use super::acl;
use super::process::arguments;
use super::process_win32::{
    process_command_line, process_creation_ticks, process_image_path, process_sid, OwnedHandle,
};
use std::{
    collections::{HashMap, HashSet},
    mem::{size_of, zeroed},
};
use windows_sys::Win32::System::{
    Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    },
    Threading::GetCurrentProcessId,
};

pub(super) fn snapshot_parent_map() -> Option<HashMap<u32, u32>> {
    let snap = OwnedHandle::from_raw(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) })?;
    let mut entry: PROCESSENTRY32W = unsafe { zeroed() };
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    if unsafe { Process32FirstW(snap.0, &mut entry) } == 0 {
        return None;
    }
    let mut map = HashMap::new();
    loop {
        if entry.th32ProcessID != 0 {
            map.insert(entry.th32ProcessID, entry.th32ParentProcessID);
        }
        if unsafe { Process32NextW(snap.0, &mut entry) } == 0 {
            break;
        }
    }
    Some(map)
}

pub(super) fn parent_processes_native() -> Option<Vec<ProcessAncestor>> {
    let owner = acl::sid().ok()?;
    let map = snapshot_parent_map()?;
    let self_pid = unsafe { GetCurrentProcessId() };
    let self_handle = OwnedHandle::open_limited(self_pid).ok()?;
    let mut child_ticks = process_creation_ticks(self_handle.0)?;
    let mut cursor = self_pid;
    let mut seen = HashSet::new();
    let mut chain = Vec::new();
    for _ in 0..16 {
        let Some(&parent_pid) = map.get(&cursor) else {
            break;
        };
        if parent_pid == 0 || !seen.insert(parent_pid) {
            break;
        }
        let Ok(handle) = OwnedHandle::open_limited(parent_pid) else {
            break;
        };
        let Some(parent_ticks) = process_creation_ticks(handle.0) else {
            break;
        };
        if parent_ticks > child_ticks || process_sid(handle.0).as_deref() != Some(owner.as_str()) {
            break;
        }
        let Some(executable) = process_image_path(handle.0) else {
            break;
        };
        let Some(command) = process_command_line(handle.0) else {
            break;
        };
        let Some(args) = arguments(&command) else {
            break;
        };
        chain.push(ProcessAncestor {
            pid: Some(parent_pid),
            executable,
            arguments: args,
            bundle: None,
        });
        child_ticks = parent_ticks;
        cursor = parent_pid;
    }
    Some(chain)
}
