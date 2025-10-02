//! Project tasks: `cargo xtask ci | fmt | clippy | test | layers | wasm`.

use std::collections::BTreeMap;
use std::process::Command;

use anyhow::{Context, Result, bail};

/// Architectural layer of every workspace crate (see docs: rule "layers").
const LAYERS: &[(&str, u8)] = &[
    ("raft-doc", 1),
    ("raft-search", 2),
    ("raft-catalog", 3),
    ("raft-store", 3),
    ("raft-render", 3),
    ("raft-text", 3),
    ("raft-engine", 5),
    ("raft-ui", 6),
    ("raft", 7),
    ("xtask", 7),
];

/// GUI crates that only layer 6 and above may depend on.
const GUI_CRATES: &[&str] = &[
    "egui",
    "eframe",
    "winit",
    "rfd",
    "egui_kittest",
    "egui_extras",
];

/// Crates that must keep compiling for `wasm32-unknown-unknown`.
const WASM_CRATES: &[&str] = &[
    "raft-text",
    "raft-doc",
    "raft-search",
    "raft-catalog",
    "raft-engine",
    "raft-ui",
];

fn main() -> Result<()> {
    let task = std::env::args().nth(1).unwrap_or_else(|| "ci".to_owned());
    match task.as_str() {
        "ci" => {
            fmt()?;
            clippy()?;
            test()?;
            layers()?;
            wasm()?;
            println!("xtask ci: all checks passed");
            Ok(())
        }
        "fmt" => fmt(),
        "clippy" => clippy(),
        "test" => test(),
        "layers" => layers(),
        "wasm" => wasm(),
        other => bail!("unknown task `{other}` (ci, fmt, clippy, test, layers, wasm)"),
    }
}

fn cargo(args: &[&str]) -> Result<()> {
    println!("$ cargo {}", args.join(" "));
    let status = Command::new(env!("CARGO"))
        .args(args)
        .status()
        .with_context(|| format!("running cargo {}", args.join(" ")))?;
    if !status.success() {
        bail!("cargo {} failed", args.join(" "));
    }
    Ok(())
}

fn fmt() -> Result<()> {
    cargo(&["fmt", "--all", "--check"])
}

fn clippy() -> Result<()> {
    cargo(&[
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ])
}

fn test() -> Result<()> {
    cargo(&["test", "--workspace", "-q"])
}

fn wasm() -> Result<()> {
    let mut args = vec!["build", "--target", "wasm32-unknown-unknown"];
    for krate in WASM_CRATES {
        args.push("-p");
        args.push(krate);
    }
    cargo(&args)
}

fn layers() -> Result<()> {
    let out = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .context("running cargo metadata")?;
    if !out.status.success() {
        bail!("cargo metadata failed");
    }
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    let layers: BTreeMap<&str, u8> = LAYERS.iter().copied().collect();
    let mut errors = Vec::new();
    let packages = meta
        .get("packages")
        .and_then(|p| p.as_array())
        .context("no packages in cargo metadata")?;
    for pkg in packages {
        let name = pkg.get("name").and_then(|n| n.as_str()).unwrap_or_default();
        let Some(&own) = layers.get(name) else {
            errors.push(format!("{name}: crate has no layer in xtask LAYERS"));
            continue;
        };
        let deps = pkg.get("dependencies").and_then(|d| d.as_array());
        for dep in deps.into_iter().flatten() {
            let dep_name = dep.get("name").and_then(|n| n.as_str()).unwrap_or_default();
            let is_dev = dep.get("kind").and_then(|k| k.as_str()) == Some("dev");
            if let Some(&theirs) = layers.get(dep_name)
                && theirs >= own
                && !is_dev
            {
                errors.push(format!("{name} (L{own}) depends on {dep_name} (L{theirs})"));
            }
            if own < 6 && GUI_CRATES.contains(&dep_name) {
                errors.push(format!("{name} (L{own}) depends on GUI crate {dep_name}"));
            }
        }
    }
    if errors.is_empty() {
        println!("layers: ok");
        Ok(())
    } else {
        for e in &errors {
            eprintln!("layers: {e}");
        }
        bail!("{} layer violation(s)", errors.len())
    }
}
