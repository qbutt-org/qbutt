//! Decode only the public .torrent attachments linked by the publisher.
//! Format: https://github.com/PrivateBin/PrivateBin/wiki/Encryption-format

use std::io::Read;
use std::num::NonZeroU32;

use aes_gcm::aead::{Aead, KeyInit, Payload, consts::U16};
use aes_gcm::aes::Aes256;
use aes_gcm::{AesGcm, Nonce};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use flate2::read::DeflateDecoder;
use ring::pbkdf2::{PBKDF2_HMAC_SHA256, derive};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::http::{SourceHttp, SourceSession, validate_https_url};
use crate::{Error, Result, TorrentMetadata};

const MAX_EXPANDED_BYTES: usize = TorrentMetadata::MAX_BYTES * 4 / 3 + 64 * 1024;
const MAX_ITERATIONS: u32 = 2_000_000;
pub(super) const HOSTS: [&str; 2] = ["paste.fitgirl-repacks.site", "pastefg.hermietkreeft.site"];

pub(super) fn torrent_paste(input: &str) -> Result<Url> {
    let url = Url::parse(input).map_err(|_| Error::UnsafeAddress)?;
    if !url.host_str().is_some_and(|host| HOSTS.contains(&host))
        || url.port().is_some()
        || url.path() != "/"
        || !url.query().is_some_and(|query| {
            query.len() == 16 && query.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
    {
        return Err(Error::UnsafeAddress);
    }
    public_key(url.fragment().unwrap_or_default())?;
    let mut request = url.clone();
    request.set_fragment(None);
    validate_https_url(&request)?;
    Ok(url)
}

pub(super) async fn download(http: &SourceHttp, address: &Url) -> Result<TorrentMetadata> {
    let key = public_key(address.fragment().unwrap_or_default())?;
    let mut url = address.clone();
    url.set_fragment(None);
    let mut session = SourceSession::anonymous(url.clone())?;
    let response = http.session_document(url.clone(), &mut session).await?;
    if response.final_url != url {
        return Err(Error::InvalidResponse(
            "FitGirl torrent paste identity changed",
        ));
    }
    if response.status != 200 {
        return Err(Error::HttpStatus(response.status));
    }
    if !response.content_type.as_deref().is_some_and(|value| {
        value
            .split(';')
            .next()
            .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"))
    }) {
        let body = String::from_utf8_lossy(&response.body);
        if body.contains("Checking your browser") || body.contains("cf-chl-") {
            return Err(Error::Challenge);
        }
        return Err(Error::InvalidResponse("FitGirl torrent paste is not JSON"));
    }
    let value: serde_json::Value = serde_json::from_slice(&response.body).map_err(Error::Json)?;
    match value.get("status").and_then(serde_json::Value::as_u64) {
        Some(0) => {}
        Some(1) => return Err(Error::VariantUnavailable),
        _ => {
            return Err(Error::InvalidResponse(
                "invalid FitGirl torrent paste status",
            ));
        }
    }
    let paste: Paste = serde_json::from_value(value).map_err(Error::Json)?;
    tokio::task::spawn_blocking(move || decode(paste, key))
        .await
        .map_err(|_| Error::InvalidResponse("FitGirl torrent decoding failed"))?
}

#[derive(Deserialize)]
struct Paste {
    v: u8,
    adata: AuthenticatedData,
    ct: String,
}

#[derive(Deserialize, Serialize)]
struct AuthenticatedData(
    (String, String, u32, u16, u16, String, String, String),
    String,
    u8,
    u8,
);

#[derive(Deserialize)]
struct Attachment {
    attachment: String,
}

fn decode(paste: Paste, public_key: [u8; 32]) -> Result<TorrentMetadata> {
    let AuthenticatedData(parameters, format, discussion, burn_after_reading) = &paste.adata;
    let (iv, salt, iterations, bits, tag, cipher, mode, compression) = parameters;
    if paste.v != 2
        || *bits != 256
        || *tag != 128
        || cipher != "aes"
        || mode != "gcm"
        || compression != "zlib"
        || *iterations == 0
        || *iterations > MAX_ITERATIONS
        || *burn_after_reading != 0
        || *discussion > 1
        || !matches!(
            format.as_str(),
            "plaintext" | "markdown" | "syntaxhighlighting"
        )
    {
        return Err(Error::InvalidResponse(
            "unsupported FitGirl torrent paste format",
        ));
    }
    if iv.len() != 24 || salt.len() != 12 {
        return Err(Error::InvalidResponse("invalid FitGirl paste parameters"));
    }
    let iv = STANDARD
        .decode(iv)
        .map_err(|_| Error::InvalidResponse("invalid FitGirl paste IV"))?;
    let salt = STANDARD
        .decode(salt)
        .map_err(|_| Error::InvalidResponse("invalid FitGirl paste salt"))?;
    if iv.len() != 16 || salt.len() != 8 {
        return Err(Error::InvalidResponse("invalid FitGirl paste parameters"));
    }
    let ciphertext = STANDARD
        .decode(&paste.ct)
        .map_err(|_| Error::InvalidResponse("invalid FitGirl paste ciphertext"))?;
    let mut key = [0; 32];
    derive(
        PBKDF2_HMAC_SHA256,
        NonZeroU32::new(*iterations).expect("bounded nonzero work factor"),
        &salt,
        &public_key,
        &mut key,
    );
    let cipher = AesGcm::<Aes256, U16>::new_from_slice(&key).expect("AES-256 key length");
    let aad = serde_json::to_vec(&paste.adata).map_err(Error::Json)?;
    let compressed = cipher
        .decrypt(
            Nonce::<U16>::from_slice(&iv),
            Payload {
                msg: &ciphertext,
                aad: &aad,
            },
        )
        .map_err(|_| Error::InvalidResponse("FitGirl torrent paste authentication failed"))?;
    let mut inflater = DeflateDecoder::new(compressed.as_slice());
    let mut expanded = Vec::new();
    inflater
        .by_ref()
        .take(MAX_EXPANDED_BYTES as u64 + 1)
        .read_to_end(&mut expanded)
        .map_err(|_| Error::InvalidResponse("invalid FitGirl paste compression"))?;
    if expanded.len() > MAX_EXPANDED_BYTES {
        return Err(Error::ResponseTooLarge);
    }
    if inflater.total_in() != compressed.len() as u64 {
        return Err(Error::InvalidResponse("trailing FitGirl paste data"));
    }
    let attachment: Attachment = serde_json::from_slice(&expanded).map_err(Error::Json)?;
    let encoded = attachment
        .attachment
        .strip_prefix("data:application/x-bittorrent;base64,")
        .ok_or(Error::InvalidResponse(
            "FitGirl attachment is not a torrent",
        ))?;
    let metadata = STANDARD
        .decode(encoded)
        .map_err(|_| Error::InvalidResponse("invalid FitGirl torrent attachment"))?;
    TorrentMetadata::new(metadata)
}

fn public_key(value: &str) -> Result<[u8; 32]> {
    const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    if value.is_empty() || value.len() > 44 {
        return Err(Error::InvalidResponse("invalid FitGirl public paste key"));
    }
    let mut decoded = [0u8; 32];
    for character in value.bytes() {
        let mut carry = ALPHABET
            .iter()
            .position(|byte| *byte == character)
            .ok_or(Error::InvalidResponse("invalid FitGirl public paste key"))?
            as u16;
        for byte in decoded.iter_mut().rev() {
            carry += u16::from(*byte) * 58;
            *byte = carry as u8;
            carry >>= 8;
        }
        if carry != 0 {
            return Err(Error::InvalidResponse("invalid FitGirl public paste key"));
        }
    }
    let leading_zeroes = decoded.iter().take_while(|byte| **byte == 0).count();
    if value.bytes().take_while(|byte| *byte == b'1').count() != leading_zeroes {
        return Err(Error::InvalidResponse("invalid FitGirl public paste key"));
    }
    Ok(decoded)
}
