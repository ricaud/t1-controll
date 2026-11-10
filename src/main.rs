#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use t1_control::app::{demo_apply, T1App};
use t1_control::config::{default_config_path, ConfigStore};
use t1_control::presets::demo_config;

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut demo = false;
    let mut config_path: Option<PathBuf> = None;
    let mut screenshot_path: Option<PathBuf> = None;
    let mut page = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--demo" => demo = true,
            "--config" => {
                index += 1;
                if let Some(path) = args.get(index) {
                    config_path = Some(PathBuf::from(path));
                }
            }
            "--screenshot" => {
                index += 1;
                if let Some(path) = args.get(index) {
                    screenshot_path = Some(PathBuf::from(path));
                }
            }
            "--page" => {
                index += 1;
                page = args.get(index).map(|value| match value.as_str() {
                    "lighting" => t1_control::model::Page::Lighting,
                    "cooling" => t1_control::model::Page::Cooling,
                    "devices" => t1_control::model::Page::Devices,
                    "profiles" => t1_control::model::Page::Profiles,
                    _ => t1_control::model::Page::Build,
                });
            }
            "--help" | "-h" => {
                println!("T1 Control");
                println!("  --demo          use simulated RGB and fan hardware");
                println!("  --config PATH   use a specific config file");
                println!("  --page NAME     build | lighting | cooling | devices | profiles");
                println!("  --screenshot PATH  capture the window and exit");
                return Ok(());
            }
            other => eprintln!("unknown argument: {other}"),
        }
        index += 1;
    }

    let path = config_path.unwrap_or_else(|| {
        if demo {
            std::env::temp_dir()
                .join("t1-control-demo")
                .join("config.json")
        } else {
            default_config_path()
        }
    });
    let store = ConfigStore::new(path.clone());
    let mut config = if demo && !path.exists() {
        demo_config()
    } else {
        store.load()
    };
    if demo {
        demo_apply(&mut config);
    }
    if let Some(page) = page {
        config.ui.page = page;
    }

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 780.0])
            .with_min_inner_size([900.0, 600.0])
            .with_title("T1 Control"),
        ..Default::default()
    };
    eframe::run_native(
        "T1 Control",
        options,
        Box::new(move |cc| {
            Ok(Box::new(T1App::new(cc, config, store, screenshot_path)) as Box<dyn eframe::App>)
        }),
    )
}
