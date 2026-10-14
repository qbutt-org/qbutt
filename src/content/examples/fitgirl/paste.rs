use std::io::Write;
use std::num::NonZeroU32;

use aes_gcm::aead::{Aead, KeyInit, Payload, consts::U16};
use aes_gcm::aes::Aes256;
use aes_gcm::{AesGcm, Nonce};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use flate2::{Compression, write::DeflateEncoder};
use ring::{digest, pbkdf2};
use serde_json::json;

pub(super) const KEY: &str = "11111111111111111111111111111111";

pub(super) fn metadata(changed: bool) -> Vec<u8> {
    let payload = b"qbutt generated legal payload\n";
    let name = if changed {
        "replaced-legal.txt"
    } else {
        "generated-legal.txt"
    };
    let mut bytes = format!(
        "d4:infod6:lengthi{}e4:name{}:{}12:piece lengthi16384e6:pieces20:",
        payload.len(),
        name.len(),
        name,
    )
    .into_bytes();
    bytes.extend_from_slice(digest::digest(&digest::SHA1_FOR_LEGACY_USE_ONLY, payload).as_ref());
    bytes.extend_from_slice(b"ee");
    bytes
}

pub(super) fn hash() -> String {
    crate::torrent::info_hash(&metadata(false)).unwrap()
}

pub(super) fn response(id: &str) -> (&'static str, Option<&'static str>, Vec<u8>) {
    if id.ends_with("08") {
        return (
            "200 OK",
            None,
            br#"{"status":1,"message":"Paste not found"}"#.to_vec(),
        );
    }
    if id.ends_with("09") {
        return ("503 Service Unavailable", None, b"{}".to_vec());
    }
    if id.ends_with("0a") {
        return ("302 Found", Some("/?0000000000000001"), Vec::new());
    }
    if id.ends_with("0c") {
        return (
            "200 OK",
            None,
            b"<html><h1>Checking your browser</h1></html>".to_vec(),
        );
    }
    let iv = [7u8; 16];
    let salt = [9u8; 8];
    let mut adata = json!([
        [
            STANDARD.encode(iv),
            STANDARD.encode(salt),
            100000,
            256,
            128,
            "aes",
            "gcm",
            "zlib"
        ],
        "markdown",
        0,
        0
    ]);
    let attachment = if id.ends_with("06") {
        json!({ "attachment": "A".repeat(15 * 1024 * 1024), "attachment_name": "generated.torrent" })
    } else {
        let mime = if id.ends_with("07") {
            "application/octet-stream"
        } else {
            "application/x-bittorrent"
        };
        let attachment = format!(
            "data:{mime};base64,{}",
            STANDARD.encode(metadata(id.ends_with("02")))
        );
        json!({
            "paste": "Generated legal metadata fixture",
            "attachment": attachment,
            "attachment_name": "generated.torrent",
        })
    };
    let mut deflater = DeflateEncoder::new(Vec::new(), Compression::default());
    deflater
        .write_all(&serde_json::to_vec(&attachment).unwrap())
        .unwrap();
    let compressed = deflater.finish().unwrap();
    let mut key = [0; 32];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        NonZeroU32::new(100000).unwrap(),
        &salt,
        &[0; 32],
        &mut key,
    );
    let cipher = AesGcm::<Aes256, U16>::new_from_slice(&key).unwrap();
    let mut ciphertext = cipher
        .encrypt(
            Nonce::<U16>::from_slice(&iv),
            Payload {
                msg: &compressed,
                aad: &serde_json::to_vec(&adata).unwrap(),
            },
        )
        .unwrap();
    if id.ends_with("03") {
        *ciphertext.last_mut().unwrap() ^= 1;
    }
    if id.ends_with("04") {
        adata[0][2] = json!(2000001);
    }
    let ciphertext = if id.ends_with("05") {
        "A".repeat(2 * 1024 * 1024 + 1)
    } else {
        STANDARD.encode(ciphertext)
    };
    (
        "200 OK",
        None,
        serde_json::to_vec(&json!({ "status": 0, "v": 2, "adata": adata, "ct": ciphertext }))
            .unwrap(),
    )
}
