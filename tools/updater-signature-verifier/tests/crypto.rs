//! Synthetic binary-level crypto regressions, not release provenance or signing-policy proof.

use std::error::Error;
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::process::{Command, Output};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use minisign::{KeyPair, sign};
use tempfile::TempDir;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const BLOCK_BYTES: usize = 64 * 1024;
const PAYLOAD_BYTES: usize = 2 * BLOCK_BYTES + 257;

struct SignedFixture {
    directory: TempDir,
    payload: PathBuf,
    signature: PathBuf,
    public_key: PathBuf,
}

impl SignedFixture {
    fn new() -> TestResult<Self> {
        let directory = tempfile::tempdir()?;
        let payload = directory.path().join("synthetic-payload.bin");
        let signature = directory.path().join("synthetic-payload.sig");
        let public_key = directory.path().join("synthetic-public-key.txt");
        let bytes = vec![0x5a_u8; PAYLOAD_BYTES];
        fs::write(&payload, &bytes)?;

        let KeyPair { pk, sk } = KeyPair::generate_unencrypted_keypair()?;
        fs::write(&public_key, STANDARD.encode(pk.to_box()?.into_string()))?;
        let signed = sign(
            None,
            &sk,
            Cursor::new(&bytes),
            Some("timestamp:1789920000\tversion:0.1.0"),
            Some("signature from a disposable Cutokyo crypto test key"),
        )?;
        fs::write(&signature, STANDARD.encode(signed.into_string()))?;
        Ok(Self {
            directory,
            payload,
            signature,
            public_key,
        })
    }

    fn verify(&self) -> TestResult<Output> {
        Ok(
            Command::new(env!("CARGO_BIN_EXE_cutokyo-updater-signature-verifier"))
                .env_clear()
                .current_dir(self.directory.path())
                .arg("--payload")
                .arg(&self.payload)
                .arg("--signature")
                .arg(&self.signature)
                .arg("--public-key")
                .arg(&self.public_key)
                .output()?,
        )
    }
}

fn assert_rejected(output: &Output) -> TestResult<&str> {
    let stderr = std::str::from_utf8(&output.stderr)?;
    assert_eq!(output.status.code(), Some(65), "{stderr}");
    assert!(
        output.stdout.is_empty(),
        "failure must not report verification"
    );
    assert!(stderr.starts_with("cutokyo-updater-signature-verifier: "));
    Ok(stderr)
}

#[test]
fn valid_signature_verifies_payload_larger_than_two_read_buffers() -> TestResult {
    let fixture = SignedFixture::new()?;
    assert!(fs::metadata(&fixture.payload)?.len() > 2 * 64 * 1024);
    let output = fixture.verify()?;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"updater signature verified\n");
    assert!(output.stderr.is_empty());
    Ok(())
}

#[test]
fn changed_payload_in_third_read_buffer_exits_65_without_success() -> TestResult {
    let fixture = SignedFixture::new()?;
    let mut bytes = fs::read(&fixture.payload)?;
    assert!(bytes.len() > 2 * BLOCK_BYTES);
    *bytes.last_mut().ok_or("synthetic payload is empty")? ^= 1;
    fs::write(&fixture.payload, bytes)?;
    let output = fixture.verify()?;
    assert!(assert_rejected(&output)?.contains("verification failed"));
    Ok(())
}

#[test]
fn wrong_public_key_exits_65_without_success() -> TestResult {
    let fixture = SignedFixture::new()?;
    let KeyPair { pk, .. } = KeyPair::generate_unencrypted_keypair()?;
    fs::write(
        &fixture.public_key,
        STANDARD.encode(pk.to_box()?.into_string()),
    )?;
    let output = fixture.verify()?;
    assert_rejected(&output)?;
    Ok(())
}

#[test]
fn corrupted_but_parseable_signature_exits_65_without_success() -> TestResult {
    let fixture = SignedFixture::new()?;
    let outer = fs::read_to_string(&fixture.signature)?;
    let text = String::from_utf8(STANDARD.decode(outer)?)?;
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let packet_line = lines.get_mut(1).ok_or("signature packet is absent")?;
    let mut packet = STANDARD.decode(&*packet_line)?;
    // Preserve the algorithm and key ID; change the cryptographic signature itself.
    assert_eq!(packet.len(), 2 + 8 + 64);
    *packet.last_mut().ok_or("signature packet is empty")? ^= 1;
    *packet_line = STANDARD.encode(packet);
    let corrupted = format!("{}\n", lines.join("\n"));
    assert!(minisign_verify::Signature::decode(&corrupted).is_ok());
    fs::write(&fixture.signature, STANDARD.encode(corrupted))?;
    let output = fixture.verify()?;
    let stderr = assert_rejected(&output)?;
    assert!(!stderr.contains("not valid Minisign text"));
    Ok(())
}

#[test]
fn malformed_signature_base64_exits_65_without_success() -> TestResult {
    let fixture = SignedFixture::new()?;
    fs::write(&fixture.signature, "%%%not-base64%%%")?;
    let output = fixture.verify()?;
    assert!(assert_rejected(&output)?.contains("signature has invalid Base64"));
    Ok(())
}

#[test]
fn malformed_public_key_base64_exits_65_without_success() -> TestResult {
    let fixture = SignedFixture::new()?;
    fs::write(&fixture.public_key, "%%%not-base64%%%")?;
    let output = fixture.verify()?;
    assert!(assert_rejected(&output)?.contains("public key has invalid Base64"));
    Ok(())
}
