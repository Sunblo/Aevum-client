#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod launcher;
mod paint;
mod starfield;
mod theme;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let viewport = egui::ViewportBuilder::default()
        .with_title("Aevum Launcher")
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([980.0, 640.0])
        .with_decorations(false);

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Aevum Launcher",
        options,
        Box::new(|cc| Ok(Box::new(app::AevumApp::new(cc)))),
    )
}

#[cfg(test)]
mod launch_test {
    use super::launcher;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    #[test]
    #[ignore = "requires network + full download (~400 MB assets); run with cargo test -- --ignored"]
    fn end_to_end_launch_1_16_5() {
        let report = Arc::new(Mutex::new(launcher::LaunchReport::default()));
        let profile = launcher::Profile {
            version_id: "1.16.5".to_string(),
            username: "NovaPilot".to_string(),
            ram_mb: 1024,
        };
        let r2 = report.clone();
        let handle = std::thread::spawn(move || launcher::run_launch(profile, r2));
        let start = Instant::now();
        loop {
            let snap = report.lock().unwrap().clone();
            let elapsed = start.elapsed().as_secs();
            if snap.phase == launcher::Phase::Exited || snap.phase == launcher::Phase::Error {
                println!(
                    "[{}s] FINAL phase={:?} error={:?} exit={:?}",
                    elapsed, snap.phase, snap.error, snap.exit_code
                );
                break;
            }
            if snap.phase == launcher::Phase::Running {
                println!("[{}s] RUNNING pid={:?} files={}/{}", elapsed, snap.pid, snap.files_done, snap.files_total);
                std::thread::sleep(Duration::from_secs(25));
                if let Some(pid) = snap.pid {
                    launcher::kill_pid(pid);
                    println!("[{}s] killed {}", elapsed + 25, pid);
                }
                let _ = handle.join();
                break;
            }
            if elapsed > 900 {
                println!("[{}s] TIMEOUT phase={:?} msg={:?}", elapsed, snap.phase, snap.message);
                break;
            }
            if elapsed % 5 == 0 {
                println!(
                    "[{}s] phase={:?} msg={:?} bytes={}/{} files={}/{}",
                    elapsed, snap.phase, snap.message, snap.bytes_done, snap.bytes_total, snap.files_done, snap.files_total
                );
            }
            std::thread::sleep(Duration::from_millis(1000));
        }
    }
}
