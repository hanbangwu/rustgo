const ROOM_ID_LEN: usize = 8;
const MAX_ROOM_ID_LEN: usize = 32;
const ROOM_ID_CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

pub fn random_room_id(mut random_byte: impl FnMut() -> u8) -> String {
    let limit = (256 / ROOM_ID_CHARS.len() * ROOM_ID_CHARS.len()) as u8;
    let mut id = String::with_capacity(ROOM_ID_LEN);
    while id.len() < ROOM_ID_LEN {
        let byte = random_byte();
        if byte < limit {
            id.push(ROOM_ID_CHARS[byte as usize % ROOM_ID_CHARS.len()] as char);
        }
    }
    id
}

pub fn valid_room_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ROOM_ID_LEN
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_ids_are_valid() {
        let mut n = 0u8;
        let id = random_room_id(|| {
            n = n.wrapping_add(37);
            n
        });
        assert_eq!(id.len(), ROOM_ID_LEN);
        assert!(valid_room_id(&id), "{id}");
    }

    #[test]
    fn biased_bytes_are_skipped() {
        let mut bytes = [255, 254, 0].into_iter().cycle();
        assert_eq!(random_room_id(|| bytes.next().unwrap()), "aaaaaaaa");
    }

    #[test]
    fn typed_ids_are_checked() {
        assert!(valid_room_id("my-game_1"));
        assert!(!valid_room_id(""));
        assert!(!valid_room_id("../etc"));
        assert!(!valid_room_id(&"a".repeat(MAX_ROOM_ID_LEN + 1)));
    }
}
