use super::acl;
use super::process::arguments;
use super::process_ancestry::snapshot_parent_map;
use super::process_win32::{process_command_line, process_image_path, process_sid, OwnedHandle};
use std::{
    collections::{HashMap, HashSet},
    mem::{offset_of, size_of},
    path::Path,
};
use windows_sys::Win32::{
    Foundation::{ERROR_INSUFFICIENT_BUFFER, NO_ERROR},
    NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCP6TABLE_OWNER_PID, MIB_TCPROW_OWNER_PID,
        MIB_TCPTABLE_OWNER_PID, MIB_TCP_STATE_LISTEN, TCP_TABLE_OWNER_PID_ALL,
    },
    System::Threading::GetCurrentProcessId,
};

const AF_INET: u32 = 2;
const AF_INET6: u32 = 23;

fn query_tcp_table(
    family: u32,
    inspect: impl Fn(&[u32], usize, &mut Vec<u32>, &mut Vec<u32>),
    listeners: &mut Vec<u32>,
    connected: &mut Vec<u32>,
) {
    let mut size = 0u32;
    let initial = unsafe {
        GetExtendedTcpTable(
            std::ptr::null_mut(),
            &mut size,
            0,
            family,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        )
    };
    if initial != ERROR_INSUFFICIENT_BUFFER || size == 0 {
        return;
    }
    let mut words = vec![0u32; (size as usize).div_ceil(size_of::<u32>())];
    if unsafe {
        GetExtendedTcpTable(
            words.as_mut_ptr().cast(),
            &mut size,
            0,
            family,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        )
    } == NO_ERROR
    {
        let byte_len = (size as usize).min(words.len() * size_of::<u32>());
        inspect(&words, byte_len, listeners, connected);
    }
}

fn tcp_pids_for_port(port: u16) -> Vec<u32> {
    let mut listeners = Vec::new();
    let mut connected = Vec::new();
    query_tcp_table(
        AF_INET,
        |words, byte_len, listeners, connected| {
            let hdr = offset_of!(MIB_TCPTABLE_OWNER_PID, table);
            if byte_len < hdr {
                return;
            }
            let ptr = words.as_ptr().cast::<u8>();
            let count = unsafe { (*(ptr.cast::<MIB_TCPTABLE_OWNER_PID>())).dwNumEntries } as usize;
            let needed =
                hdr.saturating_add(count.saturating_mul(size_of::<MIB_TCPROW_OWNER_PID>()));
            if byte_len < needed {
                return;
            }
            let rows = unsafe {
                std::slice::from_raw_parts(ptr.add(hdr).cast::<MIB_TCPROW_OWNER_PID>(), count)
            };
            for row in rows {
                if row.dwState == MIB_TCP_STATE_LISTEN as u32
                    && u16::from_be(row.dwLocalPort as u16) == port
                {
                    listeners.push(row.dwOwningPid);
                } else if row.dwRemoteAddr.to_ne_bytes() == [127, 0, 0, 1]
                    && u16::from_be(row.dwRemotePort as u16) == port
                {
                    connected.push(row.dwOwningPid);
                }
            }
        },
        &mut listeners,
        &mut connected,
    );
    query_tcp_table(
        AF_INET6,
        |words, byte_len, listeners, connected| {
            let hdr = offset_of!(MIB_TCP6TABLE_OWNER_PID, table);
            if byte_len < hdr {
                return;
            }
            let ptr = words.as_ptr().cast::<u8>();
            let count = unsafe { (*(ptr.cast::<MIB_TCP6TABLE_OWNER_PID>())).dwNumEntries } as usize;
            let needed =
                hdr.saturating_add(count.saturating_mul(size_of::<MIB_TCP6ROW_OWNER_PID>()));
            if byte_len < needed {
                return;
            }
            let rows = unsafe {
                std::slice::from_raw_parts(ptr.add(hdr).cast::<MIB_TCP6ROW_OWNER_PID>(), count)
            };
            let loopback6 = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
            for row in rows {
                if row.dwState == MIB_TCP_STATE_LISTEN as u32
                    && u16::from_be(row.dwLocalPort as u16) == port
                {
                    listeners.push(row.dwOwningPid);
                } else if row.ucRemoteAddr == loopback6
                    && u16::from_be(row.dwRemotePort as u16) == port
                {
                    connected.push(row.dwOwningPid);
                }
            }
        },
        &mut listeners,
        &mut connected,
    );
    listeners.extend(connected);
    listeners
}

fn is_codex_executable(path: &Path) -> bool {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem.eq_ignore_ascii_case("codex"))
}

fn codex_args_for_pid(pid: u32, owner: &str) -> Option<Vec<String>> {
    let handle = OwnedHandle::open_limited(pid).ok()?;
    if process_sid(handle.0).as_deref() != Some(owner) {
        return None;
    }
    if !process_image_path(handle.0).is_some_and(|path| is_codex_executable(&path)) {
        return None;
    }
    process_command_line(handle.0).and_then(|cmd| arguments(&cmd))
}

pub(crate) fn codex_cli_on_port(
    port: u16,
    wanted_env: &[&str],
) -> Option<(Vec<String>, HashMap<String, String>)> {
    if port == 0 {
        return None;
    }
    let owner = acl::sid().ok()?;
    let map = snapshot_parent_map();
    let self_pid = unsafe { GetCurrentProcessId() };
    let mut visited = HashSet::new();
    for root_pid in tcp_pids_for_port(port) {
        let mut cursor = root_pid;
        for _ in 0..8 {
            if cursor == 0 || !visited.insert(cursor) {
                break;
            }
            if let Some(args) = codex_args_for_pid(cursor, &owner) {
                let env = if cursor == self_pid {
                    wanted_env
                        .iter()
                        .filter_map(|&k| std::env::var(k).ok().map(|v| (k.to_string(), v)))
                        .collect()
                } else {
                    HashMap::new()
                };
                return Some((args, env));
            }
            let Some(next) = map.as_ref().and_then(|m| m.get(&cursor).copied()) else {
                break;
            };
            cursor = next;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_port_never_matches_codex_cli() {
        assert!(codex_cli_on_port(0, &["TERM_PROGRAM"]).is_none());
    }
}
