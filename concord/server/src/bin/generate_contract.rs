use std::env;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};

fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let output = args
        .next()
        .map(PathBuf::from)
        .context("usage: generate-contract <output.json>")?;
    if args.next().is_some() {
        bail!("usage: generate-contract <output.json>");
    }

    let schema = concord_server::contract::websocket_schema();
    let mut json = serde_json::to_string_pretty(&schema).context("serialize contract schema")?;
    json.push('\n');
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create output directory {}", parent.display()))?;
    }
    fs::write(&output, json)
        .with_context(|| format!("write contract schema to {}", output.display()))?;
    Ok(())
}
