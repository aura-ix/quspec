use crate::audio::{decode_audio, DecodedAudio};
use crate::spectrogram::{generate_spectrogram, sox_palette, SpectrogramData, SpectrogramUpdate};
use crate::utils::format_time;
use eframe::egui;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

pub struct PreloadedTrack {
    pub audio: Option<Result<DecodedAudio, String>>,
    pub receiver: Option<mpsc::Receiver<Result<DecodedAudio, String>>>,
    pub progress: Arc<AtomicU64>,
    pub total_bytes: u64,
}

pub struct QuspecApp {
    pub playlist: Vec<PathBuf>,
    pub current_index: usize,
    pub file_path: PathBuf,
    pub cache: std::collections::HashMap<PathBuf, PreloadedTrack>,
    pub spectrogram: Option<SpectrogramData>,
    pub spectrogram_receiver: Option<mpsc::Receiver<SpectrogramUpdate>>,
    pub cancel_trigger: Option<Arc<AtomicBool>>,
    pub generating_size: Option<[usize; 2]>,
    pub texture: Option<egui::TextureHandle>,
    pub fft_size: usize,
    pub current_channel: usize,
    pub zoom_minutes: Option<f64>,
    pub view_start_sec: f64,
}

impl QuspecApp {
    pub fn new(_cc: &eframe::CreationContext<'_>, playlist: Vec<PathBuf>) -> Self {
        let mut app = Self {
            playlist,
            current_index: 0,
            file_path: PathBuf::new(),
            cache: std::collections::HashMap::new(),
            spectrogram: None,
            spectrogram_receiver: None,
            cancel_trigger: None,
            generating_size: None,
            texture: None,
            fft_size: 2048,
            current_channel: 0,
            zoom_minutes: None,
            view_start_sec: 0.0,
        };
        if !app.playlist.is_empty() {
            app.load_track(0);
        }
        app
    }

    pub fn load_track(&mut self, index: usize) {
        if index >= self.playlist.len() {
            return;
        }

        // cancel active generation
        if let Some(cancel) = &self.cancel_trigger {
            cancel.store(true, Ordering::Relaxed);
        }
        self.cancel_trigger = None;
        self.spectrogram_receiver = None;
        self.generating_size = None;

        self.current_index = index;
        self.file_path = self.playlist[index].clone();
        self.spectrogram = None;
        self.texture = None;
        self.zoom_minutes = None;
        self.view_start_sec = 0.0;
        self.current_channel = 0;

        self.update_cache();
    }

    pub fn update_cache(&mut self) {
        if self.playlist.is_empty() {
            return;
        }


        let mut active_paths = std::collections::HashSet::new();
        active_paths.insert(self.playlist[self.current_index].clone());

        if self.playlist.len() > 1 {
            let next_idx = (self.current_index + 1) % self.playlist.len();
            let prev_idx = (self.current_index + self.playlist.len() - 1) % self.playlist.len();
            active_paths.insert(self.playlist[next_idx].clone());
            active_paths.insert(self.playlist[prev_idx].clone());
        }


        self.cache.retain(|path, _| active_paths.contains(path));


        for path in active_paths {
            if !self.cache.contains_key(&path) {
                let total_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                let progress = Arc::new(AtomicU64::new(0));
                let progress_clone = progress.clone();
                let (tx, rx) = mpsc::channel();
                let path_clone = path.clone();

                thread::spawn(move || {
                    let res = decode_audio(&path_clone, progress_clone);
                    let _ = tx.send(res);
                });

                self.cache.insert(
                    path,
                    PreloadedTrack {
                        audio: None,
                        receiver: Some(rx),
                        progress,
                        total_bytes,
                    },
                );
            }
        }
    }

    pub fn current_audio(&self) -> Option<&DecodedAudio> {
        self.cache
            .get(&self.file_path)
            .and_then(|t| t.audio.as_ref())
            .and_then(|res| res.as_ref().ok())
    }

    pub fn current_error(&self) -> Option<&String> {
        self.cache
            .get(&self.file_path)
            .and_then(|t| t.audio.as_ref())
            .and_then(|res| res.as_ref().err())
    }

    pub fn get_current_samples(&self) -> Vec<f32> {
        let audio = self.current_audio().unwrap();
        let channel_samples: Vec<f32> = audio.samples
            .chunks_exact(audio.channels)
            .map(|chunk| chunk[self.current_channel])
            .collect();

        if let Some(zoom_min) = self.zoom_minutes {
            let total_samples = channel_samples.len();
            let sample_rate = audio.sample_rate as f64;
            let view_width_sec = zoom_min * 60.0;

            let mut start_sample = ((self.view_start_sec * sample_rate).round() as usize).min(total_samples);
            let mut end_sample = (((self.view_start_sec + view_width_sec) * sample_rate).round() as usize).min(total_samples);

            if end_sample - start_sample < self.fft_size {
                if total_samples >= self.fft_size {
                    if end_sample == total_samples {
                        start_sample = total_samples - self.fft_size;
                    } else {
                        end_sample = start_sample + self.fft_size;
                    }
                }
            }
            channel_samples[start_sample..end_sample].to_vec()
        } else {
            channel_samples
        }
    }

    pub fn spawn_generate_spectrogram(&mut self, ctx: &egui::Context, width: usize, height: usize) {
        // cancel current generation
        if let Some(cancel) = &self.cancel_trigger {
            cancel.store(true, Ordering::Relaxed);
        }

        self.generating_size = Some([width, height]);

        let samples = self.get_current_samples();
        let fft_size = self.fft_size;

        let cancel_flag = Arc::new(AtomicBool::new(false));
        self.cancel_trigger = Some(cancel_flag.clone());

        let (tx, rx) = mpsc::channel();
        self.spectrogram_receiver = Some(rx);

        let ctx_clone = ctx.clone();
        thread::spawn(move || {
            generate_spectrogram(
                &samples,
                fft_size,
                width,
                height,
                cancel_flag,
                tx,
                ctx_clone,
            );
        });
    }
}

impl eframe::App for QuspecApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if ui.input(|i| i.key_pressed(egui::Key::Q)) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }

        if ui.input(|i| i.key_pressed(egui::Key::A)) {
            if self.playlist.len() > 1 {
                let prev_idx = if self.current_index == 0 {
                    self.playlist.len() - 1
                } else {
                    self.current_index - 1
                };
                self.load_track(prev_idx);
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::D)) {
            if self.playlist.len() > 1 {
                let next_idx = (self.current_index + 1) % self.playlist.len();
                self.load_track(next_idx);
            }
        }

        // poll background track loading
        for track in self.cache.values_mut() {
            if let Some(rx) = &track.receiver {
                if let Ok(res) = rx.try_recv() {
                    track.audio = Some(res);
                    track.receiver = None;
                }
            }
        }

        // poll background spectrogram receiver
        if let Some(rx) = &self.spectrogram_receiver {
            let mut should_clear = false;
            while let Ok(update) = rx.try_recv() {
                match update {
                    SpectrogramUpdate::Started {
                        width,
                        height,
                    } => {
                        self.spectrogram = Some(SpectrogramData {
                            width,
                            height,
                            pixels: vec![egui::Color32::BLACK; width * height],
                        });
                        self.texture = None;
                    }
                    SpectrogramUpdate::Chunk { x_start, pixels } => {
                        if let Some(spec) = &mut self.spectrogram {
                            let num_cols = pixels.len() / spec.height;
                            for col in 0..num_cols {
                                let x = x_start + col;
                                if x < spec.width {
                                    for y in 0..spec.height {
                                        let inverted_y = spec.height - 1 - y;
                                        spec.pixels[inverted_y * spec.width + x] = pixels[col * spec.height + y];
                                    }
                                }
                            }
                            self.texture = None;
                        }
                    }
                    SpectrogramUpdate::Finished(new_data) => {
                        self.spectrogram = Some(new_data);
                        self.texture = None;
                        should_clear = true;
                    }
                    SpectrogramUpdate::Failed(e) => {
                        if e != "Cancelled" {
                            eprintln!("Error generating spectrogram: {}", e);
                        }
                        should_clear = true;
                    }
                }
            }
            if should_clear {
                self.spectrogram_receiver = None;
                self.cancel_trigger = None;
                self.generating_size = None;
            }
        }

        let cur_audio = self.current_audio();
        let cur_err = self.current_error();

        // draw loading or error screen
        if cur_audio.is_none() {
            ui.centered_and_justified(|ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() / 2.0 - 50.0);
                    let filename = self.file_path.file_name().and_then(|f| f.to_str()).unwrap_or("audio file");
                    if let Some(err_msg) = cur_err {
                        ui.heading(
                            egui::RichText::new(format!("Error loading {}", filename))
                                .color(egui::Color32::LIGHT_RED)
                        );
                        ui.add_space(10.0);
                        ui.label(
                            egui::RichText::new(err_msg)
                                .color(egui::Color32::LIGHT_RED)
                        );
                    } else if let Some(track) = self.cache.get(&self.file_path) {
                        ui.heading(
                            egui::RichText::new(format!("Loading {}...", filename))
                                .color(egui::Color32::WHITE)
                        );
                        ui.add_space(10.0);
                        if track.total_bytes > 0 {
                            let progress = track.progress.load(Ordering::Relaxed);
                            let fraction = ((progress as f64 / track.total_bytes as f64) as f32).clamp(0.0, 1.0);
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(300.0, 6.0), egui::Sense::hover());
                            ui.painter().rect_filled(
                                rect,
                                0.0,
                                egui::Color32::from_black_alpha(120),
                            );
                            if fraction > 0.0 {
                                let filled_width = rect.width() * fraction;
                                let filled_rect = egui::Rect::from_min_max(
                                    rect.min,
                                    egui::pos2(rect.min.x + filled_width, rect.max.y),
                                );
                                ui.painter().rect_filled(
                                    filled_rect,
                                    0.0,
                                    egui::Color32::WHITE,
                                );
                            }
                            ui.painter().rect_stroke(
                                rect,
                                0.0,
                                egui::Stroke::new(1.0, egui::Color32::from_gray(100)),
                                egui::StrokeKind::Inside,
                            );
                        } else {
                            ui.add(egui::Spinner::new().size(30.0));
                        }
                    } else {
                        ui.add(egui::Spinner::new().size(30.0));
                    }
                });
            });
            if cur_err.is_none() {
                ui.ctx().request_repaint();
            }
            return;
        }

        let audio = self.current_audio().unwrap();
        let total_duration = audio.samples.len() as f64 / (audio.channels as f64 * audio.sample_rate as f64);

        // initialize zoom
        if self.spectrogram.is_none() && self.generating_size.is_none() {
            if total_duration > 20.0 * 60.0 {
                self.zoom_minutes = Some(10.0);
            }
        }

        let mut changed = false;

        // zoom in
        if ui.input(|i| i.key_pressed(egui::Key::K) || i.key_pressed(egui::Key::ArrowUp) || i.key_pressed(egui::Key::Equals) || i.key_pressed(egui::Key::Plus)) {
            let current_n = self.zoom_minutes.unwrap_or(total_duration / 60.0);
            let next_n = current_n / 2.0;
            if next_n >= 1.0 / 60.0 {
                let next_start = (self.view_start_sec + current_n * 30.0 - next_n * 30.0).clamp(0.0, (total_duration - next_n * 60.0).max(0.0));
                if self.zoom_minutes != Some(next_n) || self.view_start_sec != next_start {
                    self.zoom_minutes = Some(next_n);
                    self.view_start_sec = next_start;
                    changed = true;
                }
            }
        }

        // zoom out
        if ui.input(|i| i.key_pressed(egui::Key::J) || i.key_pressed(egui::Key::ArrowDown) || i.key_pressed(egui::Key::Minus)) {
            if let Some(current_n) = self.zoom_minutes {
                let next_n = current_n * 2.0;
                let (next_zoom, next_start) = if next_n * 60.0 >= total_duration {
                    (None, 0.0)
                } else {
                    let next_start = (self.view_start_sec + current_n * 30.0 - next_n * 30.0).clamp(0.0, (total_duration - next_n * 60.0).max(0.0));
                    (Some(next_n), next_start)
                };
                if self.zoom_minutes != next_zoom || self.view_start_sec != next_start {
                    self.zoom_minutes = next_zoom;
                    self.view_start_sec = next_start;
                    changed = true;
                }
            }
        }

        // pan left
        if ui.input(|i| i.key_pressed(egui::Key::H) || i.key_pressed(egui::Key::ArrowLeft)) {
            if let Some(current_n) = self.zoom_minutes {
                let next_start = (self.view_start_sec - current_n * 60.0).clamp(0.0, (total_duration - current_n * 60.0).max(0.0));
                if next_start != self.view_start_sec {
                    self.view_start_sec = next_start;
                    changed = true;
                }
            }
        }

        // pan right
        if ui.input(|i| i.key_pressed(egui::Key::L) || i.key_pressed(egui::Key::ArrowRight)) {
            if let Some(current_n) = self.zoom_minutes {
                let next_start = (self.view_start_sec + current_n * 60.0).clamp(0.0, (total_duration - current_n * 60.0).max(0.0));
                if next_start != self.view_start_sec {
                    self.view_start_sec = next_start;
                    changed = true;
                }
            }
        }

        // toggle maximized
        if ui.input(|i| i.key_pressed(egui::Key::F)) {
            let is_maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!is_maximized));
        }

        // switch channel
        if ui.input(|i| i.key_pressed(egui::Key::C)) {
            let channels = self.current_audio().unwrap().channels;
            self.current_channel = (self.current_channel + 1) % channels;
            changed = true;
        }

        // request screenshot
        if ui.input(|i| i.key_pressed(egui::Key::S)) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }

        // copy screenshot to clipboard
        ui.ctx().input(|i| {
            for event in &i.events {
                if let egui::Event::Screenshot { image, .. } = event {
                    let [width, height] = image.size;
                    let rgba_bytes: Vec<u8> = image.pixels
                        .iter()
                        .flat_map(|p| [p.r(), p.g(), p.b(), p.a()])
                        .collect();

                    match arboard::Clipboard::new() {
                        Ok(mut clipboard) => {
                            let image_data = arboard::ImageData {
                                width,
                                height,
                                bytes: std::borrow::Cow::from(rgba_bytes),
                            };
                            if let Err(e) = clipboard.set_image(image_data) {
                                eprintln!("Failed to copy image to clipboard: {}", e);
                            } else {
                                println!("Full graph screenshot copied to clipboard ({}x{}).", width, height);
                            }
                        }
                        Err(e) => {
                            eprintln!("Failed to initialize clipboard: {}", e);
                        }
                    }
                }
            }
        });

        if ui.input(|i| i.key_pressed(egui::Key::W)) {
            let shift = ui.input(|i| i.modifiers.shift);
            if shift {
                if self.fft_size > 256 {
                    self.fft_size /= 2;
                    changed = true;
                }
            } else {
                if self.fft_size < 16384 {
                    self.fft_size *= 2;
                    changed = true;
                }
            }
        }

        if changed {
            let width = self.spectrogram.as_ref().map(|s| s.width).unwrap_or(800);
            let height = self.spectrogram.as_ref().map(|s| s.height).unwrap_or(400);
            println!("Recomputing spectrogram with FFT size: {}, channel: {}", self.fft_size, self.current_channel + 1);
            self.spawn_generate_spectrogram(ui.ctx(), width, height);
        }

        ui.vertical(|ui| {
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                ui.add_space(65.0);

                let header_width = ui.available_width() - 75.0;
                ui.allocate_ui(egui::vec2(header_width, 36.0), |ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            let right_width = 100.0;
                            let left_width = (ui.available_width() - right_width - ui.spacing().item_spacing.x).max(0.0);

                            let track_prefix = format!("[{}/{}] ", self.current_index + 1, self.playlist.len());
                            let display_path = format!("{}{}", track_prefix, self.file_path.to_string_lossy());
                            ui.allocate_ui_with_layout(
                                egui::vec2(left_width, 16.0),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(display_path)
                                                .font(egui::FontId::proportional(12.0))
                                                .strong()
                                                .color(egui::Color32::WHITE)
                                        )
                                        .truncate()
                                    );
                                }
                            );

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let ver_str = format!("quspec v{}", env!("CARGO_PKG_VERSION"));
                                ui.label(
                                    egui::RichText::new(ver_str)
                                        .font(egui::FontId::proportional(10.0))
                                        .color(egui::Color32::from_gray(180))
                                );
                            });
                        });

                        let mut info_parts = Vec::new();
                        let audio = self.current_audio().unwrap();
                        info_parts.push(format!("{:.1} kHz", audio.sample_rate as f64 / 1000.0));
                        if let Some(bits) = audio.bits_per_sample {
                            info_parts.push(format!("{}-bit", bits));
                        }
                        info_parts.push(format!("channel {}/{}", self.current_channel + 1, audio.channels));
                        let info_str = info_parts.join(", ");

                        ui.horizontal(|ui| {
                            let right_width = 100.0;
                            let left_width = (ui.available_width() - right_width - ui.spacing().item_spacing.x).max(0.0);

                            ui.allocate_ui_with_layout(
                                egui::vec2(left_width, 14.0),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(info_str)
                                                .font(egui::FontId::proportional(10.0))
                                                .color(egui::Color32::from_gray(180))
                                        )
                                        .truncate()
                                    );
                                }
                            );

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let win_str = format!("window: {}", self.fft_size);
                                ui.label(
                                    egui::RichText::new(win_str)
                                        .font(egui::FontId::proportional(10.0))
                                        .color(egui::Color32::from_gray(180))
                                    );
                            });
                        });
                    });
                });
            });

            ui.add_space(2.0);

            let remaining_size = egui::vec2(
                ui.available_width().max(200.0),
                ui.available_height().max(100.0),
            );
            let (rect, _) = ui.allocate_exact_size(remaining_size, egui::Sense::hover());

            let left_ruler_width = 65.0;
            let right_ruler_width = 75.0;

            let top_padding = 4.0f32;
            let bottom_padding = 42.0f32;

            let image_width = ((rect.width() - left_ruler_width - right_ruler_width).max(10.0)) as usize;
            let image_height = ((rect.height() - top_padding - bottom_padding).max(10.0)) as usize;

            // resize spectrogram if layout changed
            let current_w = self.spectrogram.as_ref().map(|s| s.width).unwrap_or(0);
            let current_h = self.spectrogram.as_ref().map(|s| s.height).unwrap_or(0);
            let target_size = [image_width, image_height];
            if (current_w != image_width || current_h != image_height) && self.generating_size != Some(target_size) {
                self.spawn_generate_spectrogram(ui.ctx(), image_width, image_height);
            }

            let image_rect = egui::Rect::from_min_size(
                egui::pos2(rect.min.x + left_ruler_width, rect.min.y + top_padding),
                egui::vec2(image_width as f32, image_height as f32),
            );

            if let Some(spectrogram) = &self.spectrogram {
                // load hardware texture if needed
                let texture = self.texture.get_or_insert_with(|| {
                    let start = std::time::Instant::now();
                    let tex = ui.ctx().load_texture(
                        "spectrogram_render",
                        egui::ColorImage {
                            size: [spectrogram.width, spectrogram.height],
                            pixels: spectrogram.pixels.clone(),
                            source_size: egui::vec2(spectrogram.width as f32, spectrogram.height as f32),
                        },
                        Default::default(),
                    );
                    println!("Making texture took: {:?}", start.elapsed());
                    tex
                });

                ui.painter().image(
                    texture.id(),
                    image_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            } else {
                ui.painter().rect_filled(image_rect, 0.0, egui::Color32::BLACK);
            }

            ui.painter().rect_stroke(
                image_rect,
                0.0,
                egui::Stroke::new(1.0, egui::Color32::from_gray(100)),
                egui::StrokeKind::Inside,
            );

            // draw generating overlay
            if self.spectrogram_receiver.is_some() && self.spectrogram.is_none() {
                ui.painter().rect_filled(
                    image_rect,
                    0.0,
                    egui::Color32::from_black_alpha(150),
                );
                let center_x = image_rect.center().x;
                let center_y = image_rect.center().y;
                ui.painter().text(
                    egui::pos2(center_x, center_y - 10.0),
                    egui::Align2::CENTER_CENTER,
                    "Generating spectrogram...",
                    egui::FontId::proportional(14.0),
                    egui::Color32::WHITE,
                );
            }

            let painter = ui.painter();
            let text_color = egui::Color32::from_gray(200);
            let line_color = egui::Color32::from_gray(80);
            let font_id = egui::FontId::proportional(10.0);

            // --- frequency ruler (left) ---
            let audio = self.current_audio().unwrap();
            let max_freq = audio.sample_rate as f32 / 2.0;

            let min_spacing = 30.0f32;
            let max_freq_ticks = ((image_height as f32 / min_spacing) as usize).max(2);
            let ideal_freq_step = max_freq / (max_freq_ticks as f32);

            let nice_freq_steps = [100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0, 20000.0, 50000.0];
            let tick_step = nice_freq_steps
                .into_iter()
                .find(|&step| step >= ideal_freq_step)
                .unwrap_or(20000.0);

            let mut freq_ticks = Vec::new();
            let mut current_freq = 0.0f32;
            while current_freq < max_freq {
                if max_freq - current_freq < 0.7 * tick_step {
                    break;
                }
                freq_ticks.push(current_freq);
                current_freq += tick_step;
            }
            // add nyquist tick
            freq_ticks.push(max_freq);

            for freq in freq_ticks {
                let normalized = freq / max_freq;
                let y = image_rect.max.y - normalized * image_rect.height();

                let tick_start_x = image_rect.min.x - 5.0;
                painter.line_segment(
                    [egui::pos2(tick_start_x, y), egui::pos2(image_rect.min.x, y)],
                    egui::Stroke::new(1.0, line_color),
                );

                let label = format!("{:.2} kHz", freq / 1000.0);
                painter.text(
                    egui::pos2(tick_start_x - 4.0, y),
                    egui::Align2::RIGHT_CENTER,
                    label,
                    font_id.clone(),
                    text_color,
                );
            }

            // --- decibel ruler (right) ---
            let bar_left = image_rect.max.x + 8.0;
            let bar_width = 12.0;
            let bar_height = image_rect.height();

            // draw db gradient
            for y_offset in 0..(bar_height as usize) {
                let normalized = 1.0 - (y_offset as f32 / bar_height);
                let color = sox_palette(normalized);
                let y = image_rect.min.y + y_offset as f32;
                painter.line_segment(
                    [egui::pos2(bar_left, y), egui::pos2(bar_left + bar_width, y)],
                    egui::Stroke::new(1.0, color),
                );
            }

            painter.rect_stroke(
                egui::Rect::from_min_size(
                    egui::pos2(bar_left, image_rect.min.y),
                    egui::vec2(bar_width, bar_height)
                ),
                0.0,
                egui::Stroke::new(1.0, egui::Color32::from_gray(100)),
                egui::StrokeKind::Inside,
            );

            let min_db = -120.0f32;
            let max_db = 0.0f32;

            let max_db_ticks = ((image_height as f32 / min_spacing) as usize).max(2);
            let ideal_db_step = 120.0 / (max_db_ticks as f32);

            let nice_db_steps = [5.0, 10.0, 20.0, 30.0, 40.0, 60.0, 120.0];
            let db_step = nice_db_steps
                .into_iter()
                .find(|&step| step >= ideal_db_step)
                .unwrap_or(120.0);

            let mut db_ticks = Vec::new();
            let mut current_db = min_db;
            while current_db <= max_db + 0.01 {
                db_ticks.push(current_db.round() as i32);
                current_db += db_step;
            }

            for db in db_ticks {
                let normalized = (db as f32 - min_db) / (max_db - min_db);
                let y = image_rect.max.y - normalized * image_rect.height();

                let tick_start_x = bar_left + bar_width;
                painter.line_segment(
                    [egui::pos2(tick_start_x, y), egui::pos2(tick_start_x + 4.0, y)],
                    egui::Stroke::new(1.0, line_color),
                );

                let label = format!("{} dB", db);
                painter.text(
                    egui::pos2(tick_start_x + 8.0, y),
                    egui::Align2::LEFT_CENTER,
                    label,
                    font_id.clone(),
                    text_color,
                );
            }

            // --- time ruler (bottom) ---
            let duration = self.zoom_minutes.map(|n| n * 60.0).unwrap_or(total_duration);
            let min_time_spacing = 60.0f32;
            let max_time_ticks = ((image_width as f32 / min_time_spacing) as usize).max(2);
            let ideal_time_step = duration / (max_time_ticks as f64);

            let nice_time_steps = [
                0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0,
                60.0, 120.0, 300.0, 600.0, 1200.0, 1800.0, 3600.0
            ];
            let time_step = nice_time_steps
                .into_iter()
                .find(|&step| step >= ideal_time_step)
                .unwrap_or(60.0);

            let mut time_ticks = Vec::new();
            let mut current_time = 0.0f64;
            while current_time < duration {
                if duration - current_time < 0.7 * time_step {
                    break;
                }
                time_ticks.push(current_time);
                current_time += time_step;
            }
            time_ticks.push(duration);

            for time in time_ticks {
                let normalized = time / duration;
                let x = image_rect.min.x + normalized as f32 * image_rect.width();

                painter.line_segment(
                    [egui::pos2(x, image_rect.max.y), egui::pos2(x, image_rect.max.y + 4.0)],
                    egui::Stroke::new(1.0, line_color),
                );

                let label = format_time(self.view_start_sec + time);
                painter.text(
                    egui::pos2(x, image_rect.max.y + 6.0),
                    egui::Align2::CENTER_TOP,
                    label,
                    font_id.clone(),
                    text_color,
                );
            }

            // --- zoom bar ---
            let zoom_bar_height = 6.0f32;
            let zoom_bar_y = image_rect.max.y + 24.0;
            let zoom_bar_rect = egui::Rect::from_min_max(
                egui::pos2(image_rect.left(), zoom_bar_y),
                egui::pos2(image_rect.right(), zoom_bar_y + zoom_bar_height),
            );

            painter.rect_filled(
                zoom_bar_rect,
                0.0,
                egui::Color32::from_black_alpha(120),
            );

            let zoom_min = self.zoom_minutes.unwrap_or(total_duration / 60.0);
            let view_width_sec = zoom_min * 60.0;
            let start_ratio = (self.view_start_sec / total_duration).clamp(0.0, 1.0);
            let end_ratio = ((self.view_start_sec + view_width_sec) / total_duration).clamp(0.0, 1.0);

            let indicator_min_x = image_rect.left() + start_ratio as f32 * image_rect.width();
            let indicator_max_x = image_rect.left() + end_ratio as f32 * image_rect.width();

            let indicator_rect = egui::Rect::from_min_max(
                egui::pos2(indicator_min_x, zoom_bar_y),
                egui::pos2(indicator_max_x, zoom_bar_y + zoom_bar_height),
            );

            painter.rect_filled(
                indicator_rect,
                0.0,
                egui::Color32::WHITE,
            );

            painter.rect_stroke(
                zoom_bar_rect,
                0.0,
                egui::Stroke::new(1.0, egui::Color32::from_gray(100)),
                egui::StrokeKind::Inside,
            );

            painter.text(
                egui::pos2(image_rect.left() - 8.0, zoom_bar_y + zoom_bar_height / 2.0),
                egui::Align2::RIGHT_CENTER,
                "0:00",
                font_id.clone(),
                text_color,
            );

            let total_duration_str = format_time(total_duration);
            painter.text(
                egui::pos2(image_rect.right() + 8.0, zoom_bar_y + zoom_bar_height / 2.0),
                egui::Align2::LEFT_CENTER,
                total_duration_str,
                font_id.clone(),
                text_color,
            );
        });
    }
}
