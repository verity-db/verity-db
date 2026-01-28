//! `Craton` administration CLI.
//!
//! Provides commands for managing streams, querying data, and
//! administering a `Craton` instance.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use craton_client::{Client, ClientConfig, QueryParam};
use craton_types::{DataClass, Offset, StreamId, TenantId};

/// `Craton` administration CLI.
#[derive(Parser)]
#[command(name = "craton-admin")]
#[command(about = "Craton administration CLI", long_about = None)]
struct Cli {
    /// Server address.
    #[arg(short, long, default_value = "127.0.0.1:5432")]
    server: String,

    /// Tenant ID.
    #[arg(short, long, default_value = "1")]
    tenant: u64,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new stream.
    CreateStream {
        /// Stream name.
        name: String,

        /// Data classification (non-phi, phi, deidentified).
        #[arg(short, long, default_value = "non-phi")]
        class: String,
    },

    /// Append events to a stream.
    Append {
        /// Stream ID.
        stream_id: u64,

        /// Events to append (as JSON strings).
        events: Vec<String>,
    },

    /// Read events from a stream.
    ReadEvents {
        /// Stream ID.
        stream_id: u64,

        /// Starting offset.
        #[arg(short, long, default_value = "0")]
        from: u64,

        /// Maximum bytes to read.
        #[arg(short, long, default_value = "65536")]
        max_bytes: u64,
    },

    /// Execute a SQL query.
    Query {
        /// SQL query string.
        sql: String,

        /// Query at a specific position (optional).
        #[arg(short, long)]
        at: Option<u64>,
    },

    /// Sync all data to disk.
    Sync,

    /// Show server information.
    Info,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let config = ClientConfig::default();
    let tenant_id = TenantId::new(cli.tenant);

    let mut client = Client::connect(&cli.server, tenant_id, config)
        .with_context(|| format!("Failed to connect to {}", cli.server))?;

    match cli.command {
        Commands::CreateStream { name, class } => {
            let data_class = parse_data_class(&class)?;
            let stream_id = client.create_stream(&name, data_class)?;
            println!("Created stream: {}", u64::from(stream_id));
        }

        Commands::Append { stream_id, events } => {
            let stream = StreamId::new(stream_id);
            let event_data: Vec<Vec<u8>> = events.into_iter().map(String::into_bytes).collect();

            let offset = client.append(stream, event_data)?;
            println!("Appended at offset: {}", offset.as_u64());
        }

        Commands::ReadEvents {
            stream_id,
            from,
            max_bytes,
        } => {
            let stream = StreamId::new(stream_id);
            let from_offset = Offset::new(from);

            let response = client.read_events(stream, from_offset, max_bytes)?;

            println!("Events ({}):", response.events.len());
            for (i, event) in response.events.iter().enumerate() {
                let text = String::from_utf8_lossy(event);
                println!("  [{}] {}", from + i as u64, text);
            }

            if let Some(next) = response.next_offset {
                println!("Next offset: {}", next.as_u64());
            }
        }

        Commands::Query { sql, at } => {
            let params: Vec<QueryParam> = vec![];

            let result = if let Some(position) = at {
                client.query_at(&sql, &params, Offset::new(position))?
            } else {
                client.query(&sql, &params)?
            };

            // Print columns
            println!("{}", result.columns.join("\t"));
            println!("{}", "-".repeat(result.columns.len() * 12));

            // Print rows
            for row in &result.rows {
                let values: Vec<String> = row
                    .iter()
                    .map(|v| match v {
                        craton_wire::QueryValue::Null => "NULL".to_string(),
                        craton_wire::QueryValue::BigInt(n) => n.to_string(),
                        craton_wire::QueryValue::Text(s) => s.clone(),
                        craton_wire::QueryValue::Boolean(b) => b.to_string(),
                        craton_wire::QueryValue::Timestamp(t) => t.to_string(),
                    })
                    .collect();
                println!("{}", values.join("\t"));
            }

            println!("\n{} row(s)", result.rows.len());
        }

        Commands::Sync => {
            client.sync()?;
            println!("Sync complete");
        }

        Commands::Info => {
            println!("Connected to: {}", cli.server);
            println!("Tenant ID: {}", cli.tenant);
        }
    }

    Ok(())
}

fn parse_data_class(s: &str) -> Result<DataClass> {
    match s.to_lowercase().as_str() {
        "non-phi" | "nonphi" => Ok(DataClass::NonPHI),
        "phi" => Ok(DataClass::PHI),
        "deidentified" | "de-identified" => Ok(DataClass::Deidentified),
        other => anyhow::bail!("Unknown data class: {other}. Use non-phi, phi, or deidentified."),
    }
}
