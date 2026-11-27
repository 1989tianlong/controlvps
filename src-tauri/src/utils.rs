use std::sync::atomic::{compiler_fence, Ordering};

use keyring::Entry;

use crate::error::{AppError, AppResult};
use crate::ssh::ExecOutput;

pub const KEYCHAIN_SERVICE: &str = "com.controlvps.app";

pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub fn combined_output(output: &ExecOutput) -> String {
    let mut text = output.stdout.clone();
    if !output.stderr.trim().is_empty() {
        text.push_str(&output.stderr);
    }
    text
}

pub fn secret_entry(account: &str) -> AppResult<Entry> {
    Entry::new(KEYCHAIN_SERVICE, account).map_err(|error| AppError::Keyring(error.to_string()))
}

pub fn set_secret(account: &str, value: &str) -> AppResult<()> {
    if value.is_empty() {
        delete_secret(account)
    } else {
        secret_entry(account)?
            .set_password(value)
            .map_err(|error| AppError::Keyring(error.to_string()))
    }
}

pub fn get_secret(account: &str) -> AppResult<Option<String>> {
    match secret_entry(account)?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(AppError::Keyring(error.to_string())),
    }
}

pub fn delete_secret(account: &str) -> AppResult<()> {
    match secret_entry(account)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(AppError::Keyring(error.to_string())),
    }
}

pub fn credential_store_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "macOS Keychain"
    } else if cfg!(target_os = "windows") {
        "Windows Credential Manager"
    } else {
        "系统 Secret Service"
    }
}

pub fn zeroize_bytes(bytes: &mut [u8]) {
    for byte in bytes.iter_mut() {
        unsafe {
            std::ptr::write_volatile(byte, 0);
        }
    }
    compiler_fence(Ordering::SeqCst);
}

#[derive(Default)]
pub struct Utf8StreamDecoder {
    pending: Vec<u8>,
}

impl Utf8StreamDecoder {
    pub fn decode(&mut self, chunk: &[u8]) -> String {
        if chunk.is_empty() {
            return String::new();
        }
        let mut combined = if self.pending.is_empty() {
            chunk.to_vec()
        } else {
            let mut buffer = std::mem::take(&mut self.pending);
            buffer.extend_from_slice(chunk);
            buffer
        };
        let mut output = String::new();
        loop {
            match std::str::from_utf8(&combined) {
                Ok(valid) => {
                    output.push_str(valid);
                    break;
                }
                Err(error) => {
                    let valid_up_to = error.valid_up_to();
                    if valid_up_to > 0 {
                        if let Ok(valid) = std::str::from_utf8(&combined[..valid_up_to]) {
                            output.push_str(valid);
                        }
                    }
                    match error.error_len() {
                        Some(invalid_len) => {
                            output.push('\u{FFFD}');
                            combined.drain(..valid_up_to + invalid_len);
                        }
                        None => {
                            self.pending = combined.split_off(valid_up_to);
                            break;
                        }
                    }
                }
            }
        }
        output
    }

    pub fn flush(&mut self) -> String {
        if self.pending.is_empty() {
            return String::new();
        }
        let tail = std::mem::take(&mut self.pending);
        String::from_utf8_lossy(&tail).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quote_handles_special_chars() {
        assert_eq!(shell_quote("simple"), "'simple'");
        assert_eq!(shell_quote("O'Reilly"), "'O'\\''Reilly'");
        assert_eq!(shell_quote("a; rm -rf /"), "'a; rm -rf /'");
        assert_eq!(shell_quote("$(whoami)"), "'$(whoami)'");
    }

    #[test]
    fn utf8_stream_decoder_preserves_split_multibyte_chars() {
        let mut decoder = Utf8StreamDecoder::default();
        let bytes = "你好ControlVPS世界".as_bytes();
        let first = decoder.decode(&bytes[..4]); // "你" (3 bytes) + 1st byte of "好"
        let second = decoder.decode(&bytes[4..]);
        assert_eq!(format!("{first}{second}"), "你好ControlVPS世界");
        assert!(decoder.flush().is_empty());
    }

    #[test]
    fn utf8_stream_decoder_replaces_invalid_bytes() {
        let mut decoder = Utf8StreamDecoder::default();
        let text = decoder.decode(&[0xff, b'a', 0xe4, 0xbd]);
        let tail = decoder.flush();
        assert_eq!(text, "\u{FFFD}a");
        assert_eq!(tail, "\u{FFFD}");
    }

    #[test]
    fn zeroize_clears_buffer() {
        let mut secret = [1_u8, 2, 3, 4, 5];
        zeroize_bytes(&mut secret);
        assert_eq!(secret, [0; 5]);
    }
}
