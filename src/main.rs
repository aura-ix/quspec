mod utils;
mod audio;
mod spectrogram;
mod app;

use eframe::egui;
use std::env;
use std::path::PathBuf;
use utils::collect_audio_files;
use app::QuspecApp;

fn main() -> Result<(), eframe::Error> {
    // get input path from args
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        let exe = env::current_exe()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "quspec".to_string());
        eprintln!("usage: {exe} <path_to_audio_file_or_directory>");
        std::process::exit(1);
    }
    let input_path = PathBuf::from(&args[1]);
    let playlist = collect_audio_files(&input_path);
    if playlist.is_empty() {
        eprintln!("error: no supported audio files found at {:?}", input_path);
        std::process::exit(1);
    }

    // run gui
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([800.0, 500.0]),
        ..Default::default()
    };

    eframe::run_native(
        "quspec",
        options,
        Box::new(|cc| Ok(Box::new(QuspecApp::new(cc, playlist)))),
    )
}