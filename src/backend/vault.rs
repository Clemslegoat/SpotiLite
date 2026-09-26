//! Secrets on disk (client secret, refresh token).
//!
//! On Windows they are encrypted with DPAPI: only the current Windows user, on this
//! machine, can decrypt them. Elsewhere they are stored as plain files in the user's
//! configuration directory.

use std::io;
use std::path::Path;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::config::write_atomic;

/// Marks a DPAPI-encrypted file (anything else is read as plain JSON).
const MAGIC: &[u8] = b"SLDPAPI1";

pub fn save<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let plain = serde_json::to_vec(value).map_err(io::Error::other)?;
    write_atomic(path, &protect(&plain)?)
}

pub fn load<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let data = std::fs::read(path).ok()?;
    let plain = match data.strip_prefix(MAGIC) {
        Some(blob) => unprotect(blob)?,
        None => data,
    };
    serde_json::from_slice(&plain).ok()
}

#[cfg(windows)]
fn protect(plain: &[u8]) -> io::Result<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData,
    };
    let input = CRYPT_INTEGER_BLOB { cbData: plain.len() as u32, pbData: plain.as_ptr() as *mut u8 };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: `input` points to `plain`, which outlives the call; DPAPI allocates
    // `output` with LocalAlloc and it is released with LocalFree below.
    let ok = unsafe {
        CryptProtectData(
            &input,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut out = MAGIC.to_vec();
    // SAFETY: DPAPI returned a valid buffer of `cbData` bytes.
    unsafe {
        out.extend_from_slice(std::slice::from_raw_parts(output.pbData, output.cbData as usize));
        LocalFree(output.pbData.cast());
    }
    Ok(out)
}

#[cfg(windows)]
fn unprotect(blob: &[u8]) -> Option<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptUnprotectData,
    };
    let input = CRYPT_INTEGER_BLOB { cbData: blob.len() as u32, pbData: blob.as_ptr() as *mut u8 };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: same contract as in `protect`.
    let ok = unsafe {
        CryptUnprotectData(
            &input,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 {
        log::warn!("could not decrypt a saved secret: {}", io::Error::last_os_error());
        return None;
    }
    // SAFETY: DPAPI returned a valid buffer of `cbData` bytes.
    unsafe {
        let plain = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData.cast());
        Some(plain)
    }
}

#[cfg(not(windows))]
fn protect(plain: &[u8]) -> io::Result<Vec<u8>> {
    Ok(plain.to_vec())
}

#[cfg(not(windows))]
fn unprotect(_blob: &[u8]) -> Option<Vec<u8>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_encryption_on_windows() {
        let dir = std::env::temp_dir().join(format!("spotilite-vault-{}", std::process::id()));
        let path = dir.join("secret.dat");
        let value = ("client".to_string(), "very-secret".to_string());
        save(&path, &value).unwrap();
        assert_eq!(load::<(String, String)>(&path), Some(value));
        let raw = std::fs::read(&path).unwrap();
        let contains_secret = raw.windows(11).any(|w| w == b"very-secret");
        assert_eq!(contains_secret, !cfg!(windows));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reads_legacy_plain_json() {
        let dir = std::env::temp_dir().join(format!("spotilite-vault-legacy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("token.json");
        std::fs::write(&path, br#"["a","b"]"#).unwrap();
        assert_eq!(load::<Vec<String>>(&path), Some(vec!["a".into(), "b".into()]));
        let _ = std::fs::remove_dir_all(dir);
    }
}
