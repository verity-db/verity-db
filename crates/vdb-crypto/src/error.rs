use ed25519_dalek::SignatureError;

#[derive(thiserror::Error, Debug)]
pub enum CryptoError {
    #[error(transparent)]
    SignatureError(#[from] SignatureError),
}
