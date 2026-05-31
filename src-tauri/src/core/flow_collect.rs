use anyhow::{Context, Result};
use clash_verge_logging::{logging, Type};
use std::{
    path::PathBuf,
    process::{Child, Command},
    sync::Mutex,
    time::{Duration, Instant},
};

use crate::singleton;
use crate::utils::dirs;

/// Returns the target triple at compile time (e.g., "x86_64-pc-windows-msvc").
macro_rules! target_triple {
    () => {
        if cfg!(target_os = "windows") {
            if cfg!(target_arch = "x86_64") {
                "x86_64-pc-windows-msvc"
            } else if cfg!(target_arch = "aarch64") {
                "aarch64-pc-windows-msvc"
            } else {
                "i686-pc-windows-msvc"
            }
        } else if cfg!(target_os = "macos") {
            if cfg!(target_arch = "aarch64") {
                "aarch64-apple-darwin"
            } else {
                "x86_64-apple-darwin"
            }
        } else if cfg!(target_os = "linux") {
            if cfg!(target_arch = "aarch64") {
                "aarch64-unknown-linux-gnu"
            } else if cfg!(target_arch = "arm") {
                "armv7-unknown-linux-gnueabihf"
            } else {
                "x86_64-unknown-linux-gnu"
            }
        } else {
            "unknown"
        }
    };
}

/// Candidate binary names to search for, in priority order.
/// Tauri externalBin uses the `<name>-<target_triple>[.exe]` naming convention.
fn binary_candidates() -> Vec<String> {
    let ext = if cfg!(windows) { ".exe" } else { "" };
    let target = target_triple!();
    vec![
        format!("flow_collect_client-{}{}", target, ext),
        format!("flow_collect_client{}", ext),
    ]
}

pub struct FlowCollectManager {
    child: Mutex<Option<Child>>,
}

impl FlowCollectManager {
    fn new() -> Self {
        Self {
            child: Mutex::new(None),
        }
    }

    /// Locate the FlowCollect client binary.
    /// Search order:
    /// 1. Next to the current executable (installed/bundled mode, with target triple suffix)
    /// 2. Next to the current executable (plain name, development mode)
    /// 3. In the app home directory
    fn find_binary() -> Result<PathBuf> {
        let candidates = binary_candidates();

        // 1. Check next to the current executable
        if let Ok(current_exe) = tauri::utils::platform::current_exe() {
            if let Some(exe_dir) = current_exe.parent() {
                for name in &candidates {
                    let candidate = exe_dir.join(name);
                    if candidate.exists() {
                        return Ok(candidate);
                    }
                }
            }
        }

        // 2. Check the app home directory
        if let Ok(home_dir) = dirs::app_home_dir() {
            for name in &candidates {
                let candidate = home_dir.join(name);
                if candidate.exists() {
                    return Ok(candidate);
                }
            }
        }

        anyhow::bail!(
            "FlowCollect client binary not found (searched for {:?} next to executable and in app home dir)",
            candidates
        )
    }

    /// Start the FlowCollect client as a sidecar process.
    /// `config_path` is the path to the Clash config.yaml that contains `x-flow-collect`.
    pub fn start(&self, config_path: &str) -> Result<()> {
        let mut guard = self.child.lock().unwrap();

        // Already running
        if let Some(ref mut child) = *guard {
            // Check if still alive
            match child.try_wait() {
                Ok(None) => {
                    logging!(
                        info,
                        Type::Core,
                        "FlowCollect client already running (PID: {})",
                        child.id()
                    );
                    return Ok(());
                }
                Ok(Some(_)) => {
                    // Process exited, will restart below
                    *guard = None;
                }
                Err(_) => {
                    *guard = None;
                }
            }
        }

        let binary = Self::find_binary()?;
        logging!(
            info,
            Type::Core,
            "Starting FlowCollect client: {:?} -c {}",
            binary,
            config_path
        );

        let mut cmd = Command::new(&binary);
        cmd.args(["-c", config_path])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        // Windows: hide the console window for the sidecar process
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }

        let child = cmd
            .spawn()
            .with_context(|| format!("Failed to spawn FlowCollect client: {:?}", binary))?;

        logging!(
            info,
            Type::Core,
            "FlowCollect client started (PID: {})",
            child.id()
        );
        *guard = Some(child);
        Ok(())
    }

    /// Stop the FlowCollect client gracefully. Sends SIGTERM (Unix) / TerminateProcess (Windows).
    /// Falls back to force kill after a timeout.
    pub fn stop(&self) {
        let mut guard = self.child.lock().unwrap();
        if let Some(ref mut child) = *guard {
            let pid = child.id();
            logging!(
                info,
                Type::Core,
                "Stopping FlowCollect client (PID: {})...",
                pid
            );

            // Try graceful termination first
            #[cfg(unix)]
            {
                // Send SIGTERM for graceful shutdown (child.kill() sends SIGKILL)
                unsafe {
                    tauri_plugin_clash_verge_sysinfo::libc::kill(
                        pid as i32,
                        tauri_plugin_clash_verge_sysinfo::libc::SIGTERM,
                    );
                }
            }
            #[cfg(windows)]
            {
                let _ = child.kill();
            }

            // Wait up to 2 seconds for graceful exit
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        logging!(
                            info,
                            Type::Core,
                            "FlowCollect client exited with status: {}",
                            status
                        );
                        break;
                    }
                    Ok(None) => {
                        if Instant::now() >= deadline {
                            logging!(
                                warn,
                                Type::Core,
                                "FlowCollect client did not exit in time, force killing"
                            );
                            let _ = child.kill();
                            let _ = child.wait();
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    Err(e) => {
                        logging!(
                            error,
                            Type::Core,
                            "Error checking FlowCollect client status: {}",
                            e
                        );
                        break;
                    }
                }
            }
        }
        *guard = None;
    }

    /// Check if the FlowCollect client process is currently running.
    pub fn is_running(&self) -> bool {
        let mut guard = self.child.lock().unwrap();
        if let Some(ref mut child) = *guard {
            match child.try_wait() {
                Ok(None) => true,
                _ => {
                    *guard = None;
                    false
                }
            }
        } else {
            false
        }
    }
}

singleton!(FlowCollectManager, FLOW_COLLECT_MANAGER);

/// Start the FlowCollect client sidecar.
/// `config_path` should be the absolute path to the Clash config.yaml.
pub fn start_flow_collect(config_path: &str) {
    if let Err(e) = FlowCollectManager::global().start(config_path) {
        logging!(
            warn,
            Type::Core,
            "Failed to start FlowCollect client: {}",
            e
        );
    }
}

/// Stop the FlowCollect client sidecar.
pub fn stop_flow_collect() {
    FlowCollectManager::global().stop();
}

/// Check if the FlowCollect client is running.
pub fn is_flow_collect_running() -> bool {
    FlowCollectManager::global().is_running()
}
