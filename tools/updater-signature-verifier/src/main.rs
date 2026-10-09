//! Strict verification of the Base64-wrapped Minisign format consumed by Tauri.

use std::env;
use std::error::Error;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use minisign_verify::{PublicKey, Signature};

const MAX_PUBLIC_KEY_BYTES: u64 = 16 * 1024;
const MAX_SIGNATURE_BYTES: u64 = 64 * 1024;

#[derive(Debug)]
struct Inputs {
    payload: PathBuf,
    signature: PathBuf,
    public_key: PathBuf,
}

fn usage() -> &'static str {
    "usage: cutokyo-updater-signature-verifier --payload PATH --signature PATH --public-key PATH"
}

fn parse_inputs() -> Result<Inputs, String> {
    let mut arguments = env::args_os().skip(1);
    let mut payload = None;
    let mut signature = None;
    let mut public_key = None;
    while let Some(option) = arguments.next() {
        let value = arguments.next().ok_or_else(|| {
            format!(
                "missing value for {}; {}",
                option.to_string_lossy(),
                usage()
            )
        })?;
        match option.to_str() {
            Some("--payload") if payload.is_none() => payload = Some(PathBuf::from(value)),
            Some("--signature") if signature.is_none() => signature = Some(PathBuf::from(value)),
            Some("--public-key") if public_key.is_none() => public_key = Some(PathBuf::from(value)),
            _ => {
                return Err(format!(
                    "unexpected or repeated option {}; {}",
                    option.to_string_lossy(),
                    usage()
                ));
            }
        }
    }
    Ok(Inputs {
        payload: payload.ok_or_else(|| format!("--payload is required; {}", usage()))?,
        signature: signature.ok_or_else(|| format!("--signature is required; {}", usage()))?,
        public_key: public_key.ok_or_else(|| format!("--public-key is required; {}", usage()))?,
    })
}

fn bounded_text(path: &Path, maximum: u64, label: &str) -> Result<String, Box<dyn Error>> {
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() || metadata.len() > maximum {
        return Err(format!("{label} must be a regular file of at most {maximum} bytes").into());
    }
    Ok(fs::read_to_string(path)?)
}

fn strict_outer_base64(value: &str, label: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    let encoded = value.trim();
    if encoded.is_empty() || encoded.bytes().any(|byte| byte.is_ascii_whitespace()) {
        return Err(format!("{label} is not one canonical Base64 value").into());
    }
    let decoded = STANDARD
        .decode(encoded)
        .map_err(|error| format!("{label} has invalid Base64: {error}"))?;
    if STANDARD.encode(&decoded) != encoded {
        return Err(format!("{label} Base64 is not canonical").into());
    }
    Ok(decoded)
}

fn verify(inputs: &Inputs) -> Result<(), Box<dyn Error>> {
    let public_key_outer = bounded_text(&inputs.public_key, MAX_PUBLIC_KEY_BYTES, "public key")?;
    let public_key_text = String::from_utf8(strict_outer_base64(&public_key_outer, "public key")?)?;
    let public_key = PublicKey::decode(&public_key_text)
        .map_err(|error| format!("public key is not valid Minisign text: {error}"))?;

    let signature_outer = bounded_text(&inputs.signature, MAX_SIGNATURE_BYTES, "signature")?;
    let signature_text = String::from_utf8(strict_outer_base64(&signature_outer, "signature")?)?;
    let signature = Signature::decode(&signature_text)
        .map_err(|error| format!("signature is not valid Minisign text: {error}"))?;

    let mut verifier = public_key
        .verify_stream(&signature)
        .map_err(|error| format!("updater signature algorithm is not accepted: {error}"))?;
    let mut payload = fs::File::open(&inputs.payload)?;
    let mut block = vec![0_u8; 64 * 1024];
    loop {
        let count = payload.read(&mut block)?;
        if count == 0 {
            break;
        }
        verifier.update(&block[..count]);
    }
    verifier
        .finalize()
        .map_err(|error| format!("updater signature verification failed: {error}"))?;
    Ok(())
}

fn main() {
    let result =
        parse_inputs().and_then(|inputs| verify(&inputs).map_err(|error| error.to_string()));
    match result {
        Ok(()) => println!("updater signature verified"),
        Err(error) => {
            eprintln!("cutokyo-updater-signature-verifier: {error}");
            std::process::exit(65);
        }
    }
}
