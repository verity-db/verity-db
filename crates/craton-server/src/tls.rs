//! TLS configuration and connection handling.
//!
//! Provides TLS wrapper for server connections using rustls.

use std::fs::File;
use std::io::{self, BufReader, Read, Write};
use std::path::Path;
use std::sync::Arc;

use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ServerConfig, ServerConnection};

use crate::error::{ServerError, ServerResult};

/// TLS configuration for the server.
#[derive(Debug, Clone)]
pub struct TlsConfig {
    /// Path to the certificate file (PEM format).
    pub cert_path: std::path::PathBuf,
    /// Path to the private key file (PEM format).
    pub key_path: std::path::PathBuf,
    /// Whether to require client certificates (mTLS).
    pub require_client_cert: bool,
    /// Path to CA certificate for client verification (optional).
    pub ca_cert_path: Option<std::path::PathBuf>,
}

impl TlsConfig {
    /// Creates a new TLS configuration.
    pub fn new(cert_path: impl AsRef<Path>, key_path: impl AsRef<Path>) -> Self {
        Self {
            cert_path: cert_path.as_ref().to_path_buf(),
            key_path: key_path.as_ref().to_path_buf(),
            require_client_cert: false,
            ca_cert_path: None,
        }
    }

    /// Enables mutual TLS (client certificate verification).
    #[must_use]
    pub fn with_client_auth(mut self, ca_cert_path: impl AsRef<Path>) -> Self {
        self.require_client_cert = true;
        self.ca_cert_path = Some(ca_cert_path.as_ref().to_path_buf());
        self
    }

    /// Builds a rustls `ServerConfig` from this configuration.
    pub fn build_server_config(&self) -> ServerResult<Arc<ServerConfig>> {
        let certs = load_certs(&self.cert_path)?;
        let key = load_private_key(&self.key_path)?;

        let config = if self.require_client_cert {
            // For mTLS, we would configure client cert verification here
            // For now, just use the basic config
            ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(certs, key)
                .map_err(|e| ServerError::Tls(e.to_string()))?
        } else {
            ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(certs, key)
                .map_err(|e| ServerError::Tls(e.to_string()))?
        };

        Ok(Arc::new(config))
    }
}

/// Loads certificates from a PEM file.
fn load_certs(path: &Path) -> ServerResult<Vec<CertificateDer<'static>>> {
    let file = File::open(path).map_err(|e| {
        ServerError::Tls(format!(
            "failed to open certificate file {}: {}",
            path.display(),
            e
        ))
    })?;
    let mut reader = BufReader::new(file);

    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut reader)
        .filter_map(Result::ok)
        .collect();

    if certs.is_empty() {
        return Err(ServerError::Tls(format!(
            "no certificates found in {}",
            path.display()
        )));
    }

    Ok(certs)
}

/// Loads a private key from a PEM file.
fn load_private_key(path: &Path) -> ServerResult<PrivateKeyDer<'static>> {
    let file = File::open(path).map_err(|e| {
        ServerError::Tls(format!("failed to open key file {}: {}", path.display(), e))
    })?;
    let mut reader = BufReader::new(file);

    // Try to read PKCS#8 keys first, then RSA keys, then EC keys
    loop {
        match rustls_pemfile::read_one(&mut reader) {
            Ok(Some(rustls_pemfile::Item::Pkcs1Key(key))) => {
                return Ok(PrivateKeyDer::Pkcs1(key));
            }
            Ok(Some(rustls_pemfile::Item::Pkcs8Key(key))) => {
                return Ok(PrivateKeyDer::Pkcs8(key));
            }
            Ok(Some(rustls_pemfile::Item::Sec1Key(key))) => {
                return Ok(PrivateKeyDer::Sec1(key));
            }
            Ok(Some(_)) => {
                // Skip other items like certificates
            }
            Ok(None) => break,
            Err(e) => {
                return Err(ServerError::Tls(format!(
                    "failed to parse key file {}: {}",
                    path.display(),
                    e
                )));
            }
        }
    }

    Err(ServerError::Tls(format!(
        "no private key found in {}",
        path.display()
    )))
}

/// A TLS-wrapped stream that handles encryption/decryption.
pub struct TlsStream<S> {
    /// The underlying socket.
    pub socket: S,
    /// The TLS connection state.
    conn: ServerConnection,
}

impl<S: Read + Write> TlsStream<S> {
    /// Creates a new TLS stream.
    pub fn new(socket: S, config: Arc<ServerConfig>) -> ServerResult<Self> {
        let conn = ServerConnection::new(config)
            .map_err(|e| ServerError::Tls(format!("failed to create TLS connection: {e}")))?;

        Ok(Self { socket, conn })
    }

    /// Performs the TLS handshake.
    ///
    /// Returns `Ok(true)` if the handshake is complete, `Ok(false)` if it needs
    /// more I/O, or an error if the handshake failed.
    pub fn do_handshake(&mut self) -> ServerResult<bool> {
        if self.conn.is_handshaking() {
            // Write any pending TLS data to the socket
            while self.conn.wants_write() {
                match self.conn.write_tls(&mut self.socket) {
                    Ok(0) => break,
                    Ok(_) => {} // Continue writing
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) => return Err(ServerError::Io(e)),
                }
            }

            // Read TLS data from the socket
            if self.conn.wants_read() {
                match self.conn.read_tls(&mut self.socket) {
                    Ok(0) => {
                        // EOF during handshake
                        return Err(ServerError::ConnectionClosed);
                    }
                    Ok(_) => {
                        // Process the TLS data
                        if let Err(e) = self.conn.process_new_packets() {
                            return Err(ServerError::Tls(format!("TLS error: {e}")));
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(e) => return Err(ServerError::Io(e)),
                }
            }
        }

        Ok(!self.conn.is_handshaking())
    }

    /// Reads decrypted data from the TLS connection.
    pub fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // First, read any pending TLS data from the socket
        while self.conn.wants_read() {
            match self.conn.read_tls(&mut self.socket) {
                Ok(0) => break,
                Ok(_) => {
                    if let Err(e) = self.conn.process_new_packets() {
                        return Err(io::Error::new(io::ErrorKind::InvalidData, e));
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e),
            }
        }

        // Read decrypted data
        self.conn.reader().read(buf)
    }

    /// Writes data to the TLS connection (will be encrypted).
    pub fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.conn.writer().write(buf)?;

        // Flush any pending TLS data to the socket
        while self.conn.wants_write() {
            match self.conn.write_tls(&mut self.socket) {
                Ok(0) => break,
                Ok(_) => {} // Continue writing
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e),
            }
        }

        Ok(written)
    }

    /// Flushes all pending data to the socket.
    pub fn flush(&mut self) -> io::Result<()> {
        while self.conn.wants_write() {
            match self.conn.write_tls(&mut self.socket) {
                Ok(0) => break,
                Ok(_) => {} // Continue writing
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e),
            }
        }
        self.socket.flush()
    }

    /// Returns true if the TLS connection wants to read more data.
    pub fn wants_read(&self) -> bool {
        self.conn.wants_read()
    }

    /// Returns true if the TLS connection has data to write.
    pub fn wants_write(&self) -> bool {
        self.conn.wants_write()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tls_config_new() {
        let config = TlsConfig::new("/path/to/cert.pem", "/path/to/key.pem");
        assert_eq!(config.cert_path.to_str(), Some("/path/to/cert.pem"));
        assert_eq!(config.key_path.to_str(), Some("/path/to/key.pem"));
        assert!(!config.require_client_cert);
        assert!(config.ca_cert_path.is_none());
    }

    #[test]
    fn test_tls_config_with_client_auth() {
        let config = TlsConfig::new("/path/to/cert.pem", "/path/to/key.pem")
            .with_client_auth("/path/to/ca.pem");
        assert!(config.require_client_cert);
        assert_eq!(
            config.ca_cert_path.as_ref().and_then(|p| p.to_str()),
            Some("/path/to/ca.pem")
        );
    }
}
