use eframe::egui;
use num_complex::Complex;
use rustfft::FftPlanner;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::mpsc;

/// spectrogram metadata and pixel colors
pub struct SpectrogramData {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<egui::Color32>,
}

pub enum SpectrogramUpdate {
    Started {
        width: usize,
        height: usize,
    },
    Chunk {
        x_start: usize,
        pixels: Vec<egui::Color32>,
    },
    Finished(SpectrogramData),
    Failed(String),
}

// sox palette mapping
pub fn sox_palette(level: f32) -> egui::Color32 {
    let level = level as f64;
    let r = if level < 0.13 { 0.0 } else if level < 0.73 { ((level - 0.13) / 0.60 * std::f64::consts::FRAC_PI_2).sin() } else { 1.0 };
    let g = if level < 0.60 { 0.0 } else if level < 0.91 { ((level - 0.60) / 0.31 * std::f64::consts::FRAC_PI_2).sin() } else { 1.0 };
    let b = if level < 0.60 { 0.5 * (level / 0.60 * std::f64::consts::PI).sin() } else if level < 0.78 { 0.0 } else { (level - 0.78) / 0.22 };

    egui::Color32::from_rgb(
        (r * 255.0 + 0.5) as u8,
        (g * 255.0 + 0.5) as u8,
        (b * 255.0 + 0.5) as u8,
    )
}

/// generate spectrogram pixels via fft
pub fn generate_spectrogram(
    channel_samples: &[f32],
    fft_size: usize,
    target_width: usize,
    target_height: usize,
    cancel_flag: Arc<AtomicBool>,
    tx: mpsc::Sender<SpectrogramUpdate>,
    ctx: egui::Context,
) {
    let start_fft = std::time::Instant::now();
    let num_bins = fft_size / 2;

    let total_samples = channel_samples.len();
    if total_samples < fft_size {
        let _ = tx.send(SpectrogramUpdate::Failed("Audio file is too short to generate a spectrogram.".into()));
        ctx.request_repaint();
        return;
    }

    let _ = tx.send(SpectrogramUpdate::Started {
        width: target_width,
        height: target_height,
    });
    ctx.request_repaint();

    let samples_per_col = (total_samples as f64 / target_width as f64) as usize;
    let width = target_width;

    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(fft_size);

    let mut pixels = vec![egui::Color32::BLACK; width * target_height];
    let min_db = -120.0f32;
    let max_db = 0.0f32;

    let mut chunk_cols = Vec::new();
    let mut chunk_start_x = 0;
    let chunk_size = 16;

    // precompute hann window
    let hann_window: Vec<f32> = (0..fft_size)
        .map(|i| 0.5 * (1.0 - ((2.0 * std::f32::consts::PI * i as f32) / (fft_size as f32 - 1.0)).cos()))
        .collect();

    let mut fft_buffer = vec![Complex::default(); fft_size];

    let process_chunk = |chunk: &[f32], buf: &mut [Complex<f32>]| {
        for (i, &sample) in chunk.iter().enumerate() {
            buf[i] = Complex::new(sample * hann_window[i], 0.0);
        }
        fft.process(buf);
    };

    let to_db = |magnitude: f32| 20.0 * magnitude.max(1e-7).log10();

    let fill_column = |col: usize, db_values: &[f32], pixels: &mut [egui::Color32], chunk_cols: &mut Vec<egui::Color32>| {
        let mut col_pixels = vec![egui::Color32::BLACK; target_height];
        for y in 0..target_height {
            let bin_idx = if target_height > 1 {
                (y as f64 * (num_bins - 1) as f64 / (target_height - 1) as f64).round() as usize
            } else {
                0
            };
            let db_val = db_values[bin_idx];
            let normalized = ((db_val - min_db) / (max_db - min_db)).clamp(0.0, 1.0);
            let color = sox_palette(normalized);
            col_pixels[y] = color;
            pixels[(target_height - 1 - y) * width + col] = color;
        }
        chunk_cols.extend(col_pixels);
    };

    if samples_per_col < fft_size {
        // short file or wide window: generate columns using overlapping ffts
        let step = if width > 1 {
            (total_samples - fft_size) as f64 / (width - 1) as f64
        } else {
            0.0
        };

        for col in 0..width {
            if cancel_flag.load(Ordering::Relaxed) {
                return;
            }
            let col_start = (col as f64 * step) as usize;
            let chunk = &channel_samples[col_start..col_start + fft_size];

            process_chunk(chunk, &mut fft_buffer);

            let mut db_values = vec![0.0f32; num_bins];
            for bin_idx in 0..num_bins {
                let magnitude = fft_buffer[bin_idx].norm() / fft_size as f32;
                db_values[bin_idx] = to_db(magnitude);
            }

            fill_column(col, &db_values, &mut pixels, &mut chunk_cols);

            if (col + 1) % chunk_size == 0 || col == width - 1 {
                let _ = tx.send(SpectrogramUpdate::Chunk {
                    x_start: chunk_start_x,
                    pixels: std::mem::take(&mut chunk_cols),
                });
                ctx.request_repaint();
                chunk_start_x = col + 1;
            }
        }
    } else {
        // long file: average ffts in partition
        for col in 0..width {
            if cancel_flag.load(Ordering::Relaxed) {
                return;
            }
            let col_start = col * samples_per_col;
            let col_end = col_start + samples_per_col;

            let col_samples = &channel_samples[col_start..col_end];
            let mut averaged_db = vec![0.0f32; num_bins];
            let mut num_ffts = 0;

            for chunk in col_samples.chunks_exact(fft_size) {
                process_chunk(chunk, &mut fft_buffer);
                for bin_idx in 0..num_bins {
                    let magnitude = fft_buffer[bin_idx].norm() / fft_size as f32;
                    averaged_db[bin_idx] += to_db(magnitude);
                }
                num_ffts += 1;
            }

            if num_ffts == 0 && col_samples.len() >= fft_size {
                process_chunk(&col_samples[0..fft_size], &mut fft_buffer);
                for bin_idx in 0..num_bins {
                    let magnitude = fft_buffer[bin_idx].norm() / fft_size as f32;
                    averaged_db[bin_idx] += to_db(magnitude);
                }
                num_ffts = 1;
            }

            if num_ffts > 0 {
                for val in averaged_db.iter_mut() {
                    *val /= num_ffts as f32;
                }
            }

            fill_column(col, &averaged_db, &mut pixels, &mut chunk_cols);

            if (col + 1) % chunk_size == 0 || col == width - 1 {
                let _ = tx.send(SpectrogramUpdate::Chunk {
                    x_start: chunk_start_x,
                    pixels: std::mem::take(&mut chunk_cols),
                });
                ctx.request_repaint();
                chunk_start_x = col + 1;
            }
        }
    }

    println!("FFT and spectrogram processing took: {:?}", start_fft.elapsed());

    let _ = tx.send(SpectrogramUpdate::Finished(SpectrogramData {
        width,
        height: target_height,
        pixels,
    }));
    ctx.request_repaint();
}
