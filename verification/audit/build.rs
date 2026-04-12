use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use quote::ToTokens;
use serde::Deserialize;
use syn::{Fields, Item};

#[derive(Debug, Deserialize)]
struct AuditManifest {
    field: Vec<FieldExpectation>,
}

#[derive(Debug, Deserialize)]
struct FieldExpectation {
    file: String,
    #[serde(rename = "struct")]
    struct_name: String,
    field: String,
    expected_type: String,
}

fn main() {
    let strict = env::var_os("CARGO_FEATURE_STRICT_AUDIT").is_some();
    let crate_root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let workspace_root = crate_root
        .join("../..")
        .canonicalize()
        .expect("workspace root");
    let manifest_path = crate_root.join("agent_layer_manifest.toml");

    println!("cargo:rerun-if-changed={}", manifest_path.display());

    let manifest_text = fs::read_to_string(&manifest_path).unwrap_or_else(|error| {
        panic!(
            "fireline-audit could not read manifest {}: {error}",
            manifest_path.display()
        )
    });
    let manifest: AuditManifest = toml::from_str(&manifest_text).unwrap_or_else(|error| {
        panic!(
            "fireline-audit could not parse manifest {}: {error}",
            manifest_path.display()
        )
    });

    let violations: Vec<String> = manifest
        .field
        .iter()
        .filter_map(|entry| audit_field(&workspace_root, entry).err())
        .collect();

    if violations.is_empty() {
        return;
    }

    if strict {
        panic!(
            "fireline-audit strict-audit found {} violation(s):\n\n{}",
            violations.len(),
            violations.join("\n\n")
        );
    }

    for violation in violations {
        println!("cargo:warning={violation}");
    }
    println!(
        "cargo:warning=fireline-audit strict-audit is off; violations stay as warnings until ACP canonical identifiers Phase 1.5."
    );
}

fn audit_field(workspace_root: &Path, entry: &FieldExpectation) -> Result<(), String> {
    let file_path = workspace_root.join(&entry.file);
    println!("cargo:rerun-if-changed={}", file_path.display());

    let source = fs::read_to_string(&file_path).map_err(|error| {
        format!(
            "{}::{}::{} -> could not read {}: {error}",
            entry.file,
            entry.struct_name,
            entry.field,
            file_path.display()
        )
    })?;

    let parsed = syn::parse_file(&source).map_err(|error| {
        format!(
            "{}::{}::{} -> could not parse {}: {error}",
            entry.file,
            entry.struct_name,
            entry.field,
            file_path.display()
        )
    })?;

    let item_struct = parsed
        .items
        .iter()
        .find_map(|item| match item {
            Item::Struct(item_struct) if item_struct.ident == entry.struct_name.as_str() => {
                Some(item_struct)
            }
            _ => None,
        })
        .ok_or_else(|| {
            format!(
                "{}::{}::{} -> struct `{}` not found in {}",
                entry.file,
                entry.struct_name,
                entry.field,
                entry.struct_name,
                file_path.display()
            )
        })?;

    let named_fields = match &item_struct.fields {
        Fields::Named(fields) => &fields.named,
        _ => {
            return Err(format!(
                "{}::{}::{} -> struct `{}` does not have named fields",
                entry.file, entry.struct_name, entry.field, entry.struct_name
            ))
        }
    };

    let field = named_fields
        .iter()
        .find(|field| {
            field
                .ident
                .as_ref()
                .is_some_and(|ident| ident == entry.field.as_str())
        })
        .ok_or_else(|| {
            format!(
                "{}::{}::{} -> field `{}` not found",
                entry.file, entry.struct_name, entry.field, entry.field
            )
        })?;

    let actual = normalize_type_tokens(&field.ty.to_token_stream().to_string());
    let expected = normalize_type_tokens(&entry.expected_type);

    if actual == expected {
        return Ok(());
    }

    Err(format!(
        "{}::{}::{} -> expected `{}`, found `{}`",
        entry.file, entry.struct_name, entry.field, entry.expected_type, actual
    ))
}

fn normalize_type_tokens(input: &str) -> String {
    input
        .replace("std :: option :: Option", "Option")
        .replace("std::option::Option", "Option")
        .replace("core :: option :: Option", "Option")
        .replace("core::option::Option", "Option")
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect()
}
