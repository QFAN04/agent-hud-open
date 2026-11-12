use super::acl::Local;
use super::{app_support, claude_desktop_bases, home_dir};
use agent_hud_agents::Answer;
use base64::{engine::general_purpose::STANDARD, Engine};
use sha2::{Digest, Sha256};
use std::{
    mem::size_of,
    path::{Path, PathBuf},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{GetLastError, ERROR_ACCESS_DENIED, ERROR_NO_SUCH_LOGON_SESSION, STATUS_SUCCESS},
    Security::Cryptography::{
        BCryptCloseAlgorithmProvider, BCryptDecrypt, BCryptDestroyKey, BCryptGenerateSymmetricKey,
        BCryptOpenAlgorithmProvider, BCryptSetProperty, CryptUnprotectData, BCRYPT_AES_ALGORITHM,
        BCRYPT_ALG_HANDLE, BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO,
        BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO_VERSION, BCRYPT_CHAINING_MODE, BCRYPT_KEY_HANDLE,
        CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    },
};

pub(super) struct AlgHandle(pub(super) BCRYPT_ALG_HANDLE);
impl Drop for AlgHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { BCryptCloseAlgorithmProvider(self.0, 0) };
        }
    }
}

pub(super) struct KeyHandle(pub(super) BCRYPT_KEY_HANDLE);
impl Drop for KeyHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { BCryptDestroyKey(self.0) };
        }
    }
}

pub(super) fn unprotect_dpapi(encrypted: &[u8]) -> Answer<Vec<u8>> {
    let Ok(len) = u32::try_from(encrypted.len()) else {
        return Answer::Failed;
    };
    if len == 0 {
        return Answer::Missing;
    }
    let input = CRYPT_INTEGER_BLOB {
        cbData: len,
        pbData: encrypted.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };
    if unsafe {
        CryptUnprotectData(
            &input,
            ptr::null_mut(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    } == 0
    {
        return match unsafe { GetLastError() } {
            ERROR_ACCESS_DENIED | ERROR_NO_SUCH_LOGON_SESSION => Answer::Denied,
            _ => Answer::Failed,
        };
    }
    let _guard = Local(output.pbData.cast());
    if output.cbData == 0 {
        return Answer::Found(Vec::new());
    }
    if output.pbData.is_null() {
        return Answer::Failed;
    }
    let plain = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) };
    Answer::Found(plain.to_vec())
}

pub(super) fn open_gcm_key(key: &[u8]) -> Result<(AlgHandle, KeyHandle), ()> {
    if key.len() != 32 {
        return Err(());
    }
    let mut raw_alg: BCRYPT_ALG_HANDLE = ptr::null_mut();
    if unsafe { BCryptOpenAlgorithmProvider(&mut raw_alg, BCRYPT_AES_ALGORITHM, ptr::null(), 0) }
        != STATUS_SUCCESS
    {
        return Err(());
    }
    let alg = AlgHandle(raw_alg);
    let mode: Vec<u16> = "ChainingModeGCM\0".encode_utf16().collect();
    if unsafe {
        BCryptSetProperty(
            alg.0,
            BCRYPT_CHAINING_MODE,
            mode.as_ptr().cast(),
            (mode.len() * 2) as u32,
            0,
        )
    } != STATUS_SUCCESS
    {
        return Err(());
    }
    let mut raw_key: BCRYPT_KEY_HANDLE = ptr::null_mut();
    if unsafe {
        BCryptGenerateSymmetricKey(
            alg.0,
            &mut raw_key,
            ptr::null_mut(),
            0,
            key.as_ptr(),
            key.len() as u32,
            0,
        )
    } != STATUS_SUCCESS
    {
        return Err(());
    }
    Ok((alg, KeyHandle(raw_key)))
}

pub(super) fn aes_256_gcm_decrypt(
    key: &[u8],
    nonce: &[u8],
    ciphertext: &[u8],
    tag: &[u8],
) -> Result<Vec<u8>, ()> {
    if nonce.len() != 12 || tag.len() != 16 {
        return Err(());
    }
    let (_alg, key_handle) = open_gcm_key(key)?;
    let auth = BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO {
        cbSize: size_of::<BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO>() as u32,
        dwInfoVersion: BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO_VERSION,
        pbNonce: nonce.as_ptr().cast_mut(),
        cbNonce: nonce.len() as u32,
        pbTag: tag.as_ptr().cast_mut(),
        cbTag: tag.len() as u32,
        ..Default::default()
    };
    let mut out = vec![0u8; ciphertext.len().max(1)];
    let mut written = 0u32;
    let status = unsafe {
        BCryptDecrypt(
            key_handle.0,
            ciphertext.as_ptr(),
            ciphertext.len() as u32,
            (&raw const auth).cast(),
            ptr::null_mut(),
            0,
            out.as_mut_ptr(),
            out.len() as u32,
            &mut written,
            0,
        )
    };
    if status != STATUS_SUCCESS {
        return Err(());
    }
    out.truncate(written as usize);
    Ok(out)
}

pub(super) fn read_os_crypt_key(local_state: &Path) -> Answer<Vec<u8>> {
    if !local_state.is_file() {
        return Answer::Missing;
    }
    let Ok(bytes) = std::fs::read(local_state) else {
        return Answer::Failed;
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Answer::Failed;
    };
    let Some(encoded) = value
        .get("os_crypt")
        .and_then(|section| section.get("encrypted_key"))
        .and_then(|key| key.as_str())
    else {
        return Answer::Missing;
    };
    let Ok(raw) = STANDARD.decode(encoded.trim()) else {
        return Answer::Failed;
    };
    let Some(dpapi_blob) = raw.strip_prefix(b"DPAPI") else {
        return Answer::Failed;
    };
    match unprotect_dpapi(dpapi_blob) {
        Answer::Found(key) if key.len() == 32 => Answer::Found(key),
        Answer::Found(_) => Answer::Failed,
        other => other,
    }
}

pub(super) fn decrypt_blob(
    blob: &[u8],
    key: Option<&Answer<Vec<u8>>>,
    strip_host_hash: bool,
    host_key: Option<&str>,
) -> Answer<String> {
    if blob.is_empty() {
        return Answer::Missing;
    }
    if blob.starts_with(b"v20") {
        return Answer::Unsupported;
    }
    let plain = if blob.starts_with(b"v10") || blob.starts_with(b"v11") {
        let key = match key {
            Some(Answer::Found(key)) => key,
            Some(Answer::Missing) | None => return Answer::Missing,
            Some(Answer::Denied) => return Answer::Denied,
            Some(Answer::Unsupported) => return Answer::Unsupported,
            Some(Answer::Failed) => return Answer::Failed,
        };
        let rest = &blob[3..];
        if rest.len() < 12 + 16 {
            return Answer::Failed;
        }
        let split = rest.len() - 16;
        let Ok(decrypted) = aes_256_gcm_decrypt(key, &rest[..12], &rest[12..split], &rest[split..])
        else {
            return Answer::Failed;
        };
        decrypted
    } else {
        match unprotect_dpapi(blob) {
            Answer::Found(plain) => plain,
            Answer::Missing => return Answer::Missing,
            Answer::Denied => return Answer::Denied,
            Answer::Unsupported => return Answer::Unsupported,
            Answer::Failed => return Answer::Failed,
        }
    };
    let payload = if strip_host_hash {
        if plain.len() < 32 {
            return Answer::Failed;
        }
        if host_key.is_some_and(|host| Sha256::digest(host.as_bytes()).as_slice() != &plain[..32]) {
            return Answer::Failed;
        }
        &plain[32..]
    } else {
        plain.as_slice()
    };
    String::from_utf8(payload.to_vec()).map_or(Answer::Failed, Answer::Found)
}

pub(super) fn unseal_in_bases(bases: &[PathBuf], encrypted: &[u8]) -> Answer<String> {
    if encrypted.is_empty() {
        return Answer::Missing;
    }
    if encrypted.starts_with(b"v20") {
        return Answer::Unsupported;
    }
    if encrypted.starts_with(b"v10") || encrypted.starts_with(b"v11") {
        let mut last = Answer::Missing;
        for base in bases {
            let key = read_os_crypt_key(&base.join("Local State"));
            match &key {
                Answer::Found(_) => return decrypt_blob(encrypted, Some(&key), false, None),
                Answer::Missing => {}
                Answer::Denied => last = Answer::Denied,
                Answer::Unsupported => last = Answer::Unsupported,
                Answer::Failed => last = Answer::Failed,
            }
        }
        return last;
    }
    decrypt_blob(encrypted, None, false, None)
}

pub(crate) fn unseal_safe_storage(app: &str, encrypted: &[u8]) -> Answer<String> {
    let Ok(home) = home_dir() else {
        return Answer::Failed;
    };
    let env = super::current_env();
    let mut bases = vec![app_support(&home, &env).join(app)];
    if app == "Claude" {
        bases.extend(claude_desktop_bases(&home, &env));
    }
    unseal_in_bases(&bases, encrypted)
}
