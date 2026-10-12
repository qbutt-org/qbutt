use ring::digest::{SHA1_FOR_LEGACY_USE_ONLY, digest};
use url::Url;

use crate::{Error, Result};

/// Parse the magnet's single exact v1 identity before a torrent handoff.
pub(crate) fn magnet(input: &str) -> Result<(Url, String)> {
    let url = Url::parse(input).map_err(|_| Error::InvalidResponse("invalid torrent magnet"))?;
    let mut identities = url.query_pairs().filter(|(name, _)| name == "xt");
    let hash = identities
        .next()
        .and_then(|(_, value)| {
            let (prefix, hash) = value.split_at_checked("urn:btih:".len())?;
            prefix
                .eq_ignore_ascii_case("urn:btih:")
                .then(|| hash.to_ascii_lowercase())
        })
        .ok_or(Error::InvalidResponse("missing torrent hash"))?;
    if input.len() > 32 * 1024
        || url.scheme() != "magnet"
        || url.host().is_some()
        || !url.path().is_empty()
        || url.fragment().is_some()
        || identities.next().is_some()
        || hash.len() != 40
        || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(Error::InvalidResponse("invalid torrent magnet identity"));
    }
    Ok((url, hash))
}

/// Hash the exact bencoded `info` value. The native libtorrent handoff still
/// validates the complete torrent before it can be added to the session.
pub(crate) fn info_hash(bytes: &[u8]) -> Result<String> {
    let mut cursor = BencodeCursor { bytes, offset: 0 };
    if cursor.next()? != b'd' {
        return Err(invalid_metadata());
    }
    let mut info = None;
    while cursor.peek()? != b'e' {
        let key = cursor.string()?;
        let start = cursor.offset;
        cursor.value(1)?;
        if key == b"info" && info.replace((start, cursor.offset)).is_some() {
            return Err(invalid_metadata());
        }
    }
    cursor.next()?;
    if cursor.offset != bytes.len() {
        return Err(invalid_metadata());
    }
    let (start, end) = info.ok_or_else(invalid_metadata)?;
    let mut info = BencodeCursor {
        bytes: &bytes[start..end],
        offset: 0,
    };
    if info.next()? != b'd' {
        return Err(invalid_metadata());
    }
    let mut has_v1_pieces = false;
    while info.peek()? != b'e' {
        let key = info.string()?;
        has_v1_pieces |= key == b"pieces";
        info.value(1)?;
    }
    if !has_v1_pieces {
        return Err(Error::UnsupportedAcquisition(
            "v2-only torrent metadata has no v1 infohash",
        ));
    }
    let hash = digest(&SHA1_FOR_LEGACY_USE_ONLY, &bytes[start..end]);
    Ok(hash
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

struct BencodeCursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> BencodeCursor<'a> {
    fn peek(&self) -> Result<u8> {
        self.bytes
            .get(self.offset)
            .copied()
            .ok_or_else(invalid_metadata)
    }

    fn next(&mut self) -> Result<u8> {
        let byte = self.peek()?;
        self.offset += 1;
        Ok(byte)
    }

    fn string(&mut self) -> Result<&'a [u8]> {
        let start = self.offset;
        while self.peek()?.is_ascii_digit() {
            self.offset += 1;
        }
        if start == self.offset || self.next()? != b':' {
            return Err(invalid_metadata());
        }
        let size = std::str::from_utf8(&self.bytes[start..self.offset - 1])
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .ok_or_else(invalid_metadata)?;
        let end = self.offset.checked_add(size).ok_or_else(invalid_metadata)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(invalid_metadata)?;
        self.offset = end;
        Ok(value)
    }

    fn value(&mut self, depth: usize) -> Result<()> {
        if depth > 64 {
            return Err(invalid_metadata());
        }
        match self.peek()? {
            b'0'..=b'9' => {
                self.string()?;
            }
            b'i' => {
                self.next()?;
                let start = self.offset;
                while self.peek()? != b'e' {
                    let byte = self.next()?;
                    if !byte.is_ascii_digit() && !(byte == b'-' && self.offset == start + 1) {
                        return Err(invalid_metadata());
                    }
                }
                if self.offset == start {
                    return Err(invalid_metadata());
                }
                self.next()?;
            }
            b'l' | b'd' => {
                let dictionary = self.next()? == b'd';
                while self.peek()? != b'e' {
                    if dictionary {
                        self.string()?;
                    }
                    self.value(depth + 1)?;
                }
                self.next()?;
            }
            _ => return Err(invalid_metadata()),
        }
        Ok(())
    }
}

fn invalid_metadata() -> Error {
    Error::InvalidResponse("invalid torrent metadata")
}
