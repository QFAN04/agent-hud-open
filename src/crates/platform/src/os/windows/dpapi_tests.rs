use super::acl::Local;
use super::dpapi::{open_gcm_key, unseal_in_bases};
use agent_hud_agents::Answer;
use base64::{engine::general_purpose::STANDARD, Engine};
use std::{mem::size_of, ptr};
use tempfile::tempdir;
use windows_sys::Win32::{
    Foundation::STATUS_SUCCESS,
    Security::Cryptography::{
        BCryptEncrypt, CryptProtectData, BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO,
        BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO_VERSION, CRYPTPROTECT_UI_FORBIDDEN,
        CRYPT_INTEGER_BLOB,
    },
};

pub(super) fn protect_dpapi(plain: &[u8]) -> Vec<u8> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: plain.len() as u32,
        pbData: plain.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };
    assert_ne!(
        unsafe {
            CryptProtectData(
                &input,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        },
        0
    );
    let _guard = Local(output.pbData.cast());
    unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec()
}

pub(super) fn aes_256_gcm_encrypt(key: &[u8], nonce: &[u8], plain: &[u8]) -> Vec<u8> {
    let (_alg, key_handle) = open_gcm_key(key).unwrap();
    let mut tag = [0u8; 16];
    let auth = BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO {
        cbSize: size_of::<BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO>() as u32,
        dwInfoVersion: BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO_VERSION,
        pbNonce: nonce.as_ptr().cast_mut(),
        cbNonce: nonce.len() as u32,
        pbTag: tag.as_mut_ptr(),
        cbTag: tag.len() as u32,
        ..Default::default()
    };
    let mut out = vec![0u8; plain.len().max(1)];
    let mut written = 0u32;
    assert_eq!(
        unsafe {
            BCryptEncrypt(
                key_handle.0,
                plain.as_ptr(),
                plain.len() as u32,
                (&raw const auth).cast(),
                ptr::null_mut(),
                0,
                out.as_mut_ptr(),
                out.len() as u32,
                &mut written,
                0,
            )
        },
        STATUS_SUCCESS
    );
    out.truncate(written as usize);
    let mut blob = Vec::with_capacity(3 + nonce.len() + out.len() + tag.len());
    blob.extend_from_slice(b"v10");
    blob.extend_from_slice(nonce);
    blob.extend_from_slice(&out);
    blob.extend_from_slice(&tag);
    blob
}

#[test]
fn dpapi_and_bcrypt_aes_gcm_round_trip_v10_safe_storage() {
    let key = [0x42u8; 32];
    let nonce = *b"0123456789ab";
    let dpapi = protect_dpapi(&key);
    let mut raw_key = b"DPAPI".to_vec();
    raw_key.extend_from_slice(&dpapi);

    let dir = tempdir().unwrap();
    let state = serde_json::json!({
        "os_crypt": { "encrypted_key": STANDARD.encode(&raw_key) }
    });
    std::fs::write(dir.path().join("Local State"), state.to_string()).unwrap();

    let sealed = aes_256_gcm_encrypt(&key, &nonce, b"secret-token-value");
    assert_eq!(
        unseal_in_bases(&[dir.path().to_path_buf()], &sealed),
        Answer::Found("secret-token-value".into())
    );

    let legacy = protect_dpapi(b"legacy-secret");
    assert_eq!(
        unseal_in_bases(&[dir.path().to_path_buf()], &legacy),
        Answer::Found("legacy-secret".into())
    );
    assert_eq!(
        unseal_in_bases(&[dir.path().to_path_buf()], b"v20payload"),
        Answer::Unsupported
    );
}
