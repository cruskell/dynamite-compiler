//! Native platform configuration shared by preprocessing and external tools.
use std::{path::PathBuf, process::Command, sync::OnceLock};

pub fn macos_sdk_root() -> Result<PathBuf, String> {
    if let Some(root) = std::env::var_os("SDKROOT").filter(|root| !root.is_empty() && root != "/") {
        let root = PathBuf::from(root);
        if root.join("usr/include").is_dir() {
            return Ok(root);
        }
        return Err(format!(
            "SDKROOT does not contain macOS headers: {}",
            root.display()
        ));
    }
    static SDK: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    SDK.get_or_init(|| {
        let result = Command::new("/usr/bin/xcrun")
            .args(["--sdk", "macosx", "--show-sdk-path"])
            .output()
            .map_err(|e| format!("cannot locate macOS SDK: {e}"))?;
        if !result.status.success() {
            return Err("cannot locate macOS SDK; set SDKROOT".into());
        }
        let root = PathBuf::from(
            String::from_utf8(result.stdout)
                .map_err(|e| e.to_string())?
                .trim(),
        );
        if !root.join("usr/include").is_dir() {
            return Err("macOS SDK has no usr/include directory".into());
        }
        Ok(root)
    })
    .clone()
}

pub fn macos_deployment_target() -> Result<String, String> {
    let target = std::env::var("MACOSX_DEPLOYMENT_TARGET").unwrap_or_else(|_| {
        if cfg!(target_arch = "aarch64") {
            "11.0"
        } else {
            "10.13"
        }
        .into()
    });
    macos_version_number(&target)?;
    Ok(target)
}

pub fn macos_version_number(version: &str) -> Result<u32, String> {
    let parts = version
        .split('.')
        .map(str::parse::<u32>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| format!("invalid macOS deployment target: {version}"))?;
    if parts.is_empty()
        || parts.len() > 3
        || !(10..=9999).contains(&parts[0])
        || parts.iter().skip(1).any(|v| *v > 99)
    {
        return Err(format!("invalid macOS deployment target: {version}"));
    }
    Ok(parts[0] * 10000
        + parts.get(1).copied().unwrap_or(0) * 100
        + parts.get(2).copied().unwrap_or(0))
}
