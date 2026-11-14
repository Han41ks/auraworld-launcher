//! Оффлайн-аккаунты.
use md5::{Digest, Md5};

/// Тот же UUID, что считает оффлайн-сервер: UUID.nameUUIDFromBytes("OfflinePlayer:" + ник).
pub fn offline_uuid(name: &str) -> String {
    let mut b: [u8; 16] = Md5::digest(format!("OfflinePlayer:{name}").as_bytes()).into();
    b[6] = (b[6] & 0x0f) | 0x30; // версия 3
    b[8] = (b[8] & 0x3f) | 0x80; // вариант IETF
    let h = hex::encode(b);
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

pub fn valid_nick(name: &str) -> bool {
    (3..=16).contains(&name.len()) && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_matches_java() {
        // UUID.nameUUIDFromBytes("OfflinePlayer:Notch".getBytes(UTF_8))
        assert_eq!(offline_uuid("Notch"), "b50ad385-829d-3141-a216-7e7d7539ba7f");
    }

    #[test]
    fn nick() {
        assert!(valid_nick("han41k"));
        assert!(!valid_nick("ab"));
        assert!(!valid_nick("ник_русский"));
        assert!(!valid_nick("this_is_way_too_long"));
    }
}
