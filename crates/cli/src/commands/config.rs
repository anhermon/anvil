use clap::Args;
use harness_core::config::Config;
use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Args)]
pub struct ConfigArgs {
    /// Verify provider connectivity (not just static config)
    #[arg(long)]
    pub check: bool,
}

pub async fn execute(args: ConfigArgs) -> anyhow::Result<()> {
    let config = Config::load()?;
    println!("Provider:   {}", config.provider.backend);
    println!("Model:      {}", config.provider.model);
    println!("Max tokens: {}", config.provider.max_tokens);
    println!("Memory DB:  {}", config.memory.db_path.display());
    println!("Agent name: {}", config.agent.name);

    if args.check {
        check_api_key(&config);
        check_native_build_prerequisites();
        check_connectivity(&config).await;
    }
    Ok(())
}

fn check_api_key(config: &Config) {
    match config.resolved_api_key() {
        Some(_) => println!("API key:    [set]"),
        None => println!("API key:    [NOT SET] <- set ANTHROPIC_API_KEY"),
    }
}

async fn check_connectivity(config: &Config) {
    use harness_core::{
        message::Message,
        provider::{EchoProvider, Provider},
        providers::{ClaudeCodeProvider, ClaudeProvider},
    };
    use std::sync::Arc;
    use std::time::Instant;

    let backend = &config.provider.backend;

    let provider: Result<Arc<dyn Provider>, String> = match backend.as_str() {
        "echo" => Ok(Arc::new(EchoProvider::new())),
        "claude-code" | "cc" => Ok(Arc::new(ClaudeCodeProvider::new(&config.provider.model))),
        _ => ClaudeProvider::from_env(&config.provider.model, config.provider.max_tokens)
            .map(|p| Arc::new(p) as Arc<dyn Provider>)
            .map_err(|e| e.to_string()),
    };

    let provider = match provider {
        Ok(p) => p,
        Err(e) => {
            println!("Connectivity: FAILED (cannot create provider: {e})");
            return;
        }
    };

    print!("Connectivity: checking...");
    let _ = std::io::Write::flush(&mut std::io::stdout());

    let start = Instant::now();
    let ping = vec![Message::user("ping")];
    match provider.complete(&ping).await {
        Ok(resp) => {
            let elapsed = start.elapsed();
            print!("\r");
            println!("Connectivity: OK ({} -- {:.0?})", resp.model, elapsed,);
        }
        Err(e) => {
            print!("\r");
            println!("Connectivity: FAILED ({e})");
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum NativeDependency {
    Cc,
    PkgConfig,
    OpenSsl,
}

impl NativeDependency {
    fn label(self) -> &'static str {
        match self {
            Self::Cc => "cc",
            Self::PkgConfig => "pkg-config",
            Self::OpenSsl => "openssl",
        }
    }

    fn install_hint(self, os: &str) -> &'static str {
        match (os, self) {
            ("macos", Self::Cc) => {
                "Install Xcode Command Line Tools: xcode-select --install."
            }
            ("macos", Self::PkgConfig) => "Install pkg-config: brew install pkg-config.",
            ("macos", Self::OpenSsl) => {
                "Install OpenSSL + pkg-config: brew install openssl@3 pkg-config; then export PKG_CONFIG_PATH=\"$(brew --prefix openssl@3)/lib/pkgconfig:$PKG_CONFIG_PATH\"."
            }
            ("windows", Self::Cc) => {
                "Install Visual Studio Build Tools with \"Desktop development with C++\"."
            }
            ("windows", Self::PkgConfig) => {
                "Install pkg-config via MSYS2: pacman -S mingw-w64-x86_64-pkgconf."
            }
            ("windows", Self::OpenSsl) => {
                "Install OpenSSL and set OPENSSL_DIR/OPENSSL_LIB_DIR (or use vcpkg install openssl:x64-windows)."
            }
            ("linux", Self::Cc) => {
                "Install a C toolchain (Debian/Ubuntu: apt install build-essential; Fedora: dnf groupinstall 'Development Tools'; Alpine: apk add build-base)."
            }
            ("linux", Self::PkgConfig) => {
                "Install pkg-config (Debian/Ubuntu: apt install pkg-config; Fedora: dnf install pkgconf-pkg-config; Alpine: apk add pkgconf)."
            }
            ("linux", Self::OpenSsl) => {
                "Install OpenSSL development headers (Debian/Ubuntu: apt install libssl-dev; Fedora: dnf install openssl-devel; Alpine: apk add openssl-dev)."
            }
            (_, Self::Cc) => "Install a C compiler toolchain for your platform.",
            (_, Self::PkgConfig) => "Install pkg-config for your platform.",
            (_, Self::OpenSsl) => {
                "Install OpenSSL development headers/libraries and configure OPENSSL_DIR if needed."
            }
        }
    }
}

#[derive(Debug)]
struct NativeProbe {
    dependency: NativeDependency,
    ok: bool,
    detail: String,
}

impl NativeProbe {
    fn ok(dependency: NativeDependency, detail: impl Into<String>) -> Self {
        Self {
            dependency,
            ok: true,
            detail: detail.into(),
        }
    }

    fn missing(dependency: NativeDependency, detail: impl Into<String>) -> Self {
        Self {
            dependency,
            ok: false,
            detail: detail.into(),
        }
    }
}

fn check_native_build_prerequisites() {
    let target_os = env::consts::OS;
    println!("Native deps: preflight (target OS: {target_os})");

    let cc_probe = probe_command(NativeDependency::Cc);
    print_probe(&cc_probe, target_os);

    let pkg_probe = probe_command(NativeDependency::PkgConfig);
    print_probe(&pkg_probe, target_os);

    let openssl_probe = probe_openssl(pkg_probe.ok);
    print_probe(&openssl_probe, target_os);
}

fn print_probe(probe: &NativeProbe, os: &str) {
    if probe.ok {
        println!(
            "{}:         OK ({})",
            probe.dependency.label(),
            probe.detail
        );
    } else {
        println!(
            "{}:         MISSING ({})",
            probe.dependency.label(),
            probe.detail
        );
        println!("             hint: {}", probe.dependency.install_hint(os));
    }
}

fn probe_command(dependency: NativeDependency) -> NativeProbe {
    let command_name = dependency.label();
    match find_command(command_name) {
        Some(path) => NativeProbe::ok(dependency, format!("found at {}", path.display())),
        None => NativeProbe::missing(dependency, "not found on PATH"),
    }
}

fn probe_openssl(has_pkg_config: bool) -> NativeProbe {
    if env::var_os("OPENSSL_DIR").is_some() || env::var_os("OPENSSL_LIB_DIR").is_some() {
        return NativeProbe::ok(NativeDependency::OpenSsl, "OPENSSL_DIR/OPENSSL_LIB_DIR set");
    }

    if has_pkg_config {
        match Command::new("pkg-config")
            .args(["--exists", "openssl"])
            .status()
        {
            Ok(status) if status.success() => {
                return NativeProbe::ok(NativeDependency::OpenSsl, "detected via pkg-config");
            }
            Ok(_) => {
                return NativeProbe::missing(
                    NativeDependency::OpenSsl,
                    "pkg-config could not resolve openssl",
                );
            }
            Err(e) => {
                return NativeProbe::missing(
                    NativeDependency::OpenSsl,
                    format!("pkg-config probe failed: {e}"),
                );
            }
        }
    }

    if let Some(path) = find_command("openssl") {
        return NativeProbe::ok(
            NativeDependency::OpenSsl,
            format!(
                "openssl CLI found at {} (install pkg-config for stronger detection)",
                path.display()
            ),
        );
    }

    NativeProbe::missing(
        NativeDependency::OpenSsl,
        "neither pkg-config detection nor OPENSSL_DIR overrides are available",
    )
}

fn find_command(command_name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    let candidates = command_candidates(command_name);

    for dir in env::split_paths(&path) {
        for candidate in &candidates {
            let full = dir.join(candidate);
            if is_executable_file(&full) {
                return Some(full);
            }
        }
    }

    None
}

fn command_candidates(command_name: &str) -> Vec<String> {
    if env::consts::OS != "windows" {
        return vec![command_name.to_string()];
    }

    if Path::new(command_name).extension().is_some() {
        return vec![command_name.to_string()];
    }

    let mut candidates = vec![command_name.to_string()];
    let pathext = env::var_os("PATHEXT")
        .map(|exts| exts.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".to_string());

    for ext in pathext.split(';') {
        let trimmed = ext.trim();
        if !trimmed.is_empty() {
            candidates.push(format!("{command_name}{trimmed}"));
        }
    }

    candidates
}

fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn check_displays_static_config() {
        let args = ConfigArgs { check: false };
        let result = execute(args).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn check_with_echo_provider_reports_ok() {
        let mut config = Config::default();
        config.provider.backend = "echo".to_string();
        check_connectivity(&config).await;
    }

    #[test]
    fn linux_hints_are_actionable() {
        let hint = NativeDependency::OpenSsl.install_hint("linux");
        assert!(hint.contains("libssl-dev"));
    }

    #[test]
    fn windows_hints_are_actionable() {
        let hint = NativeDependency::Cc.install_hint("windows");
        assert!(hint.contains("Visual Studio Build Tools"));
    }
}
