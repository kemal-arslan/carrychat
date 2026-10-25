//! Long-term identities, contact cards and fingerprints (§3).

use core::fmt;

use data_encoding::{BASE32_NOPAD, BASE64URL_NOPAD};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rand_core::OsRng;
use x25519_dalek::{PublicKey, StaticSecret};

use crate::{sha256, truncate, Error, Result, VERSION};

pub const MAX_NAME_LEN: usize = 32;
pub const CARD_TEXT_PREFIX: &str = "kervan:";

const CARD_LABEL: &[u8] = b"kervan/v0/card";
const FINGERPRINT_LABEL: &[u8] = b"kervan/v0/fingerprint";
const CARD_FIXED_LEN: usize = 66;
const SIGNATURE_LEN: usize = 64;

/// The secret half of a user: an Ed25519 key that signs their card and an
/// X25519 key for pair keys.
pub struct Identity {
    signing: SigningKey,
    dh: StaticSecret,
    name: String,
}

impl Identity {
    pub fn generate(name: &str) -> Result<Self> {
        check_name(name)?;
        Ok(Self {
            signing: SigningKey::generate(&mut OsRng),
            dh: StaticSecret::random_from_rng(OsRng),
            name: name.to_owned(),
        })
    }

    /// Restores an identity from the secrets returned by
    /// [`Identity::secret_bytes`].
    pub fn from_secret_bytes(signing: &[u8; 32], dh: &[u8; 32], name: &str) -> Result<Self> {
        check_name(name)?;
        Ok(Self {
            signing: SigningKey::from_bytes(signing),
            dh: StaticSecret::from(*dh),
            name: name.to_owned(),
        })
    }

    /// The signing and key-agreement secrets, for storing the identity.
    pub fn secret_bytes(&self) -> ([u8; 32], [u8; 32]) {
        (self.signing.to_bytes(), self.dh.to_bytes())
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn dh_public(&self) -> PublicKey {
        PublicKey::from(&self.dh)
    }

    pub(crate) fn dh_secret(&self) -> &StaticSecret {
        &self.dh
    }

    pub fn fingerprint(&self) -> Fingerprint {
        Fingerprint::of(&self.signing.verifying_key(), &self.dh_public())
    }

    pub fn card(&self) -> ContactCard {
        let mut card = ContactCard {
            signing_key: self.signing.verifying_key(),
            dh_key: self.dh_public(),
            name: self.name.clone(),
            signature: Signature::from_bytes(&[0; SIGNATURE_LEN]),
        };
        card.signature = self.signing.sign(&card.signed_message());
        card
    }
}

impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Identity")
            .field("name", &self.name)
            .field("fingerprint", &self.fingerprint())
            .finish_non_exhaustive()
    }
}

/// The public half of a user, signed by their Ed25519 key (§3.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContactCard {
    signing_key: VerifyingKey,
    dh_key: PublicKey,
    name: String,
    signature: Signature,
}

impl ContactCard {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn signing_key(&self) -> &VerifyingKey {
        &self.signing_key
    }

    pub fn dh_key(&self) -> &PublicKey {
        &self.dh_key
    }

    pub fn fingerprint(&self) -> Fingerprint {
        Fingerprint::of(&self.signing_key, &self.dh_key)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = self.body();
        bytes.extend_from_slice(&self.signature.to_bytes());
        bytes
    }

    /// Parses a card and verifies its signature.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < CARD_FIXED_LEN + SIGNATURE_LEN {
            return Err(Error::Truncated);
        }
        if bytes[0] != VERSION {
            return Err(Error::UnsupportedVersion(bytes[0]));
        }
        let name_len = bytes[65] as usize;
        if name_len > MAX_NAME_LEN {
            return Err(Error::Invalid("name length"));
        }
        let body_len = CARD_FIXED_LEN + name_len;
        match bytes.len().cmp(&(body_len + SIGNATURE_LEN)) {
            core::cmp::Ordering::Less => return Err(Error::Truncated),
            core::cmp::Ordering::Greater => return Err(Error::TooLong),
            core::cmp::Ordering::Equal => {}
        }

        let signing_key = VerifyingKey::from_bytes(&truncate(&bytes[1..33]))
            .map_err(|_| Error::Invalid("signing key"))?;
        let dh_key = PublicKey::from(truncate::<32>(&bytes[33..65]));
        let name = String::from_utf8(bytes[CARD_FIXED_LEN..body_len].to_vec())
            .map_err(|_| Error::Invalid("name"))?;
        let signature = Signature::from_bytes(&truncate(&bytes[body_len..]));

        let card = Self {
            signing_key,
            dh_key,
            name,
            signature,
        };
        card.signing_key
            .verify_strict(&card.signed_message(), &card.signature)
            .map_err(|_| Error::BadSignature)?;
        Ok(card)
    }

    /// `kervan:` followed by the card in base64url, for QR codes and pasting.
    pub fn to_text(&self) -> String {
        format!(
            "{CARD_TEXT_PREFIX}{}",
            BASE64URL_NOPAD.encode(&self.to_bytes())
        )
    }

    pub fn from_text(text: &str) -> Result<Self> {
        let encoded = text
            .trim()
            .strip_prefix(CARD_TEXT_PREFIX)
            .ok_or(Error::Invalid("card prefix"))?;
        let bytes = BASE64URL_NOPAD
            .decode(encoded.as_bytes())
            .map_err(|_| Error::Invalid("card encoding"))?;
        Self::from_bytes(&bytes)
    }

    fn body(&self) -> Vec<u8> {
        let mut body = Vec::with_capacity(CARD_FIXED_LEN + self.name.len());
        body.push(VERSION);
        body.extend_from_slice(self.signing_key.as_bytes());
        body.extend_from_slice(self.dh_key.as_bytes());
        body.push(self.name.len() as u8);
        body.extend_from_slice(self.name.as_bytes());
        body
    }

    fn signed_message(&self) -> Vec<u8> {
        [CARD_LABEL, &self.body()].concat()
    }
}

/// What two people compare to verify each other (§3).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fingerprint(pub [u8; 16]);

impl Fingerprint {
    fn of(signing_key: &VerifyingKey, dh_key: &PublicKey) -> Self {
        Self(truncate(&sha256(&[
            FINGERPRINT_LABEL,
            signing_key.as_bytes(),
            dh_key.as_bytes(),
        ])))
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let encoded = BASE32_NOPAD.encode(&self.0).to_ascii_lowercase();
        let groups: Vec<&str> = encoded
            .as_bytes()
            .chunks(4)
            .map(|group| core::str::from_utf8(group).expect("base32 is ASCII"))
            .collect();
        f.write_str(&groups.join(" "))
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fingerprint({self})")
    }
}

fn check_name(name: &str) -> Result<()> {
    if name.len() > MAX_NAME_LEN {
        return Err(Error::Invalid("name length"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_round_trips_through_text() {
        let identity = Identity::generate("Ayşe").unwrap();
        let card = identity.card();
        let parsed = ContactCard::from_text(&card.to_text()).unwrap();
        assert_eq!(parsed, card);
        assert_eq!(parsed.name(), "Ayşe");
        assert_eq!(parsed.fingerprint(), identity.fingerprint());
    }

    #[test]
    fn tampered_card_is_rejected() {
        let mut bytes = Identity::generate("Ali").unwrap().card().to_bytes();
        bytes[66] ^= 0x01;
        assert_eq!(ContactCard::from_bytes(&bytes), Err(Error::BadSignature));
    }

    #[test]
    fn card_with_trailing_bytes_is_rejected() {
        let mut bytes = Identity::generate("Ali").unwrap().card().to_bytes();
        bytes.push(0);
        assert_eq!(ContactCard::from_bytes(&bytes), Err(Error::TooLong));
    }

    #[test]
    fn long_names_are_rejected() {
        assert_eq!(
            Identity::generate(&"a".repeat(MAX_NAME_LEN + 1)).unwrap_err(),
            Error::Invalid("name length")
        );
    }

    #[test]
    fn identity_survives_a_secret_round_trip() {
        let identity = Identity::generate("Deniz").unwrap();
        let (signing, dh) = identity.secret_bytes();
        let restored = Identity::from_secret_bytes(&signing, &dh, "Deniz").unwrap();
        assert_eq!(restored.card(), identity.card());
    }

    #[test]
    fn fingerprint_is_grouped_base32() {
        let text = Fingerprint([0; 16]).to_string();
        assert_eq!(text, "aaaa aaaa aaaa aaaa aaaa aaaa aa");
    }
}
