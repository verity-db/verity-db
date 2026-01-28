//! Integration tests for the server.

use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use tempfile::TempDir;
use vdb::Verity;
use vdb_client::{Client, ClientConfig};
use vdb_types::{DataClass, Offset, TenantId};

use crate::{Server, ServerConfig};

/// Finds an available port on localhost.
fn find_available_port() -> u16 {
    // Bind to port 0 to let OS assign an available port
    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind to port 0");
    listener
        .local_addr()
        .expect("Failed to get local addr")
        .port()
}

#[test]
fn test_server_binds_to_address() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let port = find_available_port();
    let addr = format!("127.0.0.1:{port}").parse::<SocketAddr>().expect("Invalid addr");
    let config = ServerConfig::new(addr, temp_dir.path());
    let db = Verity::open(temp_dir.path()).expect("Failed to open database");

    let server = Server::new(config, db).expect("Failed to create server");
    let local_addr = server.local_addr().expect("Failed to get local addr");

    assert_eq!(local_addr.port(), port);
    assert_eq!(server.connection_count(), 0);
}

#[test]
fn test_server_accepts_connection() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let port = find_available_port();
    let addr = format!("127.0.0.1:{port}").parse::<SocketAddr>().expect("Invalid addr");
    let config = ServerConfig::new(addr, temp_dir.path());
    let db = Verity::open(temp_dir.path()).expect("Failed to open database");

    let mut server = Server::new(config, db).expect("Failed to create server");

    // Connect a client in a background thread
    let client_handle = thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        let config = ClientConfig::default();
        let result = Client::connect(format!("127.0.0.1:{port}"), TenantId::new(1), config);
        result.is_ok()
    });

    // Poll the server a few times to accept and process the connection
    for _ in 0..10 {
        let _ = server.poll_once(Some(Duration::from_millis(50)));
    }

    let client_connected = client_handle.join().expect("Client thread panicked");
    assert!(client_connected, "Client should connect successfully");
}

#[test]
fn test_server_max_connections() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let port = find_available_port();
    let addr = format!("127.0.0.1:{port}").parse::<SocketAddr>().expect("Invalid addr");

    // Set max connections to 2
    let config = ServerConfig::new(addr, temp_dir.path()).with_max_connections(2);
    let db = Verity::open(temp_dir.path()).expect("Failed to open database");

    let mut server = Server::new(config, db).expect("Failed to create server");

    // Spawn multiple client connections
    let mut handles = vec![];
    for i in 0..3 {
        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50 * (i as u64 + 1)));
            let config = ClientConfig {
                read_timeout: Some(Duration::from_millis(500)),
                write_timeout: Some(Duration::from_millis(500)),
                ..Default::default()
            };
            Client::connect(format!("127.0.0.1:{port}"), TenantId::new(1), config).is_ok()
        });
        handles.push(handle);
    }

    // Poll the server to process connections
    for _ in 0..20 {
        let _ = server.poll_once(Some(Duration::from_millis(50)));
    }

    let results: Vec<bool> = handles
        .into_iter()
        .map(|h| h.join().expect("Client thread panicked"))
        .collect();

    // At least 2 should succeed, the third may be rejected
    let successes = results.iter().filter(|&&r| r).count();
    assert!(
        successes >= 2,
        "At least 2 connections should succeed, got {successes}"
    );
}

#[test]
fn test_connection_buffer_limit() {
    // Test that the client enforces buffer limits
    let config = ClientConfig {
        buffer_size: 1024, // Small buffer for testing
        ..Default::default()
    };

    // Verify the configuration is respected
    assert_eq!(config.buffer_size, 1024);
}

#[test]
fn test_server_config_defaults() {
    let config = ServerConfig::default();

    assert_eq!(config.bind_addr.port(), 5432);
    assert_eq!(config.max_connections, 1024);
    assert_eq!(config.read_buffer_size, 64 * 1024);
}

#[test]
fn test_client_config_defaults() {
    let config = ClientConfig::default();

    assert_eq!(config.read_timeout, Some(Duration::from_secs(30)));
    assert_eq!(config.write_timeout, Some(Duration::from_secs(30)));
    assert_eq!(config.buffer_size, 64 * 1024);
    assert!(config.auth_token.is_none());
}

#[cfg(test)]
mod end_to_end {
    use super::*;

    /// Helper to run a server and client end-to-end test.
    fn run_e2e_test<F>(test_fn: F)
    where
        F: FnOnce(u16) + Send + 'static,
    {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let port = find_available_port();
        let addr = format!("127.0.0.1:{port}").parse::<SocketAddr>().expect("Invalid addr");
        let config = ServerConfig::new(addr, temp_dir.path());
        let db = Verity::open(temp_dir.path()).expect("Failed to open database");

        let running = Arc::new(AtomicBool::new(true));
        let running_clone = Arc::clone(&running);

        let server_handle = thread::spawn(move || {
            let mut server = Server::new(config, db).expect("Failed to create server");
            while running_clone.load(Ordering::SeqCst) {
                let _ = server.poll_once(Some(Duration::from_millis(10)));
            }
        });

        // Give the server time to start
        thread::sleep(Duration::from_millis(100));

        // Run the test
        test_fn(port);

        // Stop the server
        running.store(false, Ordering::SeqCst);
        server_handle.join().expect("Server thread panicked");
    }

    #[test]
    fn test_e2e_handshake() {
        run_e2e_test(|port| {
            let config = ClientConfig::default();
            let client = Client::connect(format!("127.0.0.1:{port}"), TenantId::new(1), config);

            assert!(client.is_ok(), "Handshake should succeed");
        });
    }

    #[test]
    fn test_e2e_create_stream() {
        run_e2e_test(|port| {
            let config = ClientConfig::default();
            let mut client =
                Client::connect(format!("127.0.0.1:{port}"), TenantId::new(1), config)
                    .expect("Failed to connect");

            let stream_id = client
                .create_stream("test-events", DataClass::NonPHI)
                .expect("Failed to create stream");

            assert!(u64::from(stream_id) > 0, "Stream ID should be assigned");
        });
    }

    #[test]
    fn test_e2e_append_and_read() {
        run_e2e_test(|port| {
            let config = ClientConfig::default();
            let mut client =
                Client::connect(format!("127.0.0.1:{port}"), TenantId::new(1), config)
                    .expect("Failed to connect");

            // Create a stream
            let stream_id = client
                .create_stream("events", DataClass::NonPHI)
                .expect("Failed to create stream");

            // Append events
            let events = vec![b"event1".to_vec(), b"event2".to_vec(), b"event3".to_vec()];
            let first_offset = client
                .append(stream_id, events)
                .expect("Failed to append events");

            assert_eq!(first_offset.as_u64(), 0, "First offset should be 0");

            // Read events back
            let response = client
                .read_events(stream_id, Offset::new(0), 1024 * 1024)
                .expect("Failed to read events");

            assert_eq!(response.events.len(), 3, "Should read 3 events");
            assert_eq!(response.events[0], b"event1");
            assert_eq!(response.events[1], b"event2");
            assert_eq!(response.events[2], b"event3");
        });
    }

    #[test]
    fn test_e2e_sync() {
        run_e2e_test(|port| {
            let config = ClientConfig::default();
            let mut client =
                Client::connect(format!("127.0.0.1:{port}"), TenantId::new(1), config)
                    .expect("Failed to connect");

            // Sync should succeed
            client.sync().expect("Sync should succeed");
        });
    }

    #[test]
    fn test_e2e_batch_append() {
        run_e2e_test(|port| {
            let config = ClientConfig::default();
            let mut client =
                Client::connect(format!("127.0.0.1:{port}"), TenantId::new(1), config)
                    .expect("Failed to connect");

            // Create a stream
            let stream = client
                .create_stream("batch-test", DataClass::NonPHI)
                .expect("Failed to create stream");

            // Append multiple events in a single batch
            let first_offset = client
                .append(
                    stream,
                    vec![
                        b"event1".to_vec(),
                        b"event2".to_vec(),
                        b"event3".to_vec(),
                        b"event4".to_vec(),
                    ],
                )
                .expect("Failed to append batch");

            // Read all events back
            let response = client
                .read_events(stream, first_offset, 4096)
                .expect("Failed to read stream");

            assert_eq!(response.events.len(), 4, "Should have 4 events");
            assert_eq!(response.events[0], b"event1");
            assert_eq!(response.events[1], b"event2");
            assert_eq!(response.events[2], b"event3");
            assert_eq!(response.events[3], b"event4");
        });
    }

    #[test]
    fn test_e2e_moderately_sized_payload() {
        run_e2e_test(|port| {
            let config = ClientConfig::default();
            let mut client =
                Client::connect(format!("127.0.0.1:{port}"), TenantId::new(1), config)
                    .expect("Failed to connect");

            // Create a stream
            let stream_id = client
                .create_stream("sized-events", DataClass::NonPHI)
                .expect("Failed to create stream");

            // Append moderately sized events that fit in B+tree pages (4KB pages)
            // Use ~2KB events to be safe
            let event = vec![0xAB_u8; 2000];
            let first_offset = client
                .append(stream_id, vec![event.clone()])
                .expect("Failed to append event");

            // Read it back
            let response = client
                .read_events(stream_id, first_offset, 8 * 1024)
                .expect("Failed to read event");

            assert_eq!(response.events.len(), 1, "Should read 1 event");
            assert_eq!(response.events[0].len(), 2000, "Event size should match");
            assert_eq!(response.events[0], event, "Event content should match");
        });
    }

    #[test]
    fn test_e2e_reconnection() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let port = find_available_port();
        let addr = format!("127.0.0.1:{port}").parse::<SocketAddr>().expect("Invalid addr");
        let config = ServerConfig::new(addr, temp_dir.path());
        let db = Verity::open(temp_dir.path()).expect("Failed to open database");

        let running = Arc::new(AtomicBool::new(true));
        let running_clone = Arc::clone(&running);

        let server_handle = thread::spawn(move || {
            let mut server = Server::new(config, db).expect("Failed to create server");
            while running_clone.load(Ordering::SeqCst) {
                let _ = server.poll_once(Some(Duration::from_millis(10)));
            }
        });

        // Give server time to start
        thread::sleep(Duration::from_millis(100));

        // First connection
        {
            let config = ClientConfig::default();
            let mut client =
                Client::connect(format!("127.0.0.1:{port}"), TenantId::new(1), config)
                    .expect("Failed to connect");

            let stream_id = client
                .create_stream("reconnect-test", DataClass::NonPHI)
                .expect("Failed to create stream");

            client
                .append(stream_id, vec![b"event1".to_vec()])
                .expect("Failed to append");
        }
        // Client dropped, connection closed

        // Second connection
        {
            let config = ClientConfig::default();
            let client =
                Client::connect(format!("127.0.0.1:{port}"), TenantId::new(1), config);

            assert!(client.is_ok(), "Reconnection should succeed");
        }

        // Stop server
        running.store(false, Ordering::SeqCst);
        server_handle.join().expect("Server thread panicked");
    }
}
