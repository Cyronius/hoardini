use std::io;

use windows::Win32::Security::Cryptography::{
    CERT_FIND_SUBJECT_CERT, CERT_INFO, CERT_NAME_SIMPLE_DISPLAY_TYPE,
    CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED, CERT_QUERY_FORMAT_FLAG_BINARY, CERT_QUERY_OBJECT_BLOB,
    CMSG_SIGNER_INFO, CMSG_SIGNER_INFO_PARAM, CRYPT_INTEGER_BLOB, CertCloseStore,
    CertFindCertificateInStore, CertFreeCertificateContext, CertGetNameStringW, CryptMsgClose,
    CryptMsgGetParam, CryptQueryObject, HCERTSTORE, PKCS_7_ASN_ENCODING, X509_ASN_ENCODING,
};

use crate::util::{from_wide, werr};

struct Handles {
    store: HCERTSTORE,
    msg: *mut core::ffi::c_void,
}

impl Drop for Handles {
    fn drop(&mut self) {
        unsafe {
            if !self.store.is_invalid() {
                let _ = CertCloseStore(Some(self.store), 0);
            }
            if !self.msg.is_null() {
                let _ = CryptMsgClose(Some(self.msg));
            }
        }
    }
}

// The caller reads the blob, so the file is opened the atime-preserving way.
pub fn pkcs7_signer(pkcs7: &[u8]) -> io::Result<Option<String>> {
    let blob = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(pkcs7.len()).map_err(|_| io::Error::other("signature too large"))?,
        pbData: pkcs7.as_ptr() as *mut u8,
    };
    let mut h = Handles {
        store: HCERTSTORE::default(),
        msg: std::ptr::null_mut(),
    };
    unsafe {
        CryptQueryObject(
            CERT_QUERY_OBJECT_BLOB,
            (&blob as *const CRYPT_INTEGER_BLOB).cast(),
            CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED,
            CERT_QUERY_FORMAT_FLAG_BINARY,
            0,
            None,
            None,
            None,
            Some(&mut h.store),
            Some(&mut h.msg),
            None,
        )
    }
    .map_err(werr)?;
    let mut size = 0u32;
    unsafe { CryptMsgGetParam(h.msg, CMSG_SIGNER_INFO_PARAM, 0, None, &mut size) }.map_err(werr)?;
    let mut buf = vec![0u64; (size as usize).div_ceil(8)];
    unsafe {
        CryptMsgGetParam(
            h.msg,
            CMSG_SIGNER_INFO_PARAM,
            0,
            Some(buf.as_mut_ptr().cast()),
            &mut size,
        )
    }
    .map_err(werr)?;
    let signer = unsafe { &*(buf.as_ptr() as *const CMSG_SIGNER_INFO) };
    let find = CERT_INFO {
        Issuer: signer.Issuer,
        SerialNumber: signer.SerialNumber,
        ..Default::default()
    };
    let cert = unsafe {
        CertFindCertificateInStore(
            h.store,
            X509_ASN_ENCODING | PKCS_7_ASN_ENCODING,
            0,
            CERT_FIND_SUBJECT_CERT,
            Some((&find as *const CERT_INFO).cast()),
            None,
        )
    };
    if cert.is_null() {
        return Ok(None);
    }
    let mut name = vec![0u16; 512];
    let n = unsafe {
        CertGetNameStringW(
            cert,
            CERT_NAME_SIMPLE_DISPLAY_TYPE,
            0,
            None,
            Some(&mut name),
        )
    };
    unsafe {
        let _ = CertFreeCertificateContext(Some(cert));
    }
    let s = from_wide(&name[..(n as usize).min(name.len())]);
    Ok(Some(s.trim().to_string()).filter(|s| !s.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn garbage_is_not_a_signature() {
        assert!(pkcs7_signer(b"not a pkcs7 blob").is_err());
    }
}
