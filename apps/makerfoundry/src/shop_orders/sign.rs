//! Signing `order.placed` for StitchHub (#53): Ed25519 (RFC 8032), as StitchHub's message of
//! 2026-11-01 sets out. The hub admin registers MakerFoundry's public key (base64 SPKI DER); the
//! private key is the Railway secret `MF_STITCHHUB_SIGNING_KEY` (base64 PKCS#8 DER, e.g. from
//! `openssl genpkey -algorithm ed25519 -outform DER | base64`), never in the repository or a log.
//!
//! The signed text is these lines joined by "\n": `SBHUB1`, the sender (`contract::me()`), to,
//! conversation, seq, id, type, version, created_at and expires_at (exactly as sent; expires_at
//! empty when there is none), then the SHA-256 (hex) of the body's canonical JSON: keys sorted at
//! every level (as JavaScript sorts strings, by UTF-16 code units), no whitespace.

use base64::Engine;
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::contract::{Envelope, Sig, me};

/// The DER that comes before an Ed25519 public key in an SPKI (RFC 8410 § 4).
const SPKI_PREFIX: [u8; 12] = [0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00];

/// JSON with keys sorted at every level and no whitespace.
pub fn canonical(v: &Value) -> String {
    let mut out = String::new();
    write(v, &mut out);
    out
}

fn write(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => out.push_str(&serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(k).unwrap_or_else(|_| "\"\"".into()));
                out.push(':');
                if let Some(value) = map.get(k.as_str()) {
                    write(value, out);
                }
            }
            out.push('}');
        }
    }
}

/// What's signed for an envelope MakerFoundry sends.
pub fn signed_text(env: &Envelope) -> String {
    let body = format!("{:x}", Sha256::digest(canonical(&env.body).as_bytes()));
    [
        "SBHUB1",
        me(),
        &env.to,
        &env.conversation,
        &env.seq.to_string(),
        &env.id,
        &env.kind,
        &env.version.to_string(),
        &env.created_at,
        env.expires_at.as_deref().unwrap_or(""),
        &body,
    ]
    .join("\n")
}

/// MakerFoundry's signing key.
pub struct Signer {
    key: Ed25519KeyPair,
}

impl std::fmt::Debug for Signer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // (never the key)
        f.write_str("Signer")
    }
}

impl Signer {
    /// From base64 PKCS#8 DER (v1, as `openssl genpkey` writes it, or v2).
    pub fn from_base64(text: &str) -> Result<Signer, String> {
        let b64 = base64::engine::general_purpose::STANDARD;
        let clean: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        let der = b64
            .decode(clean.as_bytes())
            .map_err(|_| "MF_STITCHHUB_SIGNING_KEY isn't base64".to_string())?;
        let key = Ed25519KeyPair::from_pkcs8_maybe_unchecked(&der)
            .map_err(|_| "MF_STITCHHUB_SIGNING_KEY isn't an Ed25519 PKCS#8 key".to_string())?;
        Ok(Signer { key })
    }

    /// The public key as the hub registers it: base64 SPKI DER (not a secret).
    pub fn public_spki(&self) -> String {
        let mut der = SPKI_PREFIX.to_vec();
        der.extend_from_slice(self.key.public_key().as_ref());
        base64::engine::general_purpose::STANDARD.encode(der)
    }

    pub fn sign_bytes(&self, message: &[u8]) -> Vec<u8> {
        self.key.sign(message).as_ref().to_vec()
    }

    /// The envelope's signature.
    pub fn sign(&self, env: &Envelope) -> Sig {
        Sig {
            v: 1,
            sig: base64::engine::general_purpose::STANDARD.encode(self.sign_bytes(signed_text(env).as_bytes())),
        }
    }
}

/// Whether `sig` is a signature of `env` by the key `public_spki` (base64 SPKI DER), as the hub
/// checks it.
pub fn verify(public_spki: &str, env: &Envelope) -> bool {
    let b64 = base64::engine::general_purpose::STANDARD;
    let (Ok(der), Some(sig)) = (b64.decode(public_spki.as_bytes()), env.sig.as_ref()) else {
        return false;
    };
    let (Some(prefix), Some(key)) = (der.get(..12), der.get(12..)) else {
        return false;
    };
    let Ok(sig) = b64.decode(sig.sig.as_bytes()) else {
        return false;
    };
    prefix == SPKI_PREFIX
        && key.len() == 32
        && UnparsedPublicKey::new(&ED25519, key)
            .verify(signed_text(env).as_bytes(), &sig)
            .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    /// A PKCS#8 v1 key from a 32-byte seed (RFC 8410 § 7).
    fn pkcs8(seed: &[u8]) -> String {
        let mut der = hex("302e020100300506032b657004220420");
        der.extend_from_slice(seed);
        base64::engine::general_purpose::STANDARD.encode(der)
    }

    #[test]
    fn signs_as_rfc_8032_says() {
        // RFC 8032 § 7.1, TEST 1: the empty message
        let seed = hex("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60");
        let s = Signer::from_base64(&pkcs8(&seed)).unwrap();
        assert_eq!(
            s.sign_bytes(b""),
            hex(
                "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"
            )
        );
        let spki = base64::engine::general_purpose::STANDARD
            .decode(s.public_spki())
            .unwrap();
        assert_eq!(
            &spki[12..],
            &hex("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a")[..]
        );
        assert!(Signer::from_base64("not a key").is_err());
        assert!(Signer::from_base64(&base64::engine::general_purpose::STANDARD.encode(b"0123")).is_err());
    }

    #[test]
    fn canonical_json_sorts_keys_at_every_level_without_space() {
        let v = json!({"b": 1, "a": {"d": [1, {"z": true, "y": null}], "c": "x\"y\n"}});
        assert_eq!(
            canonical(&v),
            r#"{"a":{"c":"x\"y\n","d":[1,{"y":null,"z":true}]},"b":1}"#
        );
        // (JavaScript's order: by UTF-16 code units, so "Z" < "a" < "é")
        assert_eq!(canonical(&json!({"é": 1, "a": 2, "Z": 3})), r#"{"Z":3,"a":2,"é":1}"#);
    }

    #[test]
    fn a_signed_envelope_verifies_and_any_change_breaks_it() {
        let s = Signer::from_base64(&pkcs8(&[7u8; 32])).unwrap();
        let mut env = Envelope {
            id: "0192f0c4-7a1b-7c2d-8e3f-0123456789ab".into(),
            kind: "order.placed".into(),
            version: 1,
            conversation: "mf-8f3a1".into(),
            seq: 1,
            from: None,
            to: "shop:mainst".into(),
            created_at: "2026-11-02T15:04:05Z".into(),
            expires_at: None,
            body: json!({"note": "hi", "fulfilment": "pickup"}),
            sig: None,
        };
        assert!(signed_text(&env).starts_with("SBHUB1\nservice:makerfoundry\nshop:mainst\nmf-8f3a1\n1\n"));
        assert!(
            signed_text(&env).contains("\n2026-11-02T15:04:05Z\n\n"),
            "no expires_at: an empty line"
        );
        env.sig = Some(s.sign(&env));
        assert!(verify(&s.public_spki(), &env));
        // the body's key order doesn't matter; its content does
        env.body = json!({"fulfilment": "pickup", "note": "hi"});
        assert!(verify(&s.public_spki(), &env));
        for change in 0..4 {
            let mut e = env.clone();
            match change {
                0 => e.body["note"] = json!("ho"),
                1 => e.seq = 2,
                2 => e.to = "shop:other".into(),
                _ => e.created_at = "2026-11-02T15:04:06Z".into(),
            }
            assert!(!verify(&s.public_spki(), &e), "change {change}");
        }
        let other = Signer::from_base64(&pkcs8(&[8u8; 32])).unwrap();
        assert!(!verify(&other.public_spki(), &env));
    }
}
