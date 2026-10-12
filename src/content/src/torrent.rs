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
