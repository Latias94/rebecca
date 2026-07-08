use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;

use crate::cli::{OutputMode, SchemaDocumentArg};
use crate::output::CliApiContract;

#[derive(Debug, Serialize)]
struct CliSchemaPayload {
    api_version: &'static str,
    document: &'static str,
    media_type: &'static str,
    schema: Value,
}

pub fn export(output_mode: OutputMode, document: SchemaDocumentArg) -> Result<()> {
    let payload = schema_payload(document)?;
    crate::output::print_command_success_with_contract(
        CliApiContract::v1("schema export", "cli-schema"),
        output_mode,
        || &payload,
        || {
            println!("{}", serde_json::to_string_pretty(&payload.schema)?);
            Ok(())
        },
    )
}

fn schema_payload(document: SchemaDocumentArg) -> Result<CliSchemaPayload> {
    let raw = schema_raw(document);
    let schema = serde_json::from_str(raw).with_context(|| {
        format!(
            "embedded {} CLI schema is invalid JSON",
            document_name(document)
        )
    })?;
    Ok(CliSchemaPayload {
        api_version: "rebecca.cli.v1",
        document: document_name(document),
        media_type: "application/schema+json",
        schema,
    })
}

fn document_name(document: SchemaDocumentArg) -> &'static str {
    match document {
        SchemaDocumentArg::Envelope => "envelope",
        SchemaDocumentArg::Event => "event",
        SchemaDocumentArg::Error => "error",
        SchemaDocumentArg::Payloads => "payloads",
        SchemaDocumentArg::Config => "config",
        SchemaDocumentArg::CleanerManifestV1 => "cleaner-manifest-v1",
    }
}

fn schema_raw(document: SchemaDocumentArg) -> &'static str {
    match document {
        SchemaDocumentArg::Envelope => {
            include_str!("../schemas/api/cli/v1/envelope.schema.json")
        }
        SchemaDocumentArg::Event => include_str!("../schemas/api/cli/v1/event.schema.json"),
        SchemaDocumentArg::Error => include_str!("../schemas/api/cli/v1/error.schema.json"),
        SchemaDocumentArg::Payloads => {
            include_str!("../schemas/api/cli/v1/payloads.schema.json")
        }
        SchemaDocumentArg::Config => include_str!("../schemas/api/cli/v1/config.schema.json"),
        SchemaDocumentArg::CleanerManifestV1 => {
            include_str!("../schemas/api/cli/v1/cleaner-manifest-v1.schema.json")
        }
    }
}
