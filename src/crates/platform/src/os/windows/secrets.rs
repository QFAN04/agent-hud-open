//! The GitHub CLI's existing generic Windows credential, after the Copilot
//! reader's consent gate. go-keyring stores UTF-8 bytes at `service:username`;
//! the empty-username slot is gh's active-token compatibility entry.
use agent_hud_agents::Answer;
use windows_sys::Win32::{
    Foundation::{GetLastError, ERROR_ACCESS_DENIED, ERROR_NOT_FOUND, ERROR_NO_SUCH_LOGON_SESSION},
    Security::Credentials::{CredEnumerateW, CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC},
};

const LIMIT: usize = 32 * 1024;

pub(crate) fn stored_secret(service: &str) -> Answer<String> {
    if service != "gh:github.com" {
        return Answer::Unsupported;
    }
    match read_target(&format!("{service}:")) {
        Answer::Missing => enumerate_targets(&format!("{service}:*")),
        other => other,
    }
}

struct Credential(*mut CREDENTIALW);
impl Drop for Credential {
    fn drop(&mut self) {
        unsafe { CredFree(self.0.cast()) };
    }
}

struct CredentialList {
    ptr: *mut *mut CREDENTIALW,
}
impl Drop for CredentialList {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { CredFree(self.ptr.cast()) };
        }
    }
}

fn decode_credential(credential: &CREDENTIALW) -> Answer<String> {
    if credential.CredentialBlobSize == 0 {
        return Answer::Found(String::new());
    }
    if credential.CredentialBlobSize as usize > LIMIT || credential.CredentialBlob.is_null() {
        return Answer::Failed;
    }
    let bytes = unsafe {
        std::slice::from_raw_parts(
            credential.CredentialBlob,
            credential.CredentialBlobSize as usize,
        )
    };
    String::from_utf8(bytes.to_vec()).map_or(Answer::Failed, Answer::Found)
}

fn read_target(target: &str) -> Answer<String> {
    let target: Vec<_> = target.encode_utf16().chain([0]).collect();
    let mut credential = std::ptr::null_mut();
    if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) } == 0 {
        return match unsafe { GetLastError() } {
            ERROR_NOT_FOUND => Answer::Missing,
            ERROR_ACCESS_DENIED | ERROR_NO_SUCH_LOGON_SESSION => Answer::Denied,
            _ => Answer::Failed,
        };
    }
    if credential.is_null() {
        return Answer::Failed;
    }
    let allocation = Credential(credential);
    decode_credential(unsafe { &*allocation.0 })
}

fn enumerate_targets(filter: &str) -> Answer<String> {
    let filter: Vec<_> = filter.encode_utf16().chain([0]).collect();
    let mut count = 0u32;
    let mut list = std::ptr::null_mut();
    if unsafe { CredEnumerateW(filter.as_ptr(), 0, &mut count, &mut list) } == 0 {
        return match unsafe { GetLastError() } {
            ERROR_NOT_FOUND => Answer::Missing,
            ERROR_ACCESS_DENIED | ERROR_NO_SUCH_LOGON_SESSION => Answer::Denied,
            _ => Answer::Failed,
        };
    }
    if list.is_null() || count == 0 {
        return Answer::Missing;
    }
    let _guard = CredentialList { ptr: list };
    let entries = unsafe { std::slice::from_raw_parts(list, count as usize) };
    let mut fallback = Answer::Missing;
    for &entry in entries {
        if entry.is_null() {
            continue;
        }
        let cred = unsafe { &*entry };
        if cred.Type != CRED_TYPE_GENERIC {
            continue;
        }
        match decode_credential(cred) {
            Answer::Found(value) if !value.is_empty() => return Answer::Found(value),
            Answer::Found(empty) => fallback = Answer::Found(empty),
            other => fallback = other,
        }
    }
    fallback
}

pub(crate) use super::cookies::app_cookie;
pub(crate) use super::dpapi::unseal_safe_storage;

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Security::Credentials::{
        CredDeleteW, CredWriteW, CRED_PERSIST_SESSION,
    };

    #[test]
    fn generic_credential_reads_only_the_named_fixture_as_utf8() {
        let unique = tempfile::tempdir().unwrap();
        let target = format!("agenthud-test:{}:", unique.path().display());
        let wide: Vec<_> = target.encode_utf16().chain([0]).collect();
        assert_eq!(read_target(&target), Answer::Missing);
        let mut bytes = "fixture-令牌".as_bytes().to_vec();
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: wide.as_ptr().cast_mut(),
            Persist: CRED_PERSIST_SESSION,
            CredentialBlobSize: bytes.len() as u32,
            CredentialBlob: bytes.as_mut_ptr(),
            ..unsafe { std::mem::zeroed() }
        };
        assert_ne!(unsafe { CredWriteW(&credential, 0) }, 0);
        struct Cleanup(Vec<u16>);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                unsafe { CredDeleteW(self.0.as_ptr(), CRED_TYPE_GENERIC, 0) };
            }
        }
        let _cleanup = Cleanup(wide);
        assert_eq!(read_target(&target), Answer::Found("fixture-令牌".into()));
        assert_eq!(stored_secret("unrelated-service"), Answer::Unsupported);
    }
}
